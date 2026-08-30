#!/usr/bin/env bash
# Spike: core.fsmonitor fed by upperdir — cold git status with vs without.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LT_BIN="${LT_BIN:-$REPO_DIR/target/release/lazytree}"
ROOT="${SPIKE_ROOT:-/tmp/lazytree-fsm-spike}"
FILES="${FILES:-5000}"

if [[ ! -x "$LT_BIN" ]]; then
  cargo build --release -q --manifest-path "$REPO_DIR/Cargo.toml"
fi

nanos() { date +%s%N; }
ms_between() { echo $((( $2 - $1 ) / 1000000)); }

umount_all() {
  local p
  while read -r p; do
    [[ -n "$p" ]] || continue
    fusermount3 -u "$p" 2>/dev/null || sudo -n fusermount3 -u "$p" 2>/dev/null || umount "$p" 2>/dev/null || true
  done < <(mount 2>/dev/null | awk '/fuse-overlayfs|unionfs/{print $3}')
}

write_hook() {
  local hook="$1" upper="$2"
  cat >"$hook" <<'EOF'
#!/bin/bash
token="$2"
UPPER="__UPPER__"
n=1
[[ "$token" == lt:* ]] && n=$((${token#lt:}+1))
printf 'lt:%s\0' "$n"
if [[ -d "$UPPER" ]]; then
  find "$UPPER" \( -type f -o -type l -o -type c \) -printf '%P\0' 2>/dev/null \
    | while IFS= read -r -d '' rel; do
        case "$rel" in .git|.git/*) continue ;; esac
        printf '%s\0' "$rel"
      done
fi
EOF
  # Inject absolute upper path without breaking the quoted heredoc.
  sed -i "s|__UPPER__|${upper//|/\\|}|g" "$hook"
  chmod +x "$hook"
}

one_status_ms() {
  local path="$1"
  local t0 t1
  t0=$(nanos)
  git -C "$path" status --porcelain >/dev/null
  t1=$(nanos)
  ms_between "$t0" "$t1"
}

rm -rf "$ROOT"
umount_all
# If a previous spike left a busy mount, use a fresh root.
if ! mkdir -p "$ROOT/repo" 2>/dev/null; then
  ROOT="${ROOT}-$$"
  mkdir -p "$ROOT/repo"
fi
echo "[spike] generating $FILES files..." >&2
for i in $(seq 1 "$FILES"); do
  d="$ROOT/repo/src/$(printf '%03d' $((i % 100)))"
  mkdir -p "$d"
  printf 'f-%s\n' "$i" >"$d/f-$i.txt"
done
git -C "$ROOT/repo" init -q
git -C "$ROOT/repo" config user.email spike@lazytree.dev
git -C "$ROOT/repo" config user.name spike
git -C "$ROOT/repo" add -A
git -C "$ROOT/repo" commit -qm base

export LAZYTREE_HOME="$ROOT/lt"
export LAZYTREE_WARM_STATUS=0
"$LT_BIN" repo add "$ROOT/repo" >/dev/null

echo "[spike] A: cold status WITHOUT fsmonitor" >&2
path_a=$("$LT_BIN" create spike-a)
ms_a=$(one_status_ms "$path_a")
echo "  cold_no_fsm=${ms_a}ms"

echo "[spike] B: cold status WITH upperdir fsmonitor (empty upper)" >&2
path_b=$("$LT_BIN" create spike-b)
sess_b=$(dirname "$(dirname "$path_b")")
upper_b="$sess_b/upper"
hook_b="$sess_b/fsmonitor-upper.sh"
write_hook "$hook_b" "$upper_b"
git -C "$path_b" config core.fsmonitor "$hook_b"
git -C "$path_b" config core.fsmonitorHookVersion 2
ms_b=$(one_status_ms "$path_b")
echo "  cold_with_fsm_empty_upper=${ms_b}ms"

echo "[spike] C: after edit — correctness + status ms" >&2
# pick a file without SIGPIPE under pipefail
target=
while IFS= read -r -d '' f; do target=$f; break; done < <(find "$path_b/src" -type f -print0)
echo mutated >>"$target"
ms_c=$(one_status_ms "$path_b")
porc=$(git -C "$path_b" status --porcelain)
echo "  after_edit_fsm=${ms_c}ms"
echo "  porcelain=<<$porc>>"
if [[ -z "$porc" ]]; then
  echo "  FAIL: expected dirty porcelain" >&2
else
  echo "  OK: saw change"
fi

echo "[spike] D: worktree cold status" >&2
wt="$ROOT/wt"
git -C "$ROOT/repo" worktree add -b spike-wt "$wt" main >/dev/null
ms_d=$(one_status_ms "$wt")
echo "  worktree_cold=${ms_d}ms"

"$LT_BIN" destroy spike-a --force >/dev/null
"$LT_BIN" destroy spike-b --force >/dev/null
git -C "$ROOT/repo" worktree remove --force "$wt" >/dev/null 2>&1 || true
umount_all

cat <<SUM

## Spike summary ($FILES files)

| Mode | First git status |
| --- | ---: |
| LazyTree cold (no fsmonitor) | ${ms_a} ms |
| LazyTree cold (fsmonitor, empty upper) | ${ms_b} ms |
| LazyTree after 1 edit (fsmonitor) | ${ms_c} ms |
| git worktree cold | ${ms_d} ms |

SUM
