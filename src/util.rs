//! Small shared helpers — prefer these over copy-pasted one-offs.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// 12 hex chars from a fresh UUID (no allocs for hyphen stripping).
pub fn short_id() -> String {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    // 6 bytes → 12 hex chars
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(12);
    for &b in bytes.iter().take(6) {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Single-quote for bash `sh -c` / mount scripts.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Copy a file. Prefer plain `fs::copy` — process spawn dominates for small files
/// (git index), and nested cloud volumes often reject reflink anyway.
pub fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)
        .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
    Ok(())
}

/// Absolute path if possible; otherwise the input as PathBuf.
pub fn abs_path(p: &Path) -> std::path::PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Recursive directory copy with best-effort CoW (`cp --reflink=auto` on Linux,
/// `cp -c` clonefile on macOS). Falls back to a normal archive copy when the FS
/// rejects cloning.
pub fn copy_tree_cow(src: &Path, dst: &Path) -> Result<()> {
    use std::process::Command;

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    #[cfg(target_os = "macos")]
    let status = Command::new("cp")
        .args(["-a", "-c"])
        .arg(src)
        .arg(dst)
        .status()
        .context("cp -ac")?;

    #[cfg(not(target_os = "macos"))]
    let status = Command::new("cp")
        .args(["-a", "--reflink=auto"])
        .arg(src)
        .arg(dst)
        .status()
        .context("cp --reflink=auto")?;

    if status.success() {
        return Ok(());
    }

    // Fallback: plain recursive copy (no CoW).
    let status = Command::new("cp")
        .arg("-a")
        .arg(src)
        .arg(dst)
        .status()
        .context("cp -a fallback")?;
    if !status.success() {
        anyhow::bail!(
            "failed to copy tree {} -> {}",
            src.display(),
            dst.display()
        );
    }
    Ok(())
}
