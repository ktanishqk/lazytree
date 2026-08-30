#![allow(dead_code)]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn lazytree_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lazytree"))
}

pub fn run_lt(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(lazytree_bin())
        .env("LAZYTREE_HOME", home)
        .args(args)
        .output()
        .expect("run lazytree")
}

pub fn assert_ok(out: &std::process::Output, ctx: &str) {
    if !out.status.success() {
        panic!(
            "{ctx} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

pub fn assert_err(out: &std::process::Output, ctx: &str) {
    if out.status.success() {
        panic!("{ctx} unexpectedly succeeded");
    }
}

pub struct RepoOpts<'a> {
    pub branch: &'a str,
    pub files: &'a [(&'a str, &'a [u8])],
    /// Set receive.denyCurrentBranch=updateInstead (archive tests).
    pub allow_push_to_checkout: bool,
}

impl Default for RepoOpts<'static> {
    fn default() -> Self {
        Self {
            branch: "main",
            files: &[("foo.txt", b"original\n")],
            allow_push_to_checkout: false,
        }
    }
}

pub fn make_repo(path: &Path, opts: RepoOpts<'_>) {
    fs::create_dir_all(path).unwrap();
    for (name, contents) in opts.files {
        let p = path.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, contents).unwrap();
    }
    assert!(Command::new("git")
        .args(["init", "-b", opts.branch])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
    for (k, v) in [("user.email", "test@lazytree.dev"), ("user.name", "test")] {
        assert!(Command::new("git")
            .args(["config", k, v])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
    }
    if opts.allow_push_to_checkout {
        assert!(Command::new("git")
            .args(["config", "receive.denyCurrentBranch", "updateInstead"])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
    }
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
}
