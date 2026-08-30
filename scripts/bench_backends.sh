#!/usr/bin/env bash
# Compare LazyTree filesystem plugins: fuse-overlayfs vs unionfs-fuse.
#
# On Linux this is the best proxy for macOS (Auto → unionfs-fuse). Absolute
# Darwin/macFUSE numbers will differ — re-run this script on a Mac for real data.
set -euo pipefail

ROOT="${BENCH_ROOT:-/tmp/lazytree-backend-bench}"
LT_BIN="${LT_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/lazytree}"
OUT_MD="${OUT_MD:-$(cd "$(dirname "$0")/.." && pwd)/docs/benchmarks-macos-proxy.md}"
SAMPLES="${SAMPLES:-5}"
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -x "$LT_BIN" ]]; then
  echo "building release binary..." >&2
  cargo build --release -q --manifest-path "$REPO_DIR/Cargo.toml"
fi
if [[ ! -x "$LT_BIN" ]]; then
  echo "missing lazytree binary: $LT_BIN" >&2
  exit 1
fi

nanos() {
  # macOS date(1) has no %N; use python for portability.
  if [[ "$(uname -s)" == "Darwin" ]]; then
    python3 -c 'import time; print(int(time.time() * 1e9))'
  else
    date +%s%N
  fi
}
ms_between() { echo $((( $2 - $1 ) / 1000000)); }
median() {
  if [[ $# -eq 0 ]]; then echo 0; return; fi
  local sorted arr n
  sorted=$(printf '%s\n' "$@" | sort -n)
  # shellcheck disable=SC2206
  arr=($sorted)
  n=${#arr[@]}
  echo "${arr[$((n / 2))]}"
}

umount_all() {
  local p
  while read -r p; do
    [[ -n "$p" ]] || continue
    fusermount3 -u "$p" 2>/dev/null \
      || fusermount -u "$p" 2>/dev/null \
      || sudo -n fusermount3 -u "$p" 2>/dev/null \
      || sudo -n umount -l "$p" 2>/dev/null \
      || umount "$p" 2>/dev/null \
      || true
  done < <(mount 2>/dev/null | awk '/fuse-overlayfs|unionfs/{print $3}')
}

generate_repo() {
  local dest="$1" files="$2" payload_kb="$3"
  rm -rf "$dest"
  mkdir -p "$dest"
  local i d
  for i in $(seq 1 "$files"); do
    d="$dest/src/$(printf '%03d' $((i % 100)))"
    mkdir -p "$d"
    if [[ "$payload_kb" -eq 0 ]]; then
      printf 'file-%s\n' "$i" >"$d/f-$i.txt"
    else
      dd if=/dev/zero of="$d/f-$i.bin" bs=1024 count="$payload_kb" status=none 2>/dev/null
    fi
  done
  git -C "$dest" init -q
  git -C "$dest" config user.email "bench@lazytree.dev"
  git -C "$dest" config user.name "bench"
  git -C "$dest" add -A
  git -C "$dest" commit -qm "bench-base"
  git -C "$dest" branch -M main
}

write_backend_config() {
  local home="$1" backend="$2"
  mkdir -p "$home"
  cat >"$home/config.json" <<EOF
{
  "version": 1,
  "filesystem_backend": "$backend"
}
EOF
}

# Prints: create_p50 fs_p50 git_p50 status_p50 destroy_p50 used_backend create_samples fs_samples status_samples
bench_backend() {
  local label="$1" files="$2" payload_kb="$3" backend="$4"
  local case_root="$ROOT/$label/$backend"
  local shared_repo="$ROOT/$label/_repo"

  rm -rf "$case_root"
  mkdir -p "$case_root"

  if [[ ! -d "$shared_repo/.git" ]]; then
    echo "[bench] generating $label (files=$files payload_kb=$payload_kb)" >&2
    generate_repo "$shared_repo" "$files" "$payload_kb"
  fi
  cp -a "$shared_repo" "$case_root/repo"

  export LAZYTREE_HOME="$case_root/lt-home"
  rm -rf "$LAZYTREE_HOME"
  write_backend_config "$LAZYTREE_HOME" "$backend"

  "$LT_BIN" repo add "$case_root/repo" >/dev/null

  local lt_create=() lt_status=() lt_destroy=() lt_fs=() lt_git=()
  local n name json path t0 t1 used_backend="?"
  for n in $(seq 1 "$SAMPLES"); do
    name="lt-$n"
    t0=$(nanos)
    json=$("$LT_BIN" create --json "$name")
    t1=$(nanos)
    lt_create+=("$(ms_between "$t0" "$t1")")
    lt_fs+=("$(printf '%s' "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["timings"]["filesystem_ms"])')")
    lt_git+=("$(printf '%s' "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["timings"]["git_ms"])')")
    path=$(printf '%s' "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["path"])')
    if [[ "$n" -eq 1 ]]; then
      used_backend=$(printf '%s' "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["filesystem"]["backend"])')
      echo "[bench] $label $backend -> mounted as $used_backend" >&2
    fi

    t0=$(nanos)
    git -C "$path" status --porcelain >/dev/null
    t1=$(nanos)
    lt_status+=("$(ms_between "$t0" "$t1")")
  done

  for n in $(seq 1 "$SAMPLES"); do
    name="lt-$n"
    t0=$(nanos)
    "$LT_BIN" destroy "$name" --force >/dev/null
    t1=$(nanos)
    lt_destroy+=("$(ms_between "$t0" "$t1")")
  done

  umount_all

  printf '%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' \
    "$label" "$backend" "$used_backend" \
    "$(median "${lt_create[@]}")" \
    "$(median "${lt_fs[@]}")" \
    "$(median "${lt_git[@]}")" \
    "$(median "${lt_status[@]}")" \
    "$(median "${lt_destroy[@]}")" \
    "${lt_create[*]}" \
    "${lt_fs[*]}" \
    "${lt_status[*]}"
}

HOST_OS=$(uname -s)
HOST_NOTE="Linux unionfs-fuse is the macOS plugin code path (proxy). Not Darwin/macFUSE."
if [[ "$HOST_OS" == "Darwin" ]]; then
  HOST_NOTE="Native macOS run (unionfs-fuse via macFUSE/Fuse-T)."
fi

umount_all
rm -rf "$ROOT"
mkdir -p "$ROOT"

BACKENDS=("fuse_overlayfs" "unionfs_fuse")
if [[ "$HOST_OS" == "Darwin" ]]; then
  BACKENDS=("unionfs_fuse")
fi

CASES=("tiny_200_files:200:0" "medium_5000_files:5000:0" "fat_500x64KB:500:64")
ROWS=()

for spec in "${CASES[@]}"; do
  IFS=: read -r label files payload <<<"$spec"
  for backend in "${BACKENDS[@]}"; do
    echo "[bench] $label x $backend" >&2
    ROWS+=("$(bench_backend "$label" "$files" "$payload" "$backend")")
  done
done

{
  cat <<HDR
# LazyTree backend benchmarks (macOS path)

Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)  
Host: $(uname -srm)  
Binary: \`$LT_BIN\`  
Samples per metric: ${SAMPLES}

## Honesty

- **$HOST_NOTE**
- This measures the **unionfs-fuse plugin** (macOS Auto backend) against **fuse-overlayfs** (Linux default path) where both exist.
- Real Mac absolute ms will move with macFUSE vs Fuse-T, disk, and SIP; use ratios + filesystem_ms split.
- Create includes mount **and** private Git init (seed index). Registration is excluded.

## Results

| Case | Backend | Mounted as | Create P50 | FS fork P50 | Git init P50 | First status P50 | Destroy P50 |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
HDR

  for row in "${ROWS[@]}"; do
    IFS='|' read -r lab be used create_p50 fs_p50 git_p50 status_p50 destroy_p50 _c _f _s <<<"$row"
    echo "| $lab | \`$be\` | \`$used\` | ${create_p50} ms | ${fs_p50} ms | ${git_p50} ms | ${status_p50} ms | ${destroy_p50} ms |"
  done

  cat <<'HDR2'

### Samples

| Case | Backend | Create samples | FS fork samples | Status samples |
| --- | --- | --- | --- | --- |
HDR2

  for row in "${ROWS[@]}"; do
    IFS='|' read -r lab be _u _a _b _c _d _e create_s fs_s status_s <<<"$row"
    echo "| $lab | \`$be\` | $create_s | $fs_s | $status_s |"
  done

  cat <<'FOOT'

## How to re-run on a real Mac

```bash
brew install macfuse unionfs-fuse   # or Fuse-T
cargo build --release
./scripts/bench_backends.sh
# writes docs/benchmarks-macos-proxy.md — copy to docs/benchmarks-macos.md on Darwin
```

## Interpretation

- **Create stays ~O(1):** FS fork P50 should stay single-digit ms and flat across tiny → medium → fat (overlay model, not clone-tree).
- **unionfs vs fuse-overlayfs create:** expect near-parity on the create/mount axis.
- **First git status:** unionfs is often somewhat slower (FUSE tax); warm-status still matters on Darwin.
- On Linux this file is a **macOS plugin proxy**. On Darwin, treat absolute ms as the real Mac numbers.

FOOT
} | tee "$OUT_MD"

echo "Wrote $OUT_MD" >&2
