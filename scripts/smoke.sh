#!/usr/bin/env bash
# Fast smoke: unit/integration tests + cursor hooks + tiny create bench.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== cargo test --tests =="
cargo test --tests --quiet

echo "== release build =="
cargo build --release --quiet
BIN="$ROOT/target/release/lazytree"

echo "== cursor hooks =="
LAZYTREE_BIN="$BIN" ./scripts/test_cursor_hooks.sh

echo "== tiny create bench (n=5) =="
H="${TMPDIR:-/tmp}/lt-smoke-$$"
export LAZYTREE_HOME="$H"
cleanup() {
  set +e
  find "$H" -path '*/fs/root' -type d 2>/dev/null | while read -r m; do
    sudo umount "$m" 2>/dev/null || umount "$m" 2>/dev/null || true
  done
  rm -rf "$H" 2>/dev/null || true
}
trap cleanup EXIT
mkdir -p "$H/repo"
cd "$H/repo"
git init -q -b main
git config user.email smoke@lazytree.dev
git config user.name smoke
echo hi > README.md
git add README.md && git commit -qm init
"$BIN" repo add . >/dev/null
TIMES=()
for i in 1 2 3 4 5; do
  start=$(date +%s%3N)
  "$BIN" create "s$i" >/dev/null
  end=$(date +%s%3N)
  TIMES+=($((end - start)))
done
echo "create_ms: ${TIMES[*]}"

echo "SMOKE OK"
