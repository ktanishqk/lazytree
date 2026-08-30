use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::filesystem::{self, MountRequest};
use crate::git::{self, GitSetup};
use crate::locking;
use crate::metadata::{
    atomic_write_json, read_json, FilesystemBackendKind, Paths,
};
use crate::repository::RepositoryStore;

thread_local! {
    static LAST_CREATE_TIMINGS: RefCell<Option<CreateTimings>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTimings {
    pub filesystem_ms: u64,
    pub git_ms: u64,
    pub total_ms: u64,
}

pub fn take_last_create_timings() -> Option<CreateTimings> {
    LAST_CREATE_TIMINGS.with(|t| t.borrow_mut().take())
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    paths: Paths,
    repos: RepositoryStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub repository_id: String,
    pub base_revision: String,
    pub branch: String,
    pub filesystem: FilesystemMeta,
    pub git: LayerState,
    pub semantic: LayerState,
    pub runtime: RuntimeMeta,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Absolute path to session directory under LazyTree home.
    pub session_dir: String,
    #[serde(default)]
    pub lifecycle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemMeta {
    pub backend: FilesystemBackendKind,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerState {
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMeta {
    pub backend: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub dirty: bool,
    pub unexported_commits: u64,
    pub filesystem_state: String,
    pub filesystem_backend: FilesystemBackendKind,
    pub git_state: String,
    pub runtime_state: String,
    pub lifecycle: String,
    pub upper_files: u64,
    pub filesystem_bytes_written: u64,
    pub root: String,
    pub age_seconds: i64,
}

impl Session {
    pub fn root_path(&self) -> PathBuf {
        PathBuf::from(&self.session_dir).join("fs").join("root")
    }

    #[allow(dead_code)]
    pub fn upper_path(&self) -> PathBuf {
        PathBuf::from(&self.session_dir).join("fs").join("upper")
    }

    #[allow(dead_code)]
    pub fn work_path(&self) -> PathBuf {
        PathBuf::from(&self.session_dir).join("fs").join("work")
    }

    pub fn git_dir(&self) -> PathBuf {
        PathBuf::from(&self.session_dir).join("git")
    }
}

impl SessionStore {
    pub fn new(paths: Paths, repos: RepositoryStore) -> Self {
        Self { paths, repos }
    }

    pub fn create(
        &self,
        name: &str,
        repo_ref: Option<&str>,
        from: Option<&str>,
    ) -> Result<Session> {
        validate_name(name)?;
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;

        if self.find_by_name(name)?.is_some() {
            bail!("session name already exists: {name}");
        }

        let repo = match repo_ref {
            Some(r) => self.repos.get(r)?,
            None => {
                let list = self.repos.list()?;
                match list.len() {
                    0 => bail!("no repositories registered; run `lazytree repo add <path>`"),
                    1 => list.into_iter().next().unwrap(),
                    _ => bail!("multiple repositories registered; pass --repo <id>"),
                }
            }
        };

        let id = format!(
            "session_{}",
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
        );
        let session_dir = self.paths.session_dir(&id);
        let fs_dir = session_dir.join("fs");
        let upper = fs_dir.join("upper");
        let work = fs_dir.join("work");
        let root = fs_dir.join("root");
        let git_dir = session_dir.join("git");

        fs::create_dir_all(&upper)?;
        fs::create_dir_all(&work)?;
        fs::create_dir_all(&root)?;
        fs::create_dir_all(&git_dir)?;
        fs::create_dir_all(session_dir.join("semantic").join("writable"))?;
        fs::create_dir_all(session_dir.join("runtime"))?;

        let preferred = read_preferred_backend(&self.paths)?;
        let mount_start = std::time::Instant::now();
        let mounted = filesystem::mount_session(MountRequest {
            lowerdir: Path::new(&repo.base_path),
            upperdir: &upper,
            workdir: &work,
            merged: &root,
            preferred,
        })
        .with_context(|| format!("mounting session {name}"))?;
        let filesystem_ms = mount_start.elapsed().as_millis() as u64;

        let base_revision = from
            .unwrap_or(repo.base_commit.as_str())
            .to_string();
        let branch = format!("lazytree/{name}");
        let object_store = PathBuf::from(&repo.object_store);
        let seed_index = repo.seed_index.as_ref().map(PathBuf::from);

        let git_start = std::time::Instant::now();
        if let Err(err) = git::setup_session_git(&GitSetup {
            git_dir: git_dir.clone(),
            work_tree: root.clone(),
            branch: branch.clone(),
            base_revision: base_revision.clone(),
            object_store,
            seed_index,
            seed_commit: Some(repo.base_commit.clone()),
        }) {
            let _ = filesystem::umount_path(&root);
            let _ = fs::remove_dir_all(&session_dir);
            return Err(err).context("setting up session git");
        }
        let git_ms = git_start.elapsed().as_millis() as u64;

        let now = Utc::now();
        let session = Session {
            version: 1,
            id: id.clone(),
            name: name.to_string(),
            repository_id: repo.id.clone(),
            base_revision,
            branch,
            filesystem: FilesystemMeta {
                backend: mounted.backend,
                state: "mounted".into(),
            },
            git: LayerState {
                state: "ready".into(),
            },
            semantic: LayerState {
                state: "inherited".into(),
            },
            runtime: RuntimeMeta {
                backend: "local".into(),
                state: "none".into(),
            },
            created_at: now,
            updated_at: now,
            session_dir: session_dir.display().to_string(),
            lifecycle: "ready".into(),
        };
        atomic_write_json(&session_dir.join("metadata.json"), &session)?;
        // Stash timings on the session object via a side channel isn't ideal;
        // return through a thread-local for CLI JSON (M4 honesty).
        LAST_CREATE_TIMINGS.with(|t| {
            *t.borrow_mut() = Some(CreateTimings {
                filesystem_ms,
                git_ms,
                total_ms: filesystem_ms + git_ms,
            });
        });
        Ok(session)
    }

    pub fn list(&self) -> Result<Vec<Session>> {
        let mut out = Vec::new();
        let dir = self.paths.sessions_dir();
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
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn get(&self, name_or_id: &str) -> Result<Session> {
        if let Some(s) = self.find_by_name(name_or_id)? {
            return Ok(s);
        }
        for s in self.list()? {
            if s.id == name_or_id {
                return Ok(s);
            }
        }
        bail!("session not found: {name_or_id}");
    }

    pub fn status(&self, name_or_id: &str) -> Result<SessionStatus> {
        let session = self.get(name_or_id)?;
        let root = session.root_path();
        let upper = session.upper_path();

        let dirty = if root.exists() && session.filesystem.state == "mounted" {
            !git_porcelain(&root)?.is_empty()
        } else {
            false
        };

        let unexported = if session.git_dir().join("HEAD").exists() {
            count_unexported(&session)?
        } else {
            0
        };

        let (upper_files, bytes) = dir_stats(&upper)?;
        let age = (Utc::now() - session.created_at).num_seconds();

        Ok(SessionStatus {
            id: session.id,
            name: session.name,
            branch: session.branch,
            dirty,
            unexported_commits: unexported,
            filesystem_state: session.filesystem.state,
            filesystem_backend: session.filesystem.backend,
            git_state: session.git.state,
            runtime_state: session.runtime.state,
            lifecycle: session.lifecycle,
            upper_files,
            filesystem_bytes_written: bytes,
            root: root.display().to_string(),
            age_seconds: age,
        })
    }

    pub fn diff(&self, name_or_id: &str) -> Result<String> {
        let session = self.get(name_or_id)?;
        ensure_active_fs(&session)?;
        let out = Command::new("git")
            .args(["-C"])
            .arg(session.root_path())
            .args(["diff", "HEAD"])
            .output()
            .context("git diff")?;
        if !out.status.success() {
            bail!(
                "git diff failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        let staged = Command::new("git")
            .args(["-C"])
            .arg(session.root_path())
            .args(["diff", "--cached"])
            .output()
            .context("git diff --cached")?;
        if staged.status.success() && !staged.stdout.is_empty() {
            text.push_str("\n# staged\n");
            text.push_str(&String::from_utf8_lossy(&staged.stdout));
        }
        Ok(text)
    }

    pub fn archive(&self, name_or_id: &str) -> Result<Session> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
        let mut session = self.get(name_or_id)?;
        if session.lifecycle == "archived" {
            bail!("session already archived: {}", session.name);
        }
        ensure_active_fs(&session)?;

        let repo = self.repos.get(&session.repository_id)?;
        // Publish branch into the user's source repository.
        let status = Command::new("git")
            .arg("--git-dir")
            .arg(session.git_dir())
            .args([
                "push",
                &repo.source_path,
                &format!("HEAD:refs/heads/{}", session.branch),
            ])
            .output()
            .context("git push to source repository")?;
        if !status.status.success() {
            bail!(
                "failed to publish branch to {}: {}",
                repo.source_path,
                String::from_utf8_lossy(&status.stderr).trim()
            );
        }

        let root = session.root_path();
        filesystem::umount_path(&root)?;

        // Drop expensive ephemeral layers; keep metadata.
        let fs_dir = PathBuf::from(&session.session_dir).join("fs");
        for sub in ["upper", "work", "root"] {
            let p = fs_dir.join(sub);
            if p.exists() {
                let _ = fs::remove_dir_all(&p);
                let _ = fs::create_dir_all(&p);
            }
        }
        let git_dir = session.git_dir();
        if git_dir.exists() {
            fs::remove_dir_all(&git_dir)?;
        }
        let runtime = PathBuf::from(&session.session_dir).join("runtime");
        if runtime.exists() {
            let _ = fs::remove_dir_all(&runtime);
            let _ = fs::create_dir_all(&runtime);
        }
        let semantic = PathBuf::from(&session.session_dir)
            .join("semantic")
            .join("writable");
        if semantic.exists() {
            let _ = fs::remove_dir_all(&semantic);
            let _ = fs::create_dir_all(&semantic);
        }

        session.filesystem.state = "archived".into();
        session.git.state = "published".into();
        session.semantic.state = "none".into();
        session.runtime.state = "none".into();
        session.lifecycle = "archived".into();
        session.updated_at = Utc::now();
        atomic_write_json(
            &PathBuf::from(&session.session_dir).join("metadata.json"),
            &session,
        )?;
        Ok(session)
    }

    pub fn destroy(&self, name_or_id: &str, force: bool) -> Result<()> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
        let session = self.get(name_or_id)?;

        if session.lifecycle != "archived" && !force {
            if session.filesystem.state == "mounted" {
                let dirty = !git_porcelain(&session.root_path())?.is_empty();
                let unexported = count_unexported(&session)?;
                if dirty || unexported > 0 {
                    bail!(
                        "workspace has {}{}{}\nUse:\n  lazytree diff {}\n  lazytree archive {}\n  lazytree destroy {} --force",
                        if dirty { "uncommitted changes" } else { "" },
                        if dirty && unexported > 0 { " and " } else { "" },
                        if unexported > 0 {
                            "commits that have not been exported"
                        } else {
                            ""
                        },
                        session.name,
                        session.name,
                        session.name
                    );
                }
            }
        }

        let root = session.root_path();
        if filesystem::is_mounted(&root)? {
            filesystem::umount_path(&root)
                .with_context(|| format!("unmounting {}", root.display()))?;
        }

        let dir = PathBuf::from(&session.session_dir);
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("removing {}", dir.display()))?;
        }
        Ok(())
    }

    fn find_by_name(&self, name: &str) -> Result<Option<Session>> {
        for s in self.list()? {
            if s.name == name {
                return Ok(Some(s));
            }
        }
        Ok(None)
    }
}

fn ensure_active_fs(session: &Session) -> Result<()> {
    if session.lifecycle == "archived" || session.filesystem.state != "mounted" {
        bail!("session {} is not an active mounted workspace", session.name);
    }
    if !filesystem::is_mounted(&session.root_path())? {
        bail!(
            "session {} is not mounted; run `lazytree doctor`",
            session.name
        );
    }
    Ok(())
}

fn git_porcelain(work_tree: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(work_tree)
        .args(["status", "--porcelain"])
        .output()
        .context("git status")?;
    if !out.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn count_unexported(session: &Session) -> Result<u64> {
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(session.git_dir())
        .args([
            "rev-list",
            "--count",
            &format!("{}..HEAD", session.base_revision),
        ])
        .output()
        .context("git rev-list")?;
    if !out.status.success() {
        // If rev-list fails (e.g. missing git), treat as unknown/unsafe.
        bail!(
            "git rev-list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let n = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>()?;
    Ok(n)
}

fn dir_stats(path: &Path) -> Result<(u64, u64)> {
    if !path.exists() {
        return Ok((0, 0));
    }
    let mut files = 0u64;
    let mut bytes = 0u64;
    fn walk(p: &Path, files: &mut u64, bytes: &mut u64) -> Result<()> {
        if p.is_file() {
            *files += 1;
            *bytes += fs::metadata(p)?.len();
            return Ok(());
        }
        if p.is_dir() {
            for entry in fs::read_dir(p)? {
                let entry = entry?;
                // skip overlay work internals whiteouts counting as files is fine
                walk(&entry.path(), files, bytes)?;
            }
        }
        Ok(())
    }
    walk(path, &mut files, &mut bytes)?;
    Ok((files, bytes))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("session name must not be empty");
    }
    if name.contains('/') || name.contains('\0') {
        bail!("session name must not contain '/' or NUL");
    }
    Ok(())
}

fn read_preferred_backend(paths: &Paths) -> Result<FilesystemBackendKind> {
    let cfg_path = paths.config_path();
    if !cfg_path.exists() {
        return Ok(FilesystemBackendKind::Auto);
    }
    let cfg: crate::metadata::ConfigFile = read_json(&cfg_path)?;
    Ok(cfg.filesystem_backend)
}
