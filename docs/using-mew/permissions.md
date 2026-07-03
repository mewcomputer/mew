---
title: Permissions
description: How mew decides whether to run a tool, and how to control it.
---

Every tool call passes through a permission check before it runs. The
check considers the current permission mode, your declarative rules, the
tool's sensitivity level, and the workspace sandbox. This page covers
what you see and how to configure it.

## Permission modes

Modes control how aggressive the permission gate is. Switch at runtime
with `/permissions <mode>` or CLI flags.

| Mode | Behavior | CLI flag |
|------|----------|----------|
| `standard` | Prompts for Mutating and Dangerous tools. ReadOnly tools auto-allow. Rules and the workspace escape tier still apply. Default. | (default) |
| `permissive` | Mutating tools auto-allow. Bash still prompts. Secret-file guard and bash escape tier still fire. Rules and session cache are skipped. | `-P` |
| `auto` | Every tool call is classified by a small LLM. The classifier decides allow, deny, or escalate to user. Requires a classifier provider in config. | `-A` |
| `auto_plus` | Like `auto`, but the classifier cannot escalate. If the classifier is unsure or fails, the call is denied. Fails closed. | `--auto-plus` |
| `dangerous` | Every tool auto-runs. Bypasses deny rules, ask rules, the secret-file guard, and bash decomposition. Output redaction still applies. | `-D` |

Switch at runtime:

```
/permissions permissive
```

Or open the picker:

```
/permissions
```

## The permission prompt

When a tool needs approval, mew pauses the turn and shows a modal with
the tool name, its input, and three choices:

| Key | Decision | Effect |
|-----|----------|--------|
| `a` | Allow once | Run this call. The next call to the same tool prompts again. |
| `s` | Allow session | Run this call and all future calls to this tool for the rest of the session. |
| `d` / `Esc` | Deny | Don't run. The agent sees the denial and adapts. |

`Allow session` grants survive `/clear`. They're tied to the session
(the JSONL log), not the visible context. See [Sessions](/docs/getting-started/sessions/)
for the distinction.

## Tool sensitivity

Every tool declares a sensitivity level. This sets the default behavior
when no rule matches:

| Sensitivity | Default behavior | Built-in tools |
|-------------|-----------------|----------------|
| `ReadOnly` | Auto-allow (no prompt) | Read, Glob, Grep |
| `Mutating` | Prompt | Write, Edit, MCP tools |
| `Dangerous` | Prompt (highest urgency) | Bash, shell_background, shell_monitor |

MCP tools are always treated as `Mutating` regardless of what they do.

## Declarative rules

Configure rules in `config.toml` under `[permissions]`. Rules match by
tool name and optional conditions:

```toml
[[permissions.rules]]
tool = "bash"
decision = "deny"
match.command_prefix = "rm -rf"

[[permissions.rules]]
tool = "bash"
decision = "allow"
match.command_program = "git"
match.command_subcommand = "log"

[[permissions.rules]]
tool = "write"
decision = "ask"
match.path_glob = "~/secrets/**"
```

### Rule decisions

| Decision | Effect |
|----------|--------|
| `allow` | Auto-allow matching calls |
| `deny` | Block matching calls |
| `ask` | Always prompt, even if the mode would auto-allow |

Deny rules always win. If a call matches both a deny and an allow rule,
it's denied.

### Match conditions

All conditions are optional. At least one must be present for a rule to
match anything.

| Field | Applies to | Description |
|-------|-----------|-------------|
| `command_prefix` | bash | Matches the start of the full command string |
| `command_program` | bash | Matches the program name (e.g. `"git"` for `git push`) |
| `command_subcommand` | bash | Matches the first non-flag argument (e.g. `"push"` for `git push`) |
| `path_glob` | path-based tools | Glob pattern matched against the path argument |

For compound bash commands (e.g. `git log | grep fix`), each program is
checked against the rules. If any program matches a deny rule, the whole
command is denied.

## Workspace sandboxing

`workspace.roots` in config defines directories the agent is allowed to
touch. It feeds two layers:

**Agent layer** (path-based tools: read, write, edit, glob, grep): every
path argument is checked against the roots before the tool runs. Paths
outside the roots are rejected.

**Escape tier** (bash, shell_background, shell_monitor): shell commands
are parsed, and any path-shaped argument that resolves outside the
workspace roots escalates the decision from `AllowOnce` to `Prompt`.
This catches `cat /etc/passwd` or `rm ~/important-file` even when the
mode would auto-allow bash.

```toml
[workspace]
roots = ["~/code/myproject"]
```

Empty `workspace.roots` disables the escape tier. The agent layer still
applies (defaulting to the current directory when empty).

`$HOME/...` and `~/...` paths are conservatively flagged as escapes
without trying to resolve them. A `cat /etc/passwd` always escapes
regardless of cwd.

## Secret file guard

Files listed under `[secrets.files]` are protected: reading them forces
a permission prompt unless a literal (non-glob) allow rule explicitly
permits that exact path.

```toml
[[secrets.files]]
paths = [".env", "secrets.toml", "*.key"]
```

This sits above the normal rule cascade as its own tier. It fires in
`standard` and `permissive` modes. `dangerous` mode bypasses it.

## Secret redaction

Two layers of redaction prevent secrets from leaking into tool output:

**Pattern-based**: regex patterns match common secret formats (API keys,
tokens, passwords). Matches are redacted before the output reaches the
model or the display.

**File-based**: contents of files listed in `secrets.files` are loaded
and redacted from all tool output. If a bash command prints a value
that appears in your `.env`, it's replaced with `[REDACTED]`.

```toml
[[secrets.words]]
values = ["my-api-key-value", "sk-1234567890"]
```

Secret words let you redact specific values that don't match common
patterns.

## Auto mode classifier

`auto` and `auto_plus` modes route every tool call through a small LLM
classifier. Configure it in `config.toml`:

```toml
[permissions]
classifier_provider = "deepseek"
classifier_model = "deepseek-v4-flash"
```

If no classifier is configured, `auto` mode falls through to the user
modal on every call (same as `standard`). The difference between `auto`
and `auto_plus`:

- **`auto`**: if the classifier is unsure or fails, the call escalates
  to the user prompt.
- **`auto_plus`**: if the classifier is unsure or fails, the call is
  denied. No escalation.
