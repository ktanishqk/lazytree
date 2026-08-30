//! unionfs-fuse backend (macOS primary; also usable on Linux for testing).
//!
//! Mount shape matches OverlayFS sessions: read-only lower + writable upper,
//! merged at `merged`. No OverlayFS-style workdir is required; we still create
//! one for a uniform on-disk layout.

use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};

use super::{path_str, users_gid, users_uid, MountRequest};

static UNIONFS_BIN: OnceLock<Option<String>> = OnceLock::new();

/// Resolve `unionfs-fuse` or `unionfs` once per process.
pub(crate) fn resolve_binary() -> Option<&'static str> {
    UNIONFS_BIN
        .get_or_init(|| {
            for name in ["unionfs-fuse", "unionfs"] {
                let ok = Command::new(name)
                    .arg("--help")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                // Some builds exit non-zero on --help but still exist on PATH.
                let exists = ok
                    || Command::new("which")
                        .arg(name)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                if exists {
                    return Some(name.to_string());
                }
            }
            None
        })
        .as_deref()
}

pub(crate) fn try_mount(req: &MountRequest<'_>) -> Result<bool> {
    let bin = resolve_binary().ok_or_else(|| {
        anyhow::anyhow!(
            "unionfs-fuse not found (tried `unionfs-fuse` and `unionfs`); \
             on macOS install macFUSE or Fuse-T, then `brew install unionfs-fuse`"
        )
    })?;

    // Leftmost RW branch receives writes when `cow` is set.
    let branches = format!(
        "{}=RW:{}=RO",
        path_str(req.upperdir),
        path_str(req.lowerdir)
    );

    let skip_unprivileged = req.needs_sudo == Some(true);
    if !skip_unprivileged {
        let status = Command::new(bin)
            .args(["-o", "cow,use_ino"])
            .arg(&branches)
            .arg(req.merged)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Ok(s) = status {
            if s.success() {
                return Ok(false);
            }
        }
    }

    // Privileged path: allow_other so the invoking user can use the mount.
    let opts = format!(
        "cow,use_ino,allow_other,uid={},gid={}",
        users_uid(),
        users_gid()
    );
    let output = Command::new("sudo")
        .args(["-n", bin, "-o", &opts])
        .arg(&branches)
        .arg(req.merged)
        .stdin(Stdio::null())
        .output()
        .context("running sudo unionfs-fuse")?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("unionfs-fuse mount failed: {stderr}");
}
