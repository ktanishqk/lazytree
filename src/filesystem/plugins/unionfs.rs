//! unionfs-fuse plugin (macOS primary; usable on Linux for testing).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::filesystem::backend::{BackendProbe, OverlayBackend};
use crate::filesystem::cmd::{
    bin_on_path, path_str, try_cmd, try_sudo, users_gid, users_uid, with_privilege_fallback,
};
use crate::filesystem::detect::is_mounted;
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

fn unionfs_command(
    bin: &str,
    opt: &str,
    branches: &str,
    merged: &Path,
    privileged: bool,
) -> Command {
    let mut cmd = if privileged {
        let mut c = Command::new("sudo");
        c.args(["-n", bin]);
        c
    } else {
        Command::new(bin)
    };
    // Fuse-T's NFS server lives in the FUSE process. Daemonize (`unionfs` without
    // `-f`) exits 0 and never attaches the mount. Keep the process in the foreground
    // and detach it ourselves after the mount appears.
    cmd.args(["-f", "-o", opt]).arg(branches).arg(merged);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    cmd
}

fn spawn_until_mounted(mut cmd: Command, merged: &Path) -> Result<()> {
    let mut child = cmd.spawn().context("spawning unionfs-fuse")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if is_mounted(merged).unwrap_or(false) {
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut err = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut err);
                }
                let err = err.trim().to_string();
                if err.is_empty() {
                    bail!("unionfs-fuse mount failed (exit {status})");
                }
                bail!("unionfs-fuse mount failed: {err}");
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("unionfs-fuse did not become a mountpoint in time");
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e).context("waiting for unionfs-fuse"),
        }
    }
}

fn try_unionfs_opts(
    bin: &str,
    opts: &[&str],
    branches: &str,
    merged: &Path,
    privileged: bool,
) -> Result<()> {
    let mut last_err = None;
    for opt in opts {
        let cmd = unionfs_command(bin, opt, branches, merged, privileged);
        match spawn_until_mounted(cmd, merged) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    bail!(
        "{}",
        last_err.unwrap_or_else(|| "unionfs-fuse mount failed".into())
    )
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
                 on macOS install Fuse-T or macFUSE, then put `unionfs` on PATH \
                 (Fuse-T: build unionfs-fuse with -DWITH_LIBFUSE3=TRUE; \
                 macFUSE: brew install gromgit/fuse/unionfs-fuse)"
            )
        })?;

        // Leftmost RW branch receives writes when `cow` is set.
        let branches = format!(
            "{}=RW:{}=RO",
            path_str(req.upperdir),
            path_str(req.lowerdir)
        );
        // FUSE 2 accepts `use_ino`; Fuse-T's FUSE 3 rejects it (`unknown option`).
        // Try the inode-preserving set first, then the FUSE 3-safe fallback.
        let unpriv_opts: &[&str] = &["cow,use_ino", "cow"];
        let priv_suffix = format!("allow_other,uid={},gid={}", users_uid(), users_gid());
        let merged = req.merged.to_path_buf();

        with_privilege_fallback(
            req.needs_sudo,
            || try_unionfs_opts(bin, unpriv_opts, &branches, &merged, false),
            || {
                let priv_opts = [
                    format!("cow,use_ino,{priv_suffix}"),
                    format!("cow,{priv_suffix}"),
                ];
                let refs: Vec<&str> = priv_opts.iter().map(String::as_str).collect();
                try_unionfs_opts(bin, &refs, &branches, &merged, true)
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
                message: "unionfs-fuse (or unionfs) not found on PATH; see README macOS prerequisites (Fuse-T or macFUSE)"
                    .into(),
            }]
        }
    }
}
