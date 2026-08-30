#!/usr/bin/env bash
# preToolUse gate: deny mutating tools whose target path escapes LazyTree root.
# Fail-open if no mapping / env (repo may not use LazyTree yet).
set -euo pipefail

INPUT=$(cat || true)
MAP_DIR=".cursor/lazytree-sessions"

python3 - "$INPUT" "$MAP_DIR" "${LAZYTREE_ROOT:-}" <<'PY'
import json, os, sys

raw, map_dir, env_root = sys.argv[1:]
try:
    d = json.loads(raw) if raw else {}
except Exception:
    print(json.dumps({}))
    raise SystemExit(0)

session_id = d.get("session_id") or d.get("conversation_id") or ""
tool = (d.get("tool_name") or d.get("tool") or "").lower()

root = env_root.strip() if env_root else ""
if not root and session_id:
    map_file = os.path.join(map_dir, f"{session_id}.json")
    if os.path.isfile(map_file):
        root = json.load(open(map_file)).get("root", "")

if not root:
    # No LazyTree pairing — do not interfere.
    print(json.dumps({}))
    raise SystemExit(0)

root = os.path.realpath(root)

candidates = []
for key in ("file_path", "path", "working_directory", "cwd"):
    v = d.get(key)
    if isinstance(v, str) and v:
        candidates.append(v)
args = d.get("arguments") or d.get("tool_input") or {}
if isinstance(args, dict):
    for key in ("path", "file_path", "target_file", "working_directory", "cwd"):
        v = args.get(key)
        if isinstance(v, str) and v:
            candidates.append(v)

mutating = any(x in tool for x in ("write", "edit", "delete", "apply", "search_replace", "strreplace"))
if not mutating:
    print(json.dumps({}))
    raise SystemExit(0)

def under(root, path):
    try:
        rp = os.path.realpath(path)
    except Exception:
        return False
    return rp == root or rp.startswith(root + os.sep)

bad = [p for p in candidates if p and not under(root, p)]
if bad:
    print(json.dumps({
        "permission": "deny",
        "userMessage": f"LazyTree gate: refusing tool outside session root {root}: {bad[0]}",
        "agentMessage": (
            f"Edit/write targeted {bad[0]} which is outside LazyTree root {root}. "
            f"Re-run against files under that root (see LAZYTREE_ROOT / .cursor/lazytree-sessions)."
        ),
    }))
else:
    print(json.dumps({"permission": "allow"}))
PY
exit 0
