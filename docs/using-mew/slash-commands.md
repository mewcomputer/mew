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
| `/sessions` | Open conversations ordered by latest activity (`/session` also works) |
| `/resume <id>` | Resume a previous session by ID. Omit the ID to open the picker. |
| `/project` | Pick a project directory and open a new conversation there |
| `/rename <title>` | Rename the active conversation |
| `/autotitle on` / `/autotitle off` | Turn automatic titles on or off for the active conversation |
| `/autosummary on` / `/autosummary off` | Turn automatic summaries on or off for the active conversation |
| `/yield` | Let another connected client take control |

The daemon-backed TUI does not support `/rewind`. See
[Sessions](/docs/getting-started/sessions/) for persistence and resume.

## Model & reasoning

| Command | Description |
|---------|-------------|
| `/model <name>` | Switch to a different model |
| `/model` | Open the model picker (`/models` also works) |
| `/thinking <variant>` | Set thinking variant (`high`, `max`, `thinking`, `off`) |
| `/thinking` | Open the available thinking variants |

## Personas

| Command | Description |
|---------|-------------|
| `/persona <name>` | Switch to a named persona (opens confirm modal) |
| `/persona` | Open the persona picker |
| `/persona default` | Clear the active persona |

## Permissions

| Command | Description |
|---------|-------------|
| `/permissions` | Open the permission mode picker (`/permission` also works) |
| `/permissions <mode>` | Switch directly: `standard`, `permissive`, `auto`, `auto_plus`, `dangerous` |

See [Permissions](/docs/using-mew/permissions/) for mode details, rules, and sandboxing.

## Goals

| Command | Description |
|---------|-------------|
| `/goal <objective>` | Open the goal editor and confirm an objective for the agent to work toward |
| `/goal` | Show the active goal and its status |
| `/goal pause` / `/goal resume` | Pause or resume automatic continuation toward the goal |
| `/goal complete` | Mark the active goal complete |
| `/goal clear` | Remove the active goal |

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
| `/theme <name>` | Switch themes. Omit the name to open the theme picker. |
| `/unflag <path>` | Stop keeping a file marked as important for compaction |
| `/mouse` `/m` | Toggle mouse capture for text selection |
| `/quit` `/q` | Exit mew |

The bell and `Ctrl+Alt+N` open this terminal window's conversations with pending
questions, permissions, failures or unread results. `Ctrl+Alt+O` lists all views
open here, and `Tab` opens global history from either list. Each conversation
keeps its draft, attachments and queued
follow-ups when you switch. Submitted input stays visible until the daemon
acknowledges it; an uncertain submission is never replayed automatically.

Conversation icons show state: `○` idle or complete, `●` unread update, `?`
question, `!` approval, `×` a failed turn in an open conversation, and `▶` working.
Each title has a second row with its
project path on the left and pending action counts and time on the right.
Paths inside your home directory use `~/`; search accepts either form. Selected
details show failures and any clipped context, with `PgUp`/`PgDn` to scroll.
Use `/cost` for cost details.
