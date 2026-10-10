---
title: Slash Commands
description: Built-in slash commands available in the TUI.
---

Type any of these in the input box (they autocomplete as you type).

## Session management

| Command | Description |
|---------|-------------|
| `/clear` | Clear the current context (persists to disk) |
| `/compact` | Force context compaction on the next turn |
| `/rewind <n>` | Rewind to keep only the first N messages |
| `/rewind` | List recent messages with indices |
| `/sessions` | Open conversations ordered by latest activity (`/session` also works) |
| `/resume <id>` | Resume a previous session by ID |

See [Sessions](/docs/getting-started/sessions/) for details on persistence, resume, and rewind.

## Model & reasoning

| Command | Description |
|---------|-------------|
| `/model <name>` | Switch to a different model |
| `/model` | Open the model picker (`/models` also works) |
| `/thinking <variant>` | Set thinking variant (`high`, `max`, `thinking`, `off`) |
| `/thinking` | Show usage help |

## Personas

| Command | Description |
|---------|-------------|
| `/persona <name>` | Switch to a named persona (opens confirm modal) |
| `/persona` | List available personas |
| `/persona default` | Clear the active persona |

## Permissions

| Command | Description |
|---------|-------------|
| `/permissions` | Open the permission mode picker (`/permission` also works) |
| `/permissions <mode>` | Switch directly: `standard`, `permissive`, `auto`, `auto_plus`, `dangerous` |

See [Permissions](/docs/using-mew/permissions/) for mode details, rules, and sandboxing.

## Other

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/cost` | Show cost breakdown (tokens, $) |
| `/todo` | Browse and edit the session todo list |
| `/agents` | Open a subagent's recorded conversation and tool trace |
| `/jobs` | Open a background shell job and read its captured output |
| `/tools` | Browse tool calls with full arguments, results and errors |
| `/review` `/diff` | Review changes in this conversation's workspace; press `r` to refresh |
| `/mouse` `/m` | Toggle mouse capture for text selection |
| `/quit` `/q` | Exit mew |

The sidebar bell and `Ctrl+Alt+N` open conversations with pending questions,
permissions or unread outcomes. `Tab` switches between attention and all
conversations. Each conversation keeps its draft, attachments and queued
follow-ups when you switch. Submitted input stays visible until the daemon
acknowledges it; an uncertain submission is never replayed automatically.

Conversation icons show state: `○` idle or complete, `●` unread update, `?`
question, `!` approval, and `▶` working. Each title has a second row with its
project path on the left and pending action counts and time on the right.
Paths inside your home directory use `~/`; search accepts either form. Selected
details show failures and any clipped context, with `PgUp`/`PgDn` to scroll.
Use `/cost` for cost details.
