//! Linux kernel OverlayFS and fuse-overlayfs mounts.

use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use super::{path_str, users_gid, users_uid, MountRequest};

pub(crate) fn try_kernel(req: &MountRequest<'_>) -> Result<bool> {
    let opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        path_str(req.lowerdir),
        path_str(req.upperdir),
        path_str(req.workdir)
    );

    let skip_unprivileged = req.needs_sudo == Some(true);
    if !skip_unprivileged {
        let status = Command::new("mount")
            .args(["-t", "overlay", "overlay", "-o", &opts])
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

    let status = Command::new("sudo")
        .args(["-n", "mount", "-t", "overlay", "overlay", "-o", &opts])
        .arg(req.merged)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running sudo mount overlay")?;
    if status.success() {
        return Ok(true);
    }
    bail!("kernel OverlayFS mount failed");
}

pub(crate) fn try_fuse(req: &MountRequest<'_>) -> Result<bool> {
    let opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        path_str(req.lowerdir),
        path_str(req.upperdir),
        path_str(req.workdir)
    );

    let skip_unprivileged = req.needs_sudo == Some(true);
    if !skip_unprivileged {
        let status = Command::new("fuse-overlayfs")
            .arg("-o")
            .arg(&opts)
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

    let opts_priv = format!(
        "{opts},allow_other,uid={},gid={}",
        users_uid(),
        users_gid()
    );
    let output = Command::new("sudo")
        .args(["-n", "fuse-overlayfs", "-o", &opts_priv])
        .arg(req.merged)
        .stdin(Stdio::null())
        .output()
        .context("running sudo fuse-overlayfs")?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("fuse-overlayfs mount failed: {stderr}");
}
