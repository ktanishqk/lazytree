//! Parallel session create + archive publishes branch to source.
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;

#[path = "common/mod.rs"]
mod common;
use common::*;

#[test]
fn parallel_create_and_archive_publish() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(
        &repo,
        RepoOpts {
            files: &[("README.md", b"base\n")],
            allow_push_to_checkout: true,
            ..RepoOpts::default()
        },
    );

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );

    let home_s = home.clone();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let home = home_s.clone();
            thread::spawn(move || {
                let name = format!("par{i}");
                let out = run_lt(&home, &["create", &name]);
                assert_ok(&out, &format!("create {name}"));
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            })
        })
        .collect();

    let roots: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(roots.len(), 8);

    for (i, root) in roots.iter().enumerate() {
        fs::write(PathBuf::from(root).join(format!("only-{i}.txt")), b"x\n").unwrap();
    }
    for (i, root) in roots.iter().enumerate() {
        for j in 0..8 {
            let p = PathBuf::from(root).join(format!("only-{j}.txt"));
            if i == j {
                assert!(p.is_file(), "missing only-{j} in session {i}");
            } else {
                assert!(!p.exists(), "leak only-{j} into session {i}");
            }
        }
    }

    let root0 = &roots[0];
    assert!(Command::new("git")
        .args(["-C", root0, "add", "only-0.txt"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", root0, "commit", "-m", "from-session"])
        .status()
        .unwrap()
        .success());

    assert_ok(&run_lt(&home, &["archive", "par0"]), "archive par0");

    let branches = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "branch", "--list", "lazytree/par0"])
        .output()
        .unwrap();
    assert_ok(&branches, "list published branch");
    assert!(
        String::from_utf8_lossy(&branches.stdout).contains("lazytree/par0"),
        "expected published branch"
    );

    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let out = run_lt(
        &home,
        &[
            "cursor",
            "bootstrap",
            "boot1",
            "--session-id",
            "conv-1",
            "--project",
            project.to_str().unwrap(),
            "--json",
        ],
    );
    assert_ok(&out, "cursor bootstrap");
    assert!(project
        .join(".cursor/lazytree-sessions/conv-1.json")
        .is_file());
}
