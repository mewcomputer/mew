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
| `SubmitPrompt` | `session_id`, `request_id`, `text`, `attachments` | Submit input with an acceptance receipt |
| `ReadPromptReceipt` | `session_id`, `request_id` | Query an uncertain submission after reconnect |
| `ReadJobOutput` | `session_id`, `job_id`, `request_id`, `offset` | Read a byte-offset chunk of captured shell output |
| `Cancel` | | Cancel the current turn |
| `PermissionResponse` | `request_id: String`, `decision: PermissionDecision` | Respond to a permission prompt |
| `AskUserResponse` | `request_id: String`, `answers: Vec<String>`, `cancelled: bool` | Answer or explicitly close a question |
| `SlashCommand` | `command: String` | Run `/clear`, `/compact`, etc. on the daemon |
| `ListModels` | | Request the available model list |
| `SwitchModel` | `provider: String`, `model: String` | Switch to a different model |
| `SetThinkingVariant` | `variant: String` | Set or clear thinking variant ("off" or "none" disables) |
| `Ping` | | Liveness check; daemon replies with `Pong` |
| `ListProjects` | | List known project directories (for project picker UI) |
| `RegenerateTitle` | `session_id: String` | Regenerate the session title from the first user message via LLM |

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
| `JobOutput` | `session_id`, `job_id`, `request_id`, `offset`, `next_offset`, `output`, `done`, `incomplete` | UTF-8 output chunk for a session-owned job, at most 64 KiB |
| `JobOutputError` | `session_id`, `job_id`, `request_id`, `message` | Scoped job output read failure |

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
| `JobUpdate` | `job_id`, `command`, `state` | Background shell job changed |
| `SlashResult` | `text` | Slash command produced text output |
| `Error` | `message` | Error before or outside a turn |
| `ErrorEvent` | `message` | Terminal error during a turn |
| `Pong` | `version: String`, `protocol_version: u32` | Response to `Ping`; carries daemon version and wire revision |
| `PromptReceipt` | `session_id`, `request_id`, `status`, `message?` | Confirms acceptance, rejection, or unknown status of a scoped submission |
| `ProjectList` | `projects: Vec<ProjectInfo>` | Response to `ListProjects`; deduped project directories with session counts |

## Session model

Native clients require wire revision 3. `SubmitPrompt` carries `session_id`,
`request_id`, `text` and attachments. Acceptance reserves the session's next turn;
duplicate request IDs return the original receipt without starting another turn.
`ReadPromptReceipt` queries the original session after reconnect, including when
another conversation is attached. Receipts live with the daemon session runtime.
An unknown receipt requires inspecting the conversation before a manual retry;
clients must not replay an uncertain submission automatically. Legacy `Prompt`
remains available for clients that do not use acknowledged submissions.

`ReadJobOutput` carries the confirmed `session_id`, `job_id`, `request_id` and
byte `offset`. Replies retain those identities and return `next_offset` at a
UTF-8 boundary. Clients append only a matching response at their current offset.
`done` means no more captured output remains after this chunk. `incomplete`
indicates that a reader failed or did not reach EOF within the bounded reader
drain after process exit; it does not indicate a failed command by itself.

`AskUserRequest.subagent` identifies a child question when present.
`AskUserResponse.cancelled` closes that question's original reply channel and
defaults to false for older clients. Cancelling a child question leaves unrelated
parent actions pending. Cancelling its task closes its matching pending questions.

The daemon owns sessions via `SessionManager` (`mew-daemon/src/session.rs`).
Connections attach to sessions. Key behaviors:

- **Broadcasting**: `session.broadcast(msg)` sends to all attached clients
  and removes any that have disconnected.
- **Turn serialization**: `turn_lock` ensures only one turn runs at a time.
  A second `Prompt` while a turn is in progress receives an error.
- **Cancellation**: each turn gets a fresh `CancellationToken`. `Cancel`
  from any client cancels the current turn.
- **Permission/ask-user requests**: go to all clients. Any client can
  respond. `RequestResolved` dismisses the modal everywhere.

See `mew-daemon/src/session.rs` for the full `Session` struct definition.

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
