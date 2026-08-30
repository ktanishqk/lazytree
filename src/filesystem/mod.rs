//! Overlay-shaped COW mounts.
//!
//! - Linux: kernel OverlayFS, then fuse-overlayfs
//! - macOS: unionfs-fuse (via macFUSE / Fuse-T)

mod detect;
#[cfg(target_os = "linux")]
mod overlayfs;
mod unionfs;

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Result};

use crate::metadata::FilesystemBackendKind;

pub use detect::is_mounted;

#[derive(Debug, Clone)]
pub struct MountRequest<'a> {
    pub lowerdir: &'a Path,
    pub upperdir: &'a Path,
    pub workdir: &'a Path,
    pub merged: &'a Path,
    pub preferred: FilesystemBackendKind,
    /// Hint from prior successful mounts (Auto mode tries this first).
    pub last_working: Option<FilesystemBackendKind>,
    /// When Some(true), skip unprivileged mount attempts and use sudo -n directly.
    pub needs_sudo: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct MountedSession {
    pub backend: FilesystemBackendKind,
    /// Whether the successful mount used `sudo -n`.
    pub used_sudo: bool,
}

pub fn mount_session(req: MountRequest<'_>) -> Result<MountedSession> {
    std::fs::create_dir_all(req.upperdir)?;
    std::fs::create_dir_all(req.workdir)?;
    std::fs::create_dir_all(req.merged)?;

    match req.preferred {
        FilesystemBackendKind::Auto => mount_auto(&req),
        other => try_backend(other, &req),
    }
}

fn mount_auto(req: &MountRequest<'_>) -> Result<MountedSession> {
    let order = auto_backend_order(req.last_working);
    let mut last_err = None;
    for backend in order {
        match try_backend(backend, req) {
            Ok(m) => return Ok(m),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("overlay mount failed")))
}

fn auto_backend_order(last: Option<FilesystemBackendKind>) -> Vec<FilesystemBackendKind> {
    #[cfg(target_os = "macos")]
    {
        let _ = last;
        vec![FilesystemBackendKind::UnionfsFuse]
    }
    #[cfg(target_os = "linux")]
    {
        match last {
            Some(FilesystemBackendKind::FuseOverlayfs) => vec![
                FilesystemBackendKind::FuseOverlayfs,
                FilesystemBackendKind::KernelOverlayfs,
            ],
            Some(FilesystemBackendKind::KernelOverlayfs) => vec![
                FilesystemBackendKind::KernelOverlayfs,
                FilesystemBackendKind::FuseOverlayfs,
            ],
            Some(FilesystemBackendKind::UnionfsFuse) => vec![
                FilesystemBackendKind::UnionfsFuse,
                FilesystemBackendKind::KernelOverlayfs,
                FilesystemBackendKind::FuseOverlayfs,
            ],
            _ => vec![
                // Kernel is faster when available; fuse is the usual nested-VM path.
                FilesystemBackendKind::KernelOverlayfs,
                FilesystemBackendKind::FuseOverlayfs,
            ],
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = last;
        Vec::new()
    }
}

fn try_backend(backend: FilesystemBackendKind, req: &MountRequest<'_>) -> Result<MountedSession> {
    match backend {
        FilesystemBackendKind::Auto => unreachable!("auto resolved before try_backend"),
        FilesystemBackendKind::KernelOverlayfs => {
            #[cfg(target_os = "linux")]
            {
                let used_sudo = overlayfs::try_kernel(req)?;
                Ok(MountedSession {
                    backend,
                    used_sudo,
                })
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = req;
                bail!("kernel OverlayFS is only available on Linux");
            }
        }
        FilesystemBackendKind::FuseOverlayfs => {
            #[cfg(target_os = "linux")]
            {
                let used_sudo = overlayfs::try_fuse(req)?;
                Ok(MountedSession {
                    backend,
                    used_sudo,
                })
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = req;
                bail!("fuse-overlayfs is only available on Linux; use unionfs_fuse on macOS");
            }
        }
        FilesystemBackendKind::UnionfsFuse => {
            let used_sudo = unionfs::try_mount(req)?;
            Ok(MountedSession {
                backend,
                used_sudo,
            })
        }
    }
}

pub fn umount_with_backend(path: &Path, backend: Option<FilesystemBackendKind>) -> Result<()> {
    umount_inner(path, backend, false)
}

/// Always invoke unmount tools (for `destroy --force`) even if mount detection
/// is inconclusive; still succeeds if the path is already unmounted.
pub fn umount_force(path: &Path, backend: Option<FilesystemBackendKind>) -> Result<()> {
    umount_inner(path, backend, true)
}

fn umount_inner(
    path: &Path,
    backend: Option<FilesystemBackendKind>,
    force_attempt: bool,
) -> Result<()> {
    if !force_attempt && !is_mounted(path)? {
        return Ok(());
    }

    let prefer_fuse = matches!(
        backend,
        Some(FilesystemBackendKind::FuseOverlayfs)
            | Some(FilesystemBackendKind::UnionfsFuse)
            | None
    );
    let prefer_kernel = matches!(backend, Some(FilesystemBackendKind::KernelOverlayfs));

    if prefer_fuse {
        // Linux FUSE helpers first; macOS / Fuse-T typically use plain umount.
        if try_cmd(&["fusermount3", "-u"], path) || try_sudo(&["fusermount3", "-u"], path) {
            return Ok(());
        }
        if try_cmd(&["fusermount", "-u"], path) || try_sudo(&["fusermount", "-u"], path) {
            return Ok(());
        }
    }

    if prefer_kernel || prefer_fuse {
        if try_cmd(&["umount"], path) || try_sudo(&["umount", "-l"], path) {
            return Ok(());
        }
        // macOS sometimes prefers diskutil for stubborn FUSE mounts.
        #[cfg(target_os = "macos")]
        if try_cmd(&["diskutil", "unmount"], path) {
            return Ok(());
        }
    }

    if !is_mounted(path)? {
        return Ok(());
    }

    if force_attempt {
        return Ok(());
    }

    bail!("failed to unmount {}", path.display());
}

pub(crate) fn try_cmd(argv: &[&str], path: &Path) -> bool {
    let (bin, args) = argv.split_first().unwrap();
    Command::new(bin)
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(crate) fn try_sudo(argv: &[&str], path: &Path) -> bool {
    Command::new("sudo")
        .arg("-n")
        .args(argv)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(crate) fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub(crate) fn users_uid() -> u32 {
    unsafe { libc_uid() }
}

pub(crate) fn users_gid() -> u32 {
    unsafe { libc_gid() }
}

unsafe fn libc_uid() -> u32 {
    extern "C" {
        fn getuid() -> u32;
    }
    getuid()
}

unsafe fn libc_gid() -> u32 {
    extern "C" {
        fn getgid() -> u32;
    }
    getgid()
}

/// Host capability probe for `lazytree doctor`.
pub fn doctor_host_issues() -> Vec<(String, String, String)> {
    let mut out = Vec::new();

    #[cfg(target_os = "linux")]
    {
        let fuse = Command::new("fuse-overlayfs")
            .arg("--help")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !fuse {
            out.push((
                "warn".into(),
                "fuse_overlayfs_missing".into(),
                "fuse-overlayfs not found on PATH; session mounts may fail without kernel OverlayFS"
                    .into(),
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if unionfs::resolve_binary().is_none() {
            out.push((
                "warn".into(),
                "unionfs_fuse_missing".into(),
                "unionfs-fuse (or unionfs) not found on PATH; install via Homebrew and ensure macFUSE or Fuse-T is available"
                    .into(),
            ));
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        out.push((
            "error".into(),
            "unsupported_os".into(),
            "LazyTree COW mounts are only supported on Linux and macOS".into(),
        ));
    }

    let sudo_n = Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !sudo_n {
        out.push((
            "info".into(),
            "sudo_n_unavailable".into(),
            "passwordless sudo (-n) unavailable; privileged fuse/overlay mounts will not work in locked-down VMs"
                .into(),
        ));
    }

    out
}
