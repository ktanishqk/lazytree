//! Shared + per-session cache directories (Milestone 5).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

pub const ENV_SHARED_CACHE: &str = "LAZYTREE_SHARED_CACHE";
pub const ENV_SESSION_CACHE: &str = "LAZYTREE_SESSION_CACHE";

#[derive(Debug, Clone, Serialize)]
pub struct SemanticPaths {
    pub shared: PathBuf,
    pub session_writable: PathBuf,
}

impl SemanticPaths {
    pub fn for_session(repo_dir: &Path, session_dir: &Path) -> Self {
        Self {
            shared: repo_dir.join("semantic/shared"),
            session_writable: session_dir.join("semantic/writable"),
        }
    }

    /// Create roots only — subdirs appear on demand via `env_pairs` / exec.
    pub fn ensure_roots(&self) -> Result<()> {
        fs::create_dir_all(&self.shared)
            .with_context(|| format!("creating {}", self.shared.display()))?;
        fs::create_dir_all(&self.session_writable)
            .with_context(|| format!("creating {}", self.session_writable.display()))?;
        Ok(())
    }

    pub fn env_pairs(&self) -> [(&'static str, PathBuf); 3] {
        let cargo_home = self.shared.join("cargo-home");
        let _ = fs::create_dir_all(&cargo_home);
        [
            (ENV_SHARED_CACHE, self.shared.clone()),
            (ENV_SESSION_CACHE, self.session_writable.clone()),
            ("CARGO_HOME", cargo_home),
        ]
    }
}
