---
title: Context Files
description: AGENTS.md and CLAUDE.md files that give the agent project context.
---

Mew loads project context from the conversation's working directory up
to the Git worktree root. It adds these files to the system prompt when
it builds the session.

## What context files are for

Context files tell the agent about your project before it starts working.
Without one, the agent has to discover everything by reading files. With
one, it starts with your conventions, build commands, architecture, and
known gotchas already in context.

A good context file saves time on every turn. The agent doesn't have to
figure out that your project uses `cargo test` instead of `make test`, or
that your test files live in `tests/` not `src/tests/`.

## Discovery

Mew loads context from the Git worktree root down to the conversation's
working directory, in order from broad to specific. Outside a worktree,
it uses your home directory as the root when the working directory is
inside it. In other locations it checks the working directory itself.

At each level, Mew reads `AGENTS.md` or falls back to `CLAUDE.md`. It
also loads `.mew/AGENTS.md` and `.mew/wiki.md` when present.

Before project files, Mew loads a global `AGENTS.md` from the directory
reported by `mew config path`. If that file is absent it tries
`~/.claude/CLAUDE.md`. Use `mew debug context` to inspect the sources it
found.

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

Both files use the same Markdown format. Where both exist in the same
directory, Mew uses `AGENTS.md`.

## `@file` includes

You can split context across multiple files using `@path/to/file` lines.
When mew loads an AGENTS.md or CLAUDE.md file, any line starting with `@`
followed by a path is replaced with the contents of that file (as literal
text, no template rendering).

```markdown
# Project: my-app

## Build
@docs/build-commands.md

## Architecture
@docs/architecture.md

## Conventions
@docs/coding-style.md
```

Write paths relative to the file that contains the `@` reference.

Missing files leave the `@` line as-is and log a warning. This works in
both AGENTS.md and CLAUDE.md, and in `.mew/AGENTS.md` additively loaded
files.

## Project variables

Project-local variables can be stored in `.mew/project_vars.yaml` and
referenced in persona, skill, and subagent templates as `project_vars`:

```yaml
# .mew/project_vars.yaml
team: platform
channel: "#eng-platform"
escalation: "#on-call"
```

```markdown
---
name: incident-response
description: Guides incident response.
mew:
  template: true
---

When responding to incidents, escalate to {{ project_vars.escalation }}.
Team channel: {{ project_vars.channel }}.
```

The file is a flat YAML map of string keys to string values. It's
searched in `.mew/`, `.opencode/`, `.claude/`, and `.agents/` directories
from cwd up to the git root. First match wins.

## How context is used

Context files form part of the session's system prompt and use context
window tokens. Start a new session after editing a context file to use
the revised instructions.

If your context file is large, consider moving detailed procedures into
[skills](/docs/using-mew/skills/) that the agent loads on demand instead of
embedding everything in the system prompt.

## Inspecting loaded context

Run `mew debug context` from the project directory to inspect the sources
used by the CLI's debug prompt assembly. File-system context files appear under
their paths; built-in VFS resources pulled in by templates (`transclude`) appear
under `mew://`:

```text
$ mew debug context
├── mew://
│   └── system_prompts
│       ├── _builtin_file_tool_preference
│       ├── _shell_file_edit_mandate
│       ├── _shell_monitor_preference
│       ├── _tool_library
│       └── base_openai
└── ~
    ├── .claude/CLAUDE.md
    └── code/my-app/AGENTS.md
```

The VFS list is resolved, not scanned: only the branches actually taken are
shown (for example a single provider-specific `base_*` variant), and nested
transclusions are followed. Add `--full` to print the fully assembled system
prompt assembled by the debug command, including the base scaffold, context
files, and skills listing. The command writes diagnostics to stderr, so you
can redirect the prompt:

```bash
mew debug context --full > context.txt
mew debug context --full | less
```
