use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::filesystem::{self, MountRequest};
use crate::git::{self, GitSetup};
use crate::locking;
use crate::metadata::{
    atomic_write_json, load_config, read_json, remember_mount_success, FilesystemBackendKind,
    Paths,
};
use crate::repository::RepositoryStore;
use crate::runtime::{LocalRuntimeBackend, RuntimeBackend};
use crate::semantic::SemanticPaths;

thread_local! {
    static LAST_CREATE_TIMINGS: RefCell<Option<CreateTimings>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTimings {
    pub filesystem_ms: u64,
    pub git_ms: u64,
    pub total_ms: u64,
}

pub fn take_last_create_timings() -> Option<CreateTimings> {
    LAST_CREATE_TIMINGS.with(|t| t.borrow_mut().take())
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    paths: Paths,
    repos: RepositoryStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub repository_id: String,
    pub base_revision: String,
    pub branch: String,
    pub filesystem: FilesystemMeta,
    pub git: LayerState,
    pub semantic: LayerState,
    pub runtime: RuntimeMeta,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Absolute path to session directory under LazyTree home.
    pub session_dir: String,
    #[serde(default)]
    pub lifecycle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemMeta {
    pub backend: FilesystemBackendKind,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerState {
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMeta {
    pub backend: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub dirty: bool,
    pub unexported_commits: u64,
    pub filesystem_state: String,
    pub filesystem_backend: FilesystemBackendKind,
    pub git_state: String,
    pub semantic_state: String,
    pub runtime_state: String,
    pub lifecycle: String,
    pub upper_files: u64,
    pub filesystem_bytes_written: u64,
    pub root: String,
    pub shared_cache: String,
    pub session_cache: String,
    pub age_seconds: i64,
}

impl Session {
    pub fn root_path(&self) -> PathBuf {
        PathBuf::from(&self.session_dir).join("fs").join("root")
    }

    pub fn upper_path(&self) -> PathBuf {
        PathBuf::from(&self.session_dir).join("fs").join("upper")
    }

    pub fn git_dir(&self) -> PathBuf {
        PathBuf::from(&self.session_dir).join("git")
    }
}

struct CreatePlan {
    id: String,
    session_dir: PathBuf,
    upper: PathBuf,
    work: PathBuf,
    root: PathBuf,
    git_dir: PathBuf,
    repo_id: String,
    repo_base_path: PathBuf,
    repo_base_commit: String,
    object_store: PathBuf,
    seed_index: Option<PathBuf>,
    base_revision: String,
    branch: String,
    preferred: FilesystemBackendKind,
    last_working: Option<FilesystemBackendKind>,
    needs_sudo: Option<bool>,
    name: String,
}

impl SessionStore {
    pub fn new(paths: Paths, repos: RepositoryStore) -> Self {
        Self { paths, repos }
    }

    pub fn create(
        &self,
        name: &str,
        repo_ref: Option<&str>,
        from: Option<&str>,
    ) -> Result<Session> {
        validate_name(name)?;

        // Hold the sessions lock only while reserving the name and laying out dirs.
        // Mount + git setup run unlocked so parallel creates are not serialized on FUSE.
        let plan = {
            let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
            if self.find_by_name(name)?.is_some() || self.name_claimed(name)? {
                bail!("session name already exists: {name}");
            }

            let repo = match repo_ref {
                Some(r) => self.repos.get(r)?,
                None => {
                    let list = self.repos.list()?;
                    match list.len() {
                        0 => bail!("no repositories registered; run `lazytree repo add <path>`"),
                        1 => list.into_iter().next().unwrap(),
                        _ => bail!("multiple repositories registered; pass --repo <id>"),
                    }
                }
            };

            let id = format!(
                "session_{}",
                &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
            );
            let session_dir = self.paths.session_dir(&id);
            let fs_dir = session_dir.join("fs");
            let upper = fs_dir.join("upper");
            let work = fs_dir.join("work");
            let root = fs_dir.join("root");
            let git_dir = session_dir.join("git");

            fs::create_dir_all(&upper)?;
            fs::create_dir_all(&work)?;
            fs::create_dir_all(&root)?;
            fs::create_dir_all(&git_dir)?;
            fs::create_dir_all(session_dir.join("runtime"))?;
            let semantic = SemanticPaths::for_session(
                &self.paths.repo_dir(&repo.id),
                &session_dir,
            );
            semantic.ensure()?;
            self.claim_name(name, &id)?;

            let cfg = load_config(&self.paths)?;
            CreatePlan {
                id,
                session_dir,
                upper,
                work,
                root,
                git_dir,
                repo_id: repo.id.clone(),
                repo_base_path: PathBuf::from(&repo.base_path),
                repo_base_commit: repo.base_commit.clone(),
                object_store: PathBuf::from(&repo.object_store),
                seed_index: repo.seed_index.as_ref().map(PathBuf::from),
                base_revision: from.unwrap_or(repo.base_commit.as_str()).to_string(),
                branch: format!("lazytree/{name}"),
                preferred: cfg.filesystem_backend,
                last_working: cfg.last_working_backend,
                needs_sudo: cfg.mount_needs_sudo,
                name: name.to_string(),
            }
        };

        let mount_start = std::time::Instant::now();
        let mounted = match filesystem::mount_session(MountRequest {
            lowerdir: &plan.repo_base_path,
            upperdir: &plan.upper,
            workdir: &plan.work,
            merged: &plan.root,
            preferred: plan.preferred,
            last_working: plan.last_working,
            needs_sudo: plan.needs_sudo,
        }) {
            Ok(m) => m,
            Err(err) => {
                let _ = self.cleanup_failed_create(&plan);
                return Err(err).context(format!("mounting session {}", plan.name));
            }
        };
        let filesystem_ms = mount_start.elapsed().as_millis() as u64;
        let _ = remember_mount_success(&self.paths, mounted.backend, mounted.used_sudo);

        let git_start = std::time::Instant::now();
        if let Err(err) = git::setup_session_git(&GitSetup {
            git_dir: plan.git_dir.clone(),
            work_tree: plan.root.clone(),
            branch: plan.branch.clone(),
            base_revision: plan.base_revision.clone(),
            object_store: plan.object_store.clone(),
            seed_index: plan.seed_index.clone(),
            seed_commit: Some(plan.repo_base_commit.clone()),
        }) {
            let _ = filesystem::umount_with_backend(&plan.root, Some(mounted.backend));
            let _ = self.cleanup_failed_create(&plan);
            return Err(err).context("setting up session git");
        }
        let git_ms = git_start.elapsed().as_millis() as u64;

        let now = Utc::now();
        let session = Session {
            version: 1,
            id: plan.id.clone(),
            name: plan.name.clone(),
            repository_id: plan.repo_id.clone(),
            base_revision: plan.base_revision.clone(),
            branch: plan.branch.clone(),
            filesystem: FilesystemMeta {
                backend: mounted.backend,
                state: "mounted".into(),
            },
            git: LayerState {
                state: "ready".into(),
            },
            semantic: LayerState {
                state: "inherited".into(),
            },
            runtime: RuntimeMeta {
                backend: "local".into(),
                state: "none".into(),
            },
            created_at: now,
            updated_at: now,
            session_dir: plan.session_dir.display().to_string(),
            lifecycle: "ready".into(),
        };
        atomic_write_json(&plan.session_dir.join("metadata.json"), &session)?;
        LAST_CREATE_TIMINGS.with(|t| {
            *t.borrow_mut() = Some(CreateTimings {
                filesystem_ms,
                git_ms,
                total_ms: filesystem_ms + git_ms,
            });
        });
        // Warm FUSE page cache + git index for first `git status` without
        // blocking create. Cold status on fuse-overlayfs can be 10× warm.
        {
            let root = session.root_path();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("git")
                    .args(["-C"])
                    .arg(&root)
                    .args(["status", "-sb", "--porcelain"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            });
        }
        Ok(session)
    }

    pub fn list(&self) -> Result<Vec<Session>> {
        let mut out = Vec::new();
        let dir = self.paths.sessions_dir();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            // Skip the .names reservation directory.
            if path.file_name().is_some_and(|n| n == ".names") {
                continue;
            }
            let meta = path.join("metadata.json");
            if meta.exists() {
                out.push(read_json(&meta)?);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn get(&self, name_or_id: &str) -> Result<Session> {
        if let Some(s) = self.find_by_name(name_or_id)? {
            return Ok(s);
        }
        for s in self.list()? {
            if s.id == name_or_id {
                return Ok(s);
            }
        }
        bail!("session not found: {name_or_id}");
    }

    pub fn status(&self, name_or_id: &str) -> Result<SessionStatus> {
        let session = self.get(name_or_id)?;
        let root = session.root_path();
        let upper = session.upper_path();

        let dirty = if root.exists() && session.filesystem.state == "mounted" {
            !git_porcelain(&root)?.is_empty()
        } else {
            false
        };

        let unexported = if session.git_dir().join("HEAD").exists() {
            count_unexported(&session)?
        } else {
            0
        };

        let (upper_files, bytes) = dir_stats(&upper)?;
        let age = (Utc::now() - session.created_at).num_seconds();
        let semantic = self.semantic_paths(&session)?;

        Ok(SessionStatus {
            id: session.id.clone(),
            name: session.name.clone(),
            branch: session.branch.clone(),
            dirty,
            unexported_commits: unexported,
            filesystem_state: session.filesystem.state.clone(),
            filesystem_backend: session.filesystem.backend,
            git_state: session.git.state.clone(),
            semantic_state: session.semantic.state.clone(),
            runtime_state: session.runtime.state.clone(),
            lifecycle: session.lifecycle.clone(),
            upper_files,
            filesystem_bytes_written: bytes,
            root: root.display().to_string(),
            shared_cache: semantic.shared.display().to_string(),
            session_cache: semantic.session_writable.display().to_string(),
            age_seconds: age,
        })
    }

    pub fn semantic_paths(&self, session: &Session) -> Result<SemanticPaths> {
        let paths = SemanticPaths::for_session(
            &self.paths.repo_dir(&session.repository_id),
            Path::new(&session.session_dir),
        );
        paths.ensure()?;
        Ok(paths)
    }

    #[allow(dead_code)]
    pub fn exec(&self, name_or_id: &str, argv: &[String]) -> Result<i32> {
        self.exec_with(name_or_id, argv, &crate::runtime::ExecOptions::default())
    }

    pub fn exec_with(
        &self,
        name_or_id: &str,
        argv: &[String],
        opts: &crate::runtime::ExecOptions,
    ) -> Result<i32> {
        let session = self.get(name_or_id)?;
        let semantic = self.semantic_paths(&session)?;
        LocalRuntimeBackend.exec(&session, &semantic, argv, opts)
    }

    pub fn cache_promote(&self, name_or_id: &str) -> Result<()> {
        let session = self.get(name_or_id)?;
        let semantic = self.semantic_paths(&session)?;
        crate::runtime::promote_session_target_to_shared(&semantic)
    }

    pub fn cache_seed(&self, name_or_id: &str) -> Result<bool> {
        let session = self.get(name_or_id)?;
        let semantic = self.semantic_paths(&session)?;
        crate::runtime::seed_session_target_from_shared(&semantic)
    }

    pub fn diff(&self, name_or_id: &str) -> Result<String> {
        let session = self.get(name_or_id)?;
        ensure_active_fs(&session)?;
        let out = Command::new("git")
            .args(["-C"])
            .arg(session.root_path())
            .args(["diff", "HEAD"])
            .output()
            .context("git diff")?;
        if !out.status.success() {
            bail!(
                "git diff failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        let staged = Command::new("git")
            .args(["-C"])
            .arg(session.root_path())
            .args(["diff", "--cached"])
            .output()
            .context("git diff --cached")?;
        if staged.status.success() && !staged.stdout.is_empty() {
            text.push_str("\n# staged\n");
            text.push_str(&String::from_utf8_lossy(&staged.stdout));
        }
        Ok(text)
    }

    pub fn archive(&self, name_or_id: &str) -> Result<Session> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
        let mut session = self.get(name_or_id)?;
        if session.lifecycle == "archived" {
            bail!("session already archived: {}", session.name);
        }
        ensure_active_fs(&session)?;

        let repo = self.repos.get(&session.repository_id)?;
        // Publish branch into the user's source repository.
        let status = Command::new("git")
            .arg("--git-dir")
            .arg(session.git_dir())
            .args([
                "push",
                &repo.source_path,
                &format!("HEAD:refs/heads/{}", session.branch),
            ])
            .output()
            .context("git push to source repository")?;
        if !status.status.success() {
            bail!(
                "failed to publish branch to {}: {}",
                repo.source_path,
                String::from_utf8_lossy(&status.stderr).trim()
            );
        }

        let root = session.root_path();
        filesystem::umount_with_backend(&root, Some(session.filesystem.backend))?;

        // Drop expensive ephemeral layers; keep metadata.
        let fs_dir = PathBuf::from(&session.session_dir).join("fs");
        for sub in ["upper", "work", "root"] {
            let p = fs_dir.join(sub);
            if p.exists() {
                let _ = fs::remove_dir_all(&p);
                let _ = fs::create_dir_all(&p);
            }
        }
        let git_dir = session.git_dir();
        if git_dir.exists() {
            fs::remove_dir_all(&git_dir)?;
        }
        let runtime = PathBuf::from(&session.session_dir).join("runtime");
        if runtime.exists() {
            let _ = fs::remove_dir_all(&runtime);
            let _ = fs::create_dir_all(&runtime);
        }
        let semantic = PathBuf::from(&session.session_dir)
            .join("semantic")
            .join("writable");
        if semantic.exists() {
            let _ = fs::remove_dir_all(&semantic);
            let _ = fs::create_dir_all(&semantic);
        }

        session.filesystem.state = "archived".into();
        session.git.state = "published".into();
        session.semantic.state = "none".into();
        session.runtime.state = "none".into();
        session.lifecycle = "archived".into();
        session.updated_at = Utc::now();
        atomic_write_json(
            &PathBuf::from(&session.session_dir).join("metadata.json"),
            &session,
        )?;
        Ok(session)
    }

    pub fn destroy(&self, name_or_id: &str, force: bool) -> Result<()> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
        let session = self.get(name_or_id)?;

        if session.lifecycle != "archived" && !force {
            if session.filesystem.state == "mounted" {
                let dirty = !git_porcelain(&session.root_path())?.is_empty();
                let unexported = count_unexported(&session)?;
                if dirty || unexported > 0 {
                    bail!(
                        "workspace has {}{}{}\nUse:\n  lazytree diff {}\n  lazytree archive {}\n  lazytree destroy {} --force",
                        if dirty { "uncommitted changes" } else { "" },
                        if dirty && unexported > 0 { " and " } else { "" },
                        if unexported > 0 {
                            "commits that have not been exported"
                        } else {
                            ""
                        },
                        session.name,
                        session.name,
                        session.name
                    );
                }
            }
        }

        let root = session.root_path();
        let backend = Some(session.filesystem.backend);
        if force {
            // Always attempt tools — mount detection can lag behind privileged FUSE.
            let _ = filesystem::umount_force(&root, backend);
        } else if filesystem::is_mounted(&root)? {
            filesystem::umount_with_backend(&root, backend)
                .with_context(|| format!("unmounting {}", root.display()))?;
        }

        let dir = PathBuf::from(&session.session_dir);
        remove_session_tree(&dir)?;
        let _ = self.release_name(&session.name);
        Ok(())
    }

    fn find_by_name(&self, name: &str) -> Result<Option<Session>> {
        for s in self.list()? {
            if s.name == name {
                return Ok(Some(s));
            }
        }
        Ok(None)
    }

    fn name_claim_path(&self, name: &str) -> PathBuf {
        self.paths.session_name_claim_dir().join(name)
    }

    fn name_claimed(&self, name: &str) -> Result<bool> {
        Ok(self.name_claim_path(name).exists())
    }

    fn claim_name(&self, name: &str, id: &str) -> Result<()> {
        let dir = self.paths.session_name_claim_dir();
        fs::create_dir_all(&dir)?;
        fs::write(self.name_claim_path(name), id)
            .with_context(|| format!("claiming session name {name}"))?;
        Ok(())
    }

    fn release_name(&self, name: &str) -> Result<()> {
        let p = self.name_claim_path(name);
        if p.exists() {
            fs::remove_file(&p)?;
        }
        Ok(())
    }

    fn cleanup_failed_create(&self, plan: &CreatePlan) -> Result<()> {
        let _ = filesystem::umount_force(&plan.root, None);
        let _ = remove_session_tree(&plan.session_dir);
        let _ = self.release_name(&plan.name);
        Ok(())
    }
}

/// Remove a session directory tree. Privileged fuse-overlayfs leaves root-owned
/// files under `fs/work`; fall back to `sudo rm -rf` when needed.
fn remove_session_tree(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    if fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }
    // Overlay workdirs from sudo fuse-overlayfs are often root-owned.
    let _ = Command::new("sudo")
        .args(["-n", "rm", "-rf", "--"])
        .arg(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("sudo rm -rf session dir")?;
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))
}

fn ensure_active_fs(session: &Session) -> Result<()> {
    if session.lifecycle == "archived" || session.filesystem.state != "mounted" {
        bail!("session {} is not an active mounted workspace", session.name);
    }
    if !filesystem::is_mounted(&session.root_path())? {
        bail!(
            "session {} is not mounted; run `lazytree doctor`",
            session.name
        );
    }
    Ok(())
}

fn git_porcelain(work_tree: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(work_tree)
        .args(["status", "--porcelain"])
        .output()
        .context("git status")?;
    if !out.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn count_unexported(session: &Session) -> Result<u64> {
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(session.git_dir())
        .args([
            "rev-list",
            "--count",
            &format!("{}..HEAD", session.base_revision),
        ])
        .output()
        .context("git rev-list")?;
    if !out.status.success() {
        // If rev-list fails (e.g. missing git), treat as unknown/unsafe.
        bail!(
            "git rev-list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let n = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>()?;
    Ok(n)
}

fn dir_stats(path: &Path) -> Result<(u64, u64)> {
    if !path.exists() {
        return Ok((0, 0));
    }
    let mut files = 0u64;
    let mut bytes = 0u64;
    fn walk(p: &Path, files: &mut u64, bytes: &mut u64) -> Result<()> {
        if p.is_file() {
            *files += 1;
            *bytes += fs::metadata(p)?.len();
            return Ok(());
        }
        if p.is_dir() {
            for entry in fs::read_dir(p)? {
                let entry = entry?;
                walk(&entry.path(), files, bytes)?;
            }
        }
        Ok(())
    }
    walk(path, &mut files, &mut bytes)?;
    Ok((files, bytes))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("session name must not be empty");
    }
    if name.contains('/') || name.contains('\0') {
        bail!("session name must not contain '/' or NUL");
    }
    Ok(())
}
