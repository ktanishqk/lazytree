# LazyTree full benchmark suite

Generated: 2026-08-30T22:14:14Z  
Host: Linux 6.12.94+ x86_64  
Binary: `/agent/lazytree/target/release/lazytree`  
Samples per metric: 5

## Honesty

- **Linux host: unionfs-fuse is the macOS Auto plugin code path (proxy for Darwin).**
- Contenders: **git worktree** · **LazyTree fuse-overlayfs** (Linux default) · **LazyTree unionfs-fuse** (macOS plugin).
- LazyTree **create** = COW mount + private Git init (seed index). Registration (`repo add`) is one-time and listed separately.
- Worktree disk often looks small because Git hardlinks objects; LazyTree session disk is upper+git+meta only (shared base excluded).
- Absolute ms are cloud-VM numbers; **ratios vs worktree** are the product question.

## Create P50 (the headline)

| Case | worktree | fuse-overlayfs | unionfs (macOS path) | fuse vs wt | unionfs vs wt |
| --- | ---: | ---: | ---: | ---: | ---: |
| tiny_200_files | 18 ms | 6 ms | 7 ms | 3.0x faster | 2.6x faster |
| medium_5000_files | 187 ms | 7 ms | 8 ms | 27x faster | 23x faster |
| large_20000_files | 407 ms | 7 ms | 9 ms | 58x faster | 45x faster |
| fat_500x64KB | 50 ms | 7 ms | 7 ms | 7.1x faster | 7.1x faster |

## First `git status` P50

| Case | worktree | fuse-overlayfs | unionfs (macOS path) | fuse / wt | unionfs / wt |
| --- | ---: | ---: | ---: | ---: | ---: |
| tiny_200_files | 5 ms | 19 ms | 24 ms | 3.8x | 4.8x |
| medium_5000_files | 29 ms | 166 ms | 251 ms | 5.7x | 8.7x |
| large_20000_files | 101 ms | 617 ms | 974 ms | 6.1x | 9.6x |
| fat_500x64KB | 73 ms | 100 ms | 112 ms | 1.4x | 1.5x |

## Filesystem fork vs Git init (LazyTree split)

| Case | fuse FS | fuse Git | unionfs FS | unionfs Git |
| --- | ---: | ---: | ---: | ---: |
| tiny_200_files | 1 ms | 0 ms | 2 ms | 0 ms |
| medium_5000_files | 2 ms | 0 ms | 2 ms | 0 ms |
| large_20000_files | 1 ms | 0 ms | 2 ms | 1 ms |
| fat_500x64KB | 1 ms | 0 ms | 2 ms | 0 ms |

## Destroy P50

| Case | worktree | fuse-overlayfs | unionfs |
| --- | ---: | ---: | ---: |
| tiny_200_files | 9 ms | 4 ms | 4 ms |
| medium_5000_files | 61 ms | 9 ms | 11 ms |
| large_20000_files | 258 ms | 31 ms | 31 ms |
| fat_500x64KB | 14 ms | 6 ms | 6 ms |

## Disk for N sessions + registration

| Case | Repo bytes | wt disk (N sessions) | fuse sess disk | unionfs sess disk | fuse repo add | unionfs repo add |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| tiny_200_files | 62979 | 8825 B | 100190 B | 100160 B | 37 ms | 35 ms |
| medium_5000_files | 770227 | 244845 B | 2180800 B | 2180770 B | 217 ms | 209 ms |
| large_20000_files | 3012105 | 1044850 B | 8781310 B | 8781280 B | 388 ms | 353 ms |
| fat_500x64KB | 32847920 | 163840355 B | 220150 B | 220120 B | 106 ms | 49 ms |

## Upper after 1 edit

| Case | Growth |
| --- | --- |
| tiny_200_files | fuse 90->107; unionfs 89->105 |
| medium_5000_files | fuse 93->110; unionfs 92->108 |
| large_20000_files | fuse 93->111; unionfs 92->108 |
| fat_500x64KB | fuse 88->65632; unionfs 87->65631 |

## Verdict

### Are we getting the output we wanted?

1. **Create:** Yes — both LazyTree backends stay ~flat (~single-digit ms) while worktree scales with tree size. On medium/large, LazyTree create is typically **~10–40x** faster than worktree; this suite re-checks that for **unionfs** (macOS path) too.
2. **Overlay model intact on unionfs:** FS fork stays ~1–2 ms across tiny → large → fat. Not clone-tree O(n).
3. **Status tax:** First `git status` on FUSE (especially unionfs) is slower than worktree — expected. Warm-status / caching remains important for agent UX; create speed is still the parallel-agent win.
4. **macOS implication:** If unionfs create tracks fuse create on Linux, shipping unionfs on Darwin preserves the product thesis. Re-run this script on a Mac for absolute Darwin ms.

Re-run: `./scripts/bench_full_suite.sh`

