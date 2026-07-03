---
title: Sessions
description: Session persistence, history, resume, and rewind.
---

Every mew conversation is persisted to disk as it happens. You can list
past sessions, resume one, rewind to an earlier point, or clear the
current context.

## Where sessions live

Sessions are stored as JSONL files under a per-session directory:

```
~/.config/mew/sessions/<session-id>/
  ├── session.jsonl    # one JSON message per line
  ├── meta.json        # session metadata
  └── todos.json       # session todo list (if any)
```

On macOS the path is `~/Library/Application Support/computer.mew.mew/sessions/`.
You can override the location with the `MEW_SESSION_DIR` environment variable.

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

Shows up to 20 recent sessions, sorted by last modified, with file size:

```
sessions:
  01JXK4M3PQ7B9...  (14284 bytes)
  01JXJ2F8NK6Q1...  (8921 bytes)
  ...
```

## Resuming a session

```
/resume <session-id>
```

Loads the full message history from disk, replaces the current conversation,
and restores the session's todo list. The session ID is the folder name
shown by `/sessions`.

Resume reconstructs the entire conversation from the JSONL log, clear
markers included as ordinary messages. `/clear` only resets the in-memory
context for the running session; the log is append-only and resume
replays it in full.

## Rewinding

```
/rewind <n>
```

Truncates the conversation to keep only the first `n` messages. Both the
display store and the agent's API history are truncated. A synthetic
message confirms how many messages were removed.

Without an argument:

```
/rewind
```

Lists the last 15 messages with their indices and role, so you can pick
the number to keep.

Rewind doesn't modify the session file on disk. The JSONL log is
append-only. Rewind only changes what the model sees on the next turn.

You cannot rewind while streaming.

## Clearing the context

```
/clear
```

Clears the visible conversation and resets the agent's in-memory message
history. A synthetic clear marker is appended to the session log as an
audit record; the log itself is append-only and resume loads it in full.

Permission caches survive `/clear`. If you approved a tool for the
session with "Allow session", that approval persists. This is
intentional: a session is the JSONL log, the context is what the model
sees this turn, and clearing the latter doesn't invalidate your prior
grants.

## Compaction

```
/compact
```

Forces context compaction on the next turn. When the conversation
approaches the model's context window, mew summarizes earlier messages
to free space. `/compact` triggers this immediately rather than waiting
for the threshold.

Compaction runs automatically when needed. The command is for when you
want to control the timing.

## Session IDs

Session IDs are ULIDs (Universally Unique Lexicographically Sortable
Identifiers). They sort chronologically by creation time, which is why
`/sessions` can list them newest-first without reading metadata.

## Daemon sessions

When running the daemon (`mew daemon`), sessions are managed by the
`SessionManager`. Multiple clients can attach to the same session. See
[Daemon Protocol](/docs/development/dev-protocol/) for the wire-level details.
