use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::metadata::FilesystemBackendKind;

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
        FilesystemBackendKind::KernelOverlayfs => {
            let used_sudo = try_kernel(&req)?;
            Ok(MountedSession {
                backend: FilesystemBackendKind::KernelOverlayfs,
                used_sudo,
            })
        }
        FilesystemBackendKind::FuseOverlayfs => {
            let used_sudo = try_fuse(&req)?;
            Ok(MountedSession {
                backend: FilesystemBackendKind::FuseOverlayfs,
                used_sudo,
            })
        }
        FilesystemBackendKind::Auto => mount_auto(&req),
    }
}

fn mount_auto(req: &MountRequest<'_>) -> Result<MountedSession> {
    // Prefer the last backend that worked on this host; fall back to the other.
    let order: [FilesystemBackendKind; 2] = match req.last_working {
        Some(FilesystemBackendKind::FuseOverlayfs) => [
            FilesystemBackendKind::FuseOverlayfs,
            FilesystemBackendKind::KernelOverlayfs,
        ],
        Some(FilesystemBackendKind::KernelOverlayfs) => [
            FilesystemBackendKind::KernelOverlayfs,
            FilesystemBackendKind::FuseOverlayfs,
        ],
        _ => [
            // Kernel is faster when available; fuse is the usual nested-VM path.
            FilesystemBackendKind::KernelOverlayfs,
            FilesystemBackendKind::FuseOverlayfs,
        ],
    };

    let mut last_err = None;
    for backend in order {
        let result = match backend {
            FilesystemBackendKind::KernelOverlayfs => try_kernel(req).map(|used_sudo| {
                MountedSession {
                    backend,
                    used_sudo,
                }
            }),
            FilesystemBackendKind::FuseOverlayfs => try_fuse(req).map(|used_sudo| {
                MountedSession {
                    backend,
                    used_sudo,
                }
            }),
            FilesystemBackendKind::Auto => unreachable!(),
        };
        match result {
            Ok(m) => return Ok(m),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("overlay mount failed")))
}

fn try_kernel(req: &MountRequest<'_>) -> Result<bool> {
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

fn try_fuse(req: &MountRequest<'_>) -> Result<bool> {
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
        Some(FilesystemBackendKind::FuseOverlayfs) | None
    );
    let prefer_kernel = matches!(backend, Some(FilesystemBackendKind::KernelOverlayfs));

    if prefer_fuse {
        // Privileged fuse mounts need sudo fusermount; try unprivileged first.
        if try_cmd(&["fusermount3", "-u"], path) || try_sudo(&["fusermount3", "-u"], path) {
            return Ok(());
        }
    }

    // Kernel mounts, or FUSE fallback when fusermount failed.
    if prefer_kernel || prefer_fuse {
        if try_cmd(&["umount"], path) || try_sudo(&["umount", "-l"], path) {
            return Ok(());
        }
    }

    // If it is no longer a mountpoint, treat as success (race with archive/doctor).
    if !is_mounted(path)? {
        return Ok(());
    }

    if force_attempt {
        // Caller will still try to delete the tree.
        return Ok(());
    }

    bail!("failed to unmount {}", path.display());
}

fn try_cmd(argv: &[&str], path: &Path) -> bool {
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

fn try_sudo(argv: &[&str], path: &Path) -> bool {
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

/// True if `path` itself is a mountpoint (not merely on a mounted filesystem).
pub fn is_mounted(path: &Path) -> Result<bool> {
    // Avoid spawning findmnt on every check; /proc/self/mountinfo is authoritative on Linux.
    let target = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    let target = target.to_string_lossy();
    let info = fs::read_to_string("/proc/self/mountinfo").context("reading /proc/self/mountinfo")?;
    for line in info.lines() {
        if let Some(mp) = mountinfo_mount_point(line) {
            if mp == target {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Parse the mount point (field 5) from a /proc/self/mountinfo line.
fn mountinfo_mount_point(line: &str) -> Option<String> {
    // Format: … mountroot mountpoint options … —
    // Fields before `-` are space-separated; mountpoint is the 5th field (1-based).
    let mut fields = Vec::with_capacity(7);
    for (i, part) in line.split(' ').enumerate() {
        fields.push(part);
        if i >= 4 {
            break;
        }
    }
    if fields.len() < 5 {
        return None;
    }
    Some(unescape_mount_path(fields[4]))
}

fn unescape_mount_path(s: &str) -> String {
    // mountinfo escapes space, tab, newline, backslash as \040, \011, \012, \134.
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let oct = &s[i + 1..i + 4];
            if let Ok(v) = u8::from_str_radix(oct, 8) {
                out.push(v as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
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
