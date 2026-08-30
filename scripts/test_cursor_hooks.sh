#!/usr/bin/env bash
# Validate Cursor soft-integration hooks + CLI helpers without a live Cursor IDE.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${LAZYTREE_BIN:-$ROOT/target/release/lazytree}"
HOME_DIR="${TMPDIR:-/tmp}/lt-cursor-hook-test-$$"
WORK_DIR="$HOME_DIR/workspace"
trap '
  if command -v findmnt >/dev/null 2>&1; then
    findmnt -n -o TARGET | grep "^$HOME_DIR" | sort -r | while read -r m; do
      sudo umount "$m" 2>/dev/null || umount "$m" 2>/dev/null || true
    done
  fi
  find "$HOME_DIR" -path "*/fs/root" -type d 2>/dev/null | while read -r m; do
    sudo umount "$m" 2>/dev/null || umount "$m" 2>/dev/null || true
  done
  rm -rf "$HOME_DIR" 2>/dev/null || true
' EXIT
export LAZYTREE_HOME="$HOME_DIR"
mkdir -p "$WORK_DIR"
cd "$WORK_DIR"

REPO="$HOME_DIR/fixture"
mkdir -p "$REPO"
(
  cd "$REPO"
  git init -q
  git config user.email t@t.com
  git config user.name t
  echo hi > README.md
  git add README.md
  git commit -qm init
)

REPO_ID=$("$BIN" repo add --json "$REPO" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
echo "repo=$REPO_ID"

# Install hooks into workspace (simulates consumer project)
LAZYTREE_CURSOR_ASSETS="$ROOT" "$BIN" cursor setup --target "$WORK_DIR"
test -f "$WORK_DIR/.cursor/hooks.json"
test -x "$WORK_DIR/.cursor/hooks/lazytree-session-start.sh"

# --- sessionStart ---
OUT=$(printf '%s\n' '{"session_id":"agent-test-1","conversation_id":"agent-test-1","is_background_agent":false,"composer_mode":"agent","workspace_roots":[]}' \
  | LAZYTREE_BIN="$BIN" LAZYTREE_REPO_ID="$REPO_ID" \
    bash "$WORK_DIR/.cursor/hooks/lazytree-session-start.sh")
echo "sessionStart out: $OUT"
echo "$OUT" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "LAZYTREE_ROOT" in d["env"]; assert "LazyTree session" in d["additional_context"]'
ROOT_PATH=$(echo "$OUT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["env"]["LAZYTREE_ROOT"])')
test -d "$ROOT_PATH"
test -f "$ROOT_PATH/.git"
test -f "$WORK_DIR/.cursor/lazytree-sessions/agent-test-1.json"
echo "sessionStart OK -> $ROOT_PATH"

# Idempotent re-fire
OUT2=$(printf '%s\n' '{"session_id":"agent-test-1","conversation_id":"agent-test-1"}' \
  | LAZYTREE_BIN="$BIN" LAZYTREE_REPO_ID="$REPO_ID" \
    bash "$WORK_DIR/.cursor/hooks/lazytree-session-start.sh")
ROOT2=$(echo "$OUT2" | python3 -c 'import json,sys; print(json.load(sys.stdin)["env"]["LAZYTREE_ROOT"])')
test "$ROOT_PATH" = "$ROOT2"
echo "sessionStart idempotent OK"

# --- preToolUse allow (env + map) ---
printf '%s\n' "{\"session_id\":\"agent-test-1\",\"tool_name\":\"Write\",\"tool_input\":{\"path\":\"$ROOT_PATH/ok.txt\"},\"cwd\":\"$ROOT_PATH\"}" \
  | LAZYTREE_ROOT="$ROOT_PATH" bash "$WORK_DIR/.cursor/hooks/lazytree-pretool-gate.sh" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("permission")=="allow"'
echo "preToolUse allow OK"

# --- preToolUse deny ---
printf '%s\n' '{"session_id":"agent-test-1","tool_name":"Write","tool_input":{"path":"/tmp/outside.txt"},"cwd":"/tmp"}' \
  | LAZYTREE_ROOT="$ROOT_PATH" bash "$WORK_DIR/.cursor/hooks/lazytree-pretool-gate.sh" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("permission")=="deny"'
echo "preToolUse deny OK"

# --- preToolUse via map only (no env) ---
printf '%s\n' '{"session_id":"agent-test-1","tool_name":"Write","tool_input":{"path":"/tmp/outside2.txt"}}' \
  | env -u LAZYTREE_ROOT bash "$WORK_DIR/.cursor/hooks/lazytree-pretool-gate.sh" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("permission")=="deny"'
echo "preToolUse map-only deny OK"

# --- fail-open when unpaired ---
printf '%s\n' '{"session_id":"unknown","tool_name":"Write","tool_input":{"path":"/tmp/x"}}' \
  | env -u LAZYTREE_ROOT bash "$WORK_DIR/.cursor/hooks/lazytree-pretool-gate.sh" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d=={} or d.get("permission") in (None,"allow")'
echo "preToolUse fail-open OK"

# --- shell allow ---
printf '%s\n' "{\"session_id\":\"agent-test-1\",\"command\":\"git status\",\"cwd\":\"$ROOT_PATH\"}" \
  | LAZYTREE_ROOT="$ROOT_PATH" bash "$WORK_DIR/.cursor/hooks/lazytree-shell-gate.sh" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("permission")=="allow"'
echo "shell allow OK"

# --- shell deny commit outside ---
printf '%s\n' '{"session_id":"agent-test-1","command":"git commit -m x","cwd":"/tmp"}' \
  | LAZYTREE_ROOT="$ROOT_PATH" bash "$WORK_DIR/.cursor/hooks/lazytree-shell-gate.sh" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("permission")=="deny"'
echo "shell deny OK"

# --- shell allow commit inside via -C ---
printf '%s\n' "{\"session_id\":\"agent-test-1\",\"command\":\"git -C $ROOT_PATH commit -m x\",\"cwd\":\"/tmp\"}" \
  | LAZYTREE_ROOT="$ROOT_PATH" bash "$WORK_DIR/.cursor/hooks/lazytree-shell-gate.sh" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("permission")=="allow"'
echo "shell allow -C OK"

# --- cursor open --json / dry-run ---
NAME=$(python3 -c 'import json; print(json.load(open("'"$WORK_DIR"'/.cursor/lazytree-sessions/agent-test-1.json"))["name"])')
OPEN=$("$BIN" cursor open "$NAME" --json)
echo "$OPEN" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "root" in d and "branch" in d'
"$BIN" cursor open "$NAME" --dry-run | grep -q "$ROOT_PATH"
echo "cursor open OK"

echo "ALL CURSOR HOOK TESTS PASSED"
