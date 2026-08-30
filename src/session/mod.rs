use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
use crate::runtime;
use crate::semantic::SemanticPaths;
use crate::util::short_id;

#[derive(Debug, Clone, Serialize)]
pub struct CreateTimings {
    pub filesystem_ms: u64,
    pub git_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    paths: Paths,
    repos: RepositoryStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    #[default]
    Ready,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsState {
    Mounted,
    Unmounted,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerStatus {
    Ready,
    Inherited,
    Published,
    #[serde(rename = "none")]
    None,
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
    pub session_dir: PathBuf,
    #[serde(default)]
    pub lifecycle: Lifecycle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemMeta {
    pub backend: FilesystemBackendKind,
    pub state: FsState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerState {
    pub state: LayerStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMeta {
    pub backend: String,
    pub state: LayerStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub dirty: bool,
    pub unexported_commits: u64,
    pub filesystem_state: FsState,
    pub filesystem_backend: FilesystemBackendKind,
    pub git_state: LayerStatus,
    pub semantic_state: LayerStatus,
    pub runtime_state: LayerStatus,
    pub lifecycle: Lifecycle,
    pub upper_files: u64,
    pub filesystem_bytes_written: u64,
    pub root: String,
    pub shared_cache: String,
    pub session_cache: String,
    pub age_seconds: i64,
}

impl Session {
    pub fn root_path(&self) -> PathBuf {
        self.session_dir.join("fs/root")
    }

    pub fn upper_path(&self) -> PathBuf {
        self.session_dir.join("fs/upper")
    }

    pub fn git_dir(&self) -> PathBuf {
        self.session_dir.join("git")
    }

    pub fn is_active_mount(&self) -> bool {
        self.lifecycle == Lifecycle::Ready && self.filesystem.state == FsState::Mounted
    }
}

struct CreatePlan {
    id: String,
    name: String,
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
    user_name: Option<String>,
    user_email: Option<String>,
    base_revision: String,
    branch: String,
    preferred: FilesystemBackendKind,
    last_working: Option<FilesystemBackendKind>,
    needs_sudo: Option<bool>,
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
    ) -> Result<(Session, CreateTimings)> {
        validate_name(name)?;

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
        let cfg = load_config(&self.paths)?;

        let id = format!("session_{}", short_id());
        let session_dir = self.paths.session_dir(&id);
        let fs_dir = session_dir.join("fs");
        let plan = CreatePlan {
            id,
            name: name.to_string(),
            upper: fs_dir.join("upper"),
            work: fs_dir.join("work"),
            root: fs_dir.join("root"),
            git_dir: session_dir.join("git"),
            session_dir,
            repo_id: repo.id.clone(),
            repo_base_path: PathBuf::from(&repo.base_path),
            repo_base_commit: repo.base_commit.clone(),
            object_store: PathBuf::from(&repo.object_store),
            seed_index: repo.seed_index.as_ref().map(PathBuf::from),
            user_name: repo.user_name.clone(),
            user_email: repo.user_email.clone(),
            base_revision: from.unwrap_or(repo.base_commit.as_str()).to_string(),
            branch: format!("lazytree/{name}"),
            preferred: cfg.filesystem_backend,
            last_working: cfg.last_working_backend,
            needs_sudo: cfg.mount_needs_sudo,
        };

        // Lock only for name reservation — mount/git run unlocked.
        {
            let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
            if self.name_claimed(&plan.name)? {
                bail!("session name already exists: {}", plan.name);
            }
            self.claim_name(&plan.name, &plan.id)?;
        }

        fs::create_dir_all(&plan.git_dir)?;
        fs::create_dir_all(plan.session_dir.join("runtime"))?;
        SemanticPaths::for_session(&self.paths.repo_dir(&plan.repo_id), &plan.session_dir)
            .ensure_roots()?;

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
            git_dir: &plan.git_dir,
            work_tree: &plan.root,
            branch: &plan.branch,
            base_revision: &plan.base_revision,
            object_store: &plan.object_store,
            seed_index: plan.seed_index.as_deref(),
            seed_commit: Some(plan.repo_base_commit.as_str()),
            user_name: plan.user_name.as_deref(),
            user_email: plan.user_email.as_deref(),
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
            repository_id: plan.repo_id,
            base_revision: plan.base_revision,
            branch: plan.branch,
            filesystem: FilesystemMeta {
                backend: mounted.backend,
                state: FsState::Mounted,
            },
            git: LayerState {
                state: LayerStatus::Ready,
            },
            semantic: LayerState {
                state: LayerStatus::Inherited,
            },
            runtime: RuntimeMeta {
                backend: "local".into(),
                state: LayerStatus::None,
            },
            created_at: now,
            updated_at: now,
            session_dir: plan.session_dir.clone(),
            lifecycle: Lifecycle::Ready,
        };
        atomic_write_json(&plan.session_dir.join("metadata.json"), &session)?;

        let timings = CreateTimings {
            filesystem_ms,
            git_ms,
            total_ms: filesystem_ms + git_ms,
        };

        if std::env::var_os("LAZYTREE_WARM_STATUS").as_deref() != Some(std::ffi::OsStr::new("0"))
        {
            let root = session.root_path();
            std::thread::spawn(move || {
                let _ = Command::new("git")
                    .args(["-C"])
                    .arg(&root)
                    .args(["status", "-sb", "--porcelain"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            });
        }

        Ok((session, timings))
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
        for s in self.list()? {
            if s.name == name_or_id || s.id == name_or_id {
                return Ok(s);
            }
        }
        bail!("session not found: {name_or_id}");
    }

    pub fn status(&self, name_or_id: &str) -> Result<SessionStatus> {
        let session = self.get(name_or_id)?;
        let root = session.root_path();
        let upper = session.upper_path();

        let dirty = if session.is_active_mount() && root.exists() {
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
            id: session.id,
            name: session.name,
            branch: session.branch,
            dirty,
            unexported_commits: unexported,
            filesystem_state: session.filesystem.state,
            filesystem_backend: session.filesystem.backend,
            git_state: session.git.state,
            semantic_state: session.semantic.state,
            runtime_state: session.runtime.state,
            lifecycle: session.lifecycle,
            upper_files,
            filesystem_bytes_written: bytes,
            root: root.display().to_string(),
            shared_cache: semantic.shared.display().to_string(),
            session_cache: semantic.session_writable.display().to_string(),
            age_seconds: age,
        })
    }

    pub fn semantic_paths(&self, session: &Session) -> Result<SemanticPaths> {
        let paths =
            SemanticPaths::for_session(&self.paths.repo_dir(&session.repository_id), &session.session_dir);
        paths.ensure_roots()?;
        Ok(paths)
    }

    pub fn exec_with(
        &self,
        name_or_id: &str,
        argv: &[String],
        opts: &runtime::ExecOptions,
    ) -> Result<i32> {
        let session = self.get(name_or_id)?;
        let semantic = self.semantic_paths(&session)?;
        runtime::exec(&session, &semantic, argv, opts)
    }

    pub fn cache_promote(&self, name_or_id: &str) -> Result<()> {
        let session = self.get(name_or_id)?;
        let semantic = self.semantic_paths(&session)?;
        runtime::promote_session_target_to_shared(&semantic)
    }

    pub fn cache_seed(&self, name_or_id: &str) -> Result<bool> {
        let session = self.get(name_or_id)?;
        let semantic = self.semantic_paths(&session)?;
        runtime::seed_session_target_from_shared(&semantic)
    }

    pub fn diff(&self, name_or_id: &str) -> Result<String> {
        let session = self.get(name_or_id)?;
        ensure_active_fs(&session)?;
        let root = session.root_path();
        let out = git::git_c(&root, &["diff", "HEAD"])?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        if let Ok(staged) = git::git_c(&root, &["diff", "--cached"]) {
            if !staged.stdout.is_empty() {
                text.push_str("\n# staged\n");
                text.push_str(&String::from_utf8_lossy(&staged.stdout));
            }
        }
        Ok(text)
    }

    pub fn archive(&self, name_or_id: &str) -> Result<Session> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
        let mut session = self.get(name_or_id)?;
        if session.lifecycle == Lifecycle::Archived {
            bail!("session already archived: {}", session.name);
        }
        ensure_active_fs(&session)?;

        let repo = self.repos.get(&session.repository_id)?;
        let status = git::run_git(
            &session.git_dir(),
            None,
            &[
                "push",
                repo.source_path.as_str(),
                &format!("HEAD:refs/heads/{}", session.branch),
            ],
        );
        if let Err(e) = status {
            bail!("failed to publish branch to {}: {e}", repo.source_path);
        }

        let root = session.root_path();
        filesystem::umount_with_backend(&root, Some(session.filesystem.backend))?;

        let fs_dir = session.session_dir.join("fs");
        for sub in ["upper", "work", "root"] {
            let p = fs_dir.join(sub);
            let _ = fs::remove_dir_all(&p);
            let _ = fs::create_dir_all(&p);
        }
        let git_dir = session.git_dir();
        if git_dir.exists() {
            fs::remove_dir_all(&git_dir)?;
        }
        for p in [
            session.session_dir.join("runtime"),
            session.session_dir.join("semantic/writable"),
        ] {
            if p.exists() {
                let _ = fs::remove_dir_all(&p);
                let _ = fs::create_dir_all(&p);
            }
        }

        session.filesystem.state = FsState::Archived;
        session.git.state = LayerStatus::Published;
        session.semantic.state = LayerStatus::None;
        session.runtime.state = LayerStatus::None;
        session.lifecycle = Lifecycle::Archived;
        session.updated_at = Utc::now();
        atomic_write_json(&session.session_dir.join("metadata.json"), &session)?;
        Ok(session)
    }

    pub fn destroy(&self, name_or_id: &str, force: bool) -> Result<()> {
        let _lock = locking::try_lock(&self.paths.locks_dir().join("sessions.lock"))?;
        let session = self.get(name_or_id)?;

        if session.lifecycle != Lifecycle::Archived && !force {
            if session.filesystem.state == FsState::Mounted {
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
            let _ = filesystem::umount_force(&root, backend);
        } else if filesystem::is_mounted(&root)? {
            filesystem::umount_with_backend(&root, backend)
                .with_context(|| format!("unmounting {}", root.display()))?;
        }

        remove_session_tree(&session.session_dir)?;
        let _ = self.release_name(&session.name);
        Ok(())
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
        fs::write(self.name_claim_path(name), id)?;
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

fn remove_session_tree(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    if fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }
    let _ = Command::new("sudo")
        .args(["-n", "rm", "-rf", "--"])
        .arg(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if dir.exists() {
        fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    Ok(())
}

fn ensure_active_fs(session: &Session) -> Result<()> {
    if !session.is_active_mount() {
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
    let out = git::git_c(work_tree, &["status", "--porcelain"])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn count_unexported(session: &Session) -> Result<u64> {
    let out = git::run_git(
        &session.git_dir(),
        None,
        &[
            "rev-list",
            "--count",
            &format!("{}..HEAD", session.base_revision),
        ],
    )?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().parse()?)
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
                walk(&entry?.path(), files, bytes)?;
            }
        }
        Ok(())
    }
    walk(path, &mut files, &mut bytes)?;
    Ok((files, bytes))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\0') {
        bail!("session name must be non-empty and not contain '/' or NUL");
    }
    Ok(())
}
