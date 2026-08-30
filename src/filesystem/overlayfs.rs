use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::metadata::FilesystemBackendKind;

#[derive(Debug, Clone)]
pub struct MountRequest<'a> {
    pub lowerdir: &'a Path,
    pub upperdir: &'a Path,
    pub workdir: &'a Path,
    pub merged: &'a Path,
    pub preferred: FilesystemBackendKind,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MountedSession {
    pub merged: PathBuf,
    pub backend: FilesystemBackendKind,
}

pub fn mount_session(req: MountRequest<'_>) -> Result<MountedSession> {
    std::fs::create_dir_all(req.upperdir)?;
    std::fs::create_dir_all(req.workdir)?;
    std::fs::create_dir_all(req.merged)?;

    match req.preferred {
        FilesystemBackendKind::KernelOverlayfs => {
            try_kernel(&req).map(|m| MountedSession {
                merged: req.merged.to_path_buf(),
                backend: m,
            })
        }
        FilesystemBackendKind::FuseOverlayfs => {
            try_fuse(&req).map(|m| MountedSession {
                merged: req.merged.to_path_buf(),
                backend: m,
            })
        }
        FilesystemBackendKind::Auto => {
            if let Ok(backend) = try_kernel(&req) {
                return Ok(MountedSession {
                    merged: req.merged.to_path_buf(),
                    backend,
                });
            }
            let backend = try_fuse(&req)?;
            Ok(MountedSession {
                merged: req.merged.to_path_buf(),
                backend,
            })
        }
    }
}

fn try_kernel(req: &MountRequest<'_>) -> Result<FilesystemBackendKind> {
    let opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        path_str(req.lowerdir),
        path_str(req.upperdir),
        path_str(req.workdir)
    );

    // unprivileged first
    let status = Command::new("mount")
        .args(["-t", "overlay", "overlay", "-o", &opts])
        .arg(req.merged)
        .status();
    if let Ok(s) = status {
        if s.success() {
            return Ok(FilesystemBackendKind::KernelOverlayfs);
        }
    }

    // privileged helper (non-interactive)
    let status = Command::new("sudo")
        .args(["-n", "mount", "-t", "overlay", "overlay", "-o", &opts])
        .arg(req.merged)
        .status()
        .context("running sudo mount overlay")?;
    if status.success() {
        return Ok(FilesystemBackendKind::KernelOverlayfs);
    }
    bail!("kernel OverlayFS mount failed");
}

fn try_fuse(req: &MountRequest<'_>) -> Result<FilesystemBackendKind> {
    let opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        path_str(req.lowerdir),
        path_str(req.upperdir),
        path_str(req.workdir)
    );

    let status = Command::new("fuse-overlayfs")
        .arg("-o")
        .arg(&opts)
        .arg(req.merged)
        .status();
    if let Ok(s) = status {
        if s.success() {
            return Ok(FilesystemBackendKind::FuseOverlayfs);
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
        .output()
        .context("running sudo fuse-overlayfs")?;
    if output.status.success() {
        return Ok(FilesystemBackendKind::FuseOverlayfs);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("fuse-overlayfs mount failed: {stderr}");
}

pub fn umount_path(path: &Path) -> Result<()> {
    if !is_mounted(path)? {
        return Ok(());
    }

    // Prefer fusermount for FUSE mounts
    let fusermount = Command::new("fusermount3")
        .arg("-u")
        .arg(path)
        .status();
    if let Ok(s) = fusermount {
        if s.success() {
            return Ok(());
        }
    }

    let fusermount = Command::new("sudo")
        .args(["-n", "fusermount3", "-u"])
        .arg(path)
        .status();
    if let Ok(s) = fusermount {
        if s.success() {
            return Ok(());
        }
    }

    let status = Command::new("umount").arg(path).status();
    if let Ok(s) = status {
        if s.success() {
            return Ok(());
        }
    }

    let output = Command::new("sudo")
        .args(["-n", "umount", "-l"])
        .arg(path)
        .output()
        .context("sudo umount")?;
    if output.status.success() {
        return Ok(());
    }

    // If it is no longer a mountpoint, treat as success (race with archive/doctor).
    if !is_mounted(path)? {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("failed to unmount {}: {stderr}", path.display());
}

pub fn is_mounted(path: &Path) -> Result<bool> {
    // -M matches the path only if it is itself a mountpoint (not a parent FS).
    let output = Command::new("findmnt")
        .arg("-M")
        .arg(path)
        .output()
        .context("findmnt")?;
    Ok(output.status.success())
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn users_uid() -> u32 {
    unsafe { libc_uid() }
}

fn users_gid() -> u32 {
    unsafe { libc_gid() }
}

// Avoid pulling nix just for getuid/getgid in M1.
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
