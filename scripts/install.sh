#!/usr/bin/env bash
# Put the release CLI on PATH as ~/.local/bin/lazytree (no extra packages).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${LT_BIN:-$ROOT/target/release/lazytree}"
DEST_DIR="${LAZYTREE_INSTALL_DIR:-$HOME/.local/bin}"

if [[ ! -x "$BIN" ]]; then
  echo "building release binary..." >&2
  cargo build --release -q --manifest-path "$ROOT/Cargo.toml"
fi
if [[ ! -x "$BIN" ]]; then
  echo "missing $BIN" >&2
  exit 1
fi

mkdir -p "$DEST_DIR"
cp "$BIN" "$DEST_DIR/lazytree"
chmod +x "$DEST_DIR/lazytree"
echo "installed $DEST_DIR/lazytree"
if ! command -v lazytree >/dev/null 2>&1; then
  echo "add to PATH: export PATH=\"$DEST_DIR:\$PATH\"" >&2
fi
