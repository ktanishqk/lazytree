//! Session-local Git metadata with shared object database (Milestone 2+).

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
    /// Optional prebuilt index matching `base_revision`'s tree. Copied into the
    /// session git dir when the resolved commit equals the seed's commit.
    pub seed_index: Option<PathBuf>,
    /// Commit OID the seed index was built for (usually the registered base).
    pub seed_commit: Option<String>,
}

/// Initialize private Git metadata for a session and point the work tree at it
/// via a `.git` gitdir file (standards-compliant discovery).
pub fn setup_session_git(cfg: &GitSetup) -> Result<()> {
    fs::create_dir_all(&cfg.git_dir)?;
    fs::create_dir_all(cfg.git_dir.join("objects/info"))?;
    fs::create_dir_all(cfg.git_dir.join("refs/heads"))?;

    if !cfg.object_store.is_dir() {
        bail!(
            "shared object store missing: {}",
            cfg.object_store.display()
        );
    }

    run_git(&cfg.git_dir, None, &["init", "--quiet"])?;

    // Share immutable objects with the registered repository.
    let alternates = cfg.git_dir.join("objects/info/alternates");
    let abs_objects = fs::canonicalize(&cfg.object_store)
        .with_context(|| format!("canonicalizing {}", cfg.object_store.display()))?;
    fs::write(&alternates, format!("{}\n", abs_objects.display()))?;

    let base = resolve_commit(&cfg.git_dir, &cfg.base_revision)?;

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

    // Prefer copying a seed index (O(index bytes)) over read-tree (tree walk).
    // Fall back when --from points at a different revision than the seed.
    let can_copy_seed = match (&cfg.seed_index, &cfg.seed_commit) {
        (Some(idx), Some(seed)) if idx.is_file() => seed == &base || seed_matches_short(seed, &base),
        _ => false,
    };

    if can_copy_seed {
        let src = cfg.seed_index.as_ref().unwrap();
        let dst = cfg.git_dir.join("index");
        copy_index(src, &dst)?;
    } else {
        let tree = rev_parse(&cfg.git_dir, &format!("{base}^{{tree}}"))?;
        run_git(&cfg.git_dir, None, &["read-tree", &tree])?;
    }

    // Lowerdir should not contain `.git` (see repo registration). Just write
    // the gitdir pointer — no OverlayFS whiteout of thousands of git files.
    let overlay_git = cfg.work_tree.join(".git");
    if overlay_git.exists() {
        if overlay_git.is_dir() {
            // Legacy bases that still embed .git: whiteout (slow) as fallback.
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

fn seed_matches_short(seed: &str, base: &str) -> bool {
    seed.starts_with(base) || base.starts_with(seed)
}

fn copy_index(src: &Path, dst: &Path) -> Result<()> {
    // Prefer reflink when the filesystem supports it (cheap CoW of the index file).
    let status = Command::new("cp")
        .args(["--reflink=auto", "--remove-destination"])
        .arg(src)
        .arg(dst)
        .status();
    if let Ok(s) = status {
        if s.success() {
            return Ok(());
        }
    }
    fs::copy(src, dst)
        .with_context(|| format!("copying index {} -> {}", src.display(), dst.display()))?;
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
