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
4. Empty upper → token only (no paths) — Git skips full-tree walk
5. Unknown/stale token → may return `/` once (trivial = full scan) then resume

Git keeps real `git status` semantics; agents need no teaching.

## Non-goals (for this experiment)

- Fake porcelain via `lazytree status` as the primary UX
- PATH-wrapping `git` (optional later; flag compatibility risk)
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

On medium/large fixtures, **first and repeat `git status`** with LazyTree+fsmonitor should approach worktree order of magnitude (or at least crush today’s cold FUSE numbers), while create stays ~O(1).

## Spike entry

`./scripts/spike_fsmonitor_upper.sh` (when added) — create session, walk upper into hook, compare status ms with/without `core.fsmonitor`.
