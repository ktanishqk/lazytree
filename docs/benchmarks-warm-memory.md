# Warm-state & memory benchmark

Generated: 2026-08-30T19:28:23Z  
Sessions: 3 parallel agent workspaces  
Host mem: 15Gi total, 14Gi available

## Why these metrics

1. **Spawn time** after a registered/warm base (filesystem + git fork)
2. **Derived cache reuse** (`cargo check` after shared seed)
3. **Memory with N language servers**
   - **RSS** overcounts shared file-backed pages
   - **PSS** is the fair share (what you want for "how much extra RAM")

OverlayFS helps **disk + page cache** for shared files. It does **not** merge
N `rust-analyzer` heaps. Anonymous process memory still scales ~with N.

## Results

### git worktree ×3

| Metric | Value |
| --- | ---: |
| Initial warm `cargo check` | 2633 ms |
| `target/` after first warm | 54553140 B |
| Spawn P50 | 7 ms |
| Per-session `cargo check` P50 (cold target) | 2644 ms |
| Sum of N `target/` dirs | 163658724 B |
| Peak rust-analyzer Σ RSS | 6528 KB |
| Peak rust-analyzer Σ PSS | 972 KB |
| PSS/RSS | 0.15 |

### LazyTree ×3 (shared Cargo home + reflink-seeded targets outside overlay)

| Metric | Value |
| --- | ---: |
| Initial warm `cargo check` (session 1) | 2787 ms |
| Shared `CARGO_HOME` | 8440499 B |
| Shared warm target seed | 54672197 B |
| Fresh session spawn (fs+git) | 23 ms |
| Extra session spawn P50 | 27 ms |
| Per-session `cargo check` P50 (seeded) | 2645 ms |
| Sum per-session target dirs (apparent `du`) | 273360631 B |
| Shared + session semantic writable | 336473327 B |
| LazyTree upper+git+meta only | 90627 B |
| Peak rust-analyzer Σ RSS | 6864 KB |
| Peak rust-analyzer Σ PSS | 1128 KB |
| PSS/RSS | 0.16 |

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

