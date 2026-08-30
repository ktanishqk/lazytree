//! Crash recovery / consistency checks (Milestone 3).

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::filesystem::is_mounted;
use crate::metadata::{atomic_write_json, Paths};
use crate::repository::RepositoryStore;
use crate::session::{Session, SessionStore};

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
        if session.filesystem.state == "mounted" && !mounted {
            issues.push(issue(
                "warn",
                "stale_mount_metadata",
                format!(
                    "session {} metadata says mounted but mount is absent; marking unmounted",
                    session.name
                ),
            ));
            session.filesystem.state = "unmounted".into();
            session.updated_at = chrono::Utc::now();
            let _ = atomic_write_json(&dir.join("metadata.json"), &session);
        } else if session.filesystem.state == "unmounted" && mounted {
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
        if session.git.state == "ready" && !git_dir.join("HEAD").exists() && session.filesystem.state != "archived" {
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
            if entry.path().is_dir() && !entry.path().join("metadata.json").exists() {
                issues.push(issue(
                    "warn",
                    "orphan_session_dir",
                    format!("session directory without metadata: {}", entry.path().display()),
                ));
            }
        }
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

#[allow(dead_code)]
pub fn remount_if_needed(session: &Session, repos: &RepositoryStore) -> Result<()> {
    let _ = (session, repos);
    Ok(())
}
