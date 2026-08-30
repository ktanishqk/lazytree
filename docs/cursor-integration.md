# Cursor soft integration (hook + skill)

This is the recommended near-term Cursor UX for LazyTree.

## Goals A / B / C

| Mode | What it does | Status |
| --- | --- | --- |
| **A — Soft steer** | `sessionStart` creates/resolves a LazyTree session; skill tells the agent to work only there | Implemented in `.cursor/` |
| **C — Soft gate** | `preToolUse` / `beforeShellExecution` deny edits & git commit/push outside the mapped root | Implemented (fail-open if no mapping) |
| **B — UI branch sync** | Make Cursor Source Control show `lazytree/...` | Manual/helper: `lazytree cursor open <session>` |

## Install in a consumer repo

1. Build CLI: `cargo build --release && cp target/release/lazytree ~/.local/bin/`
   (or use the npm wrapper under `npm/` which shells out to the native binary)
2. Copy/symlink into your project:
   - `.cursor/hooks.json`
   - `.cursor/hooks/lazytree-*.sh`
   - `.cursor/skills/lazytree-session/`
3. Trust the workspace so project hooks run.
4. Cmd+N → hook writes `.cursor/lazytree-sessions/<id>.json`
5. Prefer `/lazytree-session` (or rely on auto skill pick) on the first turn.

## What the hook guarantees

- Side effect: a LazyTree session exists and is mapped to this composer `session_id`
- Context injection: path + branch + protocol (advisory; may be dropped by Cursor)
- Env for **later hooks**: `LAZYTREE_ROOT`, `LAZYTREE_BRANCH`, `LAZYTREE_SESSION`

It does **not** rebind Cursor’s workspace folder.

## What the gates guarantee

If a mapping exists for this chat:

- Mutating file tools with an explicit path outside `LAZYTREE_ROOT` → **deny**
- `git commit` / `git push` clearly aimed at the primary checkout → **deny**

If no mapping → fail-open (normal Cursor behavior).

## Making the IDE show our branch (B)

```bash
lazytree cursor open my-session
# opens the session root in Cursor/VS Code
```

Then Source Control / branch chrome follow that folder’s HEAD (`lazytree/my-session`).

## Failure modes

- Cloud Agents: no `sessionStart`; use project skill + bake CLI into the environment image
- Agent ignores skill and edits primary root before a gated tool fires
- Hook context dropped (Cursor race) — mapping file + skill still recover
- Chat resume may not re-fire `sessionStart` — mapping file persists

## Validate locally

```bash
./scripts/test_cursor_hooks.sh
cargo test --tests
```

## Hook schema notes (Cursor docs)

- Deny via `"permission": "deny"` (+ `userMessage` / `agentMessage`)
- `sessionStart` output is only `env` + `additional_context` (cannot rebind cwd)
- Matchers: `preToolUse` on tool names; `beforeShellExecution` on command regex
- Cloud: `sessionStart` does **not** run; `preToolUse` / `beforeShellExecution` do
