//! Canonical-path exec makes cargo caches reusable across sessions.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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

fn make_rust_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    assert!(Command::new("cargo")
        .args(["new", "--bin", "app", "--name", "canonapp"])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
    let app = path.join("app");
    // Move app contents up so repo root is the crate (simpler for lazytree)
    // Actually register app/ as the repo.
    assert!(Command::new("git")
        .args(["init"])
        .current_dir(&app)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["config", "user.email", "t@t"])
        .current_dir(&app)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(&app)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(&app)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(&app)
        .status()
        .unwrap()
        .success());
}

#[test]
fn canonical_exec_reuses_cargo_target_across_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo_parent = tmp.path().join("src");
    make_rust_repo(&repo_parent);
    let repo = repo_parent.join("app");

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );
    assert_ok(&run_lt(&home, &["create", "a"]), "create a");
    assert_ok(&run_lt(&home, &["create", "b"]), "create b");

    // Cold check in session A (canonical)
    let t0 = Instant::now();
    let out = run_lt(
        &home,
        &["exec", "a", "--", "cargo", "check", "-q"],
    );
    assert_ok(&out, "cargo check a cold");
    let cold_ms = t0.elapsed().as_millis();

    assert_ok(&run_lt(&home, &["cache", "promote", "a"]), "promote");
    assert_ok(&run_lt(&home, &["cache", "seed", "b"]), "seed b");

    // Warm check in session B should be much faster than cold.
    let t1 = Instant::now();
    let out = run_lt(
        &home,
        &["exec", "b", "--", "cargo", "check", "-q"],
    );
    assert_ok(&out, "cargo check b warm");
    let warm_ms = t1.elapsed().as_millis();

    eprintln!("cold_ms={cold_ms} warm_ms={warm_ms}");
    assert!(
        warm_ms < cold_ms / 2 || warm_ms < 500,
        "expected seeded canonical check to be clearly faster (cold={cold_ms}ms warm={warm_ms}ms)"
    );

    assert_ok(&run_lt(&home, &["destroy", "a", "--force"]), "destroy a");
    assert_ok(&run_lt(&home, &["destroy", "b", "--force"]), "destroy b");
}
