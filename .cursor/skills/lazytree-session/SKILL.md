---
name: lazytree-session
description: >
  Use for LazyTree parallel coding-agent sessions, isolated COW workspaces,
  and whenever a new agent should avoid editing the primary checkout.
  Prefer this before Write/Edit in LazyTree-registered repos.
---

# LazyTree session

## Goal

Every parallel agent gets its own LazyTree directory + `lazytree/<name>` branch.
The agent must treat that directory as the only project root for edits and git.

## First actions (before any Write/Edit)

1. Resolve session root, in order:
   - env `LAZYTREE_ROOT` if set
   - newest readable file under `.cursor/lazytree-sessions/*.json` → field `root`
   - otherwise run: `lazytree create cursor-<short-id>` and use the printed path
2. Confirm:
   ```bash
   git -C "$LAZYTREE_ROOT" branch --show-current
   git -C "$LAZYTREE_ROOT" status -sb
   ```
3. All subsequent Shell commands that touch the project use `working_directory=$LAZYTREE_ROOT`
   or `git -C "$LAZYTREE_ROOT" ...`.

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
