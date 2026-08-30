# LazyTree

Copy-on-write development workspaces for parallel coding agents.

> Status: Open cache problem addressed via canonical-path exec (~23× warm cargo check).

## Idea

Give each coding agent a normal directory backed by a COW filesystem view instead of a fully materialized `git worktree` checkout.

```bash
lazytree repo add ~/src/my-repo
lazytree create ticket-123
lazytree create experiment --from main
cd "$(lazytree path ticket-123)"
git branch --show-current   # lazytree/ticket-123
```

## Prerequisites (Linux)

- `git`
- `fuse-overlayfs` (and passwordless `sudo` for mounts in nested/cloud VMs where unprivileged FUSE is blocked)
- Or kernel OverlayFS when the host allows unprivileged/privileged `mount -t overlay`

## Build

```bash
cargo build --release
./target/release/lazytree --help
```

## CLI

```bash
lazytree repo add <path>
lazytree repo list
lazytree repo remove <repo>
lazytree create <name>
lazytree list
lazytree path <session>
lazytree status <session> [--json]
lazytree diff <session>
lazytree archive <session>
lazytree doctor [--json]
lazytree destroy <session> [--force]
lazytree exec <session> -- <command>
lazytree cache promote <session>
lazytree cache seed <session>
lazytree cursor setup [--target <project>]
lazytree cursor open <session> [--json|--dry-run]
```

`LAZYTREE_HOME` (default `~/.lazytree`) controls metadata and session storage.

## Cursor soft integration

Install hooks + skill into a consumer project:

```bash
lazytree cursor setup --target ~/src/my-repo
```

Then Cmd+N: `sessionStart` creates a LazyTree session, injects context, and gates
deny writes / `git commit|push` outside that root. Optional UI branch sync:

```bash
lazytree cursor open <session>
```

See `docs/cursor-integration.md`. Validate with `./scripts/test_cursor_hooks.sh`.

## npm wrapper

```bash
cd npm && npm link   # requires native binary on PATH or LAZYTREE_BIN
```

## Milestone 0

```bash
./scripts/m0_overlay_spike.sh
./scripts/m0_benchmark.sh
```

See `docs/canonical-cache.md`, `docs/feasibility-m0.md`, `docs/cursor-integration.md` and `docs/design-decisions.md`.

Validate: `./scripts/smoke.sh`

## License

Apache-2.0
