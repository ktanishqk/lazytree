use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Resolved LazyTree home and well-known subpaths.
#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    pub fn resolve(home: Option<PathBuf>) -> Result<Self> {
        let home = match home {
            Some(p) => p,
            None => {
                if let Some(p) = std::env::var_os("LAZYTREE_HOME") {
                    PathBuf::from(p)
                } else {
                    let h = std::env::var_os("HOME").context("HOME is not set")?;
                    PathBuf::from(h).join(".lazytree")
                }
            }
        };
        Ok(Self { home })
    }

    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.repositories_dir())?;
        fs::create_dir_all(self.sessions_dir())?;
        fs::create_dir_all(self.locks_dir())?;
        if !self.config_path().exists() {
            let cfg = ConfigFile {
                version: 1,
                filesystem_backend: FilesystemBackendKind::Auto,
            };
            atomic_write_json(&self.config_path(), &cfg)?;
        }
        Ok(())
    }

    pub fn config_path(&self) -> PathBuf {
        self.home.join("config.json")
    }

    pub fn repositories_dir(&self) -> PathBuf {
        self.home.join("repositories")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.home.join("sessions")
    }

    pub fn locks_dir(&self) -> PathBuf {
        self.home.join("locks")
    }

    pub fn repo_dir(&self, repo_id: &str) -> PathBuf {
        self.repositories_dir().join(repo_id)
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(session_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    pub version: u32,
    pub filesystem_backend: FilesystemBackendKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemBackendKind {
    Auto,
    KernelOverlayfs,
    FuseOverlayfs,
}

pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(value)?;
    fs::write(&tmp, &data)?;
    // best-effort fsync
    if let Ok(f) = fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let v = serde_json::from_str(&data)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(v)
}
