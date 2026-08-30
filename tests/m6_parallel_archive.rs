//! Parallel session create + archive publishes branch to source.
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;

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

fn init_repo(repo: &std::path::Path) {
    fs::create_dir_all(repo).unwrap();
    fs::write(repo.join("README.md"), b"base\n").unwrap();
    assert!(Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
    for (k, v) in [
        ("user.email", "test@lazytree.dev"),
        ("user.name", "test"),
    ] {
        assert!(Command::new("git")
            .args(["config", k, v])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
    }
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}

#[test]
fn parallel_create_and_archive_publish() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    init_repo(&repo);

    let out = run_lt(&home, &["repo", "add", repo.to_str().unwrap()]);
    assert_ok(&out, "repo add");

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

    // Isolation: write unique files.
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

    // Commit in one session and archive → branch appears on source.
    // Identity is inherited from the registered source repo into session git config.
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

    let out = run_lt(&home, &["archive", "par0"]);
    assert_ok(&out, "archive par0");

    let branches = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "branch", "--list", "lazytree/par0"])
        .output()
        .unwrap();
    assert_ok(&branches, "list published branch");
    let text = String::from_utf8_lossy(&branches.stdout);
    assert!(
        text.contains("lazytree/par0"),
        "expected published branch, got: {text}"
    );

    // cursor bootstrap writes mapping
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
    let map = project.join(".cursor/lazytree-sessions/conv-1.json");
    assert!(map.is_file(), "mapping missing");
}
