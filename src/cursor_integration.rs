//! Cursor IDE helpers (open session as workspace, install hooks/skills).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::session::Session;

/// Open the session root in Cursor (or VS Code) so the IDE Git UI binds to our branch.
pub fn open_session(session: &Session, editor: &str) -> Result<()> {
    let root = session.root_path();
    if !root.is_dir() {
        bail!("session root missing: {}", root.display());
    }
    let status = Command::new(editor)
        .arg(root.as_os_str())
        .status()
        .with_context(|| format!("launching {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

pub fn open_session_info(session: &Session) -> serde_json::Value {
    serde_json::json!({
        "name": session.name,
        "id": session.id,
        "root": session.root_path().display().to_string(),
        "branch": session.branch,
        "repository_id": session.repository_id,
    })
}

pub fn print_integration_hints(repo_root: &Path) {
    println!("Cursor soft integration (hook + skill):");
    println!("  1. Ensure hooks are enabled / workspace trusted");
    println!("  2. Install into a project:");
    println!("       lazytree cursor setup --target /path/to/project");
    println!("     (or keep this repo's `.cursor/hooks*` + `.cursor/skills`)");
    println!("  3. Cmd+N → sessionStart prepares a LazyTree session mapping");
    println!("  4. Use skill `/lazytree-session` so the agent edits only that root");
    println!("  5. Optional UI branch sync: `lazytree cursor open <session>`");
    println!();
    println!("Cloud Agents: sessionStart does NOT run.");
    println!("  Use: lazytree cursor bootstrap [--repo <id>] [--session-id <id>]");
    println!("  (creates/reuses session + writes .cursor/lazytree-sessions/<id>.json)");
    println!("  Bake CLI + skill into the image; gates still run in cloud.");
    println!();
    println!("Project root hint: {}", repo_root.display());
}

/// Copy LazyTree hook + skill files into `target` project's `.cursor/`.
pub fn install_into_project(bundled: &Path, target: &Path) -> Result<Vec<PathBuf>> {
    let src_hooks = bundled.join(".cursor");
    if !src_hooks.join("hooks.json").is_file() {
        bail!(
            "bundled Cursor assets missing at {}/.cursor/hooks.json",
            bundled.display()
        );
    }
    let dest = target.join(".cursor");
    let mut written = Vec::new();

    fs::create_dir_all(dest.join("hooks"))?;
    fs::create_dir_all(dest.join("skills/lazytree-session"))?;
    fs::create_dir_all(dest.join("lazytree-sessions"))?;

    let copies: &[(&str, &str)] = &[
        ("hooks.json", "hooks.json"),
        ("hooks/lazytree-session-start.sh", "hooks/lazytree-session-start.sh"),
        ("hooks/lazytree-pretool-gate.sh", "hooks/lazytree-pretool-gate.sh"),
        ("hooks/lazytree-shell-gate.sh", "hooks/lazytree-shell-gate.sh"),
        ("skills/lazytree-session/SKILL.md", "skills/lazytree-session/SKILL.md"),
    ];

    for (rel, out) in copies {
        let from = src_hooks.join(rel);
        let to = dest.join(out);
        if !from.is_file() {
            bail!("missing source asset: {}", from.display());
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&from, &to).with_context(|| {
            format!("copy {} -> {}", from.display(), to.display())
        })?;
        #[cfg(unix)]
        if out.ends_with(".sh") {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&to)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&to, perms)?;
        }
        written.push(to);
    }
    // Ignore ephemeral session maps in the consumer project.
    let ignore = dest.join(".gitignore");
    if !ignore.is_file() {
        fs::write(&ignore, "lazytree-sessions/\n")?;
        written.push(ignore);
    }
    Ok(written)
}

pub fn write_session_mapping(
    project: &Path,
    session_id: &str,
    session: &Session,
) -> Result<PathBuf> {
    let dir = project.join(".cursor/lazytree-sessions");
    fs::create_dir_all(&dir)?;
    // Keep maps out of git when possible.
    let ignore = project.join(".cursor/.gitignore");
    if !ignore.is_file() {
        let _ = fs::write(&ignore, "lazytree-sessions/\n");
    }
    let path = dir.join(format!("{session_id}.json"));
    let body = serde_json::json!({
        "session_id": session_id,
        "name": session.name,
        "root": session.root_path().display().to_string(),
        "branch": session.branch,
        "repository_id": session.repository_id,
        "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    fs::write(&path, serde_json::to_vec_pretty(&body)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn list_session_mappings(project: &Path) -> Result<Vec<serde_json::Value>> {
    let dir = project.join(".cursor/lazytree-sessions");
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut entries: Vec<_> = fs::read_dir(&dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&p)
            .with_context(|| format!("reading {}", p.display()))?;
        let mut v: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", p.display()))?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "mapping_path".into(),
                serde_json::Value::String(p.display().to_string()),
            );
            if let Some(root) = obj.get("root").and_then(|x| x.as_str()) {
                obj.insert(
                    "root_exists".into(),
                    serde_json::Value::Bool(Path::new(root).is_dir()),
                );
            }
        }
        out.push(v);
    }
    Ok(out)
}

pub fn get_session_mapping(project: &Path, session_id: &str) -> Result<serde_json::Value> {
    let path = project
        .join(".cursor/lazytree-sessions")
        .join(format!("{session_id}.json"));
    if !path.is_file() {
        bail!("no Cursor mapping for session id {session_id} under {}", project.display());
    }
    let raw = fs::read_to_string(&path)?;
    let mut v: serde_json::Value = serde_json::from_str(&raw)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "mapping_path".into(),
            serde_json::Value::String(path.display().to_string()),
        );
        if let Some(root) = obj.get("root").and_then(|x| x.as_str()) {
            obj.insert(
                "root_exists".into(),
                serde_json::Value::Bool(Path::new(root).is_dir()),
            );
        }
    }
    Ok(v)
}

/// Locate the directory that ships `.cursor/` assets (dev tree or next to binary).
pub fn find_bundled_assets() -> Result<PathBuf> {
    if let Ok(override_path) = std::env::var("LAZYTREE_CURSOR_ASSETS") {
        let p = PathBuf::from(override_path);
        if p.join(".cursor/hooks.json").is_file() {
            return Ok(p);
        }
    }
    // Walk up from cwd (common when developing LazyTree itself).
    let mut cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for _ in 0..6 {
        if cur.join(".cursor/hooks.json").is_file() {
            return Ok(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    // Next to the running binary (cargo: target/{debug,release}/lazytree → ../..).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for rel in [
                ".",
                "..",
                "../..",
                "../../..",
                "../share/lazytree",
                "../../share/lazytree",
            ] {
                let cand = dir.join(rel);
                if cand.join(".cursor/hooks.json").is_file() {
                    return Ok(cand.canonicalize().unwrap_or(cand));
                }
            }
        }
    }
    bail!(
        "could not find LazyTree .cursor assets; set LAZYTREE_CURSOR_ASSETS or run from the LazyTree checkout"
    )
}

pub fn resolve_editor() -> String {
    if let Ok(e) = std::env::var("LAZYTREE_EDITOR") {
        return e;
    }
    for cand in ["cursor", "code"] {
        if Command::new(cand)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return cand.to_string();
        }
    }
    "cursor".into()
}
