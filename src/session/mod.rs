use std::fs;
use std::path::{Path, PathBuf};

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
        let mounted = filesystem::mount_session(MountRequest {
            lowerdir: Path::new(&repo.base_path),
            upperdir: &upper,
            workdir: &work,
            merged: &root,
            preferred,
        })
        .with_context(|| format!("mounting session {name}"))?;

        let base_revision = from
            .unwrap_or(repo.base_commit.as_str())
            .to_string();
        let branch = format!("lazytree/{name}");
        let object_store = PathBuf::from(&repo.object_store);

        if let Err(err) = git::setup_session_git(&GitSetup {
            git_dir: git_dir.clone(),
            work_tree: root.clone(),
            branch: branch.clone(),
            base_revision: base_revision.clone(),
            object_store,
        }) {
            let _ = filesystem::umount_path(&root);
            let _ = fs::remove_dir_all(&session_dir);
            return Err(err).context("setting up session git");
        }

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
        };
        atomic_write_json(&session_dir.join("metadata.json"), &session)?;
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

    pub fn destroy(&self, name_or_id: &str, _force: bool) -> Result<()> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
        let session = self.get(name_or_id)?;
        let root = session.root_path();

        filesystem::umount_path(&root)
            .with_context(|| format!("unmounting {}", root.display()))?;

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
