use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::locking;
use crate::metadata::{atomic_write_json, read_json, Paths};
use crate::util::{shell_quote, short_id};

#[derive(Debug, Clone)]
pub struct RepositoryStore {
    paths: Paths,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub version: u32,
    pub id: String,
    pub source_path: String,
    pub base_path: String,
    pub base_commit: String,
    pub object_store: String,
    /// Prebuilt index matching `base_commit` (copied into sessions).
    #[serde(default)]
    pub seed_index: Option<String>,
    /// Cached from source at registration (avoid git spawns on session create).
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub state: String,
}

impl RepositoryStore {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn add(&self, source: &Path) -> Result<Repository> {
        let source = fs::canonicalize(source)
            .with_context(|| format!("canonicalizing {}", source.display()))?;
        if !source.is_dir() {
            bail!("{} is not a directory", source.display());
        }

        let _lock = locking::try_lock(&self.paths.locks_dir().join("repositories.lock"))?;

        ensure_clean_git_repo(&source)?;
        let base_commit = git_rev_parse(&source, "HEAD")?;

        let id = format!("repo_{}", short_id());
        let repo_dir = self.paths.repo_dir(&id);
        let base_path = repo_dir.join("base");
        let objects = repo_dir.join("git-objects");
        let seed_dir = repo_dir.join("seed");

        fs::create_dir_all(&repo_dir)?;
        fs::create_dir_all(&seed_dir)?;
        fs::create_dir_all(repo_dir.join("semantic").join("shared"))?;

        // Filesystem lowerdir must NOT include `.git`. Whiteouting a full .git
        // through OverlayFS/FUSE on every session create was ~O(git files) and
        // dominated "Git init" timings in M4 (~800ms for 5k-file repos).
        copy_worktree_excluding_git(&source, &base_path)
            .with_context(|| format!("copying worktree from {}", source.display()))?;

        // Shared immutable object store (separate from the COW lowerdir).
        // Do not `cp -a .git/objects` directly — concurrent auto-gc removes
        // loose object dirs mid-walk. A bare clone is a consistent snapshot.
        // Same-device: allow hardlinks (faster). Cross-device or
        // LAZYTREE_OBJECTS_COPY=1: --no-hardlinks (full copy).
        {
            if objects.exists() {
                fs::remove_dir_all(&objects)?;
            }
            let bare = repo_dir.join(".objects-clone.git");
            if bare.exists() {
                fs::remove_dir_all(&bare)?;
            }
            let force_copy = std::env::var_os("LAZYTREE_OBJECTS_COPY")
                .map(|v| v == "1")
                .unwrap_or(false);
            let cross_device = match (
                fs::metadata(&source),
                fs::metadata(&self.paths.home),
            ) {
                (Ok(src_meta), Ok(home_meta)) => src_meta.dev() != home_meta.dev(),
                _ => true,
            };
            let mut cmd = Command::new("git");
            cmd.args(["clone", "--bare", "--quiet"]);
            if force_copy || cross_device {
                cmd.arg("--no-hardlinks");
            }
            let status = cmd
                .arg(source.to_str().unwrap_or("."))
                .arg(bare.to_str().unwrap_or(".objects-clone.git"))
                .status()
                .context("git clone --bare for object store")?;
            if !status.success() {
                let _ = fs::remove_dir_all(&bare);
                bail!("failed to snapshot git objects from {}", source.display());
            }
            let cloned_objects = bare.join("objects");
            fs::rename(&cloned_objects, &objects).with_context(|| {
                format!(
                    "moving {} -> {}",
                    cloned_objects.display(),
                    objects.display()
                )
            })?;
            let _ = fs::remove_dir_all(&bare);
        }

        // Seed index: byte-copy into sessions instead of read-tree.
        let seed_index = seed_dir.join("index");
        let src_index = source.join(".git/index");
        if !src_index.is_file() {
            bail!("source repository has no index; commit before registering");
        }
        fs::copy(&src_index, &seed_index).context("copying seed index")?;

        let user_name = git_config_get(&source, "user.name");
        let user_email = git_config_get(&source, "user.email");

        let repo = Repository {
            version: 1,
            id: id.clone(),
            source_path: source.display().to_string(),
            base_path: base_path.display().to_string(),
            base_commit,
            object_store: objects.display().to_string(),
            seed_index: Some(seed_index.display().to_string()),
            user_name,
            user_email,
            created_at: Utc::now(),
            state: "ready".into(),
        };
        atomic_write_json(&repo_dir.join("metadata.json"), &repo)?;
        Ok(repo)
    }

    pub fn list(&self) -> Result<Vec<Repository>> {
        let mut out = Vec::new();
        let dir = self.paths.repositories_dir();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let meta = entry.path().join("metadata.json");
            if meta.exists() {
                out.push(read_json(&meta)?);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub fn get(&self, id_or_path: &str) -> Result<Repository> {
        for r in self.list()? {
            if r.id == id_or_path || r.source_path == id_or_path {
                return Ok(r);
            }
        }
        // Also try canonicalize path match
        if let Ok(canon) = fs::canonicalize(id_or_path) {
            let s = canon.display().to_string();
            for r in self.list()? {
                if r.source_path == s {
                    return Ok(r);
                }
            }
        }
        bail!("repository not found: {id_or_path}");
    }

    pub fn remove(&self, id_or_path: &str, force: bool) -> Result<()> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("repositories.lock"))?;
        let repo = self.get(id_or_path)?;

        // Refuse if sessions reference this repo unless force
        let sessions_dir = self.paths.sessions_dir();
        if sessions_dir.exists() {
            for entry in fs::read_dir(&sessions_dir)? {
                let entry = entry?;
                let meta = entry.path().join("metadata.json");
                if !meta.exists() {
                    continue;
                }
                let raw = fs::read_to_string(&meta)?;
                if raw.contains(&repo.id) && !force {
                    bail!(
                        "repository {} still has sessions; destroy them first or pass --force",
                        repo.id
                    );
                }
            }
        }

        let dir = self.paths.repo_dir(&repo.id);
        // base may be read-only
        chmod_writable_tree(&dir)?;
        fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        Ok(())
    }
}

fn ensure_clean_git_repo(path: &Path) -> Result<()> {
    let status = Command::new("git")
        .args(["-C"])
        .arg(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .context("git rev-parse")?;
    if !status.status.success() {
        bail!("{} is not a git repository", path.display());
    }
    let porcelain = Command::new("git")
        .args(["-C"])
        .arg(path)
        .args(["status", "--porcelain"])
        .output()
        .context("git status")?;
    if !porcelain.status.success() {
        bail!("git status failed in {}", path.display());
    }
    if !porcelain.stdout.is_empty() {
        bail!("repository contains uncommitted changes; commit/stash them before registering");
    }
    Ok(())
}

fn git_rev_parse(path: &Path, rev: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(path)
        .args(["rev-parse", rev])
        .output()
        .context("git rev-parse")?;
    if !out.status.success() {
        bail!("git rev-parse {rev} failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_config_get(repo: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Copy a worktree into `dst`, omitting `.git` so OverlayFS lowerdirs stay
/// free of Git metadata (session create then only writes a small gitdir file).
///
/// Important: do **not** `cp -a src/.` then delete `.git`. Right after a commit,
/// git auto-gc can move loose objects while `cp` walks `.git/objects`, causing
/// flaky `cannot stat` failures on medium+ repos.
fn copy_worktree_excluding_git(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    // Prefer rsync exclude (fast, correct). Fall back to tar exclude.
    let rsync = Command::new("rsync")
        .args(["-a", "--delete", "--exclude=.git"])
        .arg(format!("{}/", src.display()))
        .arg(format!("{}/", dst.display()))
        .status();
    match rsync {
        Ok(st) if st.success() => {}
        _ => {
            let status = Command::new("bash")
                .arg("-c")
                .arg(format!(
                    "tar -C {} --exclude=.git -cf - . | tar -C {} -xf -",
                    shell_quote(&src.display().to_string()),
                    shell_quote(&dst.display().to_string()),
                ))
                .status()
                .context("tar exclude .git worktree copy")?;
            if !status.success() {
                bail!(
                    "copying worktree (excluding .git) failed from {} to {}",
                    src.display(),
                    dst.display()
                );
            }
        }
    }

    let embedded_git = dst.join(".git");
    if embedded_git.exists() {
        if embedded_git.is_dir() {
            fs::remove_dir_all(&embedded_git)?;
        } else {
            fs::remove_file(&embedded_git)?;
        }
    }
    Ok(())
}

fn chmod_writable_tree(path: &Path) -> Result<()> {
    let _ = Command::new("chmod")
        .args(["-R", "u+w"])
        .arg(path)
        .status();
    Ok(())
}
