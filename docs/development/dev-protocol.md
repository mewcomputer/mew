---
title: Daemon Protocol
description: Wire protocol between the daemon and frontends.
---

The daemon speaks JSON over WebSocket with terminal, browser, desktop, and iOS
clients. Message types live in `crates/mew-protocol/src/lib.rs` (`PROTOCOL_VERSION = 4`).
Extension IPC defines an independent `EXTENSION_PROTOCOL_VERSION = 1`
(`ExtensionMessage` / `DaemonMessage`).

## Wire format

Rust enums use a `type` tag with snake_case variant names:

```json
{"type": "prompt", "text": "hello", "attachments": []}
```

```json
{"type": "session_ready", "session_id": "sess_01J...", "model": "deepseek-v4-flash"}
```

Use `encode_json` and `decode_json` in `mew-protocol`. The tables below list the
core session, streaming, and request/response operations; check
`crates/mew-protocol/src/lib.rs` for the full set (including remote auth,
confirmed deletes, workspace diffs, subagent history/guidance, goals, permission
mode changes, client presence, browser tab registration, and PTY messages).

## Client to server messages

| Message | Fields | Purpose |
|---------|--------|---------|
| `NewSession` | `cwd?`, `client_kind` | Create a session |
| `AttachSession` | `session_id`, `client_kind` | Attach to an active or idle session |
| `LoadSessionHistory` | `session_id`, `before`, `limit` | Page older history backward |
| `ListSessions` | | List top-level sessions |
| `SubmitPrompt` | `session_id`, `request_id`, `text`, `attachments` | Submit input with an acceptance receipt |
| `ReadPromptReceipt` | `session_id`, `request_id` | Query an uncertain submission after reconnect |
| `Prompt` | `text`, `attachments` | Legacy prompt without an acceptance receipt |
| `Guide` | `text` | Inject user guidance into the active turn |
| `Cancel` | | Cancel the active turn |
| `PermissionResponse` | `request_id`, `decision` | Reply to a tool or workspace permission request |
| `AskUserResponse` | `request_id`, `answers`, `cancelled` | Reply to or close a question prompt |
| `GoalResponse` | `request_id`, `accepted` | Approve or reject a proposed goal |
| `ReadJobOutput` | `session_id`, `job_id`, `request_id`, `offset` | Read a chunk of captured shell output |
| `ReadSessionDiff` | `session_id`, `request_id` | Request workspace diff stats and files |
| `ReadSubagentHistory` | `session_id`, `child_session_id`, `request_id` | Fetch a subagent's recorded transcript |
| `GuideSubagent` | `session_id`, `child_session_id`, `request_id`, `text` | Queue guidance for a running child subagent |
| `ReadSubagentMessageReceipt` | `session_id`, `child_session_id`, `request_id` | Check subagent guidance delivery after reconnect |
| `SlashCommand` | `command` | Execute a daemon-side slash command |
| `ListModels` / `SwitchModel` | (`provider`, `model` on switch) | Inspect or switch models |
| `SetThinkingVariant` | `variant` | Set variant, `"budget:<n>"`, `""`, or `"none"` |
| `SetPermissionMode` | `mode` | Switch session permission mode |
| `YieldControl` | | Emit advisory client yield notification |
| `ListProjects` / `RegenerateTitle` | (`session_id` on title) | List known projects or regenerate session title |
| `Ping` | | Request `Pong` with daemon version and `protocol_version` |

## Server to client messages

### Session lifecycle

| Message | Fields | Purpose |
|---------|--------|---------|
| `SessionReady` | `session_id`, `cwd?`, `model?`, `provider?`, `permission_mode?` | Confirm create or attach |
| `SessionOperationFailed` | `operation`, `session_id?`, `message` | Scoped error before attachment succeeds |
| `SessionHistoryPage` | `session_id`, `messages`, `next_cursor?`, `replace` | Paginated history snapshot or prepend page |
| `SessionHistory` | `messages` | Legacy full-history snapshot |
| `SessionList` | `sessions` | Response to `ListSessions` |
| `SessionCleared` | | Context cleared via `/clear` |
| `SessionTitleChanged` | `session_id`, `title` | Updated conversation title |
| `ClientAttached` / `ClientDetached` / `ControlYielded` | `client_id` (`client_kind` on attach) | Multi-client presence updates |

### Streaming events

| Message | Fields | Purpose |
|---------|--------|---------|
| `Provider` | `event: ProviderEventWire` | Raw provider stream event (`PartStart`, `PartDelta`, `PartEnd`, `MessageEnd`) |
| `ToolStart` / `ToolEnd` | `call_id` (`success` on end) | Top-level tool execution lifecycle |
| `NestedToolStart` / `NestedToolEnd` | `parent_call_id`, `call_id`, `tool_name`, `input` / `success`, `error?`, `output_bytes?` | Nested tool calls inside a Python code-mode cell (v4) |
| `ToolProgress` | `call_id`, `chunk` | Streaming tool output |
| `PartUpdated` | `part_id`, `part` | Authoritative updated message part |

### Request/response pairs

| Message | Fields | Purpose |
|---------|--------|---------|
| `PermissionRequest` | `request_id`, `tool_name`, `input` | Tool approval request |
| `WorkspacePermissionRequest` | `request_id`, `path` | Approval for a path outside `workspace.roots` |
| `AskUserRequest` | `request_id`, `subagent?`, `call_id`, `questions` | Question prompt (parent or forwarded child) |
| `GoalProposed` | `request_id`, `call_id`, `objective` | Goal confirmation prompt |
| `RequestResolved` | `request_id` | Dismisses pending prompt across all attached clients |
| `JobOutput` / `JobOutputError` | `session_id`, `job_id`, `request_id`, ... | UTF-8 shell job output chunk (up to 64 KiB) or scoped error |

### Subagent and status messages

| Message | Fields | Purpose |
|---------|--------|---------|
| `SubagentStart` | `parent_call_id`, `name`, `child_session_id`, `display_name?` | Subagent started |
| `SubagentStatus` | `parent_call_id`, `tool_name`, `message` | Mid-run progress update |
| `SubagentEnd` | `parent_call_id`, `child_session_id`, `outcome`, `manifests` | Subagent finished |
| `SubagentPermissionRequest` | `request_id`, `parent_call_id`, `tool_name`, `input` | Child tool approval prompt |
| `ModelList` / `ModelSwitched` / `ThinkingVariantChanged` | model and variant metadata | Model and thinking updates |
| `TodosUpdated` / `JobUpdate` / `SlashResult` | todos, job status, slash output | Session state updates |
| `Pong` | `version`, `protocol_version` | Daemon version and negotiated wire revision |
| `PromptReceipt` | `session_id`, `request_id`, `status`, `message?` | Acceptance, rejection, or unknown status |
| `ProjectList` | `projects` | Deduped project paths with session counts |
| `Error` / `ErrorEvent` | `message` | Out-of-turn or in-turn error |

## Session and submission rules

Native clients negotiate wire revision `4` (`PROTOCOL_VERSION = 4`) before
session commands:

- **Scoped submissions**: `SubmitPrompt` reserves the session's next turn and
  returns `PromptReceipt` (`Accepted` or `Rejected` if a turn is already
  running). Duplicate `request_id`s return the saved receipt without starting a
  second turn. `ReadPromptReceipt` checks status after reconnect; an unknown
  receipt requires user review instead of blind client replay. Legacy `Prompt`
  waits on `session.turn_lock` and errors if an active turn already holds state.
- **Chunked job reads**: `ReadJobOutput` reads at a byte `offset` and receives
  `JobOutput` with `next_offset` aligned on a UTF-8 boundary. Clients append
  only responses matching their pending `(session_id, job_id, request_id, offset)`.
  `done` signals end of captured output; `incomplete` reports a reader error or
  unreached EOF during the post-exit drain independently of exit status.
- **Forwarded child questions**: `AskUserRequest.subagent` identifies a child
  session; `AskUserResponse.cancelled` closes its reply channel without
  disturbing unrelated parent requests, and cancelling the child closes its
  pending questions.

`SessionManager` (`crates/mew-daemon/src/session.rs`) creates, caches, and
reloads sessions: `session.broadcast(msg)` drops disconnected clients, `turn_lock`
serializes turns, and `Cancel` fires the turn's `CancellationToken`.

## Connection lifecycle

```text
Client connects
  -> NewSession or AttachSession
  -> SessionReady + SessionHistoryPage (on attach)
  -> SubmitPrompt -> PromptReceipt
  -> Provider / Tool / Request events broadcast to all clients
Client disconnects
  -> detach_client
  -> daemon checks every 30s and unloads idle sessions after the last client leaves
```

Active turns and background shell jobs keep detached sessions alive until idle.
Transcripts live under `mew_session::session_dir()` (`MEW_SESSION_DIR` overrides
it).

## Event translation

- **Daemon outbound**: `translate_event` (`crates/mew-daemon/src/lib.rs`) maps
  `AgentEvent` variants to `ServerMessage` and turns oneshot channels
  (`PermissionRequest`, `WorkspacePermissionRequest`, `AskUser`, `GoalProposed`)
  into ID-keyed requests stored on `Session`.
- **Client inbound**: native clients (`TuiClient` and desktop) feed wire messages
  into `mew_client_core::ClientEngine`; `DaemonClient` (`crates/mew-daemon/src/client.rs`)
  also offers `translate_server_message` for `AgentEvent` consumers.

## Adding a new message type

1. Add the variant to `ClientMessage` or `ServerMessage` in
   `crates/mew-protocol/src/lib.rs` and add a JSON roundtrip test.
2. Handle it in `handle_connection` (`crates/mew-daemon/src/lib.rs`).
3. Update `mew-client-core` (and `DaemonClient::translate_server_message` if
   applicable) plus native frontend projections.
4. Add the TypeScript type and handler in `mew-web-client/src/index.ts`, then
   wire it into `mew-web-ui/src/stores/session.ts`.
