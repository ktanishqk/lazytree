//! Semantic/cache state (Milestone 5).
//!
//! Shared read-mostly caches live on the repository; each session gets a
//! writable delta directory. Tools are pointed at these via environment
//! variables — LazyTree does not snapshot running language servers.

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
            shared: repo_dir.join("semantic").join("shared"),
            session_writable: session_dir.join("semantic").join("writable"),
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.shared)
            .with_context(|| format!("creating {}", self.shared.display()))?;
        fs::create_dir_all(&self.session_writable)
            .with_context(|| format!("creating {}", self.session_writable.display()))?;
        // Conventional subdirs so ecosystems have somewhere obvious to put data.
        for sub in ["cargo-home", "caches", "indexes"] {
            fs::create_dir_all(self.shared.join(sub))?;
            fs::create_dir_all(self.session_writable.join(sub))?;
        }
        Ok(())
    }

    /// Env pairs for `lazytree exec` / agent wrappers.
    pub fn env_pairs(&self) -> Vec<(String, String)> {
        vec![
            (
                ENV_SHARED_CACHE.to_string(),
                self.shared.display().to_string(),
            ),
            (
                ENV_SESSION_CACHE.to_string(),
                self.session_writable.display().to_string(),
            ),
            // Handy defaults for Rust tooling when users opt in via exec.
            (
                "CARGO_HOME".to_string(),
                self.shared.join("cargo-home").display().to_string(),
            ),
        ]
    }
}
