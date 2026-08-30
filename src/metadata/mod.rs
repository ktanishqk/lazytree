use std::fs::{self, File};
use std::io::Write;
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
            let cfg = ConfigFile::default();
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

    pub fn session_name_claim_dir(&self) -> PathBuf {
        self.sessions_dir().join(".names")
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
    /// Last backend that successfully mounted (never `auto`). Speeds Auto mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_working_backend: Option<FilesystemBackendKind>,
    /// When true, unprivileged mount attempts are known to fail — skip straight to sudo -n.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_needs_sudo: Option<bool>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            version: 1,
            filesystem_backend: FilesystemBackendKind::Auto,
            last_working_backend: None,
            mount_needs_sudo: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemBackendKind {
    Auto,
    KernelOverlayfs,
    FuseOverlayfs,
    /// unionfs-fuse (macOS primary; optional on Linux).
    UnionfsFuse,
}

pub fn load_config(paths: &Paths) -> Result<ConfigFile> {
    let cfg_path = paths.config_path();
    if !cfg_path.exists() {
        return Ok(ConfigFile::default());
    }
    read_json(&cfg_path)
}

/// Persist mount hints learned at runtime (best-effort; ignore write races).
pub fn remember_mount_success(
    paths: &Paths,
    backend: FilesystemBackendKind,
    used_sudo: bool,
) -> Result<()> {
    if matches!(backend, FilesystemBackendKind::Auto) {
        return Ok(());
    }
    let cfg_path = paths.config_path();
    let mut cfg = load_config(paths)?;
    let mut dirty = false;
    if cfg.last_working_backend != Some(backend) {
        cfg.last_working_backend = Some(backend);
        dirty = true;
    }
    if cfg.mount_needs_sudo != Some(used_sudo) {
        cfg.mount_needs_sudo = Some(used_sudo);
        dirty = true;
    }
    if dirty {
        atomic_write_json(&cfg_path, &cfg)?;
    }
    Ok(())
}

pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(value)?;
    let mut f = File::create(&tmp)
        .with_context(|| format!("creating {}", tmp.display()))?;
    f.write_all(&data)?;
    // Durability for metadata; sync the same handle we wrote (avoid re-open).
    let _ = f.sync_data();
    drop(f);
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
