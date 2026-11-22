---
title: Tools
description: Built-in tools the agent uses to inspect files, edit code, and run commands.
---

Each built-in tool exposes a name, a JSON Schema parameter list, and a
sensitivity level. The model invokes tools during a turn to inspect and edit
your project.

## How tool calls work

1. The model emits a tool call with a name and arguments.
2. Mew checks permissions (see [Permissions](/docs/using-mew/permissions/)).
3. If approved, the tool executes and returns its output.
4. Mew appends the tool result to the conversation for the model's next turn.

In the TUI, tool calls render as cards showing the tool name, arguments, and
output. Press `Ctrl+O` to toggle expansion for bash output, Python cells, and
nested tool details. Diffs from `write`, `edit_str_replace`, and
`edit_hashline` render with theme-colored `+` and `-` lines.

## Built-in tools

### File operations

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `read` | `ReadOnly` | Read a file with `[path#hash]` headers and numbered lines for hashline edits. Supports `offset` and `limit`, and returns images to vision-capable models. |
| `write` | `Mutating` | Create or overwrite a file. |
| `edit_str_replace` | `Mutating` | Replace a unique substring in a file (fails if ambiguous and returns surrounding lines for recovery). |
| `glob` | `ReadOnly` | Match file paths by glob pattern, sorted by modification time (honors `.gitignore`). |
| `grep` | `ReadOnly` | Search file contents by regular expression (uses ripgrep when available) with include filters and context lines. |

See [Hashline Edits](/docs/using-mew/hashline/) for line-anchored patching.

### Shell

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `bash` | `Dangerous` | Execute a shell command and return captured output with secret redaction. |
| `shell_background` | `Dangerous` | Start a background shell command and return its job ID immediately. |
| `shell_monitor` | `Dangerous` | Poll a readiness command until it exits zero or times out. |
| `job_status` | `ReadOnly` | Check the status of a background shell job or subagent task. |
| `job_block` | `ReadOnly` | Wait for a background job to finish. |
| `job_cancel` | `Mutating` | Stop a running background job and wait for termination (signals its process group on Unix). |

A child process that spawns its own process group or session can outlive
`job_cancel`.

### Session and agent

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `echo` | `ReadOnly` | Echo input text for testing and diagnostics. |
| `exit_tool` | `ReadOnly` | Return a final answer from a subagent to its parent, or stop the current top-level turn. |
| `flag_important` | `ReadOnly` | Mark a file so context compaction retains it when summarizing earlier turns. |
| `progress_update` | `ReadOnly` | Emit a subagent progress line shown in attached client UIs. |
| `ask_user_question` | `ReadOnly` | Present 1 to 4 interactive questions to the user when input is needed. |

### Plans and goals

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `write_plan` | `ReadOnly` | Write a plan file under `plans/` or the configured `plan_path`. |
| `edit_plan` | `ReadOnly` | Replace text inside a plan file within `plan_path` bounds. |
| `propose_goal` | `ReadOnly` | Propose a multi-turn goal for user confirmation. |
| `complete_goal` | `ReadOnly` | Mark the active goal complete with a summary. |
| `block_goal` | `ReadOnly` | Mark the active goal blocked with a reason. |

Plan tools enforce path restrictions to `plan_path` even though their
sensitivity is `ReadOnly`. See [Personas](/docs/using-mew/personas/) for the
planner/builder workflow and [Slash Commands](/docs/using-mew/slash-commands/#goals)
for goal controls.

### Subagents

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `subagent_start` | `ReadOnly` | Spawn a child agent synchronously or in the background (`async: true`). |
| `subagent_wait` | `ReadOnly` | Collect the result from a background subagent task. |

See [Subagents](/docs/using-mew/subagents/) for subagent definitions and limits.

### Todos

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `todo_create` | `ReadOnly` | Create one or more tracked todo items. |
| `todo_update` | `ReadOnly` | Update a todo's content or status (`pending`, `in_progress`, `done`, `blocked`). |
| `todo_complete` | `ReadOnly` | Mark a todo done once its dependencies are done. |
| `todo_delete` | `ReadOnly` | Delete a todo if no other todo depends on it. |
| `todo_list` | `ReadOnly` | List all todos with IDs, statuses, and dependencies. |

Todos appear in the terminal sidebar. Run `/todo` (or select `/todo` from the
`Ctrl+Space` palette) to browse and edit them in a modal overlay.

### Web

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `web_fetch` | `ReadOnly` | Fetch a URL (up to 5 redirects, 30-second timeout), converting HTML to Markdown capped at 128,000 characters. |

### Conditional tools

These tools register or activate based on configuration, model mode, and client
capabilities:

| Tool | Sensitivity | Condition | Description |
|------|-------------|-----------|-------------|
| `python` | `ReadOnly` (`isolated = true`) or `Dangerous` (`isolated = false`) | Python code mode active | Run Python in a persistent worker that calls Mew and MCP tools via `tools.*`. Replaces direct tools when active. See [Python code mode](/docs/getting-started/configuration/#python-code-mode). |
| `skill` | `ReadOnly` | At least one skill loaded (always active via built-in `mew-docs`) | Load a skill's instructions into context. See [Skills](/docs/using-mew/skills/). |
| `edit_hashline` | `Mutating` | Enabled for active provider | Edit files with [hashline patches](/docs/using-mew/hashline/). Disable via `disable_hashline = true`. |
| `browser_snapshot`, `browser_screenshot` | `ReadOnly` | Active native browser tab | Inspect DOM snapshots or capture screenshots of the selected browser tab. |
| `browser_open`, `browser_click`, `browser_fill`, `browser_press`, `browser_close` | `Mutating` | Active native browser tab | Open URLs, interact with elements, send key presses, and close browser tabs. |

See [Desktop App](/docs/using-mew/desktop-app/#use-browser-tabs) for connecting
an in-app browser tab.

## Secret redaction

`read`, `grep`, shell output helpers, MCP tools, and Python code mode redact
configured `[[secrets.words]]` values. `glob` and `grep` omit paths matching
`[[secrets.files]]`. See [Permissions](/docs/using-mew/permissions/) for
configuration details.

## Adding a custom tool

See [Adding a Tool](/docs/development/dev-tools/) to implement and register a
new built-in tool.
