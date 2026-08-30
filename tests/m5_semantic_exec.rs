//! Milestone 5/6: semantic cache env + lazy local exec.
use std::path::PathBuf;

#[path = "common/mod.rs"]
mod common;
use common::*;

#[test]
fn semantic_env_and_lazy_exec() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("lt-home");
    let repo = tmp.path().join("repo");
    make_repo(&repo, RepoOpts::default());

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
    assert!(shared.is_dir(), "shared cache root");
    assert!(session_cache.is_dir(), "session cache root");

    let out = run_lt(
        &home,
        &[
            "exec",
            "s1",
            "--",
            "bash",
            "-lc",
            "test -n \"$LAZYTREE_SHARED_CACHE\" && test -n \"$LAZYTREE_SESSION_CACHE\" && test -n \"$CARGO_HOME\" && test -d \"$CARGO_HOME\" && test -f foo.txt && echo OK",
        ],
    );
    assert_ok(&out, "exec");
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK"));
    assert!(shared.join("cargo-home").is_dir());

    let st2 = run_lt(&home, &["status", "s1", "--json"]);
    assert_ok(&st2, "status after exec");
    let stv2: serde_json::Value = serde_json::from_slice(&st2.stdout).unwrap();
    assert_eq!(stv2["runtime_state"], "none");

    assert_ok(&run_lt(&home, &["destroy", "s1", "--force"]), "destroy");
}
