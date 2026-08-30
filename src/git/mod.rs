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
    pub user_name: Option<String>,
    pub user_email: Option<String>,
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

    // Share immutable objects with the registered repository.
    let abs_objects = fs::canonicalize(&cfg.object_store)
        .with_context(|| format!("canonicalizing {}", cfg.object_store.display()))?;
    fs::write(
        cfg.git_dir.join("objects/info/alternates"),
        format!("{}\n", abs_objects.display()),
    )?;

    let oid_base = looks_like_full_oid(&cfg.base_revision).then(|| cfg.base_revision.clone());

    // Prefer copying a seed index (O(index bytes)) over read-tree (tree walk).
    // Happy path: full OID + matching seed → zero git process spawns.
    let can_copy_seed = match (&cfg.seed_index, &cfg.seed_commit, &oid_base) {
        (Some(idx), Some(seed), Some(base)) if idx.is_file() => {
            seed == base || seed_matches_short(seed, base)
        }
        _ => false,
    };

    // Happy path: full OID + seed index → zero git process spawns.
    if let (true, Some(base)) = (can_copy_seed, oid_base.as_ref()) {
        write_branch_ref(&cfg.git_dir, &cfg.branch, base)?;
        fs::write(
            cfg.git_dir.join("HEAD"),
            format!("ref: refs/heads/{}\n", cfg.branch),
        )?;
        copy_index(cfg.seed_index.as_ref().unwrap(), &cfg.git_dir.join("index"))?;
        write_session_config(
            &cfg.git_dir,
            &cfg.work_tree,
            cfg.user_name.as_deref(),
            cfg.user_email.as_deref(),
        )?;
        write_gitdir_pointer(cfg)?;
        return Ok(());
    }

    // Slow path: need git to resolve a non-OID rev and/or read-tree.
    run_git(&cfg.git_dir, None, &["init", "--quiet"])?;
    // Re-write alternates — git init may recreate objects/info.
    fs::create_dir_all(cfg.git_dir.join("objects/info"))?;
    fs::write(
        cfg.git_dir.join("objects/info/alternates"),
        format!("{}\n", abs_objects.display()),
    )?;

    let base = match oid_base {
        Some(b) => b,
        None => resolve_commit(&cfg.git_dir, &cfg.base_revision)?,
    };

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

    let can_copy_seed = match (&cfg.seed_index, &cfg.seed_commit) {
        (Some(idx), Some(seed)) if idx.is_file() => {
            seed == &base || seed_matches_short(seed, &base)
        }
        _ => false,
    };

    if can_copy_seed {
        copy_index(cfg.seed_index.as_ref().unwrap(), &cfg.git_dir.join("index"))?;
    } else {
        let tree = rev_parse(&cfg.git_dir, &format!("{base}^{{tree}}"))?;
        run_git(&cfg.git_dir, None, &["read-tree", &tree])?;
    }

    write_session_config(
        &cfg.git_dir,
        &cfg.work_tree,
        cfg.user_name.as_deref(),
        cfg.user_email.as_deref(),
    )?;
    write_gitdir_pointer(cfg)?;
    Ok(())
}

fn write_gitdir_pointer(cfg: &GitSetup) -> Result<()> {
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
    Ok(())
}

fn looks_like_full_oid(s: &str) -> bool {
    s.len() == 40 && s.as_bytes().iter().all(|b| b.is_ascii_hexdigit())
}

fn seed_matches_short(seed: &str, base: &str) -> bool {
    seed.starts_with(base) || base.starts_with(seed)
}

fn write_branch_ref(git_dir: &Path, branch: &str, oid: &str) -> Result<()> {
    let ref_path = git_dir.join("refs/heads").join(branch);
    if let Some(parent) = ref_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&ref_path, format!("{oid}\n"))
        .with_context(|| format!("writing {}", ref_path.display()))?;
    Ok(())
}

fn write_session_config(
    git_dir: &Path,
    work_tree: &Path,
    user_name: Option<&str>,
    user_email: Option<&str>,
) -> Result<()> {
    // Absolute worktree avoids surprises when callers chdir.
    let abs_wt = fs::canonicalize(work_tree).unwrap_or_else(|_| work_tree.to_path_buf());
    let mut body = format!(
        "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\tlogallrefupdates = true\n\tworktree = {}\n",
        abs_wt.display()
    );
    if user_name.is_some() || user_email.is_some() {
        body.push_str("[user]\n");
        if let Some(n) = user_name {
            body.push_str(&format!("\tname = {n}\n"));
        }
        if let Some(e) = user_email {
            body.push_str(&format!("\temail = {e}\n"));
        }
    }
    fs::write(git_dir.join("config"), body)
        .with_context(|| format!("writing {}/config", git_dir.display()))?;
    Ok(())
}

fn copy_index(src: &Path, dst: &Path) -> Result<()> {
    // Prefer reflink when the filesystem supports it (cheap CoW of the index file).
    let status = Command::new("cp")
        .args(["--reflink=auto", "--remove-destination"])
        .arg(src)
        .arg(dst)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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
