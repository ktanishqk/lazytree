//! Concrete overlay backend plugins.

#[cfg(target_os = "linux")]
mod fuse_overlay;
#[cfg(target_os = "linux")]
mod kernel_overlay;
mod unionfs;

#[cfg(target_os = "linux")]
pub use fuse_overlay::FuseOverlayFs;
#[cfg(target_os = "linux")]
pub use kernel_overlay::KernelOverlayFs;
pub use unionfs::UnionfsFuse;
