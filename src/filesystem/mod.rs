//! Overlay-shaped COW mounts.
//!
//! Entry point is a thin orchestrator over [`backend::OverlayBackend`] plugins:
//! - Linux: kernel OverlayFS, fuse-overlayfs (+ optional unionfs-fuse)
//! - macOS: unionfs-fuse (via macFUSE / Fuse-T)

mod backend;
mod cmd;
mod detect;
mod plugins;
mod registry;

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::metadata::FilesystemBackendKind;

pub use detect::is_mounted;

use backend::OverlayBackend;
use registry::{auto_order, lookup, platform_plugins, registered};

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

/// Orchestrator: prepare dirs, resolve plugin(s), mount.
pub fn mount_session(req: MountRequest<'_>) -> Result<MountedSession> {
    std::fs::create_dir_all(req.upperdir)?;
    std::fs::create_dir_all(req.workdir)?;
    std::fs::create_dir_all(req.merged)?;

    let kinds = match req.preferred {
        FilesystemBackendKind::Auto => auto_order(req.last_working),
        other => vec![other],
    };

    let mut last_err = None;
    for kind in kinds {
        let Some(plugin) = lookup(kind) else {
            last_err = Some(anyhow::anyhow!(
                "filesystem backend {kind:?} is not available on this OS"
            ));
            continue;
        };
        if !plugin.supported_on_host() {
            last_err = Some(anyhow::anyhow!(
                "filesystem backend {kind:?} is not supported on this host"
            ));
            continue;
        }
        match plugin.mount(&req) {
            Ok(used_sudo) => {
                return Ok(MountedSession {
                    backend: plugin.kind(),
                    used_sudo,
                });
            }
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("overlay mount failed")))
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

    // Prefer the plugin that mounted the session; otherwise try all registered.
    let plugins: Vec<&dyn OverlayBackend> = match backend.and_then(lookup) {
        Some(p) => vec![p],
        None => registered(),
    };

    let mut last_err = None;
    for plugin in plugins {
        match plugin.unmount(path, force_attempt) {
            Ok(()) => {
                if force_attempt || !is_mounted(path).unwrap_or(true) {
                    return Ok(());
                }
            }
            Err(e) => last_err = Some(e),
        }
    }

    if !is_mounted(path)? {
        return Ok(());
    }
    if force_attempt {
        return Ok(());
    }
    Err(last_err.unwrap_or_else(|| bail_unmount(path)))
}

fn bail_unmount(path: &Path) -> anyhow::Error {
    anyhow::anyhow!("failed to unmount {}", path.display())
}

/// Host capability probe for `lazytree doctor` — aggregates platform plugins.
pub fn doctor_host_issues() -> Vec<(String, String, String)> {
    let mut out = Vec::new();

    if platform_plugins().is_empty() {
        out.push((
            "error".into(),
            "unsupported_os".into(),
            "LazyTree COW mounts are only supported on Linux and macOS".into(),
        ));
    }

    for plugin in platform_plugins() {
        for probe in plugin.doctor_probes() {
            out.push((
                probe.severity.into(),
                probe.code.into(),
                probe.message,
            ));
        }
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

    #[cfg(target_os = "macos")]
    out.push((
        "info".into(),
        "fs_backend_macos".into(),
        "macOS sessions use the unionfs-fuse plugin (OverlayFS is Linux-only)".into(),
    ));
    #[cfg(target_os = "linux")]
    out.push((
        "info".into(),
        "fs_backend_linux".into(),
        "Linux sessions prefer kernel OverlayFS, then fuse-overlayfs plugins".into(),
    ));

    out
}
