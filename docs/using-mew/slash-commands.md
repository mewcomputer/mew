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
| `/sessions` | List previous sessions (up to 20) |
| `/resume <id>` | Resume a previous session by ID |

See [Sessions](/docs/getting-started/sessions/) for details on persistence, resume, and rewind.

## Model & reasoning

| Command | Description |
|---------|-------------|
| `/model <name>` | Switch to a different model |
| `/model` | Open the model picker |
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
| `/permissions` | Open the permission mode picker |
| `/permissions <mode>` | Switch directly: `standard`, `permissive`, `auto`, `auto_plus`, `dangerous` |

See [Permissions](/docs/using-mew/permissions/) for mode details, rules, and sandboxing.

## Other

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/cost` | Show cost breakdown (tokens, $) |
| `/todo` | Show the session todo list |
| `/mouse` `/m` | Toggle mouse capture for text selection |
| `/quit` `/q` | Exit mew |

## Slash commands vs tools

Slash commands are shortcuts you type to steer mew. Tools are called by
the model during a turn. If you want to trigger something yourself, it is
a slash command; if the model should do it autonomously, it is a tool.
See [Comparing Features](/docs/using-mew/comparisons/) for the full
breakdown.
