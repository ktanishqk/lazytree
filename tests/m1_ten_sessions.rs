//! Milestone 1 acceptance: 10 isolated writable sessions.
use std::fs;
use std::path::PathBuf;

#[path = "common/mod.rs"]
mod common;
use common::*;

#[test]
fn ten_isolated_writable_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(
        &repo,
        RepoOpts {
            files: &[("foo.txt", b"original\n"), ("shared.txt", b"shared\n")],
            ..RepoOpts::default()
        },
    );

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );

    let mut roots = Vec::new();
    for i in 0..10 {
        let name = format!("s{i}");
        let out = run_lt(&home, &["create", &name]);
        assert_ok(&out, &format!("create {name}"));
        let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(PathBuf::from(&root).join("foo.txt").exists());
        roots.push(root);
    }

    for (i, root) in roots.iter().enumerate() {
        fs::write(
            PathBuf::from(root).join("foo.txt"),
            format!("session-{i}\n"),
        )
        .unwrap();
        fs::write(PathBuf::from(root).join(format!("only-{i}.txt")), b"mine\n").unwrap();
    }

    for (i, root) in roots.iter().enumerate() {
        let content = fs::read_to_string(PathBuf::from(root).join("foo.txt")).unwrap();
        assert_eq!(content, format!("session-{i}\n"));
        assert!(PathBuf::from(root).join(format!("only-{i}.txt")).exists());
        let j = (i + 1) % 10;
        assert!(!PathBuf::from(root).join(format!("only-{j}.txt")).exists());
    }

    let shared = fs::read_to_string(PathBuf::from(&roots[0]).join("shared.txt")).unwrap();
    assert_eq!(shared, "shared\n");

    let out = run_lt(&home, &["list"]);
    assert_ok(&out, "list");
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 10);

    for i in 0..10 {
        assert_ok(
            &run_lt(&home, &["destroy", &format!("s{i}"), "--force"]),
            &format!("destroy s{i}"),
        );
    }

    let out = run_lt(&home, &["list"]);
    assert_ok(&out, "list empty");
    assert!(String::from_utf8_lossy(&out.stdout).contains("no sessions"));
}
