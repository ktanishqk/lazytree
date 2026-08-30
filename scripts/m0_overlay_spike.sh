#!/usr/bin/env bash
# Milestone 0 feasibility spike: immutable base + two COW sessions + Git smoke.
# Prefer kernel OverlayFS; fall back to fuse-overlayfs (often via sudo in nested VMs).
set -euo pipefail

ROOT="${1:-/tmp/lazytree-m0-spike}"
BACKEND="${LAZYTREE_FS_BACKEND:-auto}"

log() { printf '[m0] %s\n' "$*"; }
die() { printf '[m0] ERROR: %s\n' "$*" >&2; exit 1; }

cleanup() {
  if [[ -n "${MERGED_A:-}" ]]; then
    umount_path "$MERGED_A" || true
  fi
  if [[ -n "${MERGED_B:-}" ]]; then
    umount_path "$MERGED_B" || true
  fi
}
trap cleanup EXIT

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

umount_path() {
  local p="$1"
  if findmnt -T "$p" >/dev/null 2>&1; then
    if fusermount3 -u "$p" 2>/dev/null; then
      return 0
    fi
    if sudo -n fusermount3 -u "$p" 2>/dev/null; then
      return 0
    fi
    if umount "$p" 2>/dev/null; then
      return 0
    fi
    sudo -n umount "$p" 2>/dev/null || sudo -n umount -l "$p" 2>/dev/null || true
  fi
}

try_kernel_overlay() {
  local lower="$1" upper="$2" work="$3" merged="$4"
  mount -t overlay overlay \
    -o "lowerdir=${lower},upperdir=${upper},workdir=${work}" \
    "$merged" 2>/dev/null && return 0
  sudo -n mount -t overlay overlay \
    -o "lowerdir=${lower},upperdir=${upper},workdir=${work}" \
    "$merged" 2>/dev/null && return 0
  return 1
}

try_fuse_overlay() {
  local lower="$1" upper="$2" work="$3" merged="$4"
  need_cmd fuse-overlayfs
  if fuse-overlayfs -o "lowerdir=${lower},upperdir=${upper},workdir=${work}" "$merged" 2>/dev/null; then
    return 0
  fi
  if sudo -n fuse-overlayfs -o "lowerdir=${lower},upperdir=${upper},workdir=${work},allow_other" "$merged" 2>/dev/null; then
    return 0
  fi
  return 1
}

mount_overlay() {
  local lower="$1" upper="$2" work="$3" merged="$4"
  case "$BACKEND" in
    kernel)
      try_kernel_overlay "$lower" "$upper" "$work" "$merged" || die "kernel OverlayFS mount failed"
      ACTIVE_BACKEND=kernel
      ;;
    fuse)
      try_fuse_overlay "$lower" "$upper" "$work" "$merged" || die "fuse-overlayfs mount failed"
      ACTIVE_BACKEND=fuse-overlayfs
      ;;
    auto)
      if try_kernel_overlay "$lower" "$upper" "$work" "$merged"; then
        ACTIVE_BACKEND=kernel
      elif try_fuse_overlay "$lower" "$upper" "$work" "$merged"; then
        ACTIVE_BACKEND=fuse-overlayfs
      else
        die "neither kernel OverlayFS nor fuse-overlayfs could mount"
      fi
      ;;
    *)
      die "unknown LAZYTREE_FS_BACKEND=$BACKEND"
      ;;
  esac
}

main() {
  need_cmd git
  need_cmd findmnt

  rm -rf "$ROOT"
  mkdir -p "$ROOT"/{lower,a/{upper,work,merged},b/{upper,work,merged}}

  # Immutable base tree + Git repo
  printf 'original\n' >"$ROOT/lower/foo"
  printf 'shared\n' >"$ROOT/lower/bar"
  mkdir -p "$ROOT/lower/src"
  printf 'int main(){return 0;}\n' >"$ROOT/lower/src/main.c"

  git -C "$ROOT/lower" init -q
  git -C "$ROOT/lower" config user.email "m0@lazytree.dev"
  git -C "$ROOT/lower" config user.name "LazyTree M0"
  git -C "$ROOT/lower" add .
  git -C "$ROOT/lower" commit -qm "base"

  MERGED_A="$ROOT/a/merged"
  MERGED_B="$ROOT/b/merged"

  mount_overlay "$ROOT/lower" "$ROOT/a/upper" "$ROOT/a/work" "$MERGED_A"
  local backend_a="$ACTIVE_BACKEND"
  mount_overlay "$ROOT/lower" "$ROOT/b/upper" "$ROOT/b/work" "$MERGED_B"
  local backend_b="$ACTIVE_BACKEND"

  [[ "$backend_a" == "$backend_b" ]] || log "warning: sessions used different backends ($backend_a vs $backend_b)"
  log "backend=$backend_a"

  # Isolation checks
  [[ "$(cat "$MERGED_A/foo")" == "original" ]] || die "A initial read mismatch"
  [[ "$(cat "$MERGED_B/foo")" == "original" ]] || die "B initial read mismatch"

  printf 'A\n' >"$MERGED_A/foo"
  [[ "$(cat "$MERGED_A/foo")" == "A" ]] || die "A write failed"
  [[ "$(cat "$MERGED_B/foo")" == "original" ]] || die "B saw A's write"
  [[ "$(cat "$ROOT/lower/foo")" == "original" ]] || die "lowerdir mutated"

  printf 'B-only\n' >"$MERGED_B/new.txt"
  [[ -f "$MERGED_B/new.txt" ]] || die "B create failed"
  [[ ! -f "$MERGED_A/new.txt" ]] || die "A saw B's create"

  rm -f "$MERGED_A/bar"
  [[ ! -f "$MERGED_A/bar" ]] || die "A delete failed"
  [[ -f "$MERGED_B/bar" ]] || die "B lost bar after A's delete"
  [[ -f "$ROOT/lower/bar" ]] || die "lowerdir lost bar"

  # Git smoke (shared .git via overlay — NOT session-isolated; M2 concern)
  git -C "$MERGED_A" status -sb >/dev/null
  git -C "$MERGED_B" status -sb >/dev/null
  log "git status works in both merged views (shared .git until M2)"

  log "PASS: independent COW sessions over immutable base"
  log "root=$ROOT"
  printf 'ACTIVE_BACKEND=%s\n' "$backend_a"
}

main "$@"
