//! Local runtime with optional canonical mount-namespace paths.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::metadata::Paths;
use crate::semantic::SemanticPaths;
use crate::session::{Lifecycle, Session};
use crate::util::shell_quote;

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

pub fn exec(
    session: &Session,
    semantic: &SemanticPaths,
    argv: &[String],
    opts: &ExecOptions,
) -> Result<i32> {
    if argv.is_empty() {
        bail!("exec requires a command");
    }
    if session.lifecycle == Lifecycle::Archived || !session.is_active_mount() {
        bail!("session {} is not an active mounted workspace", session.name);
    }

    let root = session.root_path();
    if !root.is_dir() {
        bail!("session root missing: {}", root.display());
    }

    semantic.ensure_roots()?;
    let target_dir = semantic.session_writable.join("target");
    fs::create_dir_all(&target_dir)?;

    if opts.canonical && canonical_supported() {
        return exec_canonical(session, semantic, &root, &target_dir, argv);
    }
    exec_plain(session, semantic, &root, &target_dir, argv)
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
    cmd.current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
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
    script.push_str(&format!(
        "cd {}\n",
        shell_quote(&canon_ws.display().to_string())
    ));
    script.push_str("exec");
    for a in argv {
        script.push(' ');
        script.push_str(&shell_quote(a));
    }
    script.push('\n');

    let mut cmd = Command::new("unshare");
    cmd.args(["--user", "--map-root-user", "--mount", "bash", "-c", &script])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    apply_common_env(&mut cmd, session, semantic, root, &canon_tg);
    cmd.env("PWD", canon_ws.as_os_str());
    cmd.env("CARGO_TARGET_DIR", canon_tg.as_os_str());
    cmd.env("LAZYTREE_CANONICAL_ROOT", canon_ws.as_os_str());

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
    cmd.env("LAZYTREE_SESSION_ROOT", root.as_os_str());
    cmd.env("LAZYTREE_SESSION_NAME", &session.name);
    cmd.env("CARGO_TARGET_DIR", target_dir.as_os_str());
}

fn canonical_dir() -> Result<PathBuf> {
    let paths = Paths::resolve(None).unwrap_or_else(|_| Paths {
        home: PathBuf::from("/tmp/lazytree"),
    });
    let dir = paths.home.join("canonical");
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

pub fn promote_session_target_to_shared(semantic: &SemanticPaths) -> Result<()> {
    let src = semantic.session_writable.join("target");
    if !src.is_dir() {
        bail!("no session target at {}", src.display());
    }
    let dst = semantic.shared.join("target-seed");
    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
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
