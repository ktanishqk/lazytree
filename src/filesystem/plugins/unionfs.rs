//! unionfs-fuse plugin (macOS primary; usable on Linux for testing).

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};

use crate::filesystem::backend::{BackendProbe, OverlayBackend};
use crate::filesystem::cmd::{
    bin_on_path, path_str, try_cmd, try_sudo, users_gid, users_uid, with_privilege_fallback,
};
use crate::filesystem::MountRequest;
use crate::metadata::FilesystemBackendKind;

static UNIONFS_BIN: OnceLock<Option<String>> = OnceLock::new();

fn resolve_binary() -> Option<&'static str> {
    UNIONFS_BIN
        .get_or_init(|| {
            for name in ["unionfs-fuse", "unionfs"] {
                if bin_on_path(name) {
                    return Some(name.to_string());
                }
            }
            None
        })
        .as_deref()
}

pub struct UnionfsFuse;

impl OverlayBackend for UnionfsFuse {
    fn kind(&self) -> FilesystemBackendKind {
        FilesystemBackendKind::UnionfsFuse
    }

    fn supported_on_host(&self) -> bool {
        // Available wherever the binary exists; Auto registry decides platform priority.
        true
    }

    fn mount(&self, req: &MountRequest<'_>) -> Result<bool> {
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
        let opts_priv = format!(
            "cow,use_ino,allow_other,uid={},gid={}",
            users_uid(),
            users_gid()
        );
        let merged = req.merged.to_path_buf();

        with_privilege_fallback(
            req.needs_sudo,
            || {
                let status = Command::new(bin)
                    .args(["-o", "cow,use_ino"])
                    .arg(&branches)
                    .arg(&merged)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .context("running unionfs-fuse")?;
                if status.success() {
                    Ok(())
                } else {
                    bail!("unprivileged unionfs-fuse mount failed")
                }
            },
            || {
                let output = Command::new("sudo")
                    .args(["-n", bin, "-o", &opts_priv])
                    .arg(&branches)
                    .arg(&merged)
                    .stdin(Stdio::null())
                    .output()
                    .context("running sudo unionfs-fuse")?;
                if output.status.success() {
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("unionfs-fuse mount failed: {stderr}")
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
        #[cfg(target_os = "macos")]
        if try_cmd(&["diskutil", "unmount"], path) {
            return Ok(());
        }
        if force {
            return Ok(());
        }
        bail!("failed to unmount unionfs-fuse at {}", path.display())
    }

    fn doctor_probes(&self) -> Vec<BackendProbe> {
        // Only warn when this plugin is part of the active platform set (macOS),
        // or when explicitly useful: registry filters doctor to registered plugins.
        if resolve_binary().is_some() {
            Vec::new()
        } else {
            vec![BackendProbe {
                severity: "warn",
                code: "unionfs_fuse_missing",
                message: "unionfs-fuse (or unionfs) not found on PATH; install via Homebrew and ensure macFUSE or Fuse-T is available"
                    .into(),
            }]
        }
    }
}
