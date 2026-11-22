---
title: Slash Commands
description: Built-in slash commands available in the TUI.
---

Type slash commands in the composer; autocomplete filters matching commands as
you type. Loaded plugins can also register custom slash commands when the hook
runtime is enabled.

## Session management

| Command | Description |
|---------|-------------|
| `/clear` | Clear conversation context while keeping earlier lines in `session.jsonl` |
| `/compact` | Compact earlier messages immediately (waits for an active turn) |
| `/sessions`, `/session` | Open the session history picker ordered by latest activity |
| `/resume <id>` | Resume `<id>`, or open the session picker when no ID is given |
| `/project` | Pick a project directory and start a new conversation there |
| `/rename <title>` | Rename the active conversation |
| `/autotitle on`, `/autotitle off` | Toggle automatic session titles for the active conversation |
| `/autosummary on`, `/autosummary off` | Toggle automatic session summaries for the active conversation |
| `/yield` | Yield control so another attached client can take over |

Daemon-backed TUI sessions do not support `/rewind`. See
[Sessions](/docs/getting-started/sessions/) for storage and resume details.

## Model and reasoning

| Command | Description |
|---------|-------------|
| `/model <name>` | Switch directly to a model |
| `/model`, `/models` | Open the model picker |
| `/thinking <variant>` | Set a thinking variant (`low`, `medium`, `high`, `xhigh`, `max`, `thinking`, `off`, `none`, or `budget:<n>`) |
| `/thinking` | Open the thinking-variant picker |

## Personas

| Command | Description |
|---------|-------------|
| `/persona <name>` | Preview and confirm switching to `<name>` |
| `/persona` | Open the persona picker |
| `/persona default`, `/persona none` | Clear the active persona immediately |

## Permissions

| Command | Description |
|---------|-------------|
| `/permissions`, `/permission` | Open the permission mode picker |
| `/permissions <mode>` | Switch directly (`standard`, `permissive`, `auto`, `auto_plus`, or `dangerous`) |

See [Permissions](/docs/using-mew/permissions/) for mode behavior, rules, and
workspace checks.

## Goals

| Command | Description |
|---------|-------------|
| `/goal <objective>` | Open the goal editor and confirm a new objective |
| `/goal` | Inspect the active goal and its status |
| `/goal pause`, `/goal resume` | Pause or resume automatic continuation toward the goal |
| `/goal complete` | Mark the active goal complete |
| `/goal clear` | Clear the active goal |

## Other

| Command | Description |
|---------|-------------|
| `/help` | Open the keyboard shortcuts and help overlay |
| `/cost` | Show session token and cost breakdown |
| `/todo` | Browse and edit session todos |
| `/agents` | Open a subagent's recorded transcript and tool trace |
| `/jobs` | Open a background shell job and read its captured output |
| `/tools` | Browse tool calls with arguments, results, and errors |
| `/review`, `/diff` | Inspect workspace changes for this conversation (`r` refreshes) |
| `/theme <name>` | Switch theme, or open the theme picker when omitted |
| `/unflag <path>` | Remove a file's compaction flag |
| `/mouse`, `/m` | Toggle mouse capture for text selection |
| `/quit`, `/q` | Exit Mew |

## Conversation switcher and drafts

Press `Ctrl+Alt+N` (or click the bell) to list conversations open in this
terminal window that have pending questions, approvals, failures, or unread
results. Press `Ctrl+Alt+O` to list all views open in this terminal window, and
press `Tab` in either picker to open global session history.

Each conversation preserves its own draft, attachments, and queued follow-ups
when you switch views. Sent input remains pending in the UI until the daemon
acknowledges it, and uncertain submissions are left unretried after reconnect.

## Picker status icons

| Icon | Meaning |
|------|---------|
| `○` | Idle or complete |
| `●` | Unread update |
| `?` | Pending question |
| `!` | Pending permission approval |
| `×` | Failed latest turn in an open conversation |
| `▶` | Turn in progress |

Each conversation row shows its title on the first line and its project path
(with `~/` substitution, searchable both ways), pending counts, and timestamp on
the second. The details pane shows failure notes and clipped context (`PgUp` and
`PgDn` scroll it). Run `/cost` for full cost details.
