---
title: Daemon Protocol
description: Wire protocol between the daemon and frontends.
---

The daemon communicates with frontends (TUI, web UI) over WebSocket using
JSON-encoded messages. The protocol is defined in `mew-protocol`.

## Message types

### Client → Server (`ClientMessage`)

| Message | Purpose |
|---------|---------|
| `NewSession` | Create a fresh session |
| `AttachSession` | Attach to an existing session by ID |
| `ListSessions` | List all known sessions |
| `Prompt` | Send a user prompt (with optional attachments) |
| `Cancel` | Cancel the current turn |
| `PermissionResponse` | Respond to a permission request |
| `AskUserResponse` | Respond to an ask-user question |
| `SlashCommand` | Run a slash command on the daemon |
| `ListModels` | List available models |
| `SwitchModel` | Switch to a different model |
| `SetThinkingVariant` | Set or clear the thinking variant |

### Server → Client (`ServerMessage`)

| Message | Purpose |
|---------|---------|
| `SessionReady` | Session created/attached, ready for prompts |
| `Error` | Error before or outside a session turn |
| `Provider` | Raw provider streaming event (text, tool calls, etc.) |
| `ToolStart` / `ToolEnd` | Tool execution lifecycle |
| `ToolProgress` | Intermediate tool output while running |
| `PartUpdated` | A part's content or state changed |
| `PermissionRequest` | Request user approval for a tool call |
| `AskUserRequest` | Ask the user free-text questions |
| `SubagentStart` / `SubagentStatus` / `SubagentEnd` | Subagent lifecycle |
| `TodosUpdated` | Session todo list changed |
| `SessionList` | Response to `ListSessions` |
| `SessionHistory` | Full message replay for a resumed session |
| `SessionCleared` | Context cleared (broadcast to all clients) |
| `ModelList` | Response to `ListModels` |
| `ModelSwitched` | Confirms model switch |
| `ThinkingVariantChanged` | Confirms thinking variant change |
| `SessionTitleChanged` | Daemon generated a session title |
| `SlashResult` | Slash command produced text output |
| `RequestResolved` | A pending request was resolved by any client |

## Session model

The daemon owns sessions via `SessionManager`. Connections attach to
sessions. A `Session` broadcasts events to all attached clients via
per-client `mpsc::UnboundedSender<ServerMessage>`.

Only one turn runs at a time per session (serialized via `turn_lock`).
A fresh `CancellationToken` is created per turn so cancelling one turn
does not poison future turns.

Permission and ask-user requests go to all clients. Any client can
respond. `RequestResolved` dismisses the modal on all frontends.

## Adding a new message type

1. Add the variant to `ClientMessage` or `ServerMessage` in `mew-protocol/src/lib.rs`
2. Add a roundtrip test in the protocol test module
3. Handle the new message in `handle_connection` (`mew-daemon/src/lib.rs`)
4. Update `translate_server_message` in `mew-daemon/src/client.rs` if it's
   a `ServerMessage` that the TUI daemon-client needs to handle
5. Add the TypeScript type + dispatch to `mew-web-client/src/index.ts`
6. Wire the store action in `mew-web-ui/src/stores/session.ts`

## Connection lifecycle

```
Client connects → NewSession/AttachSession → SessionReady + SessionHistory
  → Prompt → Provider events stream → MessageEnd
  → (optional) PermissionRequest → PermissionResponse → RequestResolved
  → (optional) more turns
Client disconnects → detach_client → if last client: cancel turn, remove session
```

Idle sessions can be resumed from disk via `Agent::load_messages`. The
session writer persists every message to `~/.local/share/mew/sessions/<id>/session.jsonl`.
