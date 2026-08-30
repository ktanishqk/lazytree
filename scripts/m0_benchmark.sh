#!/usr/bin/env bash
# Compare LazyTree-style COW session create vs git worktree add.
set -euo pipefail

ROOT="${1:-/tmp/lazytree-m0-bench}"
FILE_COUNT="${FILE_COUNT:-2000}"
BACKEND="${LAZYTREE_FS_BACKEND:-auto}"

log() { printf '[bench] %s\n' "$*"; }

umount_path() {
  local p="$1"
  if findmnt -T "$p" >/dev/null 2>&1; then
    fusermount3 -u "$p" 2>/dev/null \
      || sudo -n fusermount3 -u "$p" 2>/dev/null \
      || umount "$p" 2>/dev/null \
      || sudo -n umount -l "$p" 2>/dev/null \
      || true
  fi
}

mount_overlay() {
  local lower="$1" upper="$2" work="$3" merged="$4"
  if [[ "$BACKEND" != "fuse" ]]; then
    if mount -t overlay overlay -o "lowerdir=${lower},upperdir=${upper},workdir=${work}" "$merged" 2>/dev/null \
      || sudo -n mount -t overlay overlay -o "lowerdir=${lower},upperdir=${upper},workdir=${work}" "$merged" 2>/dev/null; then
      echo kernel
      return 0
    fi
  fi
  if [[ "$BACKEND" != "kernel" ]]; then
    if fuse-overlayfs -o "lowerdir=${lower},upperdir=${upper},workdir=${work}" "$merged" 2>/dev/null \
      || sudo -n fuse-overlayfs -o "lowerdir=${lower},upperdir=${upper},workdir=${work},allow_other" "$merged" 2>/dev/null; then
      echo fuse-overlayfs
      return 0
    fi
  fi
  echo FAIL
  return 1
}

nanos() { date +%s%N; }

elapsed_ms() {
  local start="$1" end="$2"
  echo $(((end - start) / 1000000))
}

du_bytes() {
  du -sb "$1" 2>/dev/null | awk '{print $1}'
}

rm -rf "$ROOT"
mkdir -p "$ROOT"/{repo,worktrees,cow/{base,sessions}}

log "generating synthetic repo with ${FILE_COUNT} files"
for i in $(seq 1 "$FILE_COUNT"); do
  d="$ROOT/repo/src/$(printf '%03d' $((i % 50)))"
  mkdir -p "$d"
  printf 'file-%s payload-%s\n' "$i" "$i" >"$d/f-$i.txt"
done
git -C "$ROOT/repo" init -q
git -C "$ROOT/repo" config user.email "m0@lazytree.dev"
git -C "$ROOT/repo" config user.name "LazyTree M0"
git -C "$ROOT/repo" add .
git -C "$ROOT/repo" commit -qm "bench-base"
git -C "$ROOT/repo" branch -M main

# Materialize immutable base once (allowed to be O(repo) per PRD §9)
log "creating immutable base copy (registration cost)"
t0=$(nanos)
cp -a "$ROOT/repo/." "$ROOT/cow/base/"
t1=$(nanos)
BASE_COPY_MS=$(elapsed_ms "$t0" "$t1")
BASE_BYTES=$(du_bytes "$ROOT/cow/base")

# git worktree create latency (5 samples)
WT_TIMES=()
for n in 1 2 3 4 5; do
  t0=$(nanos)
  git -C "$ROOT/repo" worktree add -b "wt-$n" "$ROOT/worktrees/wt-$n" main >/dev/null
  t1=$(nanos)
  WT_TIMES+=("$(elapsed_ms "$t0" "$t1")")
done
WT_DISK=$(du_bytes "$ROOT/worktrees")

# COW session create latency (5 samples) — filesystem fork only
COW_TIMES=()
BACKEND_USED=""
for n in 1 2 3 4 5; do
  s="$ROOT/cow/sessions/s$n"
  mkdir -p "$s"/{upper,work,merged}
  t0=$(nanos)
  BACKEND_USED=$(mount_overlay "$ROOT/cow/base" "$s/upper" "$s/work" "$s/merged")
  t1=$(nanos)
  [[ "$BACKEND_USED" != "FAIL" ]]
  COW_TIMES+=("$(elapsed_ms "$t0" "$t1")")
done

# Upper-layer disk after untouched sessions
COW_UPPER_BYTES=0
for n in 1 2 3 4 5; do
  b=$(du_bytes "$ROOT/cow/sessions/s$n/upper")
  COW_UPPER_BYTES=$((COW_UPPER_BYTES + b))
done

# Modify one file in session 1 and measure upper growth
printf 'mutated\n' >"$ROOT/cow/sessions/s1/merged/src/000/f-1.txt"
COW_UPPER_AFTER=$(du_bytes "$ROOT/cow/sessions/s1/upper")

avg() {
  local sum=0 c=0 x
  for x in "$@"; do
    sum=$((sum + x))
    c=$((c + 1))
  done
  echo $((sum / c))
}

# cleanup mounts
for n in 1 2 3 4 5; do
  umount_path "$ROOT/cow/sessions/s$n/merged"
done

cat <<REPORT
=== Milestone 0 benchmark ===
files:                 ${FILE_COUNT}
backend:               ${BACKEND_USED}
base_copy_ms:          ${BASE_COPY_MS}
base_bytes:            ${BASE_BYTES}
git_worktree_ms:       ${WT_TIMES[*]}  (avg $(avg "${WT_TIMES[@]}"))
cow_session_ms:        ${COW_TIMES[*]}  (avg $(avg "${COW_TIMES[@]}"))
git_worktree_disk_B:   ${WT_DISK}
cow_upper_untouched_B: ${COW_UPPER_BYTES}
cow_upper_after_1edit: ${COW_UPPER_AFTER}
note: cow times are filesystem mount only (no private Git state yet)
REPORT
