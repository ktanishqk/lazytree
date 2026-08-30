# LazyTree backend benchmarks (macOS path)

Generated: 2026-08-30T22:09:37Z  
Host: Linux 6.12.94+ x86_64  
Binary: `/agent/lazytree/target/release/lazytree`  
Samples per metric: 5

## Honesty

- **Linux unionfs-fuse is the macOS plugin code path (proxy). Not Darwin/macFUSE.**
- This measures the **unionfs-fuse plugin** (macOS Auto backend) against **fuse-overlayfs** (Linux default path) where both exist.
- Real Mac absolute ms will move with macFUSE vs Fuse-T, disk, and SIP; use ratios + filesystem_ms split.
- Create includes mount **and** private Git init (seed index). Registration is excluded.

## Results

| Case | Backend | Mounted as | Create P50 | FS fork P50 | Git init P50 | First status P50 | Destroy P50 |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| tiny_200_files | `fuse_overlayfs` | `fuse_overlayfs` | 7 ms | 1 ms | 0 ms | 19 ms | 4 ms |
| tiny_200_files | `unionfs_fuse` | `unionfs_fuse` | 6 ms | 2 ms | 0 ms | 24 ms | 4 ms |
| medium_5000_files | `fuse_overlayfs` | `fuse_overlayfs` | 7 ms | 1 ms | 0 ms | 162 ms | 9 ms |
| medium_5000_files | `unionfs_fuse` | `unionfs_fuse` | 7 ms | 2 ms | 0 ms | 253 ms | 10 ms |
| fat_500x64KB | `fuse_overlayfs` | `fuse_overlayfs` | 6 ms | 1 ms | 0 ms | 108 ms | 6 ms |
| fat_500x64KB | `unionfs_fuse` | `unionfs_fuse` | 6 ms | 2 ms | 0 ms | 138 ms | 5 ms |

### Samples

| Case | Backend | Create samples | FS fork samples | Status samples |
| --- | --- | --- | --- | --- |
| tiny_200_files | `fuse_overlayfs` | 8 6 7 6 7 | 1 2 1 1 1 | 21 19 18 19 19 |
| tiny_200_files | `unionfs_fuse` | 8 6 6 8 6 | 2 2 2 2 2 | 25 24 23 25 24 |
| medium_5000_files | `fuse_overlayfs` | 7 9 7 6 7 | 1 1 1 1 1 | 182 160 160 163 162 |
| medium_5000_files | `unionfs_fuse` | 9 8 7 6 7 | 2 2 2 2 2 | 252 258 253 255 253 |
| fat_500x64KB | `fuse_overlayfs` | 8 7 6 6 6 | 2 1 1 1 2 | 108 108 110 108 109 |
| fat_500x64KB | `unionfs_fuse` | 7 6 6 6 6 | 2 2 2 2 2 | 141 127 137 138 140 |

## How to re-run

```bash
# Fuse-T or macFUSE + unionfs on PATH (see README)
cargo build --release
./scripts/bench_backends.sh
# Linux → this file · Darwin → docs/benchmarks-macos.md
```

## Interpretation

- **Create stays ~O(1):** FS fork P50 is 1–2 ms on both backends and does not grow with file count (tiny → medium → fat). That is the overlay model we want on macOS — not clone-tree.
- **unionfs vs fuse-overlayfs create:** essentially tied (6–7 ms P50). macOS Auto using unionfs should not regress create vs Linux fuse on this axis.
- **First `git status`:** unionfs is slower (~1.3–1.6× here). Expect a similar FUSE tax on Darwin; warm-status after create still matters.
- These are **Linux proxy** numbers. Native Darwin results live in `docs/benchmarks-macos.md`.

