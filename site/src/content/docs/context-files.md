---
title: Context Files
description: AGENTS.md and CLAUDE.md files that give the agent project context.
---

mew discovers context files (AGENTS.md or CLAUDE.md) from the working
directory up to the git root. These files are loaded at startup and injected
into the agent's context, giving it project-specific instructions.

## Discovery

`mew-context` walks from the current directory up to the git root,
collecting `AGENTS.md` and `CLAUDE.md` files. All found files are loaded.

## What to put in a context file

- Project structure overview
- Coding conventions and style rules
- Build and test commands (`cargo build`, `just ci`, etc.)
- Architecture notes (which crate does what)
- Common pitfalls and gotchas
- File locations (where config lives, where tests are)

## Example

```markdown
# Project: my-app

## Build
- `cargo build -p my-app` to build
- `cargo test --all` to test

## Architecture
- `src/api/` — HTTP handlers
- `src/db/` — database layer
- `src/auth/` — authentication

## Conventions
- Use `thiserror` for error types
- All public functions need doc comments
- Tests go in `#[cfg(test)] mod tests` at the bottom of the file
```

## CLAUDE.md vs AGENTS.md

Both filenames are checked. `AGENTS.md` is the standard name for
agent-readable context files. `CLAUDE.md` is supported for compatibility
with Claude Code projects. If both exist in the same directory, both
are loaded.
