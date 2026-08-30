#!/usr/bin/env bash
# Milestone 4: LazyTree vs git worktree benchmarks (honest, separated timings).
set -euo pipefail

ROOT="${BENCH_ROOT:-/tmp/lazytree-m4-bench}"
LT_BIN="${LT_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/lazytree}"
OUT_MD="${OUT_MD:-$(cd "$(dirname "$0")/.." && pwd)/docs/benchmarks.md}"
SAMPLES="${SAMPLES:-5}"

if [[ ! -x "$LT_BIN" ]]; then
  echo "missing lazytree binary: $LT_BIN (run cargo build --release)" >&2
  exit 1
fi

nanos() { date +%s%N; }
ms_between() { echo $((( $2 - $1 ) / 1000000)); }
avg() {
  local sum=0 c=0 x
  for x in "$@"; do sum=$((sum + x)); c=$((c + 1)); done
  if [[ $c -eq 0 ]]; then echo 0; else echo $((sum / c)); fi
}
median() {
  local sorted
  sorted=$(printf '%s\n' "$@" | sort -n)
  local arr=($sorted)
  local n=${#arr[@]}
  echo "${arr[$((n / 2))]}"
}
du_bytes() { du -sb "$1" 2>/dev/null | awk '{print $1}'; }
umount_all() {
  local p
  for p in $(mount | awk '/fuse-overlayfs/{print $3}'); do
    fusermount3 -u "$p" 2>/dev/null || sudo -n fusermount3 -u "$p" 2>/dev/null || sudo -n umount -l "$p" 2>/dev/null || true
  done
}

generate_repo() {
  local dest="$1" files="$2" payload_kb="$3"
  rm -rf "$dest"
  mkdir -p "$dest"
  local i d
  for i in $(seq 1 "$files"); do
    d="$dest/src/$(printf '%03d' $((i % 100)))"
    mkdir -p "$d"
    # payload_kb=0 → tiny text; else dd sparse-ish payload
    if [[ "$payload_kb" -eq 0 ]]; then
      printf 'file-%s\n' "$i" >"$d/f-$i.txt"
    else
      dd if=/dev/zero of="$d/f-$i.bin" bs=1024 count="$payload_kb" status=none 2>/dev/null
    fi
  done
  git -C "$dest" init -q
  git -C "$dest" config user.email "bench@lazytree.dev"
  git -C "$dest" config user.name "bench"
  git -C "$dest" config receive.denyCurrentBranch updateInstead
  git -C "$dest" add -A
  git -C "$dest" commit -qm "bench-base"
  git -C "$dest" branch -M main
}

bench_case() {
  local label="$1" files="$2" payload_kb="$3"
  local case_root="$ROOT/$label"
  rm -rf "$case_root"
  mkdir -p "$case_root"/{repo,worktrees,lt-home}

  echo "[bench] generating $label (files=$files payload_kb=$payload_kb)" >&2
  local t0 t1 gen_ms
  t0=$(nanos)
  generate_repo "$case_root/repo" "$files" "$payload_kb"
  t1=$(nanos)
  gen_ms=$(ms_between "$t0" "$t1")
  local repo_bytes
  repo_bytes=$(du_bytes "$case_root/repo")

  # --- git worktree ---
  local wt_create=() wt_status=() wt_destroy=()
  local n name wt_path
  for n in $(seq 1 "$SAMPLES"); do
    name="wt-$n"
    wt_path="$case_root/worktrees/$name"
    t0=$(nanos)
    git -C "$case_root/repo" worktree add -b "$name" "$wt_path" main >/dev/null
    t1=$(nanos)
    wt_create+=("$(ms_between "$t0" "$t1")")

    t0=$(nanos)
    git -C "$wt_path" status --porcelain >/dev/null
    t1=$(nanos)
    wt_status+=("$(ms_between "$t0" "$t1")")
  done
  local wt_disk
  wt_disk=$(du_bytes "$case_root/worktrees")

  for n in $(seq 1 "$SAMPLES"); do
    name="wt-$n"
    wt_path="$case_root/worktrees/$name"
    t0=$(nanos)
    git -C "$case_root/repo" worktree remove --force "$wt_path" >/dev/null
    git -C "$case_root/repo" branch -D "$name" >/dev/null 2>&1 || true
    t1=$(nanos)
    wt_destroy+=("$(ms_between "$t0" "$t1")")
  done

  # --- lazytree ---
  export LAZYTREE_HOME="$case_root/lt-home"
  t0=$(nanos)
  "$LT_BIN" repo add "$case_root/repo" >/dev/null
  t1=$(nanos)
  local reg_ms
  reg_ms=$(ms_between "$t0" "$t1")

  local lt_create=() lt_status=() lt_destroy=() lt_fs=() lt_git=()
  for n in $(seq 1 "$SAMPLES"); do
    name="lt-$n"
    t0=$(nanos)
    json=$("$LT_BIN" create --json "$name")
    t1=$(nanos)
    lt_create+=("$(ms_between "$t0" "$t1")")
    lt_fs+=("$(echo "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["timings"]["filesystem_ms"])')")
    lt_git+=("$(echo "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["timings"]["git_ms"])')")
    path=$(echo "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["path"])')

    t0=$(nanos)
    git -C "$path" status --porcelain >/dev/null
    t1=$(nanos)
    lt_status+=("$(ms_between "$t0" "$t1")")
  done

  local lt_sess_bytes=0
  local sdir u g m
  for sdir in "$LAZYTREE_HOME"/sessions/session_*; do
    [[ -d "$sdir" ]] || continue
    u=$(du_bytes "$sdir/fs/upper")
    g=$(du_bytes "$sdir/git")
    m=$(du_bytes "$sdir/metadata.json")
    lt_sess_bytes=$((lt_sess_bytes + u + g + m))
  done
  local lt_base_bytes
  lt_base_bytes=$(du_bytes "$LAZYTREE_HOME/repositories")

  for n in $(seq 1 "$SAMPLES"); do
    name="lt-$n"
    t0=$(nanos)
    "$LT_BIN" destroy "$name" --force >/dev/null
    t1=$(nanos)
    lt_destroy+=("$(ms_between "$t0" "$t1")")
  done

  # Single-edit upper growth on a fresh session
  path=$("$LT_BIN" create "edit-probe")
  local upper_before upper_after
  upper_before=$(du_bytes "$(dirname "$path")/upper")
  # edit one existing file if present
  local target
  target=$(find "$path/src" -type f 2>/dev/null | head -1 || true)
  if [[ -n "$target" ]]; then
    echo mutated >>"$target"
  else
    echo mutated >"$path/mut.txt"
  fi
  upper_after=$(du_bytes "$(dirname "$path")/upper")
  "$LT_BIN" destroy edit-probe --force >/dev/null

  umount_all

  cat <<ROW
### $label

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | ${gen_ms} ms | (same repo) |
| Repo bytes | ${repo_bytes} | ${repo_bytes} |
| Registration (one-time) | n/a | ${reg_ms} ms |
| Registered base bytes | n/a | ${lt_base_bytes} |
| Create P50 (n=${SAMPLES}) | $(median "${wt_create[@]}") ms | $(median "${lt_create[@]}") ms |
| Create avg | $(avg "${wt_create[@]}") ms | $(avg "${lt_create[@]}") ms |
| Create samples | ${wt_create[*]} | ${lt_create[*]} |
| Filesystem fork P50 | n/a | $(median "${lt_fs[@]}") ms |
| Git state init P50 | n/a | $(median "${lt_git[@]}") ms |
| First \`git status\` P50 | $(median "${wt_status[@]}") ms | $(median "${lt_status[@]}") ms |
| Destroy P50 | $(median "${wt_destroy[@]}") ms | $(median "${lt_destroy[@]}") ms |
| Disk for ${SAMPLES} sessions | ${wt_disk} B | ${lt_sess_bytes} B (upper+git+meta only) |
| Upper after 1 edit | n/a | ${upper_before} → ${upper_after} B |

ROW
}

umount_all
rm -rf "$ROOT"
mkdir -p "$ROOT"

{
  cat <<HDR
# LazyTree Benchmarks (Milestone 4)

Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)  
Host: $(uname -srm)  
Backend: fuse-overlayfs (sudo) in Cursor Cloud Agent VM  
Binary: \`$LT_BIN\`  
Samples per metric: ${SAMPLES}

## Honesty notes

- LazyTree **create** includes OverlayFS mount **and** private Git metadata init (index via \`read-tree\`).
- Registration (\`repo add\`) is intentionally O(repository size) and is **not** counted as session create.
- Worktree disk often looks small because Git hardlinks objects; compare working-tree materialization and session-local metadata, not object DB size alone.
- These numbers are from a nested cloud VM. Absolute milliseconds will differ on bare metal; ratios matter more.

## Results

HDR

  bench_case "tiny_200_files" 200 0
  bench_case "medium_5000_files" 5000 0
  bench_case "large_20000_files" 20000 0
  # fewer large-payload files to keep runtime reasonable
  bench_case "fat_500x64KB" 500 64

  cat <<FOOT

## Interpretation

See commit notes / design-decisions for whether filesystem fork or Git index setup dominates.

FOOT
} | tee "$OUT_MD"

