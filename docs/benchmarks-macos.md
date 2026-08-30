# Native macOS benchmarks (unionfs-fuse)

Generated: 2026-08-30T23:29:16Z (user MacBook, Darwin 25.6.0 arm64)  
Binary: local `target/release/lazytree`  
Backend: `unionfs_fuse` only (macOS Auto)

## Results (before `fsmonitor.allowRemote` fix)

| Case | Create P50 | FS fork P50 | Git init P50 | First status P50 | Destroy P50 |
| --- | ---: | ---: | ---: | ---: | ---: |
| tiny_200_files | 175 ms | 121 ms | 16 ms | 220 ms | 81 ms |
| medium_5000_files | 178 ms | 117 ms | 18 ms | **7436 ms** | 97 ms |
| fat_500x64KB | 180 ms | 117 ms | 18 ms | 1000 ms | 112 ms |

Warnings observed:

```text
warning: remote repository '.../fs/root' is incompatible with fsmonitor
```

Git classifies macFUSE/Fuse-T mounts as “remote” and **disables** `core.fsmonitor` unless `fsmonitor.allowRemote=true`. Without the hook, `git status` full-walks the FUSE tree → multi-second medium status.

## Reading

- **Create / FS fork stay flat** (~175 / ~117 ms) across tiny → medium → fat → overlay model intact (not clone-tree). Absolute fork is ~50–100× slower than Linux fuse-overlayfs (~1–2 ms) — macFUSE/unionfs mount tax.
- **Status was broken for the product thesis** until allowRemote; re-bench after pull.

## Re-run after fix

```bash
git pull
cargo build --release
./scripts/bench_backends.sh
# confirm no "incompatible with fsmonitor" warnings
git -C "$(lazytree path <session>)" config --get fsmonitor.allowRemote   # true
```
