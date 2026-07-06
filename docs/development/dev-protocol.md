---
title: Daemon Protocol
description: Wire protocol between the daemon and frontends.
---

The daemon communicates with frontends (TUI, web UI) over WebSocket using
JSON-encoded messages. The protocol is defined in `mew-protocol`.

## Wire format

Messages are tagged enums serialized with `serde(tag = "type")`:

```json
{"type": "Prompt", "text": "hello", "attachments": []}
```

```json
{"type": "SessionReady", "session_id": "sess_01J...", "model": "deepseek-v4-flash"}
```

Encoding/decoding via `encode_json` / `decode_json` in `mew-protocol`.

## Client → Server messages

| Message | Fields | Purpose |
|---------|--------|---------|
| `NewSession` | `cwd?: String` | Create a fresh session |
| `AttachSession` | `session_id: String` | Attach to an existing session |
| `ListSessions` | | List all known sessions |
| `Prompt` | `text: String`, `attachments: Vec<Attachment>` | Send a user prompt |
| `Cancel` | | Cancel the current turn |
| `PermissionResponse` | `request_id: u64`, `decision: PermissionDecision` | Respond to a permission prompt |
| `AskUserResponse` | `request_id: u64`, `answers: Vec<String>` | Respond to an ask-user question |
| `SlashCommand` | `command: String` | Run `/clear`, `/compact`, etc. on the daemon |
| `ListModels` | | Request the available model list |
| `SwitchModel` | `provider: String`, `model: String` | Switch to a different model |
| `SetThinkingVariant` | `variant: String` | Set or clear thinking variant ("off" or "none" disables) |
| `Ping` | | Liveness check; daemon replies with `Pong` |
| `ListProjects` | | List known project directories (for project picker UI) |

## Server → Client messages

### Session lifecycle

| Message | Fields | Purpose |
|---------|--------|---------|
| `SessionReady` | `session_id`, `model?`, `provider?` | Session ready for prompts |
| `SessionHistory` | `messages: Vec<Message>` | Full message replay on resume |
| `SessionList` | `sessions: Vec<SessionInfo>` | Response to `ListSessions` |
| `SessionCleared` | | Context cleared (broadcast to all clients) |
| `SessionTitleChanged` | `session_id`, `title` | Daemon generated a title |

### Streaming events

| Message | Fields | Purpose |
|---------|--------|---------|
| `Provider` | `event: ProviderEventWire` | Raw provider event (PartStart, PartDelta, etc.) |
| `ToolStart` | `call_id` | Tool execution started |
| `ToolEnd` | `call_id`, `success` | Tool execution finished |
| `ToolProgress` | `call_id`, `chunk` | Intermediate tool output |
| `PartUpdated` | `part_id`, `part` | A part's content or state changed |

### Request/response pairs

| Message | Fields | Purpose |
|---------|--------|---------|
| `PermissionRequest` | `request_id`, `tool_name`, `input` | Ask user to approve a tool call |
| `AskUserRequest` | `request_id`, `call_id`, `questions` | Ask user free-text questions |
| `RequestResolved` | `request_id` | A pending request was resolved by any client |

### Subagent events

| Message | Fields | Purpose |
|---------|--------|---------|
| `SubagentStart` | `parent_call_id`, `name`, `child_session_id`, `display_name?` | Subagent spawned |
| `SubagentStatus` | `parent_call_id`, `tool_name`, `message` | Subagent progress update |
| `SubagentEnd` | `parent_call_id`, `child_session_id`, `outcome` | Subagent finished |

### Other

| Message | Fields | Purpose |
|---------|--------|---------|
| `ModelList` | `models: Vec<ModelInfo>` | Response to `ListModels` |
| `ModelSwitched` | `provider`, `model` | Confirms model switch |
| `ThinkingVariantChanged` | `variant?: String` | Confirms thinking variant (null = disabled) |
| `TodosUpdated` | `todos: Vec<Todo>` | Session todo list changed |
| `PersonaSwitchRequested` | `name` | `switch_persona` tool was called |
| `JobUpdate` | `job_id`, `command`, `state` | Background shell job changed |
| `SlashResult` | `text` | Slash command produced text output |
| `Error` | `message` | Error before or outside a turn |
| `ErrorEvent` | `message` | Terminal error during a turn |
| `Pong` | `version: String` | Response to `Ping`; carries daemon version |
| `ProjectList` | `projects: Vec<ProjectInfo>` | Response to `ListProjects`; deduped project directories with session counts |

## Session model

The daemon owns sessions via `SessionManager`. Connections attach to
sessions. Key fields on `Session`:

```rust
pub struct Session {
    pub id: String,
    pub agent: Mutex<Agent>,
    pub turn_lock: Mutex<()>,           // serializes turns
    pub clients: Mutex<Vec<(u64, Sender<ServerMessage>)>>,  // broadcast targets
    pub pending_permissions: Mutex<HashMap<u64, oneshot::Sender<PermissionDecision>>>,
    pub pending_ask_user: Mutex<HashMap<u64, oneshot::Sender<Vec<String>>>>,
    pub current_turn_cancel: Mutex<Option<CancellationToken>>,
    pub model: Mutex<Option<String>>,
    pub provider: Mutex<Option<String>>,
}
```

- **Broadcasting**: `session.broadcast(msg)` sends to all attached clients
  and removes any that have disconnected.
- **Turn serialization**: `turn_lock` ensures only one turn runs at a time.
  A second `Prompt` while a turn is in progress receives an error.
- **Cancellation**: each turn gets a fresh `CancellationToken`. `Cancel`
  from any client cancels the current turn.
- **Permission/ask-user requests**: go to all clients. Any client can
  respond. `RequestResolved` dismisses the modal everywhere.

## Connection lifecycle

```
Client connects
  → NewSession or AttachSession
  → SessionReady + SessionHistory
  → Prompt
  → Provider events stream (PartStart → PartDelta → ... → MessageEnd)
  → (optional) PermissionRequest → PermissionResponse → RequestResolved
  → (optional) ToolStart → ToolProgress → ToolEnd
  → (optional) more turns
Client disconnects
  → detach_client
  → if last client: cancel turn, remove session from active
```

Idle sessions can be resumed from disk. The session writer persists every
message to `~/.local/share/mew/sessions/<id>/session.jsonl`. On resume,
`Agent::load_messages` replays the history.

## AgentEvent → ServerMessage translation

`translate_event` (`mew-daemon/src/lib.rs`) converts each `AgentEvent` into
zero or more `ServerMessage`s:

- `AgentEvent::Provider(pe)` → `ServerMessage::Provider { event: ProviderEventWire::from(pe) }`
- `AgentEvent::PermissionRequest { call, tx }` → assigns a `request_id`,
  stashes the `oneshot::Sender` in `session.pending_permissions`, emits
  `ServerMessage::PermissionRequest`
- `AgentEvent::AskUser { questions, tx }` → same pattern with `pending_ask_user`

## ServerMessage → AgentEvent translation

For the TUI daemon-client (`mew-daemon/src/client.rs`), `translate_server_message`
reverses the translation. Channel-bearing messages reconstruct `oneshot`
channels and return `AgentEvent::PermissionRequest { call, tx }` with a
fresh `oneshot::Sender` that the client maps to `ClientMessage::PermissionResponse`.

Messages that don't map to `AgentEvent` (model list, session list, etc.)
return `Vec::new()` and are handled by the `DaemonClient` directly.

## Adding a new message type

1. Add the variant to `ClientMessage` or `ServerMessage` in `mew-protocol/src/lib.rs`
2. Add a roundtrip test in the protocol test module
3. Handle the new message in `handle_connection` (`mew-daemon/src/lib.rs`)
4. Update `translate_server_message` in `mew-daemon/src/client.rs` if it's
   a `ServerMessage` the TUI daemon-client needs to handle
5. Add the TypeScript type + dispatch to `mew-web-client/src/index.ts`
6. Wire the store action in `mew-web-ui/src/stores/session.ts`
