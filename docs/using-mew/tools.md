---
title: Tools
description: Built-in tools the agent uses to read, write, and run things.
---

Tools are how the agent interacts with your system. Each tool has a name,
a sensitivity level, and a set of parameters. The model decides which
tool to call based on the tool's description and the current task.

## How tool calls work

1. The model emits a tool call with a name and input arguments
2. mew checks permissions (see [Permissions](/docs/using-mew/permissions/))
3. If approved, the tool runs and produces output
4. The output goes back to the model as context for the next turn

In the TUI, tool calls appear as cards inline in the conversation. Each
card shows the tool name, its input, and the output. Bash output can be
expanded or collapsed with `Ctrl+O`. Diffs (from Write and Edit) get
syntax-colored `+`/`-` highlighting.

## Built-in tools

### File operations

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `read` | ReadOnly | Read file contents. Returns a `[path#hash]` header and numbered lines for hashline edits. Supports offset and limit for pagination. Detects binary files and refuses to read them. |
| `write` | Mutating | Write content to a file. Creates the file if it doesn't exist, overwrites if it does. |
| `edit` | Mutating | Replace a unique string in a file. Fails if the old string appears more than once. Includes first/last line snippets in error messages to help recovery. |
| `edit_hashline` | Mutating | Edit files using line-anchored hashline patches with file-hash staleness detection. Supports block-aware ops and stale-tag recovery. |
| `glob` | ReadOnly | Find files matching a glob pattern. Sorted by modification time. Honors `.gitignore` by default. |
| `grep` | ReadOnly | Search file contents for a regex pattern. Prefers ripgrep if available. Supports include filters and context lines. |

See [Hashline Edits](/docs/using-mew/hashline/) for the patch format and examples.

### Shell

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `bash` | Dangerous | Execute a shell command. Output is captured and returned. Secret redaction applies to output. |
| `shell_background` | Dangerous | Launch a shell command in the background and return a job ID immediately. |
| `shell_monitor` | Dangerous | Run a command and wait for it to exit successfully, retrying until a timeout. For readiness probes. |
| `job_status` | ReadOnly | Check the status of a background job (shell or subagent). |
| `job_block` | Mutating | Wait for a background job to reach a terminal state. |
| `job_cancel` | Dangerous | Cancel a running background job by killing its process. |

### Session and agent

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `echo` | ReadOnly | Echoes back the provided input. Used for testing and diagnostics. |
| `exit_tool` | ReadOnly | Stop the current subagent run and return a final answer to the parent. Top-level use ends the session. |
| `flag_important` | ReadOnly | Mark a file as important so it survives context compaction. Flagged files are kept in context even when older messages are summarized. |
| `progress_update` | ReadOnly | Report a status update to the parent agent. Used by subagents mid-run to show what they're doing in the sidebar. |
| `ask_user_question` | ReadOnly | Ask the user 1-4 multiple-choice questions when their answer would change how to proceed. Renders an interactive question card. |

### Subagents

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `subagent_start` | ReadOnly | Spawn a child agent with a fresh conversation context, tool allowlist, and system prompt. Returns a job ID. |
| `subagent_wait` | ReadOnly | Wait for a background subagent to complete and get its result. |

See [Subagents](/docs/using-mew/subagents/) for how subagents work and how to
define custom ones.

### Todos

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `todo_create` | ReadOnly | Create one or more todos. Each todo tracks a step of the work. |
| `todo_update` | ReadOnly | Update a todo's content and/or status (`pending`, `in_progress`, `done`, `blocked`). |
| `todo_complete` | ReadOnly | Mark a todo done. Refused if dependencies aren't done. |
| `todo_delete` | ReadOnly | Delete a todo. Refused if another todo depends on it. |
| `todo_list` | ReadOnly | List all todos with their IDs, statuses, and dependencies. |

The todo list renders in a panel below the chat. Press `Ctrl+P` and
select "Todo" to toggle its visibility, or use `/todo` to print it as
text.

### Web

| Tool | Sensitivity | Description |
|------|-------------|-------------|
| `web_fetch` | ReadOnly | Fetch a URL and return its content as markdown. HTML pages are converted to readable markdown. Content is truncated at 128KB. |

`web_fetch` is useful for reading documentation pages, API references,
and articles without leaving the conversation. It follows up to 5
redirects and sets a 30-second timeout.

### Conditional tools

These tools only register when their prerequisites are met:

| Tool | Condition | Description |
|------|-----------|-------------|
| `skill` | At least one skill discovered | Load a skill's full instructions into context. See [Skills](/docs/using-mew/skills/). |
| `switch_persona` | At least one persona defined | Queue a persona switch for the end of the current turn. See [Personas](/docs/using-mew/personas/). |
| `mcp__*` | MCP servers configured | Tools from connected MCP servers. See [MCP Servers](/docs/using-mew/mcp-servers/). |

## Secret redaction

All tools that return file content or command output run the result
through `SecretSet::redact()` before returning. This catches:

- Pattern-based secrets (API keys, tokens, passwords matching common formats)
- File-based secrets (contents of files listed in `secrets.files`)
- Word-based secrets (specific values listed in `secrets.words`)

See [Permissions](/docs/using-mew/permissions/) for how to configure secret
redaction.

## Adding a custom tool

See [Adding a Tool](/docs/development/dev-tools/) for the developer guide on
implementing and registering new tools.
