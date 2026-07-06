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

### Session lifecycle

| Message | Fields | Purpose |
|---------|--------|---------|
| `NewSession` | `cwd?: String`, `client_kind: ClientKind` | Create a fresh session |
| `AttachSession` | `session_id: String`, `client_kind: ClientKind` | Attach to an existing session |
| `ListSessions` | | List all known sessions |
| `DeleteSession` | `session_id: String` | Delete a session from disk and memory |
| `RenameSession` | `session_id: String`, `title: String` | Set a custom title |
| `SetAutoTitle` | `enabled: bool` | Enable/disable auto-generated titles |
| `SetAutoSummary` | `enabled: bool` | Enable/disable idle session summaries |

### Turn interaction

| Message | Fields | Purpose |
|---------|--------|---------|
| `Prompt` | `text: String`, `attachments: Vec<Attachment>` | Send a user prompt |
| `Cancel` | | Cancel the current turn |
| `PermissionResponse` | `request_id: u64`, `decision: PermissionDecision` | Respond to a permission prompt |
| `AskUserResponse` | `request_id: u64`, `answers: Vec<String>` | Respond to an ask-user question |
| `SlashCommand` | `command: String` | Run `/clear`, `/compact`, etc. on the daemon |
| `YieldControl` | | Advisory yield — other clients can become active |

### Model & mode

| Message | Fields | Purpose |
|---------|--------|---------|
| `ListModels` | | Request the available model list |
| `SwitchModel` | `provider: String`, `model: String` | Switch to a different model |
| `SetThinkingVariant` | `variant: String` | Set or clear thinking variant ("off" or "none" disables) |
| `SetPermissionMode` | `mode: String` | Set permission mode (standard, permissive, auto, auto_plus, dangerous) |

### Groups & organization

| Message | Fields | Purpose |
|---------|--------|---------|
| `CreateGroup` | `name`, `color?` | Create a session group |
| `UpdateGroup` | `group_id`, `name?`, `color?`, `order?` | Rename or recolor a group |
| `DeleteGroup` | `group_id` | Delete a group |
| `AssignSessionGroup` | `session_id`, `group_id?`, `position?` | Assign/reorder a session in a group |
| `ArchiveSession` | `session_id`, `archived: bool` | Archive or unarchive a session |
| `PinSession` | `session_id`, `pinned: bool` | Pin or unpin a session |

### Projects & file service

| Message | Fields | Purpose |
|---------|--------|---------|
| `ListProjects` | | List known projects (recent cwds + `workspace.roots`) |
| `ListDir` | `session_id`, `path?: String` | List directory contents |
| `ReadFilePreview` | `session_id`, `path`, `max_bytes?` | Read a file preview |
| `GitStatus` | `session_id` | Get git status for the session cwd |
| `WatchWorkspace` | `session_id`, `enabled: bool` | Toggle filesystem change notifications |
| `OpenPath` | `session_id`, `path` | Open a path (platform-specific) |
| `UnflagFile` | `session_id`, `path` | Remove a file from the flagged-files set |

### Other

| Message | Fields | Purpose |
|---------|--------|---------|
| `Ping` | | Liveness check + version negotiation |

## Server → Client messages

### Session lifecycle

| Message | Fields | Purpose |
|---------|--------|---------|
| `SessionReady` | `session_id`, `model?`, `provider?`, `permission_mode?` | Session ready for prompts |
| `SessionHistory` | `messages: Vec<Message>` | Full message replay on resume |
| `SessionList` | `sessions: Vec<SessionInfo>` | Response to `ListSessions` |
| `SessionCleared` | | Context cleared (broadcast to all clients) |
| `SessionTitleChanged` | `session_id`, `title` | Daemon generated a title |
| `SessionSummaryChanged` | `session_id`, `summary` | Daemon generated a summary |
| `SessionMetaChanged` | `session_id`, `archived?`, `pinned?`, `group_id?` | Archive/pin/group changed |
| `SessionAttentionChanged` | `session_id`, `pending_permissions`, `pending_questions` | Pending count changed |
| `SessionActivityChanged` | `session_id`, `activity: SessionState` | Session activity state changed |
| `SessionStatsChanged` | `session_id`, `added`, `removed`, `files_changed` | Diff stats updated |
| `SessionUsageChanged` | `session_id`, `usage: SessionUsageWire` | Cumulative token/cost updated |

### Streaming events

| Message | Fields | Purpose |
|---------|--------|---------|
| `Provider` | `event: ProviderEventWire` | Raw provider event (PartStart, PartDelta, etc.) |
| `UserMessage` | `text: String` | A user prompt was sent (broadcast to all clients) |
| `ToolStart` | `call_id` | Tool execution started |
| `ToolEnd` | `call_id`, `success` | Tool execution finished |
| `ToolProgress` | `call_id`, `chunk` | Intermediate tool output |
| `PartUpdated` | `part_id`, `part` | A part's content or state changed |
| `ErrorEvent` | `message` | Terminal error during a turn |

### Request/response pairs

| Message | Fields | Purpose |
|---------|--------|---------|
| `PermissionRequest` | `request_id`, `tool_name`, `input` | Ask user to approve a tool call |
| `WorkspacePermissionRequest` | `request_id`, `path` | Ask user to approve a path outside workspace |
| `SubagentPermissionRequest` | `request_id`, `parent_call_id`, `tool_name`, `input` | Permission for a subagent's tool call |
| `AskUserRequest` | `request_id`, `call_id`, `questions` | Ask user free-text questions |
| `RequestResolved` | `request_id` | A pending request was resolved by any client |

### Subagent events

| Message | Fields | Purpose |
|---------|--------|---------|
| `SubagentStart` | `parent_call_id`, `name`, `child_session_id`, `display_name?` | Subagent spawned |
| `SubagentStatus` | `parent_call_id`, `tool_name`, `message` | Subagent progress update |
| `SubagentEnd` | `parent_call_id`, `child_session_id`, `outcome` | Subagent finished |

### Client presence

| Message | Fields | Purpose |
|---------|--------|---------|
| `ClientAttached` | `client_id`, `client_kind: ClientKind` | A new client attached to the session |
| `ClientDetached` | `client_id` | A client detached |
| `ControlYielded` | `client_id` | A client yielded control |

### Model & mode

| Message | Fields | Purpose |
|---------|--------|---------|
| `ModelList` | `models: Vec<ModelInfo>` | Response to `ListModels` |
| `ModelSwitched` | `provider`, `model` | Confirms model switch |
| `ThinkingVariantChanged` | `variant?: String` | Confirms thinking variant (null = disabled) |
| `PermissionModeChanged` | `mode: String` | Permission mode changed (broadcast) |

### Groups

| Message | Fields | Purpose |
|---------|--------|---------|
| `GroupList` | `groups: Vec<GroupInfo>` | Response to group list request |
| `GroupsChanged` | `groups: Vec<GroupInfo>` | Groups changed (broadcast) |

### Projects & file service

| Message | Fields | Purpose |
|---------|--------|---------|
| `ProjectList` | `projects: Vec<ProjectInfo>` | Response to `ListProjects` |
| `DirListing` | `path`, `entries: Vec<DirEntry>` | Response to `ListDir` |
| `FilePreview` | `path`, `content`, `truncated`, `language?` | Response to `ReadFilePreview` |
| `GitStatusResult` | `entries: Vec<GitEntry>` | Response to `GitStatus` |
| `FsChanged` | `paths: Vec<String>` | Filesystem change notification |

### Other

| Message | Fields | Purpose |
|---------|--------|---------|
| `TodosUpdated` | `todos: Vec<Todo>` | Session todo list changed |
| `PersonaSwitchRequested` | `name` | `switch_persona` tool was called |
| `JobUpdate` | `job_id`, `command`, `state` | Background shell job changed |
| `SlashResult` | `text` | Slash command produced text output |
| `SessionAlert` | `session_id`, `title`, `kind: AlertKind`, `detail?` | Cross-session alert (broadcast to all clients) |
| `FlaggedFilesChanged` | `session_id`, `files: Vec<FlaggedFileWire>` | Flagged-files set changed |
| `Error` | `message` | Error before or outside a turn |
| `Pong` | `version: String` | Response to `Ping` |

## Session model

The daemon owns sessions via `SessionManager`. Connections attach to
sessions. Key fields on `Session`:

```rust
pub struct Session {
    pub id: String,
    pub agent: Mutex<Agent>,
    pub turn_lock: Mutex<()>,           // serializes turns
    pub clients: Mutex<Vec<(u64, Sender<ServerMessage>, ClientKind)>>,
    pub pending_permissions: Mutex<HashMap<u64, PendingRequest<PermissionDecision>>>,
    pub pending_ask_user: Mutex<HashMap<u64, PendingRequest<Vec<String>>>>,
    pub current_turn_cancel: Mutex<Option<CancellationToken>>,
    pub model: Mutex<Option<String>>,
    pub provider: Mutex<Option<String>>,
    pub title_generated: Mutex<bool>,
    pub is_running: Mutex<bool>,         // running indicator for the session rail
    pub session_dir: PathBuf,
}
```

`PendingRequest<T>` bundles the wire payload (`ServerMessage`) alongside
the `oneshot::Sender<T>` responder. The payload is stored so it can be
replayed to a client that attaches while the agent is blocked on a
permission or ask-user request — those requests aren't in the message
history, so without replay the new client would never see them.

`ClientKind` (`Tui`, `Web`, `Cli`, `Mobile`, `Unknown`) is sent with
`NewSession` and `AttachSession` so the daemon and other clients know
what type of frontend connected. `ClientAttached` broadcasts it to
existing clients.

## Project discovery

`ListProjects` returns `ProjectInfo` entries derived from session metas
on disk (their `cwd` field) plus configured `workspace.roots`. Paths are
canonicalized on the daemon for dedupe; clients display `display_name`
(last path component).

```rust
pub struct ProjectInfo {
    pub path: String,
    pub display_name: String,
    pub session_count: u32,
    pub last_used_at: Option<i64>,
}
```

`NewSession { cwd }` validates the cwd before creating the session —
the path must exist and be a directory. Bad paths return
`ServerMessage::Error` and no session is created.

### File service

`ListDir` / `ReadFilePreview` / `GitStatus` provide file browsing for
frontends. `DirEntry` is the per-file record:

```rust
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}
```

`WatchWorkspace` toggles filesystem change notifications; the daemon
broadcasts `FsChanged { paths }` when watched files change.

- **Broadcasting**: `session.broadcast(msg)` sends to all attached clients
  and removes any that have disconnected.
- **Turn serialization**: `turn_lock` ensures only one turn runs at a time.
  A second `Prompt` while a turn is in progress receives an error.
- **Cancellation**: each turn gets a fresh `CancellationToken`. `Cancel`
  from any client cancels the current turn and drains pending requests so
  the agent loop unblocks.
- **Permission/ask-user requests**: go to all clients. Any client can
  respond. `RequestResolved` dismisses the modal everywhere. When a new
  client attaches while a request is outstanding, the daemon replays the
  pending request payloads (they aren't in `SessionHistory`). Clients
  deduplicate by `request_id` to avoid double-showing.

## Connection lifecycle

```
Client connects
  → NewSession or AttachSession
  → SessionReady + SessionHistory
  → (replay) outstanding PermissionRequest / AskUserRequest payloads
  → (replay) current flagged-files set
  → Prompt
  → Provider events stream (PartStart → PartDelta → ... → MessageEnd)
  → (optional) PermissionRequest → PermissionResponse → RequestResolved
  → (optional) ToolStart → ToolProgress → ToolEnd
  → (optional) more turns
Client disconnects
  → detach_client
  → if last client: cancel turn, drain pending requests, remove session from active
```

Idle sessions can be resumed from disk. The session writer persists every
message to `~/.local/share/mew/sessions/<id>/session.jsonl`. On resume,
`Agent::load_messages` replays the history.

## AgentEvent → ServerMessage translation

`translate_event` (`mew-daemon/src/lib.rs`) converts each `AgentEvent` into
zero or more `ServerMessage`s:

- `AgentEvent::Provider(pe)` → `ServerMessage::Provider { event: ProviderEventWire::from(pe) }`
- `AgentEvent::PermissionRequest { call, tx }` → assigns a `request_id`,
  stashes a `PendingRequest { payload, responder: tx }` in
  `session.pending_permissions`, emits `ServerMessage::PermissionRequest`
- `AgentEvent::WorkspacePermissionRequest { path, tx }` → same pattern,
  emits `ServerMessage::WorkspacePermissionRequest`
- `AgentEvent::SubagentPermissionRequest { ... }` → same pattern,
  emits `ServerMessage::SubagentPermissionRequest`
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
