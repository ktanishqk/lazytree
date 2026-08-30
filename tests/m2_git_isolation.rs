//! Milestone 2: independent Git state across COW sessions.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/mod.rs"]
mod common;
use common::*;

fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git")
}

fn git_ok(cwd: &Path, args: &[&str]) -> String {
    let out = git(cwd, args);
    assert_ok(&out, &format!("git {args:?}"));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn independent_git_state_and_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(
        &repo,
        RepoOpts {
            files: &[("foo.txt", b"original\n"), ("bar.txt", b"bar\n")],
            ..RepoOpts::default()
        },
    );

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );

    let out = run_lt(&home, &["create", "ticket-a"]);
    assert_ok(&out, "create a");
    let a = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    let out = run_lt(&home, &["create", "ticket-b"]);
    assert_ok(&out, "create b");
    let b = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    assert_eq!(git_ok(&a, &["branch", "--show-current"]), "lazytree/ticket-a");
    assert_eq!(git_ok(&b, &["branch", "--show-current"]), "lazytree/ticket-b");
    assert!(git_ok(&a, &["status", "--porcelain"]).is_empty());

    fs::write(a.join("foo.txt"), b"aaa\n").unwrap();
    assert_ok(&git(&a, &["add", "foo.txt"]), "git add a");
    assert_eq!(git_ok(&a, &["diff", "--cached", "--name-only"]), "foo.txt");
    assert!(git_ok(&b, &["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!(fs::read_to_string(b.join("foo.txt")).unwrap(), "original\n");

    assert_ok(
        &git(
            &a,
            &[
                "-c",
                "user.email=a@lazytree.dev",
                "-c",
                "user.name=a",
                "commit",
                "-m",
                "a-change",
            ],
        ),
        "commit a",
    );
    assert_ne!(
        git_ok(&a, &["rev-parse", "HEAD"]),
        git_ok(&b, &["rev-parse", "HEAD"])
    );
    assert_eq!(git_ok(&b, &["log", "--oneline"]).lines().count(), 1);

    fs::write(b.join("foo.txt"), b"bbb\n").unwrap();
    assert_ok(&git(&b, &["add", "foo.txt"]), "git add b");
    assert_ok(
        &git(
            &b,
            &[
                "-c",
                "user.email=b@lazytree.dev",
                "-c",
                "user.name=b",
                "commit",
                "-m",
                "b-change",
            ],
        ),
        "commit b",
    );
    assert_eq!(fs::read_to_string(a.join("foo.txt")).unwrap(), "aaa\n");
    assert_eq!(fs::read_to_string(b.join("foo.txt")).unwrap(), "bbb\n");

    let base_objects = {
        let repos = home.join("repositories");
        let entry = fs::read_dir(&repos).unwrap().next().unwrap().unwrap();
        let meta: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(entry.path().join("metadata.json")).unwrap())
                .unwrap();
        PathBuf::from(meta["object_store"].as_str().unwrap())
    };
    let shared_before = count_object_files(&base_objects);

    for i in 0..10 {
        let name = format!("untouched-{i}");
        assert_ok(&run_lt(&home, &["create", &name]), &name);
    }
    assert_eq!(
        shared_before,
        count_object_files(&base_objects),
        "shared object store should not grow when creating untouched sessions"
    );

    for name in ["ticket-a", "ticket-b"] {
        assert_ok(&run_lt(&home, &["destroy", name, "--force"]), name);
    }
    for i in 0..10 {
        assert_ok(
            &run_lt(&home, &["destroy", &format!("untouched-{i}"), "--force"]),
            "destroy untouched",
        );
    }
}

fn count_object_files(objects: &Path) -> usize {
    let mut n = 0;
    if !objects.exists() {
        return 0;
    }
    for entry in fs::read_dir(objects).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "info" {
            continue;
        }
        if name == "pack" {
            n += walk_files(&entry.path());
            continue;
        }
        n += walk_files(&entry.path());
    }
    n
}

fn walk_files(path: &Path) -> usize {
    if path.is_file() {
        return 1;
    }
    let mut n = 0;
    if let Ok(rd) = fs::read_dir(path) {
        for entry in rd.flatten() {
            n += walk_files(&entry.path());
        }
    }
    n
}
