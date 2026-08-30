#!/usr/bin/env bash
# Warm-state + memory benchmark: N parallel agent sessions.
# Compares git worktrees vs LazyTree with shared semantic caches.
set -euo pipefail

ROOT="${BENCH_ROOT:-/tmp/lazytree-warm-mem}"
LT_BIN="${LT_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/lazytree}"
OUT_MD="${OUT_MD:-$(cd "$(dirname "$0")/.." && pwd)/docs/benchmarks-warm-memory.md}"
N_SESSIONS="${N_SESSIONS:-3}"

need() { command -v "$1" >/dev/null || { echo "missing $1" >&2; exit 1; }; }
need git; need cargo; need python3; need "$LT_BIN"
RA=$(command -v rust-analyzer)

nanos() { date +%s%N; }
ms_between() { echo $((( $2 - $1 ) / 1000000)); }
du_bytes() { du -sb "$1" 2>/dev/null | awk '{print $1}'; }

umount_all() {
  local p
  for p in $(mount | awk '/fuse-overlayfs/{print $3}'); do
    fusermount3 -u "$p" 2>/dev/null || sudo -n fusermount3 -u "$p" 2>/dev/null || true
  done
}

# Peak RSS/PSS while pids run (samples until all exit).
peak_mem_while_running() {
  python3 - "$@" <<'PY'
import os, sys, time
pids = [int(x) for x in sys.argv[1:]]
peak_rss = peak_pss = 0
alive = set(pids)
while alive:
    rss = pss = 0
    gone = []
    for pid in list(alive):
        path = f"/proc/{pid}/smaps_rollup"
        try:
            with open(path) as f:
                for line in f:
                    if line.startswith("Rss:"):
                        rss += int(line.split()[1])
                    elif line.startswith("Pss:"):
                        pss += int(line.split()[1])
        except FileNotFoundError:
            gone.append(pid)
    for pid in gone:
        alive.discard(pid)
    peak_rss = max(peak_rss, rss)
    peak_pss = max(peak_pss, pss)
    if alive:
        time.sleep(0.05)
print(peak_rss, peak_pss)
PY
}

seed_dir() {
  # reflink when possible so seeded targets share extents on disk
  local src="$1" dst="$2"
  rm -rf "$dst"
  mkdir -p "$(dirname "$dst")"
  if cp -a --reflink=always "$src" "$dst" 2>/dev/null; then
    return 0
  fi
  if cp -a --reflink=auto "$src" "$dst" 2>/dev/null; then
    return 0
  fi
  cp -a "$src" "$dst"
}

make_rust_repo() {
  local dest="$1"
  rm -rf "$dest"
  mkdir -p "$dest"
  cargo new --bin "$dest/app" --name warmapp >/dev/null 2>&1
  cat >>"$dest/app/Cargo.toml" <<'TOM'
serde = { version = "1", features = ["derive"] }
serde_json = "1"
TOM
  cat >"$dest/app/src/main.rs" <<'RS'
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Debug)]
struct Msg { n: u32, s: String }
fn main() {
    let m = Msg { n: 1, s: "warm".into() };
    println!("{}", serde_json::to_string(&m).unwrap());
}
RS
  git -C "$dest/app" init -q
  git -C "$dest/app" config user.email bench@lazytree.dev
  git -C "$dest/app" config user.name bench
  git -C "$dest/app" add -A
  git -C "$dest/app" commit -qm base
  git -C "$dest/app" branch -M main
}

avg() {
  python3 -c 'import sys; xs=list(map(int,sys.argv[1:])); print(sum(xs)//len(xs))' "$@"
}
p50() {
  python3 -c 'import sys; xs=sorted(map(int,sys.argv[1:])); print(xs[len(xs)//2])' "$@"
}

measure_scenario_worktree() {
  local base="$ROOT/worktree-scenario"
  rm -rf "$base"
  mkdir -p "$base"
  make_rust_repo "$base/repo"
  local app="$base/repo/app"

  local t0 t1 warm_ms
  t0=$(nanos)
  (cd "$app" && cargo check -q)
  t1=$(nanos)
  warm_ms=$(ms_between "$t0" "$t1")
  local cache_after_warm; cache_after_warm=$(du_bytes "$app/target")

  local create_ms=() i path
  for i in $(seq 1 "$N_SESSIONS"); do
    path="$base/wt-$i"
    t0=$(nanos)
    git -C "$app" worktree add -b "agent-$i" "$path" main >/dev/null
    t1=$(nanos)
    create_ms+=("$(ms_between "$t0" "$t1")")
  done

  local check_ms=()
  for i in $(seq 1 "$N_SESSIONS"); do
    t0=$(nanos)
    (cd "$base/wt-$i" && cargo check -q)
    t1=$(nanos)
    check_ms+=("$(ms_between "$t0" "$t1")")
  done

  local disk_targets=0
  for i in $(seq 1 "$N_SESSIONS"); do
    disk_targets=$((disk_targets + $(du_bytes "$base/wt-$i/target")))
  done

  local pids=()
  for i in $(seq 1 "$N_SESSIONS"); do
    (cd "$base/wt-$i" && "$RA" analysis-stats . >"$base/ra-$i.log" 2>&1) &
    pids+=($!)
  done
  local mem; mem=$(peak_mem_while_running "${pids[@]}")
  wait || true
  local rss pss
  rss=$(echo "$mem" | awk '{print $1}')
  pss=$(echo "$mem" | awk '{print $2}')

  cat <<ROW
### git worktree ×${N_SESSIONS}

| Metric | Value |
| --- | ---: |
| Initial warm \`cargo check\` | ${warm_ms} ms |
| \`target/\` after first warm | ${cache_after_warm} B |
| Spawn P50 | $(p50 "${create_ms[@]}") ms |
| Per-session \`cargo check\` P50 (cold target) | $(p50 "${check_ms[@]}") ms |
| Sum of N \`target/\` dirs | ${disk_targets} B |
| Peak rust-analyzer Σ RSS | ${rss} KB |
| Peak rust-analyzer Σ PSS | ${pss} KB |
| PSS/RSS | $(python3 -c "print(round($pss/$rss,2) if $rss else 0)") |

ROW
}

measure_scenario_lazytree() {
  local base="$ROOT/lazytree-scenario"
  rm -rf "$base"
  mkdir -p "$base"
  make_rust_repo "$base/repo"
  local app="$base/repo/app"

  export LAZYTREE_HOME="$base/lt-home"
  "$LT_BIN" repo add "$app" >/dev/null
  local repo_id
  repo_id=$("$LT_BIN" repo list --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[0]["id"])')
  local shared="$LAZYTREE_HOME/repositories/$repo_id/semantic/shared"
  mkdir -p "$shared/cargo-home" "$shared/target-warm"

  # Session paths + semantic writable OUTSIDE overlay (critical for memory/disk)
  local p1 sess1_id
  p1=$("$LT_BIN" create --json agent-1 | python3 -c 'import sys,json; print(json.load(sys.stdin)["path"])')
  sess1_id=$("$LT_BIN" list --json | python3 -c 'import sys,json; print([s for s in json.load(sys.stdin) if s["name"]=="agent-1"][0]["id"])')
  local sem1="$LAZYTREE_HOME/sessions/$sess1_id/semantic/writable"
  mkdir -p "$sem1"

  local t0 t1 warm_ms
  t0=$(nanos)
  (
    cd "$p1"
    export CARGO_HOME="$shared/cargo-home"
    export CARGO_TARGET_DIR="$sem1/target"
    cargo check -q
  )
  t1=$(nanos)
  warm_ms=$(ms_between "$t0" "$t1")

  # Promote warm target into shared (read-mostly seed)
  seed_dir "$sem1/target" "$shared/target-warm/target"
  local shared_cargo_bytes shared_target_bytes
  shared_cargo_bytes=$(du_bytes "$shared/cargo-home")
  shared_target_bytes=$(du_bytes "$shared/target-warm")

  local paths=("$p1") sem_dirs=("$sem1") create_ms=() i path json sid sem
  for i in $(seq 2 "$N_SESSIONS"); do
    t0=$(nanos)
    json=$("$LT_BIN" create --json "agent-$i")
    t1=$(nanos)
    create_ms+=("$(ms_between "$t0" "$t1")")
    path=$(echo "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["path"])')
    sid=$(echo "$json" | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
    sem="$LAZYTREE_HOME/sessions/$sid/semantic/writable"
    mkdir -p "$sem"
    paths+=("$path")
    sem_dirs+=("$sem")
  done

  t0=$(nanos)
  path=$("$LT_BIN" create agent-spawnprobe)
  t1=$(nanos)
  local spawn_ms; spawn_ms=$(ms_between "$t0" "$t1")
  "$LT_BIN" destroy agent-spawnprobe --force >/dev/null

  local check_ms=() sess_target_sum=0
  for i in $(seq 0 $((N_SESSIONS - 1))); do
    path="${paths[$i]}"
    sem="${sem_dirs[$i]}"
    # Seed per-session writable target from shared via reflink when possible
    if [[ "$i" -gt 0 || ! -d "$sem/target" ]]; then
      seed_dir "$shared/target-warm/target" "$sem/target"
    fi
    t0=$(nanos)
    (
      cd "$path"
      export CARGO_HOME="$shared/cargo-home"
      export CARGO_TARGET_DIR="$sem/target"
      cargo check -q
    )
    t1=$(nanos)
    check_ms+=("$(ms_between "$t0" "$t1")")
    sess_target_sum=$((sess_target_sum + $(du_bytes "$sem/target")))
  done

  local pids=()
  for i in $(seq 0 $((N_SESSIONS - 1))); do
    path="${paths[$i]}"
    sem="${sem_dirs[$i]}"
    (
      cd "$path"
      export CARGO_HOME="$shared/cargo-home"
      export CARGO_TARGET_DIR="$sem/target"
      "$RA" analysis-stats . >"$base/ra-lt-$((i+1)).log" 2>&1
    ) &
    pids+=($!)
  done
  local mem rss pss
  mem=$(peak_mem_while_running "${pids[@]}")
  wait || true
  rss=$(echo "$mem" | awk '{print $1}')
  pss=$(echo "$mem" | awk '{print $2}')

  local lt_local=0 s
  for s in "$LAZYTREE_HOME"/sessions/session_*; do
    [[ -d "$s" ]] || continue
    lt_local=$((lt_local + $(du_bytes "$s/fs/upper") + $(du_bytes "$s/git") + $(du_bytes "$s/metadata.json")))
  done
  local sem_writable_sum=0
  for s in "$LAZYTREE_HOME"/sessions/session_*/semantic/writable; do
    [[ -d "$s" ]] || continue
    sem_writable_sum=$((sem_writable_sum + $(du_bytes "$s")))
  done

  cat <<ROW
### LazyTree ×${N_SESSIONS} (shared Cargo home + reflink-seeded targets outside overlay)

| Metric | Value |
| --- | ---: |
| Initial warm \`cargo check\` (session 1) | ${warm_ms} ms |
| Shared \`CARGO_HOME\` | ${shared_cargo_bytes} B |
| Shared warm target seed | ${shared_target_bytes} B |
| Fresh session spawn (fs+git) | ${spawn_ms} ms |
| Extra session spawn P50 | $(p50 "${create_ms[@]}") ms |
| Per-session \`cargo check\` P50 (seeded) | $(p50 "${check_ms[@]}") ms |
| Sum per-session target dirs (apparent \`du\`) | ${sess_target_sum} B |
| Shared + session semantic writable | $((shared_cargo_bytes + shared_target_bytes + sem_writable_sum)) B |
| LazyTree upper+git+meta only | ${lt_local} B |
| Peak rust-analyzer Σ RSS | ${rss} KB |
| Peak rust-analyzer Σ PSS | ${pss} KB |
| PSS/RSS | $(python3 -c "print(round($pss/$rss,2) if $rss else 0)") |

ROW

  for i in $(seq 1 "$N_SESSIONS"); do
    "$LT_BIN" destroy "agent-$i" --force >/dev/null 2>&1 || true
  done
}

umount_all
rm -rf "$ROOT"
mkdir -p "$ROOT"

{
  cat <<HDR
# Warm-state & memory benchmark

Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)  
Sessions: ${N_SESSIONS} parallel agent workspaces  
Host mem: $(free -h | awk '/Mem:/{print $2" total, "$7" available"}')

## Why these metrics

1. **Spawn time** after a registered/warm base (filesystem + git fork)
2. **Derived cache reuse** (\`cargo check\` after shared seed)
3. **Memory with N language servers**
   - **RSS** overcounts shared file-backed pages
   - **PSS** is the fair share (what you want for "how much extra RAM")

OverlayFS helps **disk + page cache** for shared files. It does **not** merge
N \`rust-analyzer\` heaps. Anonymous process memory still scales ~with N.

## Results

HDR
  echo "[bench] worktree scenario" >&2
  measure_scenario_worktree
  echo "[bench] lazytree scenario" >&2
  measure_scenario_lazytree
  cat <<'FOOT'
## How to read this

| Observation | Meaning |
| --- | --- |
| Spawn ~25ms on LazyTree | Carrying filesystem+git forward is cheap |
| Seeded check ≪ cold check | Shared semantic cache works |
| PSS ≈ RSS for rust-analyzer | Heaps dominate; little cross-process RAM sharing |
| PSS ≪ RSS | File-backed sharing is helping (registry/index pages) |
| Large upper+git when target inside overlay | **Anti-pattern** — keep derived state in `semantic/` |

## Memory optimization playbook

1. **Put caches outside the COW upper** (`semantic/shared` + `semantic/writable`). Never let `target/` or LSP indexes land in the overlay upper by default.
2. **Share read-only bytes once** (`CARGO_HOME`, immutable index snapshots). That shrinks disk and page cache; PSS improves only for the file-backed portion.
3. **Reflink/hardlink seed** session writable caches from shared warm snapshots (avoid full `cp -a` physical duplication when FS supports it).
4. **Accept ~N× anonymous RSS for N LSPs** unless you change the process model (one shared server — usually a bad isolation fit for agents).
5. **Measure PSS not RSS** when judging "how crazy is memory."

FOOT
} | tee "$OUT_MD"

umount_all
echo "[bench] wrote $OUT_MD" >&2
