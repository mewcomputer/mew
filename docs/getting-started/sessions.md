---
title: Sessions
description: Find and resume conversations, clear context, and compact history.
---

Mew saves conversations to disk as you work so you can reopen a past session,
archive it, or manage its context.

## Where sessions live

Mew writes each session as JSON files under its own ULID directory:

```text
~/.local/share/mew/sessions/<session-id>/
  ├── session.jsonl    # one JSON event/message per line
  ├── meta.json        # session metadata
  ├── policy.json      # persisted model, persona, thinking, and permission policy
  └── todos.json       # session todo list (when present)
```

On macOS and Linux, Mew defaults to `~/.local/share/mew/sessions/`. Set
`XDG_DATA_HOME` to move it to `$XDG_DATA_HOME/mew/sessions/`, `MEW_DATA_DIR` to
change the parent data directory, or `MEW_SESSION_DIR` to override the session
directory directly. Remote daemons store sessions on the daemon host.

Subagent sessions nest beneath their parent session:

```text
sessions/<parent-id>/subagents/<child-id>/
```

Session listings show top-level conversations and omit subagent sessions.

## Session metadata

`meta.json` records:

| Field | Description |
|-------|-------------|
| `id` | Session ULID (sorts chronologically by creation time) |
| `created_at` | Creation timestamp in Unix milliseconds |
| `last_message_at` | Timestamp of the latest message in Unix milliseconds |
| `custom_title` | User-assigned title |
| `summary` | Auto-generated conversation summary |
| `cwd` | Session working directory |
| `model` | Last model used in the session |
| `parent_session_id` | Parent ULID for subagent sessions |
| `children_session_ids` | Spawned subagent session IDs |
| `depth` | Nesting depth (`0` for top-level sessions) |
| `subagent_name` | Subagent definition name (when applicable) |
| `last_turn_failed` | Whether the latest turn ended with an error |
| `archived` | Whether the session is archived |
| `group_id` | Sidebar group identifier |
| `pinned` | Exempts the session from auto-archiving |
| `change_stats` | Cumulative diff line counts |
| `usage` | Cumulative token counts, cost, and turn count |
| `context_tokens` | Latest request prompt token count |

## Listing sessions

```text
/sessions
```

Opens the terminal conversation picker. Type to filter by title or metadata,
then press `Enter` to attach.

- `Tab`: switch between active and archived conversations.
- `Ctrl+Alt+A`: archive or restore the selected conversation.
- `Ctrl+Alt+P`: pin or unpin the selected conversation.

## Resuming a session

```text
/resume <session-id>
```

Attaches to `<session-id>` and restores its history, todos, working directory,
and `policy.json` settings (provider, model, persona, thinking variant, and
permission mode). Run `/resume` with no ID to open the conversation picker.
Daemon-backed clients do not support rewinding a session to an earlier turn.

Multiple clients can attach to the same daemon session at once. See
[Daemon Protocol](/docs/development/dev-protocol/) and
[Session handover](/docs/development/handover/) for details.

## Clearing the context

```text
/clear
```

Appends a clear marker to `session.jsonl` while preserving earlier lines on disk.
Later turns and resumed sessions start from messages after the marker. Tool
approvals granted with "Allow session" remain active.

## Compaction

```text
/compact
```

Compacts earlier messages into a summary to free context space. Mew also runs
compaction automatically when a conversation approaches the model's context
limit.
