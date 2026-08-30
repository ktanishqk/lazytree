//! Git `core.fsmonitor` hook backed by the session OverlayFS/unionfs upperdir.
//!
//! Real `git status` still runs — we only answer “which paths may have changed?”

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::util::abs_path;

const TOKEN_PREFIX: &str = "lt:";

/// Run the protocol v2 query and write the response to stdout (NUL-framed).
pub fn query_v2(upper: &Path, last_token: &str) -> Result<()> {
    let new_token = next_token(last_token);
    let paths = list_upper_paths(upper)?;

    let mut out = io::stdout().lock();
    out.write_all(new_token.as_bytes())?;
    out.write_all(&[0])?;
    for p in paths {
        out.write_all(p.as_bytes())?;
        out.write_all(&[0])?;
    }
    out.flush()?;
    Ok(())
}

fn next_token(last: &str) -> String {
    let n = last
        .strip_prefix(TOKEN_PREFIX)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    format!("{TOKEN_PREFIX}{n}")
}

/// Inclusive list of worktree-relative paths that may have changed.
pub fn list_upper_paths(upper: &Path) -> Result<Vec<String>> {
    if !upper.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    walk_upper(upper, upper, &mut out)?;
    out.sort();
    out.dedup();
    Ok(out)
}

fn walk_upper(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("reading upper {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Overlay opaque-dir marker — report the directory itself.
        if name_str == ".wh..wh..opq" {
            if let Ok(rel) = path.parent().unwrap_or(dir).strip_prefix(root) {
                let s = rel.to_string_lossy().replace('\\', "/");
                if !s.is_empty() {
                    out.push(s);
                }
            }
            continue;
        }

        // AUFS-style whiteout prefix.
        let logical = name_str
            .strip_prefix(".wh.")
            .map(|s| s.to_string())
            .unwrap_or_else(|| name_str.to_string());

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let ft = meta.file_type();

        let rel = match path.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        // If whiteout prefix was stripped, rebuild relative path with logical name.
        let rel_str = if logical != name_str.as_ref() {
            let parent = rel.parent().unwrap_or_else(|| Path::new(""));
            if parent.as_os_str().is_empty() {
                logical.clone()
            } else {
                format!("{}/{}", parent.to_string_lossy(), logical)
            }
        } else {
            rel.to_string_lossy().replace('\\', "/")
        };

        if rel_str.is_empty() || rel_str == ".git" || rel_str.starts_with(".git/") {
            continue;
        }

        // Char-device whiteouts (overlayfs / fuse-overlayfs): still a changed path.
        if ft.is_char_device() || ft.is_file() || ft.is_symlink() {
            out.push(rel_str);
            continue;
        }
        if ft.is_dir() {
            // Directories in upper matter for untracked discovery.
            out.push(rel_str);
            walk_upper(root, &path, out)?;
        }
    }
    Ok(())
}

/// Install session-local fsmonitor hook + git config. No-op if `LAZYTREE_FSMONITOR=0`.
pub fn install_session_fsmonitor(git_dir: &Path, upper: &Path) -> Result<()> {
    if std::env::var_os("LAZYTREE_FSMONITOR").as_deref() == Some(std::ffi::OsStr::new("0")) {
        return Ok(());
    }

    let abs_git = abs_path(git_dir);
    let abs_upper = abs_path(upper);
    let hook_path = abs_git.join("lazytree-fsmonitor");
    let exe = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("lazytree"));
    let abs_exe = abs_path(&exe);

    let script = format!(
        r#"#!/usr/bin/env bash
# LazyTree → Git core.fsmonitor (protocol v2). Real git status still runs.
set -euo pipefail
UPPER={upper}
LT_BIN={exe}
if [[ -x "$LT_BIN" ]]; then
  exec "$LT_BIN" git-fsmonitor --upper "$UPPER" "$@"
fi
# Fallback if the creating binary moved: walk upper with find.
version="${{1:-2}}"
token="${{2:-}}"
n=1
if [[ "$token" == lt:* ]]; then
  n=$((${{token#lt:}}+1))
fi
printf 'lt:%s\0' "$n"
if [[ -d "$UPPER" ]]; then
  # Portable walk (GNU find -printf is Linux-only; macOS uses this path if LT_BIN missing).
  find "$UPPER" \( -type f -o -type l -o -type c -o -type d \) ! -path "$UPPER" 2>/dev/null \
    | while IFS= read -r p; do
        rel="${{p#"$UPPER"/}}"
        case "$rel" in .git|.git/*) continue ;; esac
        printf '%s\0' "$rel"
      done
fi
"#,
        upper = shell_single_quote(&abs_upper.to_string_lossy()),
        exe = shell_single_quote(&abs_exe.to_string_lossy()),
    );

    fs::write(&hook_path, script)
        .with_context(|| format!("writing {}", hook_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }

    // Use git-config so quoting stays valid.
    let configs = [
        ("core.fsmonitor", hook_path.to_string_lossy().into_owned()),
        ("core.fsmonitorHookVersion", "2".into()),
        // macFUSE / Fuse-T / unionfs mounts are classified as "remote" by Git;
        // without this, Darwin disables our hook and status full-scans the FUSE tree.
        ("fsmonitor.allowRemote", "true".into()),
    ];
    for (key, val) in configs {
        let status = Command::new("git")
            .args(["--git-dir"])
            .arg(&abs_git)
            .args(["config", key])
            .arg(&val)
            .status()
            .with_context(|| format!("git config {key}"))?;
        if !status.success() {
            bail!("failed to set {key}");
        }
    }
    Ok(())
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn empty_upper_lists_nothing() {
        let dir = std::env::temp_dir().join(format!("lt-fsm-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(list_upper_paths(&dir).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_files_and_strips_wh_prefix() {
        let dir = std::env::temp_dir().join(format!("lt-fsm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/a.rs"), b"x").unwrap();
        fs::write(dir.join(".wh.gone.txt"), b"").unwrap();
        let mut paths = list_upper_paths(&dir).unwrap();
        paths.sort();
        let _ = fs::remove_dir_all(&dir);
        assert!(paths.iter().any(|p| p == "src" || p == "src/a.rs"));
        assert!(paths.iter().any(|p| p == "gone.txt"));
    }
}
