---
name: lazytree-session
description: >
  Use for LazyTree parallel coding-agent sessions, isolated COW workspaces,
  and whenever a new agent should avoid editing the primary checkout.
  Prefer this before Write/Edit in LazyTree-registered repos. Also use on
  Cloud Agents where sessionStart hooks do not run.
---

# LazyTree session

## Goal

Every parallel agent gets its own LazyTree directory + `lazytree/<name>` branch.
The agent must treat that directory as the only project root for edits and git.

## First actions (before any Write/Edit)

1. Resolve session root, in order:
   - env `LAZYTREE_ROOT` if set
   - mapping file for this chat under `.cursor/lazytree-sessions/<session_id>.json` → `root`
   - newest readable file under `.cursor/lazytree-sessions/*.json` → `root`
   - otherwise bootstrap (see below)
2. Confirm:
   ```bash
   git -C "$LAZYTREE_ROOT" branch --show-current
   git -C "$LAZYTREE_ROOT" status -sb
   ```
3. All subsequent Shell commands that touch the project use `working_directory=$LAZYTREE_ROOT`
   or `git -C "$LAZYTREE_ROOT" ...`.

## Bootstrap (IDE sessionStart missing / Cloud Agents)

Cloud Agents do **not** fire `sessionStart`. Prefer the one-shot CLI:

```bash
# once per machine/image: CLI on PATH, repo registered
lazytree repo add "$PWD"   # if not already registered

ROOT=$(lazytree cursor bootstrap \
  --session-id "${COMPOSER_SESSION_ID:-$$}" \
  --project "$PWD")
export LAZYTREE_ROOT="$ROOT"
```

`lazytree cursor bootstrap` creates/reuses a session and writes
`.cursor/lazytree-sessions/<session_id>.json` (prints the root on stdout).

Manual fallback (same effect):

```bash
NAME="cursor-${COMPOSER_SESSION_ID:-$$}"
NAME=$(printf '%s' "$NAME" | tr -cd 'a-zA-Z0-9_-' | cut -c1-40)
ROOT=$(lazytree create "$NAME")
mkdir -p .cursor/lazytree-sessions
printf '%s\n' "{\"session_id\":\"$NAME\",\"name\":\"$NAME\",\"root\":\"$ROOT\",\"branch\":\"lazytree/$NAME\"}" \
  > ".cursor/lazytree-sessions/${NAME}.json"
export LAZYTREE_ROOT="$ROOT"
```

Gates (`preToolUse` / `beforeShellExecution`) still run in cloud once a mapping
or `LAZYTREE_ROOT` exists — so writing the mapping file matters.

## Rules

- **Do not** modify files in the primary checkout outside `$LAZYTREE_ROOT`.
- **Do not** create a `git worktree` for isolation — LazyTree already is the isolation.
- Prefer `lazytree exec <name> -- <cmd>` for builds so canonical-path caches apply.
- When done with durable work: `lazytree archive <name>` (publishes branch) or leave the session for the user.

## Optional: make Cursor UI show this branch

Opening `$LAZYTREE_ROOT` as the workspace folder (File → Open Folder, or
`lazytree cursor open <name>`) makes the IDE Source Control view match the
session branch. Hooks cannot do that automatically.

## When unsure

Ask which LazyTree session to use. Do not invent a second clone.
