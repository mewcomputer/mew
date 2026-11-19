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
| `permissive` | ReadOnly and Mutating tools usually run without asking. Dangerous tools usually prompt. Secret-file guards, shell workspace checks, and deny rules still apply. Single-command checks skip allow rules, ask rules, and session grants. | `-P` |
| `auto` | The classifier decides whether to allow, deny, or ask you about a tool call. If the classifier is unavailable, mew asks you. | `-A` |
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
| `s` | Allow session | Run this call and remember the allowance for this active session. Rules and guards can still block or prompt for other calls. |
| `d` / `Esc` | Deny | Don't run. The agent sees the denial and adapts. |

`Allow session` grants survive `/clear` while the session stays active in
the daemon. The daemon keeps these grants in memory.

## Tool sensitivity

Every tool declares a sensitivity level. This sets the default behavior
when no rule matches:

| Sensitivity | Default behavior | Built-in tools |
|-------------|-----------------|----------------|
| `ReadOnly` | Auto-allow (no prompt) | Read, Glob, Grep |
| `Mutating` | Prompt | `write`, `edit_str_replace`, `job_cancel`, MCP tools |
| `Dangerous` | Prompt | `bash`, `shell_background`, `shell_monitor` |

MCP tools are always treated as `Mutating` regardless of what they do.

## Declarative rules

Configure rules in `config.toml` under `[permissions]`. Rules match by
tool name and optional conditions:

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
| `ask` | Prompt in standard mode for a single-command or file call when no deny or allow rule matches |

In standard mode, single-command checks evaluate deny rules before allow
rules. An allow rule takes precedence over an ask rule.

### Match conditions

A rule with no conditions matches every call to the named tool. Use `*`
as the tool name to match every tool.

| Field | Applies to | Description |
|-------|-----------|-------------|
| `command_prefix` | Single shell command | Matches the start of the command string, including spaces. |
| `command_program` | Compound shell command | Matches a program name, such as `git`. |
| `command_subcommand` | Compound shell command | Matches a program’s first non-flag argument, such as `log` in `git log`. |
| `path_glob` | path-based tools | Glob pattern matched against the path argument |

For pipelines and compound shell commands, mew checks workspace paths
first, then checks each program against the `bash` deny and allow rules.
An explicit allow needs a match for every program. The parser may prompt
for commands it cannot safely decompose.

Use `command_prefix` when you want to match a single shell command. A rule
with only `command_program` or `command_subcommand` matches every simple
call to that shell tool in the current implementation, so avoid those
fields in rules intended for simple calls.

## Workspace sandboxing

`workspace.roots` in config defines the directories the agent can access
without workspace approval. It feeds two layers.

**Agent layer** (path-based file tools): every
path argument is checked against the roots before the tool runs. Paths
outside the roots require workspace approval.

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

The `read` tool prompts for paths listed in `[[secrets.files]]` unless
an allow rule permits that exact path with a literal `path_glob`.

```toml
[[secrets.files]]
paths = [".env", "secrets.toml", "*.key"]
```

This sits above the normal rule cascade as its own tier. It fires in
`standard` and `permissive` modes. `dangerous` mode bypasses it.

## Secret redaction

`glob` and `grep` omit results from files that match `secrets.files`.
`read` and the shell output helpers replace configured secret values
with `[REDACTED]`. `grep` redacts matching output lines.

List the exact, case-sensitive values to redact in `[[secrets.words]]`.
Adding a file glob only protects the path. It does not add the file’s
contents to the redaction list.

```toml
[[secrets.words]]
values = ["my-secret-value"]
```

## Auto mode classifier

`auto` and `auto_plus` use a classifier in place of standard tool rules,
the secret-file prompt, and shell decomposition. Configure a provider in
`config.toml` or set the `micro` tier of a [router](/docs/using-mew/providers/#router-provider).

```toml
[permissions]
classifier_provider = "deepseek"
classifier_model = "deepseek-v4-flash"
```

The router’s `micro` tier takes precedence over the explicit classifier
settings. The two modes handle uncertainty differently.

- **`auto`**: if the classifier is unsure or fails, the call escalates
  to the user prompt.
- **`auto_plus`**: if the classifier is unsure or fails, the call is
  denied. No escalation.
