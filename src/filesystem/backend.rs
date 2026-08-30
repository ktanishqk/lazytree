//! Overlay filesystem backend plugin contract.

use std::path::Path;

use anyhow::Result;

use crate::metadata::FilesystemBackendKind;

use super::MountRequest;

/// One host/doctor finding contributed by a backend plugin.
#[derive(Debug, Clone)]
pub struct BackendProbe {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
}

/// Strategy object for a single COW mount implementation.
///
/// Linux and macOS ship different plugins; the orchestrator only knows this trait.
pub trait OverlayBackend: Send + Sync {
    fn kind(&self) -> FilesystemBackendKind;

    /// Whether this backend is compiled/enabled for the current OS.
    fn supported_on_host(&self) -> bool;

    /// Mount lower+upper into `req.merged`. Returns whether sudo was used.
    fn mount(&self, req: &MountRequest<'_>) -> Result<bool>;

    /// Unmount `path`. `force` means best-effort (destroy path).
    fn unmount(&self, path: &Path, force: bool) -> Result<()>;

    /// Optional doctor notes (missing binaries, etc.).
    fn doctor_probes(&self) -> Vec<BackendProbe> {
        Vec::new()
    }
}
