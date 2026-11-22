---
title: Context Files
description: AGENTS.md and CLAUDE.md files that give the agent project context.
---

Context files add project conventions, commands, and repository layout to the
system prompt when Mew starts a session.

## Discovery

Mew searches from the Git worktree root down to the conversation's working
directory, loading files from general to specific. Outside a Git worktree, Mew
starts from your home directory if the working directory sits inside it, or reads
the working directory alone otherwise.

At each directory level, Mew reads `AGENTS.md` (falling back to `CLAUDE.md` if
absent), then loads `.mew/AGENTS.md` and `.mew/wiki.md` when present.

Before project files, Mew loads a global `AGENTS.md` from the config directory
(the parent directory of the file printed by `mew config path`). When absent,
it falls back to `~/.claude/CLAUDE.md`. Run `mew debug context` to inspect the
resolved files.

## Recommended contents

### Essential

- **Build and test commands**: build, test, lint, and format commands.
- **Architecture overview**: directory layout and key types.
- **Conventions**: naming rules, module structure, and error patterns.

### Helpful

- **Gotchas**: non-obvious pitfalls and invariants.
- **File paths**: config, tests, and generated output locations.
- **Dependencies**: key libraries and why the project uses them.

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
- Document public functions
- Put unit tests in `#[cfg(test)] mod tests`

## Gotchas
- The db layer caches connections; reuse the existing pool
- Auth tokens expire after 1 hour
```

## CLAUDE.md and AGENTS.md

Both files use the same Markdown format. When both exist in the same directory,
Mew loads `AGENTS.md`.

## `@file` includes

Split long instructions across files using `@path/to/file` lines. When Mew
reads a context file (`AGENTS.md`, `CLAUDE.md`, `.mew/AGENTS.md`, `.mew/wiki.md`,
or the global fallback file), each line starting with `@` followed by a relative
path expands to that file's contents as literal text.

```markdown
# Project: my-app

## Build
@docs/build-commands.md

## Architecture
@docs/architecture.md

## Conventions
@docs/coding-style.md
```

Write include paths relative to the file containing the `@` line. Mew rejects
paths containing `..` parent traversal, leaves missing or invalid `@` lines in
place, and logs a warning.

## Templates and project variables

Context files also support minijinja templating when YAML frontmatter sets
`mew: true` or `polytoken: true`.

Store project-local template variables in `project_vars.yaml`, a flat YAML map
of string keys to string values:

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

Mew searches `.mew/`, `.opencode/`, `.claude/`, and `.agents/` for
`project_vars.yaml` from the Git root (or `$HOME`) down to the working
directory. The first match wins.

## Reloading and token budget

Context files enter the session's system prompt at startup and consume context
tokens on every turn. Start a new session to apply edits to context files. For
long task-specific guides, move them into [skills](/docs/using-mew/skills/) so
the agent loads them only when needed.

## Inspecting loaded context

Run `mew debug context` from your project directory to inspect the active
prompt sources. Disk files appear under their filesystem paths; built-in VFS
resources pulled in via `transclude` appear under `mew://`:

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

The `mew://` tree lists only the resolved template branches and nested
transclusions (such as a single provider-specific `base_*` prompt). Pass
`--full` to print the complete system prompt, including the base scaffold,
context files, and skill catalog. Diagnostics go to stderr, so stdout can be
redirected cleanly:

```sh
mew debug context --full > context.txt
mew debug context --full | less
```
