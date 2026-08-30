//! Platform registry: which overlay plugins exist on this host, and Auto order.

use crate::filesystem::backend::OverlayBackend;
use crate::filesystem::plugins::UnionfsFuse;
use crate::metadata::FilesystemBackendKind;

#[cfg(target_os = "linux")]
use crate::filesystem::plugins::{FuseOverlayFs, KernelOverlayFs};

#[cfg(target_os = "linux")]
static KERNEL: KernelOverlayFs = KernelOverlayFs;
#[cfg(target_os = "linux")]
static FUSE: FuseOverlayFs = FuseOverlayFs;
static UNIONFS: UnionfsFuse = UnionfsFuse;

/// All backends compiled into this build that the orchestrator may select.
pub fn registered() -> Vec<&'static dyn OverlayBackend> {
    #[cfg(target_os = "linux")]
    {
        vec![&KERNEL, &FUSE, &UNIONFS]
    }
    #[cfg(target_os = "macos")]
    {
        vec![&UNIONFS]
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

pub fn lookup(kind: FilesystemBackendKind) -> Option<&'static dyn OverlayBackend> {
    registered().into_iter().find(|b| b.kind() == kind)
}

/// Ordered candidates for Auto (and for explicit preferred when falling through).
pub fn auto_order(last_working: Option<FilesystemBackendKind>) -> Vec<FilesystemBackendKind> {
    #[cfg(target_os = "macos")]
    {
        let _ = last_working;
        vec![FilesystemBackendKind::UnionfsFuse]
    }
    #[cfg(target_os = "linux")]
    {
        match last_working {
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
                FilesystemBackendKind::KernelOverlayfs,
                FilesystemBackendKind::FuseOverlayfs,
            ],
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = last_working;
        Vec::new()
    }
}

/// Plugins that participate in Auto/doctor for this OS (excludes optional test backends).
pub fn platform_plugins() -> Vec<&'static dyn OverlayBackend> {
    #[cfg(target_os = "linux")]
    {
        vec![&KERNEL, &FUSE]
    }
    #[cfg(target_os = "macos")]
    {
        vec![&UNIONFS]
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}
