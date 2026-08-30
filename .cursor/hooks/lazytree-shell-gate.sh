#!/usr/bin/env bash
# beforeShellExecution: deny risky git outside LazyTree root when paired.
set -euo pipefail

INPUT=$(cat || true)
MAP_DIR=".cursor/lazytree-sessions"

python3 - "$INPUT" "$MAP_DIR" "${LAZYTREE_ROOT:-}" <<'PY'
import json, os, sys, shlex

raw, map_dir, env_root = sys.argv[1:]
try:
    d = json.loads(raw) if raw else {}
except Exception:
    print(json.dumps({}))
    raise SystemExit(0)

session_id = d.get("session_id") or d.get("conversation_id") or ""
root = env_root.strip() if env_root else ""
if not root and session_id:
    map_file = os.path.join(map_dir, f"{session_id}.json")
    if os.path.isfile(map_file):
        root = json.load(open(map_file)).get("root", "")

if not root:
    print(json.dumps({}))
    raise SystemExit(0)

root = os.path.realpath(root)
cmd = d.get("command") or d.get("shell_command") or ""
cwd = d.get("working_directory") or d.get("cwd") or ""

def under(root, path):
    if not path:
        return False
    try:
        rp = os.path.realpath(path)
    except Exception:
        return False
    return rp == root or rp.startswith(root + os.sep)

try:
    parts = shlex.split(cmd)
except Exception:
    parts = cmd.split()

while parts and "=" in parts[0] and not parts[0].startswith("-"):
    parts = parts[1:]

BLOCK = {
    "commit", "push", "merge", "rebase", "cherry-pick", "am",
    "reset", "clean", "stash", "tag", "worktree",
}

dangerous = False
reason = "git write"

if parts and parts[0] == "git":
    sub = None
    i = 1
    target_c = None
    while i < len(parts):
        p = parts[i]
        if p == "-C" and i + 1 < len(parts):
            target_c = parts[i + 1]
            i += 2
            continue
        if p in ("-c", "--git-dir", "--work-tree", "--namespace") and i + 1 < len(parts):
            i += 2
            continue
        if p.startswith("-"):
            i += 1
            continue
        sub = p
        break

    rest = parts[i + 1 :] if sub else []

    # Mutating branch ops (create/delete/rename), not listing.
    branch_mutate = False
    if sub == "branch":
        flags = {
            "-d", "-D", "-m", "-M", "-c", "-C",
            "--delete", "--move", "--copy", "--unset-upstream",
        }
        if any(x in flags or x.startswith("--delete") or x.startswith("--move") for x in rest):
            branch_mutate = True
        elif any(not x.startswith("-") for x in rest):
            branch_mutate = True  # git branch newname

    blocked = (sub in BLOCK) or branch_mutate
    if blocked:
        label = sub or "branch"
        if target_c is not None:
            if not under(root, target_c):
                dangerous = True
                reason = f"git {label}"
        elif cwd and not under(root, cwd):
            dangerous = True
            reason = f"git {label}"
        elif not cwd:
            dangerous = True
            reason = f"git {label}"

if dangerous:
    print(json.dumps({
        "permission": "deny",
        "user_message": f"LazyTree gate: {reason} must run inside {root}",
        "agent_message": (
            f"Run git -C {root} ... or set working_directory to LAZYTREE_ROOT. "
            f"Primary checkout mutations are out of policy for this session."
        ),
    }))
else:
    print(json.dumps({"permission": "allow"}))
PY
exit 0
