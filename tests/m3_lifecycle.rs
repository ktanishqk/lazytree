//! Milestone 3 lifecycle: dirty destroy protection, status, archive, doctor.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[path = "common/mod.rs"]
mod common;
use common::*;

#[test]
fn dirty_destroy_archive_status_doctor() {
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
    let out = run_lt(&home, &["create", "eng-1"]);
    assert_ok(&out, "create");
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    fs::write(root.join("foo.txt"), b"dirty\n").unwrap();
    assert_err(&run_lt(&home, &["destroy", "eng-1"]), "destroy dirty");

    let st = run_lt(&home, &["status", "eng-1", "--json"]);
    assert_ok(&st, "status");
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    assert_eq!(v["dirty"], true);

    let diff = run_lt(&home, &["diff", "eng-1"]);
    assert_ok(&diff, "diff");
    assert!(String::from_utf8_lossy(&diff.stdout).contains("dirty"));

    assert!(Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args(["add", "foo.txt"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            "work",
        ])
        .status()
        .unwrap()
        .success());
    assert_err(
        &run_lt(&home, &["destroy", "eng-1"]),
        "destroy unexported",
    );

    assert_ok(&run_lt(&home, &["archive", "eng-1"]), "archive");
    let branches = Command::new("git")
        .args(["-C"])
        .arg(&repo)
        .args(["branch", "--list", "lazytree/eng-1"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&branches.stdout).contains("lazytree/eng-1"));

    let list = run_lt(&home, &["list"]);
    assert_ok(&list, "list");
    assert!(String::from_utf8_lossy(&list.stdout).contains("archived"));

    assert_ok(&run_lt(&home, &["destroy", "eng-1"]), "destroy archived");

    let out = run_lt(&home, &["create", "eng-2"]);
    assert_ok(&out, "create 2");
    let root2 = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    fs::write(root2.join("foo.txt"), b"x\n").unwrap();
    assert_ok(
        &run_lt(&home, &["destroy", "eng-2", "--force"]),
        "force destroy",
    );

    assert_ok(&run_lt(&home, &["doctor", "--json"]), "doctor");
}
