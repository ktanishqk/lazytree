//! Milestone 2: independent Git state across COW sessions.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn lazytree_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lazytree"))
}

fn run_lt(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(lazytree_bin())
        .env("LAZYTREE_HOME", home)
        .args(args)
        .output()
        .expect("run lazytree")
}

fn assert_ok(out: &std::process::Output, ctx: &str) {
    if !out.status.success() {
        panic!(
            "{ctx} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git")
}

fn git_ok(cwd: &Path, args: &[&str]) -> String {
    let out = git(cwd, args);
    if !out.status.success() {
        panic!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn make_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("foo.txt"), b"original\n").unwrap();
    fs::write(path.join("bar.txt"), b"bar\n").unwrap();
    assert!(Command::new("git")
        .args(["init"])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["config", "user.email", "test@lazytree.dev"])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["config", "user.name", "test"])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
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
fn independent_git_state_and_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(&repo);

    let out = run_lt(&home, &["repo", "add", repo.to_str().unwrap()]);
    assert_ok(&out, "repo add");

    let out = run_lt(&home, &["create", "ticket-a"]);
    assert_ok(&out, "create a");
    let a = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    let out = run_lt(&home, &["create", "ticket-b"]);
    assert_ok(&out, "create b");
    let b = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

    // Normal discovery: no GIT_DIR required
    assert_eq!(git_ok(&a, &["branch", "--show-current"]), "lazytree/ticket-a");
    assert_eq!(git_ok(&b, &["branch", "--show-current"]), "lazytree/ticket-b");

    let status_a = git_ok(&a, &["status", "--porcelain"]);
    assert!(status_a.is_empty(), "expected clean tree, got {status_a}");

    // Independent staging
    fs::write(a.join("foo.txt"), b"aaa\n").unwrap();
    assert_ok(&git(&a, &["add", "foo.txt"]), "git add a");
    let staged_a = git_ok(&a, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged_a, "foo.txt");
    let staged_b = git_ok(&b, &["diff", "--cached", "--name-only"]);
    assert!(staged_b.is_empty(), "B index should be clean, got {staged_b}");
    assert_eq!(fs::read_to_string(b.join("foo.txt")).unwrap(), "original\n");

    // Independent HEAD
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
    let head_a = git_ok(&a, &["rev-parse", "HEAD"]);
    let head_b = git_ok(&b, &["rev-parse", "HEAD"]);
    assert_ne!(head_a, head_b, "A commit must not move B HEAD");
    assert_eq!(git_ok(&b, &["log", "--oneline"]).lines().count(), 1);

    // Same-file divergence on B
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

    // Object reuse: 10 untouched sessions should not multiply shared object files.
    let base_objects = {
        // find registered repo object store via session metadata home
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
        let out = run_lt(&home, &["create", &name]);
        assert_ok(&out, &name);
    }
    let shared_after = count_object_files(&base_objects);
    assert_eq!(
        shared_before, shared_after,
        "shared object store should not grow when creating untouched sessions"
    );

    // Cleanup
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
    for entry in fs::read_dir(objects).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "info" || name == "pack" {
            // count packed + loose under pack/; skip info/alternates noise
            if name == "pack" {
                n += walk_files(&entry.path());
            }
            continue;
        }
        n += walk_files(&entry.path());
    }
    n
}

fn walk_files(path: &Path) -> usize {
    let mut n = 0;
    if path.is_file() {
        return 1;
    }
    if let Ok(rd) = fs::read_dir(path) {
        for entry in rd.flatten() {
            n += walk_files(&entry.path());
        }
    }
    n
}
