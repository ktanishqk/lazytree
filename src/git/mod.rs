//! Session-local Git metadata with a shared object database.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use anyhow::{bail, Context, Result};

use crate::util::{abs_path, copy_file};

/// Borrowed setup — callers own the paths; no clone storm on create.
pub struct GitSetup<'a> {
    pub git_dir: &'a Path,
    pub work_tree: &'a Path,
    pub branch: &'a str,
    pub base_revision: &'a str,
    pub object_store: &'a Path,
    pub seed_index: Option<&'a Path>,
    pub seed_commit: Option<&'a str>,
    pub user_name: Option<&'a str>,
    pub user_email: Option<&'a str>,
}

/// Private gitdir + worktree gitdir file. Happy path: zero `git` process spawns.
pub fn setup_session_git(cfg: &GitSetup<'_>) -> Result<()> {
    fs::create_dir_all(cfg.git_dir.join("objects/info"))?;
    fs::create_dir_all(cfg.git_dir.join("refs/heads"))?;

    if !cfg.object_store.is_dir() {
        bail!("shared object store missing: {}", cfg.object_store.display());
    }

    let abs_objects = abs_path(cfg.object_store);
    write_alternates(cfg.git_dir, &abs_objects)?;

    let seed_ok = cfg.seed_index.is_some_and(|p| p.is_file())
        && cfg.seed_commit.is_some_and(|seed| {
            looks_like_full_oid(cfg.base_revision)
                && (seed == cfg.base_revision || seed_matches_short(seed, cfg.base_revision))
        });

    if seed_ok {
        let base = cfg.base_revision;
        write_branch_ref(cfg.git_dir, cfg.branch, base)?;
        fs::write(
            cfg.git_dir.join("HEAD"),
            format!("ref: refs/heads/{}\n", cfg.branch),
        )?;
        copy_file(cfg.seed_index.unwrap(), &cfg.git_dir.join("index"))?;
        write_session_config(cfg)?;
        write_gitdir_pointer(cfg)?;
        return Ok(());
    }

    // Slow path: resolve non-OID rev and/or read-tree.
    run_git(cfg.git_dir, None, &["init", "--quiet"])?;
    write_alternates(cfg.git_dir, &abs_objects)?;

    let base = if looks_like_full_oid(cfg.base_revision) {
        cfg.base_revision.to_string()
    } else {
        rev_parse(cfg.git_dir, cfg.base_revision)?
    };

    let branch_ref = format!("refs/heads/{}", cfg.branch);
    run_git(cfg.git_dir, None, &["update-ref", &branch_ref, &base])?;
    run_git(cfg.git_dir, None, &["symbolic-ref", "HEAD", &branch_ref])?;

    let can_copy = cfg.seed_index.is_some_and(|p| p.is_file())
        && cfg
            .seed_commit
            .is_some_and(|seed| seed == base || seed_matches_short(seed, &base));

    if can_copy {
        copy_file(cfg.seed_index.unwrap(), &cfg.git_dir.join("index"))?;
    } else {
        let tree = rev_parse(cfg.git_dir, &format!("{base}^{{tree}}"))?;
        run_git(cfg.git_dir, None, &["read-tree", &tree])?;
    }

    write_session_config(cfg)?;
    write_gitdir_pointer(cfg)?;
    Ok(())
}

fn write_alternates(git_dir: &Path, abs_objects: &Path) -> Result<()> {
    fs::create_dir_all(git_dir.join("objects/info"))?;
    fs::write(
        git_dir.join("objects/info/alternates"),
        format!("{}\n", abs_objects.display()),
    )?;
    Ok(())
}

fn write_gitdir_pointer(cfg: &GitSetup<'_>) -> Result<()> {
    let overlay_git = cfg.work_tree.join(".git");
    if overlay_git.exists() {
        if overlay_git.is_dir() {
            fs::remove_dir_all(&overlay_git)
                .with_context(|| format!("whiteout {}", overlay_git.display()))?;
        } else {
            fs::remove_file(&overlay_git)?;
        }
    }
    let abs_git = abs_path(cfg.git_dir);
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
    fs::write(&ref_path, format!("{oid}\n"))?;
    Ok(())
}

fn write_session_config(cfg: &GitSetup<'_>) -> Result<()> {
    let abs_wt = abs_path(cfg.work_tree);
    let mut body = format!(
        "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\tlogallrefupdates = true\n\tworktree = {}\n",
        abs_wt.display()
    );
    if cfg.user_name.is_some() || cfg.user_email.is_some() {
        body.push_str("[user]\n");
        if let Some(n) = cfg.user_name {
            body.push_str("\tname = ");
            body.push_str(n);
            body.push('\n');
        }
        if let Some(e) = cfg.user_email {
            body.push_str("\temail = ");
            body.push_str(e);
            body.push('\n');
        }
    }
    fs::write(cfg.git_dir.join("config"), body)?;
    Ok(())
}

fn rev_parse(git_dir: &Path, rev: &str) -> Result<String> {
    let out = run_git(git_dir, None, &["rev-parse", "--verify", rev])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `git -C <work_tree> …`
pub fn git_c(work_tree: &Path, args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("git");
    cmd.args(["-C"]).arg(work_tree).args(args);
    cmd.stdin(Stdio::null());
    let out = cmd
        .output()
        .with_context(|| format!("git -C {} {args:?}", work_tree.display()))?;
    if !out.status.success() {
        bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out)
}

/// `git --git-dir <dir> …`
pub fn run_git(git_dir: &Path, work_tree: Option<&Path>, args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("git");
    cmd.arg("--git-dir").arg(git_dir);
    if let Some(wt) = work_tree {
        cmd.arg("--work-tree").arg(wt);
    }
    cmd.args(args);
    cmd.stdin(Stdio::null());
    let out = cmd
        .output()
        .with_context(|| format!("git --git-dir={} {args:?}", git_dir.display()))?;
    if !out.status.success() {
        bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out)
}
