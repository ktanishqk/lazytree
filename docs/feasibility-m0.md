# Milestone 0 — Feasibility Notes

Date: 2026-08-30  
Environment: Cursor Cloud Agent Linux VM (nested container; root filesystem already OverlayFS)

## Question

Can we create multiple independent, ordinary-looking directory views over one immutable repository base using OverlayFS-style copy-on-write, without re-materializing the tree per session?

## Result

**Yes, with environmental caveats.** Dual sessions over one immutable lowerdir show correct isolation:

| Check | Result |
| --- | --- |
| Both sessions read same initial `foo` | Pass |
| Write in A does not appear in B | Pass |
| Lowerdir remains `original` | Pass |
| Create in B invisible to A | Pass |
| Delete in A does not remove file from B/lower | Pass |
| `git status` works inside merged views | Pass (shared `.git`; not yet session-isolated) |

## Backend discovery (this VM)

Tried in PRD order (§33):

1. **Unprivileged user-namespace + kernel OverlayFS** — failed (`wrong fs type` / nested-overlay constraints).
2. **Unprivileged fuse-overlayfs** — failed (`Operation not permitted` on FUSE mount).
3. **Privileged helper** — **works** via `sudo fuse-overlayfs ... allow_other`.
4. **Privileged kernel OverlayFS** — also failed on this host (`wrong fs type`), even with `sudo`; likely nested-overlay / container policy.

MVP implication: first working path here is **sudo + fuse-overlayfs**, not raw kernel OverlayFS. Document clearly; do not pretend mounts are unprivileged.

## Blockers / risks

1. **Privilege**: mounts currently require passwordless `sudo`. Unprivileged FUSE is blocked in this environment.
2. **Nested container**: kernel OverlayFS unavailable or broken on nested overlay root; FUSE fallback is required for Cloud Agent testing.
3. **Git isolation not proven**: M0 mounts share the lowerdir `.git`. Independent HEAD/index/refs is Milestone 2 work. M0 only proves filesystem COW + that Git commands *run*.
4. **fuse-overlayfs quirks**: logs `unknown argument ignored: lazytime`; behavior under heavy mmap / file-watcher workloads is untested (design-decision questions 4–6).
5. **No attached GitHub remote** on this agent run: code can be built and committed locally, but push/PR needs a repo.

## What this does *not* yet answer

- Whether Git index init dominates create latency (measure in M2/M4).
- Whether editor/LSP reindexing erases the win (M4+/M7).
- macOS path (deferred).
- Security isolation (explicitly non-goal; OverlayFS ≠ sandbox).

## Scripts

- `scripts/m0_overlay_spike.sh` — isolation proof
- `scripts/m0_benchmark.sh` — create latency vs `git worktree add`

## Benchmark (synthetic, 2000 files, this VM)

| Metric | Value |
| --- | --- |
| Backend | fuse-overlayfs (sudo) |
| Base copy (registration) | 98 ms |
| `git worktree add` create | avg **75 ms** (5 samples) |
| COW session mount only | avg **14 ms** (5 samples) |
| Disk for 5 worktrees | ~219 KB |
| Disk for 5 untouched COW uppers | **0 B** |
| Upper after 1-file edit | 8 B |

Notes:

- Worktree disk looks small here because Git hardlinks objects; LazyTree’s win is avoiding another working-tree materialization and (later) index/ co duplication costs on huge trees.
- These COW timings exclude private Git state setup (M2). Do not claim full workspace create = 14 ms yet (PRD §30).
- Re-run with larger `FILE_COUNT` / real repos in M4.

## Go / no-go for Milestone 1

**Go**, with backend interface abstracting kernel OverlayFS vs fuse-overlayfs and an explicit privileged mount helper.

Stop condition from PRD §36 was “OverlayFS fundamentally cannot provide the required semantics.” Semantics work; privilege/environment is the hard part, not COW correctness.
EOF