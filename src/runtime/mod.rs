//! Runtime backends (Milestone 6 — local first).

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::semantic::SemanticPaths;
use crate::session::Session;

pub trait RuntimeBackend {
    fn exec(&self, session: &Session, semantic: &SemanticPaths, argv: &[String]) -> Result<i32>;
}

#[derive(Debug, Default)]
pub struct LocalRuntimeBackend;

impl RuntimeBackend for LocalRuntimeBackend {
    fn exec(&self, session: &Session, semantic: &SemanticPaths, argv: &[String]) -> Result<i32> {
        if argv.is_empty() {
            bail!("exec requires a command");
        }
        if session.lifecycle == "archived" || session.filesystem.state != "mounted" {
            bail!("session {} is not an active mounted workspace", session.name);
        }

        let root = session.root_path();
        if !root.is_dir() {
            bail!("session root missing: {}", root.display());
        }

        let mut cmd = Command::new(&argv[0]);
        if argv.len() > 1 {
            cmd.args(&argv[1..]);
        }
        cmd.current_dir(&root);
        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        for (k, v) in semantic.env_pairs() {
            cmd.env(k, v);
        }
        // Working directory is the merged COW view.
        cmd.env("LAZYTREE_SESSION_ROOT", root.display().to_string());
        cmd.env("LAZYTREE_SESSION_NAME", &session.name);

        let status = cmd
            .status()
            .with_context(|| format!("exec {:?} in {}", argv, root.display()))?;
        Ok(status.code().unwrap_or(1))
    }
}

/// Confirm we do not invent runtime resources at session create time.
#[allow(dead_code)]
pub fn runtime_state_after_create() -> &'static str {
    "none"
}

#[allow(dead_code)]
pub fn ensure_not_started(_session_root: &Path) {}
