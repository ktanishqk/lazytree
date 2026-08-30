#!/usr/bin/env bash
# beforeShellExecution: deny git commit/push outside LazyTree root when paired.
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
    rp = os.path.realpath(path)
    return rp == root or rp.startswith(root + os.sep)

dangerous = False
try:
    parts = shlex.split(cmd)
except Exception:
    parts = cmd.split()

if parts and parts[0] == "git" and ("commit" in parts or "push" in parts):
    if "-C" in parts:
        i = parts.index("-C")
        target = parts[i + 1] if i + 1 < len(parts) else ""
        if not under(root, target):
            dangerous = True
    elif cwd and not under(root, cwd):
        dangerous = True
    elif not cwd:
        dangerous = True

if dangerous:
    print(json.dumps({
        "permission": "deny",
        "userMessage": f"LazyTree gate: git commit/push must run inside {root}",
        "agentMessage": (
            f"Run git -C {root} ... or cd to LAZYTREE_ROOT before commit/push. "
            f"Primary checkout edits are out of policy for this session."
        ),
    }))
else:
    print(json.dumps({"permission": "allow"}))
PY
exit 0
