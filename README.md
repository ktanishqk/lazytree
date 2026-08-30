# LazyTree

Copy-on-write development workspaces for parallel coding agents.

> Status: Milestone 0 feasibility spike. Not yet a usable product CLI.

## Idea

Give each coding agent a normal directory and Git branch, backed by a COW filesystem view instead of a fully materialized `git worktree` checkout.

```text
lazytree create ticket-123
# → ~/.lazytree/workspaces/ticket-123/root
```

## Milestone 0

Prove dual OverlayFS-style sessions over one immutable base.

```bash
./scripts/m0_overlay_spike.sh
./scripts/m0_benchmark.sh
```

See `docs/feasibility-m0.md` and `docs/design-decisions.md`.

## License

Apache-2.0 or MIT (TBD before first release).
EOF