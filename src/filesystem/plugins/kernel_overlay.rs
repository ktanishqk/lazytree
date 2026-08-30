//! Kernel OverlayFS plugin (Linux only).

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::filesystem::backend::OverlayBackend;
use crate::filesystem::cmd::{path_str, try_cmd, try_sudo, with_privilege_fallback};
use crate::filesystem::MountRequest;
use crate::metadata::FilesystemBackendKind;

pub struct KernelOverlayFs;

impl OverlayBackend for KernelOverlayFs {
    fn kind(&self) -> FilesystemBackendKind {
        FilesystemBackendKind::KernelOverlayfs
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
        let merged = req.merged.to_path_buf();
        with_privilege_fallback(
            req.needs_sudo,
            || {
                let status = Command::new("mount")
                    .args(["-t", "overlay", "overlay", "-o", &opts])
                    .arg(&merged)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .context("running mount overlay")?;
                if status.success() {
                    Ok(())
                } else {
                    bail!("unprivileged kernel overlay mount failed")
                }
            },
            || {
                let status = Command::new("sudo")
                    .args(["-n", "mount", "-t", "overlay", "overlay", "-o", &opts])
                    .arg(&merged)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .context("running sudo mount overlay")?;
                if status.success() {
                    Ok(())
                } else {
                    bail!("kernel OverlayFS mount failed")
                }
            },
        )
    }

    fn unmount(&self, path: &Path, force: bool) -> Result<()> {
        if try_cmd(&["umount"], path) || try_sudo(&["umount", "-l"], path) {
            return Ok(());
        }
        if force {
            return Ok(());
        }
        bail!("failed to unmount kernel overlay at {}", path.display())
    }
}
