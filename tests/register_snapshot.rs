//! Registration snapshots committed files only (no gitignored `target/`).
use std::fs;

#[path = "common/mod.rs"]
mod common;
use common::*;

#[test]
fn repo_add_omits_gitignored_target() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(
        &repo,
        RepoOpts {
            files: &[("foo.txt", b"ok\n"), (".gitignore", b"target/\n")],
            ..RepoOpts::default()
        },
    );
    fs::create_dir_all(repo.join("target/release")).unwrap();
    fs::write(repo.join("target/release/junk"), b"blob").unwrap();

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );

    let repos = home.join("repositories");
    let mut found_foo = false;
    let mut found_junk = false;
    for entry in fs::read_dir(&repos).unwrap() {
        let base = entry.unwrap().path().join("base");
        if base.join("foo.txt").is_file() {
            found_foo = true;
        }
        if base.join("target/release/junk").is_file() {
            found_junk = true;
        }
    }
    assert!(found_foo, "tracked foo.txt should be in lowerdir");
    assert!(!found_junk, "gitignored target/ must not be in lowerdir");
}
