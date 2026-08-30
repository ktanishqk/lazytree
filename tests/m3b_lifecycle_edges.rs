//! Lifecycle edge cases: force destroy, name reuse, double archive.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/mod.rs"]
mod common;
use common::*;

fn make_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("foo.txt"), b"original\n").unwrap();
    assert!(Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
    for (k, v) in [
        ("user.email", "test@lazytree.dev"),
        ("user.name", "test"),
        ("receive.denyCurrentBranch", "updateInstead"),
    ] {
        assert!(Command::new("git")
            .args(["config", k, v])
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

#[test]
fn force_destroy_name_reuse_double_archive() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(&repo);

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );

    let out = run_lt(&home, &["create", "edge"]);
    assert_ok(&out, "create");
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    // Unexported commit blocks destroy.
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

    // Force destroy succeeds and frees the name.
    assert_ok(
        &run_lt(&home, &["destroy", "edge", "--force"]),
        "force destroy",
    );
    let out = run_lt(&home, &["create", "edge"]);
    assert_ok(&out, "recreate after destroy");
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    // Archive then destroy without force.
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

    // Name reusable again.
    assert_ok(&run_lt(&home, &["create", "edge"]), "create after archive destroy");
}
