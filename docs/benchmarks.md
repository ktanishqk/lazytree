# LazyTree Benchmarks (Milestone 4)

Generated: 2026-08-30T19:05:43Z  
Host: Linux 6.12.94+ x86_64  
Backend: fuse-overlayfs (sudo) in Cursor Cloud Agent VM  
Binary: `/agent/lazytree/target/release/lazytree`  
Samples per metric: 5

## Honesty notes

- LazyTree **create** includes OverlayFS mount **and** private Git metadata init (index via `read-tree`). Timings are reported separately.
- Registration (`repo add`) is intentionally O(repository size) and is **not** counted as session create.
- First disk rows below for create runs overcounted LazyTree because `du` walked the **mounted** merged root. Corrected disk probe is in a separate section.
- Worktree working trees often hardlink; object DB sharing is not a LazyTree-only advantage.
- Nested cloud VM; ratios matter more than absolute milliseconds.

## Create / destroy / status (wall clock)

### tiny_200_files (~63 KB repo)

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Create P50 | **17 ms** | 131 ms |
| Filesystem fork P50 | n/a | 12 ms |
| Git state init P50 | n/a | **115 ms** |
| First `git status` P50 | 3 ms | 19 ms |
| Destroy P50 | 7 ms | 9 ms |

### medium_5000_files (~770 KB repo)

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Create P50 | **180 ms** | 823 ms |
| Filesystem fork P50 | n/a | 12 ms |
| Git state init P50 | n/a | **804 ms** |
| First `git status` P50 | 27 ms | 170 ms |
| Destroy P50 | 61 ms | **15 ms** |

### large_20000_files (~3.0 MB repo)

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Create P50 | 417 ms | **48 ms** |
| Filesystem fork P50 | n/a | 13 ms |
| Git state init P50 | n/a | 31 ms |
| First `git status` P50 | **103 ms** | 649 ms |
| Destroy P50 | 257 ms | **40 ms** |

> Note: the 20k-file Git-init P50 (31 ms) vs 5k-file (804 ms) is unexpected and may reflect page-cache / packing effects in this VM. Do not overfit to that single cell; re-run on bare metal before claiming a crossover point.

### fat_500x64KB (~33 MB repo, large file content)

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Create P50 | **48 ms** | 72 ms |
| Filesystem fork P50 | n/a | 12 ms |
| Git state init P50 | n/a | 56 ms |
| First `git status` P50 | 72 ms | 101 ms |
| Upper after 1×64KB edit | n/a | 86 → **65,630 B** (copy-up of one file) |

## Corrected disk probe (2000 tiny files, 5 sessions)

Measured **session-local only**: `fs/upper` + `git/` + `metadata.json` (excludes mounted merged view).

| | Bytes |
| --- | ---: |
| git worktree ×5 (full `du` of worktree dirs) | ~55 KB |
| LazyTree ×5 (upper+git+meta) | ~983 KB |
| LazyTree upper per untouched session | ~64 B |
| LazyTree `git/index` per session | ~170 KB |

**Reading:** untouched LazyTree uppers are essentially empty (COW works). Per-session cost is dominated by a **full Git index**, which worktrees avoid paying again in the same way. For large *file contents*, LazyTree wins (see 64KB copy-up). For many tiny files, index metadata can erase the storage win.

## Interpretation (Milestone 4 verdict)

1. **Filesystem COW hypothesis: confirmed.** Mount fork is ~12–13 ms and independent of repo byte size in these runs. One-file edits copy up approximately that file.
2. **End-to-end create hypothesis: mixed / often false today.** On 200 and 5k file repos, LazyTree create is **slower** than `git worktree add` because Git index seeding dominates.
3. **`git status` on FUSE overlays is slower**, sometimes much slower (20k-file case). That hurts the “feels like a normal checkout” story for agent loops that spam status.
4. **Privilege story is a real adoption tax** in this environment (sudo fuse-overlayfs).
5. **Promising direction, not yet a win:** the next high-leverage work is cheap index inheritance (reflink/clone/shared index research — design question #2), and kernel OverlayFS on hosts that allow it (lower status overhead than FUSE).

## PRD success criteria check

| Criterion | Status |
| --- | --- |
| Looks like a normal Git repo | Mostly yes (M2) |
| Agent needs only a path | Yes |
| Sessions isolated | Yes (tests) |
| File contents not duplicated on create | Yes (upper ~0) |
| Materially faster than worktree | **Not generally, yet** |
| Cheap safe destroy | Yes |
| No custom agent framework | Yes |

**Bottom line:** LazyTree is a good infrastructure prototype and validates COW *files*. It does **not** yet beat worktrees on the metric users feel first (create + status) except in specific large-checkout cases. That is a valuable negative/partial result, not a reason to stop — but it should drive M5+ priorities toward Git-index cost and mount backend quality, not semantic caches.
