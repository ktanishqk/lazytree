#![allow(dead_code)]
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
