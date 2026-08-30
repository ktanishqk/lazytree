//! Mountpoint detection (Linux mountinfo; portable st_dev fallback elsewhere).

use std::fs;
use std::path::Path;

use anyhow::Result;

/// True if `path` itself is a mountpoint (not merely on a mounted filesystem).
pub fn is_mounted(path: &Path) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        is_mounted_linux(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        is_mounted_dev_compare(path)
    }
}

#[cfg(target_os = "linux")]
fn is_mounted_linux(path: &Path) -> Result<bool> {
    use anyhow::Context;
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

/// FUSE / overlay mounts typically get a distinct device id from their parent directory.
#[cfg(not(target_os = "linux"))]
fn is_mounted_dev_compare(path: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(false),
    };
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return Ok(false),
    };
    let parent_meta = match fs::metadata(parent) {
        Ok(m) => m,
        Err(_) => return Ok(false),
    };
    Ok(meta.dev() != parent_meta.dev())
}

/// Parse the mount point (field 5) from a /proc/self/mountinfo line.
#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_field5() {
        let line = "36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue";
        assert_eq!(mountinfo_mount_point(line).as_deref(), Some("/mnt2"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_unescape_space() {
        let line = "1 2 3:4 / /tmp/foo\\040bar rw - tmpfs tmpfs rw";
        assert_eq!(
            mountinfo_mount_point(line).as_deref(),
            Some("/tmp/foo bar")
        );
    }
}
