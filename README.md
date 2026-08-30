# LazyTree

Copy-on-write development workspaces for parallel coding agents.

> Status: Milestone 4 — benchmarks recorded (see `docs/benchmarks.md`). Mixed result: COW files work; Git index often dominates.

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

## Milestone 1 CLI

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
```

`LAZYTREE_HOME` (default `~/.lazytree`) controls metadata and session storage.

## Milestone 0

```bash
./scripts/m0_overlay_spike.sh
./scripts/m0_benchmark.sh
```

See `docs/feasibility-m0.md` and `docs/design-decisions.md`.

## License

Apache-2.0
