//! Local runtime with optional canonical mount-namespace paths.
//!
//! Many build/LSP caches fingerprint absolute workspace paths. Parallel LazyTree
//! sessions have different merged roots, so a copied `target/` looks cold.
//!
//! Fix: run each `exec` in a private mount namespace (`unshare --user --map-root-user
//! --mount`) and bind-mount that session's root (and target dir) onto fixed
//! paths. Concurrent sessions each get their own namespace, so they can all use
//! the same canonical path without colliding.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::semantic::SemanticPaths;
use crate::session::Session;

const CANON_WORKSPACE: &str = "workspace";
const CANON_TARGET: &str = "target";

#[derive(Debug, Clone)]
pub struct ExecOptions {
    pub canonical: bool,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self { canonical: true }
    }
}

pub trait RuntimeBackend {
    fn exec(
        &self,
        session: &Session,
        semantic: &SemanticPaths,
        argv: &[String],
        opts: &ExecOptions,
    ) -> Result<i32>;
}

#[derive(Debug, Default)]
pub struct LocalRuntimeBackend;

impl RuntimeBackend for LocalRuntimeBackend {
    fn exec(
        &self,
        session: &Session,
        semantic: &SemanticPaths,
        argv: &[String],
        opts: &ExecOptions,
    ) -> Result<i32> {
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

        semantic.ensure()?;
        let target_dir = semantic.session_writable.join("target");
        fs::create_dir_all(&target_dir)?;

        if opts.canonical && canonical_supported() {
            return exec_canonical(session, semantic, &root, &target_dir, argv);
        }
        exec_plain(session, semantic, &root, &target_dir, argv)
    }
}

fn exec_plain(
    session: &Session,
    semantic: &SemanticPaths,
    root: &Path,
    target_dir: &Path,
    argv: &[String],
) -> Result<i32> {
    let mut cmd = Command::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    cmd.current_dir(root);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    apply_common_env(&mut cmd, session, semantic, root, target_dir);
    let status = cmd
        .status()
        .with_context(|| format!("exec {:?} in {}", argv, root.display()))?;
    Ok(status.code().unwrap_or(1))
}

fn exec_canonical(
    session: &Session,
    semantic: &SemanticPaths,
    root: &Path,
    target_dir: &Path,
    argv: &[String],
) -> Result<i32> {
    let canon_root = canonical_dir()?;
    let canon_ws = canon_root.join(CANON_WORKSPACE);
    let canon_tg = canon_root.join(CANON_TARGET);
    fs::create_dir_all(&canon_ws)?;
    fs::create_dir_all(&canon_tg)?;

    // Build a small in-namespace script: bind mounts then exec.
    // Using bash keeps us free of a helper binary.
    let mut script = String::from("set -euo pipefail\n");
    script.push_str(&format!(
        "mount --bind {} {}\n",
        shell_quote(&root.display().to_string()),
        shell_quote(&canon_ws.display().to_string())
    ));
    script.push_str(&format!(
        "mount --bind {} {}\n",
        shell_quote(&target_dir.display().to_string()),
        shell_quote(&canon_tg.display().to_string())
    ));
    script.push_str(&format!("cd {}\n", shell_quote(&canon_ws.display().to_string())));
    script.push_str("exec");
    for a in argv {
        script.push(' ');
        script.push_str(&shell_quote(a));
    }
    script.push('\n');

    let mut cmd = Command::new("unshare");
    cmd.args(["--user", "--map-root-user", "--mount", "bash", "-c", &script]);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    apply_common_env(&mut cmd, session, semantic, root, &canon_tg);
    // Override cwd-sensitive vars to canonical locations.
    cmd.env("PWD", canon_ws.display().to_string());
    cmd.env("CARGO_TARGET_DIR", canon_tg.display().to_string());
    cmd.env("LAZYTREE_CANONICAL_ROOT", canon_ws.display().to_string());

    let status = cmd
        .status()
        .with_context(|| format!("canonical exec {:?} for {}", argv, session.name))?;
    Ok(status.code().unwrap_or(1))
}

fn apply_common_env(
    cmd: &mut Command,
    session: &Session,
    semantic: &SemanticPaths,
    root: &Path,
    target_dir: &Path,
) {
    for (k, v) in semantic.env_pairs() {
        cmd.env(k, v);
    }
    cmd.env("LAZYTREE_SESSION_ROOT", root.display().to_string());
    cmd.env("LAZYTREE_SESSION_NAME", &session.name);
    cmd.env("CARGO_TARGET_DIR", target_dir.display().to_string());
}

fn canonical_dir() -> Result<PathBuf> {
    let base = std::env::var_os("LAZYTREE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".lazytree"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp/lazytree-canonical"));
    let dir = base.join("canonical");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn canonical_supported() -> bool {
    Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount", "true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn shell_quote(s: &str) -> String {
    // Safe single-quote wrapping for paths/args.
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

pub fn promote_session_target_to_shared(semantic: &SemanticPaths) -> Result<()> {
    let src = semantic.session_writable.join("target");
    if !src.is_dir() {
        bail!("no session target at {}", src.display());
    }
    let dst = semantic.shared.join("target-seed");
    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
    // Prefer hardlink trees when possible; fall back to cp -a.
    let status = Command::new("cp")
        .args(["-a", "--reflink=auto"])
        .arg(&src)
        .arg(&dst)
        .status()
        .context("cp target seed")?;
    if !status.success() {
        bail!("failed to promote target seed");
    }
    Ok(())
}

pub fn seed_session_target_from_shared(semantic: &SemanticPaths) -> Result<bool> {
    let src = semantic.shared.join("target-seed");
    if !src.is_dir() {
        return Ok(false);
    }
    let dst = semantic.session_writable.join("target");
    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
    let status = Command::new("cp")
        .args(["-a", "--reflink=auto"])
        .arg(&src)
        .arg(&dst)
        .status()
        .context("seed session target")?;
    if !status.success() {
        bail!("failed to seed session target from shared");
    }
    Ok(true)
}
