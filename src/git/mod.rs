//! Session-local Git metadata with shared object database (Milestone 2).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct GitSetup {
    pub git_dir: PathBuf,
    pub work_tree: PathBuf,
    pub branch: String,
    pub base_revision: String,
    pub object_store: PathBuf,
}

/// Initialize private Git metadata for a session and point the work tree at it
/// via a `.git` gitdir file (standards-compliant discovery).
pub fn setup_session_git(cfg: &GitSetup) -> Result<()> {
    fs::create_dir_all(&cfg.git_dir)?;
    fs::create_dir_all(cfg.git_dir.join("objects/info"))?;
    fs::create_dir_all(cfg.git_dir.join("refs/heads"))?;

    // Ensure the shared object store exists.
    if !cfg.object_store.is_dir() {
        bail!(
            "shared object store missing: {}",
            cfg.object_store.display()
        );
    }

    run_git(
        &cfg.git_dir,
        None,
        &["init", "--quiet"],
    )?;

    // Share immutable objects with the registered repository.
    let alternates = cfg.git_dir.join("objects/info/alternates");
    let abs_objects = fs::canonicalize(&cfg.object_store)
        .with_context(|| format!("canonicalizing {}", cfg.object_store.display()))?;
    fs::write(&alternates, format!("{}\n", abs_objects.display()))?;

    // Resolve base revision through the shared object store.
    let base = resolve_commit(&cfg.git_dir, &cfg.base_revision)?;
    let tree = rev_parse(&cfg.git_dir, &format!("{base}^{{tree}}"))?;

    run_git(
        &cfg.git_dir,
        None,
        &["update-ref", &format!("refs/heads/{}", cfg.branch), &base],
    )?;
    run_git(
        &cfg.git_dir,
        None,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{}", cfg.branch)],
    )?;
    run_git(&cfg.git_dir, None, &["read-tree", &tree])?;

    // Mask lowerdir `.git` (directory) then install gitdir indirection.
    let overlay_git = cfg.work_tree.join(".git");
    if overlay_git.exists() {
        if overlay_git.is_dir() {
            fs::remove_dir_all(&overlay_git)
                .with_context(|| format!("whiteout {}", overlay_git.display()))?;
        } else {
            fs::remove_file(&overlay_git)?;
        }
    }

    let abs_git = fs::canonicalize(&cfg.git_dir)
        .with_context(|| format!("canonicalizing {}", cfg.git_dir.display()))?;
    fs::write(&overlay_git, format!("gitdir: {}\n", abs_git.display()))
        .with_context(|| format!("writing {}", overlay_git.display()))?;

    // Disable bare; work tree is discovered via the `.git` file.
    run_git(&cfg.git_dir, None, &["config", "core.bare", "false"])?;
    run_git(
        &cfg.git_dir,
        None,
        &[
            "config",
            "core.worktree",
            &cfg.work_tree.display().to_string(),
        ],
    )?;

    Ok(())
}

pub fn resolve_commit(git_dir: &Path, rev: &str) -> Result<String> {
    rev_parse(git_dir, rev)
}

fn rev_parse(git_dir: &Path, rev: &str) -> Result<String> {
    let out = run_git(git_dir, None, &["rev-parse", "--verify", rev])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[allow(dead_code)]
pub fn run_git_in_worktree(work_tree: &Path, args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(work_tree).args(args);
    cmd.stdin(Stdio::null());
    let out = cmd
        .output()
        .with_context(|| format!("git -C {} {:?}", work_tree.display(), args))?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out)
}

fn run_git(git_dir: &Path, work_tree: Option<&Path>, args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("git");
    cmd.arg("--git-dir").arg(git_dir);
    if let Some(wt) = work_tree {
        cmd.arg("--work-tree").arg(wt);
    }
    cmd.args(args);
    cmd.stdin(Stdio::null());
    let out = cmd.output().with_context(|| {
        format!(
            "git --git-dir={} {:?} {:?}",
            git_dir.display(),
            work_tree.map(|p| p.display().to_string()),
            args
        )
    })?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out)
}

/// Shared object store path for a registered repository base checkout.
#[allow(dead_code)]
pub fn object_store_from_base(base_path: &Path) -> PathBuf {
    base_path.join(".git").join("objects")
}
