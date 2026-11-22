---
title: Permissions
description: How Mew checks tool calls and how to configure approval rules.
---

Every tool call passes through a permission check before running. Mew evaluates
the active permission mode, declarative rules in `config.toml`, tool
sensitivity, and workspace boundaries.

## Permission modes

Switch modes at runtime with `/permissions <mode>` (or `/permissions` to open
the picker), or launch `mew chat`, `mew run`, or `mew daemon` with CLI flags or
environment variables:

| Mode | Behavior | CLI flag / env var |
|------|----------|--------------------|
| `standard` | Prompts for `Mutating` and `Dangerous` tools; auto-allows `ReadOnly`. Rules and workspace checks apply (default). | (default) |
| `permissive` | Auto-allows `ReadOnly` and `Mutating` tools; prompts for `Dangerous` tools. Secret-file guards, shell workspace checks, and deny rules apply. Single-command checks skip allow rules, ask rules, and session grants. | `-P`, `--permissive` / `MEW_PERMISSIVE` |
| `auto` | Routes every tool call to the classifier model (`allow`, `deny`, or prompt). Prompts you when the classifier is unavailable. | `-A`, `--auto` / `MEW_AUTO` |
| `auto_plus` | Routes tool calls to the classifier and fails closed: uncertainty or classifier failure denies the call. | `--auto-plus` / `MEW_AUTO_PLUS` |
| `dangerous` | Auto-allows every tool and bypasses deny rules, ask rules, secret-file guards, and shell decomposition. Output redaction still applies. | `-D`, `--dangerously-skip-permissions` / `MEW_DANGEROUS` |

Running bare `mew` starts in `standard` mode and ignores the permission
environment variables; use `mew chat`, `mew run`, or `mew daemon` when passing
them.

## Permission prompt

When a call requires approval, Mew pauses the turn and shows the tool name, its
arguments, and three choices:

| Key | Decision | Effect |
|-----|----------|--------|
| `a` | Allow once | Run this call once; later calls prompt again. |
| `s` | Allow session | Run this call and cache approval for this daemon session (survives `/clear`). |
| `d` / `Esc` | Deny | Skip execution and return a denial error to the agent. |

## Tool sensitivity

Each tool declares a sensitivity level used when no rule matches:

| Sensitivity | Default behavior | Built-in tools |
|-------------|------------------|----------------|
| `ReadOnly` | Auto-allow | `read`, `glob`, `grep`, `browser_snapshot`, `browser_screenshot` |
| `Mutating` | Prompt in `standard` | `write`, `edit_str_replace`, `edit_hashline`, `job_cancel`, `browser_open`, `browser_click`, `browser_fill`, `browser_press`, `browser_close` |
| `Dangerous` | Prompt in `standard` and `permissive` | `bash`, `shell_background`, `shell_monitor`, MCP tools (`mcp__*`) |

MCP tools are always classified as `Dangerous` regardless of the action they
perform.

## Declarative rules

Define rules under `[permissions]` in `config.toml`:

```toml
[[permissions.rules]]
tool = "bash"
decision = "deny"
match.command_prefix = "git push"

[[permissions.rules]]
tool = "bash"
decision = "allow"
match.command_prefix = "git log "

[[permissions.rules]]
tool = "write"
decision = "ask"
match.path_glob = "secrets/**"
```

### Rule decisions

| Decision | Effect |
|----------|--------|
| `allow` | Auto-allow matching calls |
| `deny` | Block matching calls |
| `ask` | Prompt in `standard` mode when no `deny` or `allow` rule matches |

In `standard` mode, single-command checks evaluate `deny` before `allow`, and
`allow` before `ask`.

### Match conditions

A rule without `match` conditions matches every call to `tool` (use `"*"` to
match all tools):

| Field | Applies to | Description |
|-------|------------|-------------|
| `command_prefix` | Single shell commands | Matches the start of the command string, including trailing spaces. |
| `command_program` | Compound shell commands | Matches a program name such as `git`. |
| `command_subcommand` | Compound shell commands | Matches a program's first non-flag argument, such as `log` in `git log`. |
| `path_glob` | Path-based tools | Matches a glob pattern against the tool's `path` argument. |

For pipelines and compound shell commands, Mew checks workspace paths first and
then evaluates each program against `bash` `deny` and `allow` rules. Compound
allows require a match for every program, and commands that cannot be parsed
safely trigger a prompt.

Use `command_prefix` for single shell commands; rules with only
`command_program` or `command_subcommand` match every simple shell call and
should be paired with compound workflows only.

## Workspace sandboxing

`workspace.roots` in `config.toml` lists directories the agent may access
without workspace prompts:

```toml
[workspace]
roots = ["~/code/myproject"]
```

It drives two enforcement layers:

- **Agent layer** (`read`, `write`, `edit_str_replace`, `edit_hashline`, `glob`,
  `grep`): targets outside `roots` (or the current directory when `roots` is
  empty) prompt once for the containing directory and cache granted directories
  for the session.
- **Escape tier** (`bash`, `shell_background`, `shell_monitor`): when `roots` is
  non-empty, Mew parses shell commands and escalates paths outside `roots` (plus
  conservative `$HOME/...` and `~/...` references) from `allow` to a prompt.
  Empty `roots` disable the escape tier.

## Secret file guard

`read` prompts for files matching `[[secrets.files]]` unless a literal
`path_glob` `allow` rule permits that exact path:

```toml
[[secrets.files]]
paths = [".env", "secrets.toml", "*.key"]
```

This guard runs above regular rules in `standard` and `permissive` modes.
`dangerous` mode bypasses it.

## Secret redaction

`glob` and `grep` skip filenames matching `[[secrets.files]]`. `read`, `grep`,
shell output helpers, MCP tools, and Python code mode replace exact
case-sensitive matches from `[[secrets.words]]` with `[REDACTED]`.

```toml
[[secrets.words]]
values = ["my-secret-value"]
```

File globs protect filenames only; list literal sensitive values under
`[[secrets.words]]` to redact file contents.

## Auto mode classifier

`auto` and `auto_plus` delegate decisions to an LLM classifier configured under
`[permissions]` or via the `micro` tier of a
[router provider](/docs/using-mew/providers/#router-provider):

```toml
[permissions]
classifier_provider = "deepseek"
classifier_model = "deepseek-v4-flash"
```

A router's `micro` tier takes precedence over explicit `classifier_*` fields.
When the classifier is uncertain or errors, `auto` prompts you while `auto_plus`
denies the call.
