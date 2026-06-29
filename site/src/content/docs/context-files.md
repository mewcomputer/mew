---
title: Context Files
description: AGENTS.md and CLAUDE.md files that give the agent project context.
---

mew discovers context files (AGENTS.md or CLAUDE.md) from the working
directory up to the git root. These files are loaded at startup and
injected into the agent's context, giving it project-specific
instructions, conventions, and architecture notes.

## What context files are for

Context files tell the agent about your project before it starts working.
Without one, the agent has to discover everything by reading files. With
one, it starts with your conventions, build commands, architecture, and
known gotchas already in context.

A good context file saves time on every turn. The agent doesn't have to
figure out that your project uses `cargo test` instead of `make test`, or
that your test files live in `tests/` not `src/tests/`.

## Discovery

`mew-context` walks from the current directory up to the git root,
collecting `AGENTS.md` and `CLAUDE.md` files. All found files are loaded.
If multiple files exist along the path (e.g. one in a subdirectory and
one at the root), they all get loaded and concatenated.

## What to put in a context file

### Essential

- **Build and test commands**: how to build, test, lint, and format
- **Architecture overview**: which directory does what, key abstractions
- **Conventions**: naming, file layout, error handling patterns

### Helpful

- **Common pitfalls**: things that look right but aren't
- **File locations**: where config lives, where tests go, where output lands
- **Dependencies**: what the project depends on and why

### Example

```markdown
# Project: my-app

## Build
- `cargo build -p my-app` to build
- `cargo test --all` to test
- `cargo clippy --all -- -D warnings` to lint

## Architecture
- `src/api/` - HTTP handlers
- `src/db/` - database layer
- `src/auth/` - authentication

## Conventions
- Use thiserror for error types
- All public functions need doc comments
- Tests go in #[cfg(test)] mod tests at the bottom of the file

## Gotchas
- The db layer caches connections, don't create new pools
- Auth tokens expire after 1 hour, refresh before API calls
```

## CLAUDE.md vs AGENTS.md

Both filenames are checked. `AGENTS.md` is the standard name for
agent-readable context files. `CLAUDE.md` is supported for compatibility
with Claude Code projects. If both exist in the same directory, both are
loaded.

The content format is the same for both. Use whichever name your team
prefers, or both if you work across different tools.

## How context is used

The context file content is injected into the system prompt on every turn.
This means:

- It's always available to the agent, not just at startup
- Changes to the file take effect on the next turn (no restart needed)
- It consumes context window tokens, so keep it focused

If your context file is large, consider moving detailed procedures into
[skills](/docs/skills/) that the agent loads on demand instead of
embedding everything in the system prompt.
