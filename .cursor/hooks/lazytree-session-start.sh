#!/usr/bin/env bash
# sessionStart: prepare/resolve a LazyTree session for this composer conversation.
# Fire-and-forget — keep this fast. Emit JSON on stdout only.
# Cannot change Cursor workspace cwd; env + additional_context only.
set -euo pipefail

INPUT=$(cat || true)
HOME_DIR="${LAZYTREE_HOME:-$HOME/.lazytree}"
MAP_DIR=".cursor/lazytree-sessions"
mkdir -p "$MAP_DIR"

SESSION_ID=$(printf '%s' "$INPUT" | python3 -c 'import sys,json
try:
  d=json.load(sys.stdin)
  print(d.get("session_id") or d.get("conversation_id") or "")
except Exception:
  print("")' 2>/dev/null || true)

WORKSPACE=$(printf '%s' "$INPUT" | python3 -c 'import sys,json
try:
  d=json.load(sys.stdin)
  roots=d.get("workspace_roots") or []
  print(roots[0] if roots else "")
except Exception:
  print("")' 2>/dev/null || true)

if [[ -z "$SESSION_ID" ]]; then
  SESSION_ID="anon-$(date +%s)"
fi

MAP_FILE="$MAP_DIR/${SESSION_ID}.json"

resolve_lt_bin() {
  if [[ -n "${LAZYTREE_BIN:-}" ]]; then
    printf '%s' "$LAZYTREE_BIN"
    return
  fi
  if command -v lazytree >/dev/null 2>&1; then
    command -v lazytree
    return
  fi
  for cand in \
    "$HOME/.local/bin/lazytree" \
    "$HOME/.cargo/bin/lazytree" \
    /usr/local/bin/lazytree
  do
    if [[ -x "$cand" ]]; then
      printf '%s' "$cand"
      return
    fi
  done
  # Dev tree: walk up from cwd for target/release/lazytree
  local dir="$PWD"
  for _ in 1 2 3 4 5 6; do
    if [[ -x "$dir/target/release/lazytree" ]]; then
      printf '%s' "$dir/target/release/lazytree"
      return
    fi
    dir="$(dirname "$dir")"
  done
  printf '%s' "lazytree"
}

LT_BIN="$(resolve_lt_bin)"

# Prefer existing mapping (idempotent across accidental re-fires).
if [[ -f "$MAP_FILE" ]]; then
  ROOT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["root"])' "$MAP_FILE")
  BRANCH=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("branch",""))' "$MAP_FILE")
else
  SHORT=$(printf '%s' "$SESSION_ID" | tr -cd 'a-zA-Z0-9' | cut -c1-12)
  NAME="cursor-${SHORT:-session}"

  if [[ ! -x "$LT_BIN" ]] && ! command -v "$LT_BIN" >/dev/null 2>&1; then
    python3 -c 'import json; print(json.dumps({"additional_context":
      "LazyTree CLI not found on PATH. Install/build lazytree, then run "
      "`lazytree repo add <path>` and `/lazytree-session` before editing."}))'
    exit 0
  fi

  # Resolve existing session by name, else create.
  if ! ROOT=$("$LT_BIN" path "$NAME" 2>/dev/null); then
    CREATE_ARGS=("$NAME")
    if [[ -n "${LAZYTREE_REPO_ID:-}" ]]; then
      CREATE_ARGS=(--repo "$LAZYTREE_REPO_ID" "$NAME")
    elif [[ -n "$WORKSPACE" ]]; then
      CREATE_ARGS=(--repo "$WORKSPACE" "$NAME")
    fi
    ROOT=$("$LT_BIN" create "${CREATE_ARGS[@]}" 2>/dev/null || true)
  fi

  if [[ -z "${ROOT:-}" || ! -d "${ROOT:-}" ]]; then
    python3 -c 'import json; print(json.dumps({"additional_context":
      "LazyTree session could not be created automatically. "
      "Register the repo with `lazytree repo add` then `/lazytree-session`."}))'
    exit 0
  fi

  BRANCH=$(git -C "$ROOT" branch --show-current 2>/dev/null || echo "lazytree/$NAME")
  python3 - "$MAP_FILE" "$SESSION_ID" "$NAME" "$ROOT" "$BRANCH" "$WORKSPACE" <<'PY'
import json, sys
from datetime import datetime, timezone
map_file, session_id, name, root, branch, workspace = sys.argv[1:]
json.dump({
    "session_id": session_id,
    "name": name,
    "root": root,
    "branch": branch,
    "workspace": workspace,
    "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
}, open(map_file, "w"), indent=2)
PY
fi

python3 - "$SESSION_ID" "$ROOT" "$BRANCH" <<'PY'
import json, sys
session_id, root, branch = sys.argv[1:]
ctx = (
    "# LazyTree session (auto)\n"
    "This composer session is paired with a LazyTree workspace.\n\n"
    f"- LazyTree root: `{root}`\n"
    f"- Git branch: `{branch}`\n"
    "- Mapping: `.cursor/lazytree-sessions/`\n\n"
    "Mandatory protocol before any Write/Edit/Shell that changes the repo:\n"
    "1. Prefer the skill `lazytree-session` if available.\n"
    f"2. Use `{root}` as the working directory for all project edits and git commands.\n"
    "3. Do NOT modify files under the primary checkout outside that root.\n"
    f"4. Confirm with `git -C {root} branch --show-current` (expect `{branch}`).\n"
)
print(json.dumps({
    "env": {
        "LAZYTREE_SESSION": session_id,
        "LAZYTREE_ROOT": root,
        "LAZYTREE_BRANCH": branch,
    },
    "additional_context": ctx,
}))
PY
exit 0
