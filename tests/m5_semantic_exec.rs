//! Milestone 5/6: semantic cache env + lazy local exec.
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

fn make_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("foo.txt"), b"original\n").unwrap();
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
fn semantic_env_and_lazy_exec() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(&repo);

    assert_ok(
        &run_lt(&home, &["repo", "add", repo.to_str().unwrap()]),
        "repo add",
    );
    let out = run_lt(&home, &["create", "--json", "s1"]);
    assert_ok(&out, "create");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["runtime"]["state"], "none");
    assert_eq!(v["semantic"]["state"], "inherited");

    let st = run_lt(&home, &["status", "s1", "--json"]);
    assert_ok(&st, "status");
    let stv: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    let shared = PathBuf::from(stv["shared_cache"].as_str().unwrap());
    let session_cache = PathBuf::from(stv["session_cache"].as_str().unwrap());
    assert!(shared.join("cargo-home").is_dir());
    assert!(session_cache.join("caches").is_dir());

    // exec injects env; runtime stays lazy (no daemon)
    let out = run_lt(
        &home,
        &[
            "exec",
            "s1",
            "--",
            "bash",
            "-lc",
            "test -n \"$LAZYTREE_SHARED_CACHE\" && test -n \"$LAZYTREE_SESSION_CACHE\" && test -f foo.txt && echo OK",
        ],
    );
    assert_ok(&out, "exec");
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK"));

    let st2 = run_lt(&home, &["status", "s1", "--json"]);
    assert_ok(&st2, "status after exec");
    let stv2: serde_json::Value = serde_json::from_slice(&st2.stdout).unwrap();
    assert_eq!(stv2["runtime_state"], "none");

    assert_ok(&run_lt(&home, &["destroy", "s1", "--force"]), "destroy");
}
