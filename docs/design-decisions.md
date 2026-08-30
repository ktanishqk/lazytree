# Design Decisions

Living document for PRD §37 questions. Updated as evidence lands.

## Milestone 0 answers (partial)

| # | Question | Current answer |
| --- | --- | --- |
| 1 | Cost of independent Git state vs OverlayFS mount | **Unknown (M2).** M0 measured filesystem fork only. |
| 2 | Cheap clone/reflink of Git index | **Not yet investigated.** |
| 3 | OverlayFS copy-up of unexpectedly large data | **Not yet profiled.** fuse-overlayfs used in M0. |
| 4 | File watchers | **Untested.** |
| 5 | mmap for compilers/tools | **Untested.** |
| 6 | Inode differences vs build systems | **Untested.** |
| 7 | Deletion/whiteout vs Git | **Partial:** whiteout delete isolated across sessions; Git interaction deferred to M2. |
| 8 | Builds inside merged view | **Untested.** |
| 9 | Is `git status` dominant startup cost? | **Unknown until M2 timings.** |
| 10 | Elevated privileges | **Yes in this Cloud VM:** unprivileged FUSE denied; kernel overlay failed; `sudo fuse-overlayfs` works. |
| 11 | Docker / Cursor cloud breakage | **Observed:** nested overlay + FUSE policy forces privileged fuse-overlayfs. Kernel OverlayFS not usable here. |
| 12 | macOS changes | **Deferred.** Likely APFS clones or FUSE; not in MVP. |
| 13 | When LazyTree beats worktree | **See `docs/feasibility-m0.md` + benchmark output.** Filesystem fork already much cheaper than worktree on synthetic repos; full product comparison needs Git state. |
| 14 | Disk usage lower in practice? | **Promising:** untouched session upperdirs stay tiny vs full worktree trees; quantify in M4. |
| 15 | Does editor/LSP indexing dominate? | **Unknown.** Out of M0 scope. |

## Decisions locked in M0

- **D1.** Keep a `FilesystemBackend` interface; implement `FuseOverlayFs` first for Cloud Agent viability, `KernelOverlayFs` when the host allows.
- **D2.** Prefer least privilege, but allow an explicit sudo mount helper rather than silently escalating arbitrary commands.
- **D3.** Do not claim O(1) workspace creation until Git init is measured separately (PRD §30).

## Decisions locked in M1

- **D4.** Do not `chmod a-w` the immutable base when using fuse-overlayfs: `default_permissions` then denies writes before copy-up. Immutability is by LazyTree never writing the base.
- **D5.** Privileged fuse-overlayfs mounts pass `uid`/`gid` of the invoking user plus `allow_other` so agents can write the merged view.
- **D6.** M1 session `git.state` is `placeholder`; shared lower `.git` is visible but not yet session-private (M2).
EOF