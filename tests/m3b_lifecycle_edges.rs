//! Lifecycle edge cases: force destroy, name reuse, double archive.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[path = "common/mod.rs"]
mod common;
use common::*;

#[test]
fn force_destroy_name_reuse_double_archive() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(
        &repo,
        RepoOpts {
            allow_push_to_checkout: true,
            ..RepoOpts::default()
        },
    );

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );

    let out = run_lt(&home, &["create", "edge"]);
    assert_ok(&out, "create");
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    fs::write(root.join("foo.txt"), b"commit-me\n").unwrap();
    assert!(Command::new("git")
        .args(["-C", root.to_str().unwrap(), "add", "foo.txt"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", root.to_str().unwrap(), "commit", "-m", "wip"])
        .status()
        .unwrap()
        .success());
    assert_err(
        &run_lt(&home, &["destroy", "edge"]),
        "destroy with unexported commit",
    );

    assert_ok(
        &run_lt(&home, &["destroy", "edge", "--force"]),
        "force destroy",
    );
    let out = run_lt(&home, &["create", "edge"]);
    assert_ok(&out, "recreate after destroy");
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    fs::write(root.join("note.txt"), b"n\n").unwrap();
    assert!(Command::new("git")
        .args(["-C", root.to_str().unwrap(), "add", "note.txt"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", root.to_str().unwrap(), "commit", "-m", "note"])
        .status()
        .unwrap()
        .success());
    assert_ok(&run_lt(&home, &["archive", "edge"]), "archive");
    assert_err(&run_lt(&home, &["archive", "edge"]), "double archive");
    assert_ok(&run_lt(&home, &["destroy", "edge"]), "destroy archived");

    assert_ok(
        &run_lt(&home, &["create", "edge"]),
        "create after archive destroy",
    );
}
