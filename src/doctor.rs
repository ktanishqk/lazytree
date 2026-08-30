//! Crash recovery / consistency checks (Milestone 3).

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::filesystem::is_mounted;
use crate::metadata::{atomic_write_json, Paths};
use crate::repository::RepositoryStore;
use crate::session::{FsState, LayerStatus, Lifecycle, SessionStore};


#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub issues: Vec<DoctorIssue>,
}

#[derive(Debug, Serialize)]
pub struct DoctorIssue {
    pub severity: String,
    pub code: String,
    pub message: String,
}

pub fn run_doctor(paths: &Paths, sessions: &SessionStore, repos: &RepositoryStore) -> Result<DoctorReport> {
    let mut issues = Vec::new();

    // Repositories
    for repo in repos.list()? {
        let base = PathBuf::from(&repo.base_path);
        if !base.is_dir() {
            issues.push(issue(
                "error",
                "missing_repo_base",
                format!("repository {} base missing: {}", repo.id, repo.base_path),
            ));
        }
        let objects = PathBuf::from(&repo.object_store);
        if !objects.is_dir() {
            issues.push(issue(
                "error",
                "missing_object_store",
                format!("repository {} object store missing: {}", repo.id, repo.object_store),
            ));
        }
    }

    // Sessions
    for mut session in sessions.list()? {
        let root = session.root_path();
        let dir = PathBuf::from(&session.session_dir);
        if !dir.is_dir() {
            issues.push(issue(
                "error",
                "missing_session_dir",
                format!("session {} directory missing", session.name),
            ));
            continue;
        }

        let mounted = is_mounted(&root).unwrap_or(false);
        if session.filesystem.state == FsState::Mounted && !mounted {
            issues.push(issue(
                "warn",
                "stale_mount_metadata",
                format!(
                    "session {} metadata says mounted but mount is absent; marking unmounted",
                    session.name
                ),
            ));
            session.filesystem.state = FsState::Unmounted;
            session.updated_at = chrono::Utc::now();
            let _ = atomic_write_json(&dir.join("metadata.json"), &session);
        } else if session.filesystem.state == FsState::Unmounted && mounted {
            issues.push(issue(
                "warn",
                "unexpected_mount",
                format!(
                    "session {} metadata says unmounted but mount is present",
                    session.name
                ),
            ));
        }

        let git_dir = dir.join("git");
        if session.git.state == LayerStatus::Ready
            && !git_dir.join("HEAD").exists()
            && session.filesystem.state != FsState::Archived
            && session.lifecycle != Lifecycle::Archived
        {
            issues.push(issue(
                "error",
                "missing_git_dir",
                format!("session {} missing Git metadata", session.name),
            ));
        }

        // repository back-reference
        if repos.get(&session.repository_id).is_err() {
            issues.push(issue(
                "error",
                "missing_repository",
                format!(
                    "session {} references missing repository {}",
                    session.name, session.repository_id
                ),
            ));
        }
    }

    // Orphan upperdirs / session dirs without metadata
    let sessions_dir = paths.sessions_dir();
    if sessions_dir.is_dir() {
        for entry in fs::read_dir(sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".names") {
                continue;
            }
            if path.is_dir() && !path.join("metadata.json").exists() {
                issues.push(issue(
                    "warn",
                    "orphan_session_dir",
                    format!(
                        "session directory without metadata: {} (incomplete create? try destroy --force or remove the dir)",
                        path.display()
                    ),
                ));
            }
        }
    }

    // Cursor soft-integration health (advisory; cwd may not be a project).
    if let Ok(cwd) = std::env::current_dir() {
        let hooks = cwd.join(".cursor/hooks.json");
        if hooks.is_file() {
            match fs::read_to_string(&hooks) {
                Ok(text) if text.contains("lazytree-session-start") => {}
                Ok(_) => issues.push(issue(
                    "info",
                    "cursor_hooks_unrelated",
                    format!(
                        ".cursor/hooks.json exists but does not reference LazyTree hooks ({})",
                        hooks.display()
                    ),
                )),
                Err(e) => issues.push(issue(
                    "warn",
                    "cursor_hooks_unreadable",
                    format!("cannot read {}: {e}", hooks.display()),
                )),
            }
            let skill = cwd.join(".cursor/skills/lazytree-session/SKILL.md");
            if !skill.is_file() {
                issues.push(issue(
                    "info",
                    "cursor_skill_missing",
                    "LazyTree hooks present but skill .cursor/skills/lazytree-session/SKILL.md is missing"
                        .into(),
                ));
            }
        }
        let maps = cwd.join(".cursor/lazytree-sessions");
        if maps.is_dir() {
            let mut stale = 0usize;
            if let Ok(rd) = fs::read_dir(&maps) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    if let Ok(raw) = fs::read_to_string(&p) {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                            if let Some(root) = v.get("root").and_then(|x| x.as_str()) {
                                if !PathBuf::from(root).is_dir() {
                                    stale += 1;
                                }
                            }
                        }
                    }
                }
            }
            if stale > 0 {
                issues.push(issue(
                    "warn",
                    "cursor_stale_mappings",
                    format!(
                        "{stale} Cursor LazyTree mapping(s) under {} point at missing roots",
                        maps.display()
                    ),
                ));
            }
        }
    }

    // Host capabilities (informational).
    {
        use std::process::Command;
        let fuse = Command::new("fuse-overlayfs")
            .arg("--help")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !fuse {
            issues.push(issue(
                "warn",
                "fuse_overlayfs_missing",
                "fuse-overlayfs not found on PATH; session mounts may fail without kernel OverlayFS"
                    .into(),
            ));
        }
        let sudo_n = Command::new("sudo")
            .args(["-n", "true"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !sudo_n {
            issues.push(issue(
                "info",
                "sudo_n_unavailable",
                "passwordless sudo (-n) unavailable; privileged fuse/overlay mounts will not work in locked-down VMs"
                    .into(),
            ));
        }
        issues.push(issue(
            "info",
            "lazytree_home",
            format!("LAZYTREE_HOME={}", paths.home.display()),
        ));
    }

    let ok = !issues.iter().any(|i| i.severity == "error");
    Ok(DoctorReport { ok, issues })
}

fn issue(severity: &str, code: &str, message: String) -> DoctorIssue {
    DoctorIssue {
        severity: severity.into(),
        code: code.into(),
        message,
    }
}
