#!/usr/bin/env bash
# Local dogfood: build LazyTree, create a session, verify real git status + fsmonitor.
#
# Linux: fuse-overlayfs (and often passwordless sudo in VMs)
# macOS: macFUSE or Fuse-T + unionfs-fuse (brew install macfuse unionfs-fuse)
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

echo "== build =="
cargo build --release

LT="$REPO_DIR/target/release/lazytree"
export PATH="$REPO_DIR/target/release:$PATH"

ROOT="${DOGFOOD_ROOT:-$HOME/lazytree-dogfood}"
rm -rf "$ROOT"
mkdir -p "$ROOT/repo"
export LAZYTREE_HOME="$ROOT/lt-home"

echo "== sample repo =="
cd "$ROOT/repo"
git init -q
git config user.email "you@example.com"
git config user.name "Dogfood"
mkdir -p src
echo 'fn main() { println!("hi"); }' > src/main.rs
echo 'hello' > README.md
git add -A
git commit -qm "dogfood base"
cd "$REPO_DIR"

echo "== doctor =="
"$LT" doctor || true

echo "== register + create =="
"$LT" repo add "$ROOT/repo"
SESSION=dogfood-1
ROOT_PATH=$("$LT" create "$SESSION")
echo "session root: $ROOT_PATH"

echo "== fsmonitor config (session-local) =="
git -C "$ROOT_PATH" config --get core.fsmonitor || echo "(fsmonitor not set — check LAZYTREE_FSMONITOR)"
git -C "$ROOT_PATH" config --get core.fsmonitorHookVersion || true

echo "== real git status (1st / 2nd) =="
if command -v time >/dev/null 2>&1; then
  TIMEFORMAT='real %R'
  time git -C "$ROOT_PATH" status --porcelain
  time git -C "$ROOT_PATH" status --porcelain
else
  git -C "$ROOT_PATH" status --porcelain
  git -C "$ROOT_PATH" status --porcelain
fi

echo "== edit a file, status should show it =="
echo '// edited' >> "$ROOT_PATH/src/main.rs"
git -C "$ROOT_PATH" status --porcelain
echo "(expect:  M src/main.rs)"

echo "== upperdir should contain the copy-up =="
UPPER="$(dirname "$ROOT_PATH")/upper"
find "$UPPER" -type f 2>/dev/null | head -10 || true

echo ""
echo "OK. Work in:  cd \"$ROOT_PATH\""
echo "Destroy with: LAZYTREE_HOME=\"$LAZYTREE_HOME\" $LT destroy $SESSION --force"
echo "Disable fsm:  LAZYTREE_FSMONITOR=0 $LT create ..."
