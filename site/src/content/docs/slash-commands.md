---
title: Slash Commands
description: Built-in slash commands available in the TUI.
---

Type any of these in the input box (they autocomplete as you type).

## Session management

| Command | Description |
|---------|-------------|
| `/clear` | Clear all messages from the current session |
| `/compact` | Force context compaction on the next turn |
| `/rewind <n>` | Rewind to keep only the first N messages |
| `/sessions` | List previous sessions |
| `/resume <id>` | Resume a previous session by ID |

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

## Other

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/cost` | Show cost breakdown (tokens, $) |
| `/todo` | Show the session todo list |
| `/mouse` | Toggle mouse capture for text selection |
| `/quit` | Exit mew |
