# Design Decisions

Living document for PRD §37 questions. Updated as evidence lands.

## Answers (current)

| # | Question | Current answer |
| --- | --- | --- |
| 1 | Cost of independent Git state vs OverlayFS mount | **FS + seed-index dominate create** (~4ms + ~1ms). Git process spawn path avoided on happy path. |
| 2 | Cheap clone/reflink of Git index | **Done (D13):** copy `seed/index`; reflink when FS supports it (nested cloud often does not). |
| 3 | OverlayFS copy-up of unexpectedly large data | **Partial:** 64KB file edit → ~65KB upper (expected). Broader profiling TBD. |
| 4 | File watchers | **Works on fuse-overlayfs session root:** `inotifywait` saw `MODIFY` on overwrite and `CREATE` for a new file (Cloud VM probe, 2026-08-30). |
| 5 | mmap for compilers/tools | **Works on fuse-overlayfs** (8MB random blob mmap scan succeeds). Path-sensitive caches still need canonical exec. |
| 6 | Inode differences vs build systems | **Mitigated for Cargo** via bind-mount to stable canonical paths in `lazytree exec`. |
| 7 | Deletion/whiteout vs Git | **Works;** `.git` is never in lowerdir — session only writes a `gitdir:` file. |
| 8 | Builds inside merged view | **Works** with `lazytree exec`; keep `target/` out of upper when possible (canonical target). |
| 9 | Is `git status` dominant startup cost? | **Cold status on fuse-overlayfs yes** (~195ms / 5k); **warm ~7–20ms**. Create now spawns a background warm. |
| 10 | Elevated privileges | **Yes in this Cloud VM:** sudo fuse-overlayfs required. |
| 11 | Docker / Cursor cloud breakage | **Observed:** nested overlay + FUSE policy forces privileged fuse-overlayfs. |
| 12 | macOS changes | **unionfs-fuse** (macFUSE/Fuse-T). Same lower/upper mount model; Auto picks it on Darwin. No clone-tree default (create must stay ~O(1)). |
| 13 | When LazyTree beats worktree | **Create: always in refreshed M4** (tiny ~2×, medium ~19×, large ~40×). Fat disk: ~220KB vs ~164MB. |
| 14 | Disk usage lower in practice? | **Fat content: yes.** Metadata-only sessions can look larger than hardlinked worktrees on tiny repos. |
| 15 | Does editor/LSP indexing dominate? | **PSS matters, not RSS.** OverlayFS does not merge N LSP heaps — still the real multi-agent tax. |
| 16 | Cursor UI branch sync | **Hooks cannot rebind workspace cwd.** Soft path: skill + gates; optional `lazytree cursor open`. Cloud: no `sessionStart` → `cursor bootstrap`. |

## Decisions locked

- **D1–D3 (M0):** Backend interface; explicit sudo helper; never claim O(1) without split timings.
- **D4–D6 (M1):** No chmod a-w on fuse bases; uid/gid on privileged mounts; Git placeholder until M2.
- **D7–D9 (M2):** Alternates + private gitdir; branch `lazytree/<name>`.
- **D10 (M4):** Index inheritance prioritized — **shipped**.
- **D11 (M4):** Treat FUSE `git status` overhead as first-class; prefer kernel OverlayFS where available; background-warm after create.
- **D12.** Do not put `.git` inside the OverlayFS lowerdir.
- **D13.** Seed sessions by copying `seed/index` when `--from` matches `base_commit`; else `read-tree`.
- **D14.** Registration must **exclude** `.git` from the worktree copy (never `cp` then delete) — races with auto-gc on large commits. Object snapshot: bare clone hardlinks same-device; `--no-hardlinks` cross-device or `LAZYTREE_OBJECTS_COPY=1`.
- **D15.** Cursor soft integration: hook + skill + gates; `sessionStart` is advisory; gates fail-open without mapping.
- **D16.** Path-sensitive build caches: `lazytree exec` remaps to `$LAZYTREE_HOME/canonical/{workspace,target}` via user mount namespace.
- **D17.** Cross-OS COW: preserve overlay semantics. Linux = OverlayFS / fuse-overlayfs; macOS = unionfs-fuse. Reject clone-tree as the portable default (O(n) create). Windows deferred.
- **D18.** Filesystem backends are **plugins** (`OverlayBackend` trait): orchestrator in `filesystem/mod.rs`, registry per OS, concrete plugins under `filesystem/plugins/`. Session/CLI stay backend-agnostic.

## Open follow-ups

- Kernel OverlayFS path when unprivileged mounts work (drop sudo fuse).
- Optional sync create `--warm-status` for CI that needs immediate status numbers.
- npm binary download in postinstall (currently PATH/LAZYTREE_BIN only).
- macOS: measure create/status vs Linux fuse; optional APFS `clonefile` only for unionfs copy-up.
- Windows: not in scope (WSL2 or experimental WinFsp later).
