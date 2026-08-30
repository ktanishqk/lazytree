//! Milestone 1 acceptance: 10 isolated writable sessions.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn lazytree_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lazytree"))
}

fn run_lt(home: &std::path::Path, args: &[&str]) -> std::process::Output {
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

#[test]
fn ten_isolated_writable_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("foo.txt"), b"original\n").unwrap();
    fs::write(repo.join("shared.txt"), b"shared\n").unwrap();

    assert!(Command::new("git").args(["init"]).current_dir(&repo).status().unwrap().success());
    assert!(Command::new("git")
        .args(["config", "user.email", "test@lazytree.dev"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["config", "user.name", "test"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());

    let out = run_lt(&home, &["repo", "add", repo.to_str().unwrap()]);
    assert_ok(&out, "repo add");

    let mut roots = Vec::new();
    for i in 0..10 {
        let name = format!("s{i}");
        let out = run_lt(&home, &["create", &name]);
        assert_ok(&out, &format!("create {name}"));
        let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(PathBuf::from(&root).join("foo.txt").exists());
        roots.push(root);
    }

    // Each session writes a distinct value; siblings must not see it.
    for (i, root) in roots.iter().enumerate() {
        fs::write(
            PathBuf::from(root).join("foo.txt"),
            format!("session-{i}\n"),
        )
        .unwrap();
        fs::write(
            PathBuf::from(root).join(format!("only-{i}.txt")),
            b"mine\n",
        )
        .unwrap();
    }

    for (i, root) in roots.iter().enumerate() {
        let content = fs::read_to_string(PathBuf::from(root).join("foo.txt")).unwrap();
        assert_eq!(content, format!("session-{i}\n"));
        assert!(PathBuf::from(root).join(format!("only-{i}.txt")).exists());
        // sibling file must not exist
        let j = (i + 1) % 10;
        assert!(!PathBuf::from(root).join(format!("only-{j}.txt")).exists());
    }

    // shared untouched file still readable
    let shared = fs::read_to_string(PathBuf::from(&roots[0]).join("shared.txt")).unwrap();
    assert_eq!(shared, "shared\n");

    let out = run_lt(&home, &["list"]);
    assert_ok(&out, "list");
    let list = String::from_utf8_lossy(&out.stdout);
    assert_eq!(list.lines().count(), 10);

    for i in 0..10 {
        let out = run_lt(&home, &["destroy", &format!("s{i}"), "--force"]);
        assert_ok(&out, &format!("destroy s{i}"));
    }

    let out = run_lt(&home, &["list"]);
    assert_ok(&out, "list empty");
    assert!(String::from_utf8_lossy(&out.stdout).contains("no sessions"));
}
