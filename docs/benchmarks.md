# LazyTree Benchmarks (Milestone 4)

Generated: 2026-08-30T20:42:35Z  
Host: Linux 6.12.94+ x86_64  
Backend: fuse-overlayfs (sudo) in Cursor Cloud Agent VM  
Binary: `/agent/lazytree/target/release/lazytree`  
Samples per metric: 5

## Honesty notes

- LazyTree **create** includes OverlayFS mount **and** private Git metadata init (seed index byte-copy; `read-tree` only when `--from` ≠ seed).
- Registration (`repo add`) is intentionally O(repository size) (~235ms for 5k files via `git clone --bare` object snapshot — hardlinks when source and `$LAZYTREE_HOME` share a device; `--no-hardlinks` cross-device or with `LAZYTREE_OBJECTS_COPY=1` — + rsync worktree exclude `.git`) and is **not** counted as session create.
- Worktree disk often looks small because Git hardlinks objects; compare working-tree materialization and session-local metadata, not object DB size alone.
- These numbers are from a nested cloud VM. Absolute milliseconds will differ on bare metal; ratios matter more.

## Results

### tiny_200_files

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 240 ms | (same repo) |
| Repo bytes | 62978 | 62978 |
| Registration (one-time) | n/a | 52 ms |
| Registered base bytes | n/a | 36328 |
| Create P50 (n=5) | 17 ms | 8 ms |
| Create avg | 17 ms | 10 ms |
| Create samples | 17 17 17 17 17 | 18 9 8 8 8 |
| Filesystem fork P50 | n/a | 4 ms |
| Git state init P50 | n/a | 1 ms |
| First `git status` P50 | 3 ms | 19 ms |
| Destroy P50 | 7 ms | 10 ms |
| Disk for 5 sessions | 8855 B | 100065 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 96 → 113 B |

### medium_5000_files

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 5674 ms | (same repo) |
| Repo bytes | 770227 | 770227 |
| Registration (one-time) | n/a | 391 ms |
| Registered base bytes | n/a | 743589 |
| Create P50 (n=5) | 173 ms | 9 ms |
| Create avg | 174 ms | 11 ms |
| Create samples | 179 173 173 173 173 | 19 12 9 9 8 |
| Filesystem fork P50 | n/a | 4 ms |
| Git state init P50 | n/a | 1 ms |
| First `git status` P50 | 27 ms | 169 ms |
| Destroy P50 | 59 ms | 15 ms |
| Disk for 5 sessions | 244875 B | 2180675 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 99 → 116 B |

### large_20000_files

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 23975 ms | (same repo) |
| Repo bytes | 3012104 | 3012104 |
| Registration (one-time) | n/a | 466 ms |
| Registered base bytes | n/a | 3486693 |
| Create P50 (n=5) | 437 ms | 11 ms |
| Create avg | 449 ms | 13 ms |
| Create samples | 603 350 396 437 462 | 19 16 9 10 11 |
| Filesystem fork P50 | n/a | 4 ms |
| Git state init P50 | n/a | 1 ms |
| First `git status` P50 | 97 ms | 649 ms |
| Destroy P50 | 257 ms | 38 ms |
| Disk for 5 sessions | 1044880 B | 8781185 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 99 → 117 B |

### fat_500x64KB

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 1111 ms | (same repo) |
| Repo bytes | 32847920 | 32847920 |
| Registration (one-time) | n/a | 50 ms |
| Registered base bytes | n/a | 32821262 |
| Create P50 (n=5) | 48 ms | 9 ms |
| Create avg | 48 ms | 11 ms |
| Create samples | 48 48 48 48 48 | 20 9 10 9 8 |
| Filesystem fork P50 | n/a | 4 ms |
| Git state init P50 | n/a | 1 ms |
| First `git status` P50 | 72 ms | 100 ms |
| Destroy P50 | 12 ms | 13 ms |
| Disk for 5 sessions | 163840385 B | 220025 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 94 → 65638 B |


## Interpretation

- **Create wins** across all sizes here (tiny ~2×, medium ~19×, large ~40× vs worktree).
- Filesystem fork + seed-index copy dominate create (~4ms + ~1ms); first sample is colder.
- **First `git status` loses** on fuse-overlayfs (large ~649ms vs ~97ms worktree) — FUSE stat tax, not create.
- **Fat files**: LazyTree session-local disk stays tiny (~220KB for 5 sessions) vs ~164MB materialized worktrees.
- Absolute ms are cloud-VM fuse numbers; ratios are the takeaway.

Refresh notes vs prior M4 write-up: create P50 dropped from ~23–28ms → ~8–11ms after seed-index + mount-backend remember + zero-spawn happy path.

Parallel create (this VM): 10 sessions in ~42ms wall clock (unlocked mount+git path).
Registration (5k files, bare-clone objects): ~235ms one-time.

