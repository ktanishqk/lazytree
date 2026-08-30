# LazyTree Benchmarks (Milestone 4)

Generated: 2026-08-30T19:17:32Z  
Host: Linux 6.12.94+ x86_64  
Backend: fuse-overlayfs (sudo) in Cursor Cloud Agent VM  
Binary: `/agent/lazytree/target/release/lazytree`  
Samples per metric: 5

## Honesty notes

- LazyTree **create** includes OverlayFS mount **and** private Git metadata init (index via `read-tree`).
- Registration (`repo add`) is intentionally O(repository size) and is **not** counted as session create.
- Worktree disk often looks small because Git hardlinks objects; compare working-tree materialization and session-local metadata, not object DB size alone.
- These numbers are from a nested cloud VM. Absolute milliseconds will differ on bare metal; ratios matter more.

## Results

### tiny_200_files

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 235 ms | (same repo) |
| Repo bytes | 62979 | 62979 |
| Registration (one-time) | n/a | 67 ms |
| Registered base bytes | n/a | 36297 |
| Create P50 (n=5) | 16 ms | 23 ms |
| Create avg | 16 ms | 22 ms |
| Create samples | 17 16 16 17 16 | 24 22 22 23 23 |
| Filesystem fork P50 | n/a | 12 ms |
| Git state init P50 | n/a | 7 ms |
| First `git status` P50 | 3 ms | 18 ms |
| Destroy P50 | 7 ms | 8 ms |
| Disk for 5 sessions | 8815 B | 230450 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 88 → 105 B |

### medium_5000_files

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 5700 ms | (same repo) |
| Repo bytes | 770227 | 770227 |
| Registration (one-time) | n/a | 387 ms |
| Registered base bytes | n/a | 743557 |
| Create P50 (n=5) | 173 ms | 24 ms |
| Create avg | 175 ms | 24 ms |
| Create samples | 178 172 173 172 180 | 24 26 24 23 23 |
| Filesystem fork P50 | n/a | 12 ms |
| Git state init P50 | n/a | 7 ms |
| First `git status` P50 | 28 ms | 170 ms |
| Destroy P50 | 60 ms | 14 ms |
| Disk for 5 sessions | 244835 B | 2311060 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 91 → 108 B |

### large_20000_files

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 24008 ms | (same repo) |
| Repo bytes | 3012104 | 3012104 |
| Registration (one-time) | n/a | 473 ms |
| Registered base bytes | n/a | 3486661 |
| Create P50 (n=5) | 417 ms | 28 ms |
| Create avg | 444 ms | 32 ms |
| Create samples | 587 357 417 412 448 | 27 32 49 28 26 |
| Filesystem fork P50 | n/a | 13 ms |
| Git state init P50 | n/a | 9 ms |
| First `git status` P50 | 101 ms | 688 ms |
| Destroy P50 | 253 ms | 38 ms |
| Disk for 5 sessions | 1044840 B | 8911570 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 91 → 109 B |

### fat_500x64KB

| Metric | git worktree | lazytree |
| --- | ---: | ---: |
| Repo generate | 1137 ms | (same repo) |
| Repo bytes | 32847919 | 32847919 |
| Registration (one-time) | n/a | 69 ms |
| Registered base bytes | n/a | 32821229 |
| Create P50 (n=5) | 48 ms | 24 ms |
| Create avg | 48 ms | 24 ms |
| Create samples | 48 48 48 48 48 | 26 25 24 23 24 |
| Filesystem fork P50 | n/a | 13 ms |
| Git state init P50 | n/a | 8 ms |
| First `git status` P50 | 73 ms | 102 ms |
| Destroy P50 | 13 ms | 11 ms |
| Disk for 5 sessions | 163840345 B | 350410 B (upper+git+meta only) |
| Upper after 1 edit | n/a | 86 → 65630 B |


## Interpretation

See commit notes / design-decisions for whether filesystem fork or Git index setup dominates.

