use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::locking;
use crate::metadata::{atomic_write_json, read_json, Paths};

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

        let id = format!("repo_{}", &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]);
        let repo_dir = self.paths.repo_dir(&id);
        let base_path = repo_dir.join("base");
        let objects = repo_dir.join("git-objects");

        fs::create_dir_all(&repo_dir)?;
        fs::create_dir_all(&objects)?;

        // Initial base copy is allowed to be O(repo size) (PRD §9).
        copy_dir_recursive(&source, &base_path)
            .with_context(|| format!("copying base from {}", source.display()))?;

        // Shared object store: use the copied base's object database.
        // (Dedicated git-objects/ is reserved for later CAS/reflink backends.)
        let object_store = base_path.join(".git").join("objects");
        if !object_store.is_dir() {
            bail!("registered copy is missing .git/objects");
        }

        let repo = Repository {
            version: 1,
            id: id.clone(),
            source_path: source.display().to_string(),
            base_path: base_path.display().to_string(),
            base_commit,
            object_store: object_store.display().to_string(),
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    // Prefer cp -a for speed/metadata; fall back to walk if needed.
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let status = Command::new("cp")
        .args(["-a"])
        .arg(src)
        .arg(dst)
        .status()
        .context("cp -a")?;
    if !status.success() {
        bail!("cp -a failed from {} to {}", src.display(), dst.display());
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

#[allow(dead_code)]
pub fn repo_base_path(repo: &Repository) -> PathBuf {
    PathBuf::from(&repo.base_path)
}
