//! fuse-overlayfs plugin (Linux only).

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::filesystem::backend::{BackendProbe, OverlayBackend};
use crate::filesystem::cmd::{
    bin_on_path, path_str, try_cmd, try_sudo, users_gid, users_uid, with_privilege_fallback,
};
use crate::filesystem::MountRequest;
use crate::metadata::FilesystemBackendKind;

pub struct FuseOverlayFs;

impl OverlayBackend for FuseOverlayFs {
    fn kind(&self) -> FilesystemBackendKind {
        FilesystemBackendKind::FuseOverlayfs
    }

    fn supported_on_host(&self) -> bool {
        true
    }

    fn mount(&self, req: &MountRequest<'_>) -> Result<bool> {
        let opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            path_str(req.lowerdir),
            path_str(req.upperdir),
            path_str(req.workdir)
        );
        let opts_priv = format!(
            "{opts},allow_other,uid={},gid={}",
            users_uid(),
            users_gid()
        );
        let merged = req.merged.to_path_buf();
        with_privilege_fallback(
            req.needs_sudo,
            || {
                let status = Command::new("fuse-overlayfs")
                    .arg("-o")
                    .arg(&opts)
                    .arg(&merged)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .context("running fuse-overlayfs")?;
                if status.success() {
                    Ok(())
                } else {
                    bail!("unprivileged fuse-overlayfs mount failed")
                }
            },
            || {
                let output = Command::new("sudo")
                    .args(["-n", "fuse-overlayfs", "-o", &opts_priv])
                    .arg(&merged)
                    .stdin(Stdio::null())
                    .output()
                    .context("running sudo fuse-overlayfs")?;
                if output.status.success() {
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("fuse-overlayfs mount failed: {stderr}")
                }
            },
        )
    }

    fn unmount(&self, path: &Path, force: bool) -> Result<()> {
        if try_cmd(&["fusermount3", "-u"], path) || try_sudo(&["fusermount3", "-u"], path) {
            return Ok(());
        }
        if try_cmd(&["fusermount", "-u"], path) || try_sudo(&["fusermount", "-u"], path) {
            return Ok(());
        }
        if try_cmd(&["umount"], path) || try_sudo(&["umount", "-l"], path) {
            return Ok(());
        }
        if force {
            return Ok(());
        }
        bail!("failed to unmount fuse-overlayfs at {}", path.display())
    }

    fn doctor_probes(&self) -> Vec<BackendProbe> {
        if bin_on_path("fuse-overlayfs") {
            Vec::new()
        } else {
            vec![BackendProbe {
                severity: "warn",
                code: "fuse_overlayfs_missing",
                message: "fuse-overlayfs not found on PATH; session mounts may fail without kernel OverlayFS"
                    .into(),
            }]
        }
    }
}
