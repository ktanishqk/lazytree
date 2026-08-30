# Design Decisions

Living document for PRD §37 questions. Updated as evidence lands.

## Answers (current)

| # | Question | Current answer |
| --- | --- | --- |
| 1 | Cost of independent Git state vs OverlayFS mount | **Git usually dominates.** M4: FS fork ~12ms; Git init often 50–800ms+. |
| 2 | Cheap clone/reflink of Git index | **Not implemented.** Highest-leverage next experiment — index is ~170KB per 2k-file session. |
| 3 | OverlayFS copy-up of unexpectedly large data | **Partial:** 64KB file edit → ~65KB upper (expected). Broader profiling TBD. |
| 4 | File watchers | **Untested.** |
| 5 | mmap for compilers/tools | **Untested.** |
| 6 | Inode differences vs build systems | **Untested.** |
| 7 | Deletion/whiteout vs Git | **Works** for isolation; session `.git` is a gitdir file over whiteout. |
| 8 | Builds inside merged view | **Untested.** |
| 9 | Is `git status` dominant startup cost? | **Often after create**, especially on fuse-overlayfs (M4: up to ~649ms vs ~103ms worktree). |
| 10 | Elevated privileges | **Yes in this Cloud VM:** sudo fuse-overlayfs required. |
| 11 | Docker / Cursor cloud breakage | **Observed:** nested overlay + FUSE policy forces privileged fuse-overlayfs. |
| 12 | macOS changes | **Deferred.** |
| 13 | When LazyTree beats worktree | **Create:** sometimes on large checkouts; **not** on small/medium in M4. |
| 14 | Disk usage lower in practice? | **File content: yes. Total session-local: not always** (Git index can dominate). |
| 15 | Does editor/LSP indexing dominate? | **Unknown.** Still likely the real UX bottleneck for agents. |

## Decisions locked

- **D1–D3 (M0):** Backend interface; explicit sudo helper; never claim O(1) without split timings.
- **D4–D6 (M1):** No chmod a-w on fuse bases; uid/gid on privileged mounts; Git placeholder until M2.
- **D7–D9 (M2):** Alternates + private gitdir; whiteout lower `.git`; branch `lazytree/<name>`.
- **D10 (M4):** Prioritize **index inheritance / reflink** before semantic-cache work — M5 in the PRD should wait until create/status are competitive.
- **D11 (M4):** Treat FUSE `git status` overhead as a first-class risk; prefer kernel OverlayFS where available.

## Index inheritance (post-M4)

- **D12.** Do not put `.git` inside the OverlayFS lowerdir. Registration stores worktree files in `base/` and objects in `git-objects/` separately. Session create only writes a small `gitdir:` file — no whiteout of thousands of git metadata paths (~840ms saved on 5k-file repos).
- **D13.** Seed sessions by **copying** (`cp --reflink=auto`) a registered `seed/index` when `--from` matches `base_commit`; fall back to `read-tree` otherwise. We *can* just copy the index; the earlier bottleneck was whiteout + misattributed timing, not index semantics.
