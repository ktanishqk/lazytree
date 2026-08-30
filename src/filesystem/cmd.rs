//! Shared process helpers for overlay backends.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn users_uid() -> u32 {
    unsafe { libc_uid() }
}

pub fn users_gid() -> u32 {
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

pub fn try_cmd(argv: &[&str], path: &Path) -> bool {
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

pub fn try_sudo(argv: &[&str], path: &Path) -> bool {
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

/// Try an unprivileged mount first (unless `needs_sudo` says skip), then sudo.
/// Returns whether the successful attempt used sudo.
pub fn with_privilege_fallback(
    needs_sudo: Option<bool>,
    mut unprivileged: impl FnMut() -> Result<()>,
    mut privileged: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let skip_unprivileged = needs_sudo == Some(true);
    if !skip_unprivileged {
        if unprivileged().is_ok() {
            return Ok(false);
        }
    }
    privileged()?;
    Ok(true)
}

pub fn bin_on_path(name: &str) -> bool {
    let help_ok = Command::new(name)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if help_ok {
        return true;
    }
    // Some tools exit non-zero on --help but are still installed.
    Command::new("which")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
