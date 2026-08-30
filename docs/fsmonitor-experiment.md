# Experiment: Git fsmonitor from LazyTree upperdir

## Problem

Agents freely run `git status`. Instructions / `lazytree status` / PATH wrappers are brittle.
Cold status on FUSE is the remaining create-path disadvantage (large ~600–900 ms vs worktree ~100 ms).

## Insight

`lowerdir` is immutable; every working-tree mutation lives under `upperdir` (plus whiteouts).
That set is the change journal Git’s fsmonitor protocol wants.

## Approach (preferred)

Install a **core.fsmonitor hook (protocol v2)** per session at Git setup:

```
core.fsmonitor = <lazytree fsmonitor-query>
core.fsmonitorHookVersion = 2
```

Hook contract:

1. Args: `2 <last_token>`
2.Stdout: `<new_token>\0` then NUL-separated paths relative to worktree
3. Inclusive: list every upper path that *may* have changed (files, dirs, whiteout targets)
4. Empty upper → token only (no paths) — Git skips full-tree walk **once the index already has an fsmonitor last-update token**
5. Unknown/stale token → may return `/` once (trivial = full scan) then resume

Git keeps real `git status` semantics; agents need no teaching.

## Spike results (2026-08-30, 5k files, fuse-overlayfs VM)

| Mode | `git status` |
| --- | ---: |
| LazyTree **1st** status (no fsmonitor) | ~176 ms |
| LazyTree **1st** status (fsmonitor on, empty upper) | ~160 ms |
| LazyTree **2nd** status (no fsmonitor) | ~13 ms |
| LazyTree **2nd** status (fsmonitor) | ~16 ms |
| LazyTree after 1 edit (fsmonitor) | ~9 ms (correct ` M …`) |
| git worktree 1st | ~26 ms |

### What this means

1. **You cannot avoid agents calling `git status`.** Teach / `lazytree status` / deny-hooks are brittle.
2. **Fsmonitor does not fix the first status by itself** if the index has no FSMO last-update token — Git full-scans once to establish baseline (same cold FUSE tax).
3. **Background warm-status after create** *is* that priming scan. Keep it.
4. **Fsmonitor’s real job:** after priming, keep subsequent `git status` correct and O(upper delta), including when page cache is cold.
5. **Stretch:** seed the index FSMO extension at session create so the *first* client `git status` can trust empty upper and skip the tree walk.

## Non-goals (for this experiment)

- Fake porcelain via `lazytree status` as the primary UX (optional debug helper later)
- PATH-wrapping `git` (optional; flag-compatibility risk)
- Replacing builtin `fsmonitor--daemon` IPC (hook is enough to prove the thesis)

## Edge cases to validate

| Case | Expected |
| --- | --- |
| Edit file | upper has file → hook lists it → ` M` |
| New file | upper has file → `??` or `A` |
| Delete | whiteout in upper → list path → ` D` |
| Edit then restore identical bytes | may still list path; Git hash confirms clean |
| Metadata-only copy-up | list path; Git may no-op |
| Opaque dirs / unionfs whiteouts | map to deleted children correctly |
| Untracked only possible via upper | do not require scanning lower |

## Success metric

- **Near-term:** primed + fsmonitor → repeat status ~10 ms and correct under edits.
- **Stretch:** seed FSMO at create → **first** `git status` also O(delta), approaching worktree cold.

## Spike entry

`./scripts/spike_fsmonitor_upper.sh`
