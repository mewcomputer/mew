---
title: Sessions
description: Find and resume conversations, clear context, and compact history.
---

Mew saves conversations to disk as you work. You can reopen a past
conversation, archive it, or clear the context used by the model.

## Where sessions live

Sessions are stored as JSONL files under a per-session directory:

```
~/.local/share/mew/sessions/<session-id>/
  ├── session.jsonl    # one JSON message per line
  ├── meta.json        # session metadata
  └── todos.json       # session todo list (if any)
```

On macOS and Linux, Mew uses `~/.local/share/mew/sessions/` by default.
With `XDG_DATA_HOME` set, it uses `$XDG_DATA_HOME/mew/sessions/`.
Setting `MEW_DATA_DIR` changes the parent data directory. To override
the session directory directly, set `MEW_SESSION_DIR` before launching
Mew. For a remote session, the daemon stores these files on its host.

Subagent sessions nest under their parent:

```
sessions/<parent-id>/subagents/<child-id>/
```

The `/sessions` command lists top-level sessions only. Subagent sessions
are hidden from the list.

## Session metadata

`meta.json` records:

| Field | Description |
|-------|-------------|
| `id` | Session identifier (ULID) |
| `model` | Model used for this session |
| `parent_session_id` | Set for subagent sessions |
| `children_session_ids` | Subagent sessions spawned under this one |
| `depth` | Nesting level (0 for top-level) |
| `subagent_name` | Subagent type name (if applicable) |
| `created_at` | Unix timestamp in milliseconds |

## Listing sessions

```
/sessions
```

Opens the terminal conversation picker. Type to filter conversations
and press `Enter` to open one.

- `Tab` switches between current and archived conversations.
- `Ctrl+Alt+A` archives or restores the selected conversation.
- `Ctrl+Alt+P` toggles the selected conversation's pin.

## Resuming a session

```
/resume <session-id>
```

Attaches to a conversation by ID and restores its history and todos.
The session ID is the directory name under the session storage root.
Run `/resume` without an ID to open the conversation picker.

## Rewinding

The terminal client uses daemon sessions, which currently lack rewind
support.

## Clearing the context

```
/clear
```

Resets the model's conversational context. Mew writes a clear marker
to the session log and keeps earlier messages on disk. When you
resume, the model uses the context after the marker.

Tool approvals granted with "Allow session" remain active.

## Compaction

```
/compact
```

Schedules compaction for the next turn. Mew can also compact
automatically when a conversation approaches the model's context
limit, summarizing earlier messages to free space.

## Session IDs

Session IDs are ULIDs (Universally Unique Lexicographically Sortable
Identifiers). They sort chronologically by creation time.

## Daemon sessions

The daemon manages sessions, and multiple clients can attach to the
same one. See
[Daemon Protocol](/docs/development/dev-protocol/) for the wire-level details.
