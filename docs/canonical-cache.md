# Canonical-path cache carryover

## Problem

Cargo (and many LSPs) fingerprint **absolute workspace paths**. LazyTree sessions
have different merged roots, so copying `target/` across sessions often looks
cold — the open M5 issue.

## Approach

`lazytree exec` (default) runs the command inside:

```text
unshare --user --map-root-user --mount
  mount --bind $SESSION_ROOT  $LAZYTREE_HOME/canonical/workspace
  mount --bind $SESSION_TARGET $LAZYTREE_HOME/canonical/target
  cd workspace && exec …
```

Each process has a **private mount namespace**, so three agents can all use the
same canonical paths concurrently without colliding.

Workflow:

```bash
lazytree create a
lazytree create b
lazytree exec a -- cargo check
lazytree cache promote a    # shared semantic seed
lazytree cache seed b
lazytree exec b -- cargo check   # warm
```

## Measurement (serde demo, this VM)

| Step | Time |
| --- | ---: |
| Cold `cargo check` (session A) | **2781 ms** |
| Seeded + canonical (session B) | **124 ms** |
| Seeded + canonical (session C) | **120 ms** |

≈ **23×** faster for the warm sessions on this fixture.

## Limits

- Needs user namespaces (`unshare --user --map-root-user --mount`).
- Use `--no-canonical` to disable.
- Registry deps are already mostly path-agnostic; the big win is **workspace
  package** incremental reuse under a stable apparent path.
- LSP process heaps are still per-agent (unchanged).
