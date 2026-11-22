---
title: Architecture
description: Internal architecture and crate structure for Mew contributors.
---

Mew is a multi-frontend agent server following the runtime path
`Frontend -> Transport -> Daemon -> Session -> Agent -> Provider`. Terminal,
browser, desktop, and iOS clients share one daemon and wire protocol. For the
ratatui event loop, display store, and streaming Markdown implementation, see
[TUI Architecture](/docs/development/dev-tui/).

## The pipeline

```text
Frontend -> WebSocket -> mew-daemon (Unix socket or TCP)
                             |
                SessionManager::create / attach
                             |
                Session { agent, clients, turn_lock }
                             |
                agent.run_with_parts(...) -> AgentEvent
                             |
                translate_event -> ServerMessage
                             |
                session.broadcast -> all attached clients
```

The browser UI reaches the daemon through `mew-web-bridge`, a TCP and WebSocket
bridge serving the compiled React app:

```text
Browser --ws/http--> mew-web-bridge (127.0.0.1:9847)
                         |
                         +-- unix ws --> mew-daemon
```

For remote access, builds with the `iroh` feature listen over iroh (QUIC with
hole punching and relays) via `mew daemon --remote` or `mew daemon --iroh`.
`mew pair` prints a QR invite for `mew daemon --remote`. See
[iOS App](/docs/using-mew/ios-app/) and
[Mobile Core Development](/docs/development/dev-mobile/).

## Native and Electron desktop clients

The native GPUI desktop client (`apps/mew-desktop`) connects to a local daemon
on loopback port `25566` (overridable with `MEW_DESKTOP_DAEMON_PORT` or
`MEW_DESKTOP_DAEMON_URL`). `mew-desktop-supervisor` checks WebSocket health on
that port and attaches to an existing daemon, or spawns `mew daemon` and stops it
when the app exits (`MEW_DESKTOP_DAEMON_BINARY` selects an alternate
executable). Remote desktop profiles connect over iroh via `mew-client-iroh`.

Browser workbench tabs in the native desktop client embed Chromium via
`mew-browser-host` and `native/cef-host`, exposing a loopback CDP endpoint to
`agent-browser` while tracking the active tab per conversation on the daemon.
Embedded PTY terminal tabs run through `mew-pty`. An opt-in macOS Electron
shell (`apps/mew-electron`) runs the shared `mew-web-ui` bundle over `mew
desktop-host`.

## Workspace map

| Member | Purpose | Key types / modules |
|--------|---------|---------------------|
| `mew` | CLI binary and TUI/daemon wiring | `Cli`, `Commands`, `build_provider`, `build_tools` |
| `mew-message` | Canonical messages, parts, tool state, manifests | `Message`, `Part`, `ToolState`, `TurnManifest` |
| `mew-agent` | Conversation state, turn loop, compaction, todos | `Agent`, `AgentEvent`, `turn_loop` |
| `mew-provider` | Provider trait, streaming events, OAuth/auth helpers | `Provider`, `ProviderEvent`, `EventStream` |
| `mew-provider-openai` | OpenAI Chat Completions SSE adapter | `Adapter`, `ToolCallAccumulator`, `RetryPolicy` |
| `mew-provider-anthropic` | Anthropic Messages SSE adapter | `Adapter` |
| `mew-provider-responses` | OpenAI Responses and Codex OAuth adapter | `Adapter` |
| `mew-provider-router` | Tiered router (`nano` / `micro` / `deci`) | `Router`, `Routed` |
| `mew-provider-fake` | Deterministic scripted test provider | `FakeProvider` |
| `mew-tools` | Tool trait and built-in tools | `Tool`, `ToolCtx`, `ToolOutput`, `Sensitivity` |
| `mew-hashline` | Line-anchored edits and 3-way-merge recovery | `Patcher`, `SnapshotStore`, `Patch` |
| `mew-diff` | Line-level workspace diff calculation | `ChangeStats` |
| `mew-protocol` | JSON WebSocket wire types and command registry | `ClientMessage`, `ServerMessage`, `PROTOCOL_VERSION` |
| `mew-daemon` | WebSocket server, session ownership, iroh listener | `DaemonServer`, `Session`, `SessionManager` |
| `mew-config` | Config, credentials, permissions, shell parsing | `Config`, `State`, `PermissionEngine` |
| `mew-session` | JSONL transcripts, `meta.json`, `policy.json`, groups | `Writer`, `Reader`, `Meta`, `SessionPolicy` |
| `mew-catalog` | models.dev catalog and thinking-variant cache | `Catalog`, `Model`, `ThinkingVariant` |
| `mew-context` | `AGENTS.md` / `CLAUDE.md` discovery and VFS context | `Loader` |
| `mew-skills` | `SKILL.md` discovery and loading | `Skill`, `Loader` |
| `mew-personas` | `PERSONA.md` definitions and model/tool policies | `Persona`, `Loader` |
| `mew-subagents` | Subagent definitions and outcomes | `SubagentDef`, `Loader`, `SubagentOutcome` |
| `mew-prompts` | Embedded prompt templates and minijinja rendering | `template` |
| `mew-harness` | Shared Markdown discovery and frontmatter parsing | `load_markdown_dirs_with_extra` |
| `mew-mcp` | MCP stdio/HTTP client and `McpTool` wrapper | `McpClient`, `McpTool` |
| `mew-hooks` | Hook dispatcher trait, types, and `PluginHost` | `Dispatcher`, `PluginHost`, `PermissionMode` |
| `mew-hooks-runtime` | Subprocess JSON-RPC plugin transport | `SubprocessDispatcher` |
| `mew-ext-broker` | Extension manifests, capabilities, consent, Seatbelt | `ExtensionBroker`, `Capability`, `ConsentState` |
| `mew-tui` | Ratatui app, event loop, display store, harness | `Event`, `EventLoop`, `App`, `Action` |
| `mew-raster` | Deterministic ratatui buffer to PNG/MP4 rasterizer | `rasterize` |
| `ratatui-mdstream` | Streaming Markdown rendering and syntax themes | `MdStream`, `DocumentState`, `RenderCache` |
| `mew-client-core` | Shared session and reconnect state for native clients | `ClientEngine` |
| `mew-client-local` | Local WebSocket transport | `LocalWebSocketTransport` |
| `mew-client-iroh` | Remote iroh WebSocket transport | `IrohWebSocketTransport` |
| `mew-ui-model` | Shared native UI projection models | `ConversationModel` |
| `mew-desktop-supervisor` | Loopback daemon attach/spawn lifecycle | `Supervisor` |
| `mew-browser-host` | Embedded browser abstraction and target tracking | `BrowserHost` |
| `mew-pty` | Embedded terminal PTY backend | `PtySession` |
| `apps/mew-desktop` | Native GPUI desktop application | `shell`, `workbench` |
| `native/cef-host` | macOS CEF helper process and `NSView` bridge | `CefHost` |
| `mew-web-bridge` | Browser TCP/WS bridge and embedded UI server | `handle_connection`, `proxy` |
| `mew-mobile-core` | Rust core and UniFFI bindings for iOS | `MobileCore`, `CoreEvent`, `SessionState` |
| `mew-web-client` | TypeScript wire-protocol client (`@mew/web-client`) | `MewClient`, `MewClientEvents` |
| `mew-web-ui` | React chat UI | `SessionState`, `VirtualChatSurface` |

## Startup: building the agent

For each session, the daemon runs its `AgentBuilder` closure:

1. Resolve the provider and model from the session's `policy.json`, state, and
   configured defaults.
2. Select a provider adapter (`openai`, `anthropic`, or `responses`) in
   `crates/mew/src/setup/providers.rs`, wrapping router providers with `Routed`.
3. Register built-in and conditional tools (`skill`, `edit_hashline`, `python`,
   browser tools) in `crates/mew/src/setup/agent.rs`.
4. Append the daemon's MCP tools and configure the session's permission engine.
5. Attach catalog capabilities and a `mew_session::Writer` bound to the
   session's JSONL transcript.

## How input becomes a provider stream

See [TUI Architecture](/docs/development/dev-tui/) for TUI key handling. On the
daemon side, every frontend follows the same path:

1. The frontend sends `ClientMessage::SubmitPrompt` with `session_id`,
   `request_id`, text, and attachments (older clients may send `Prompt`).
2. `handle_connection` locates the attached `Session` and acquires
   `session.turn_lock` to serialize turns.
3. The daemon broadcasts `ServerMessage::UserMessage` to every attached client.
4. The daemon calls `agent.run_with_parts` with the prompt parts and a
   `CancellationToken`, then pumps `AgentEvent`s through `forward_events`.
5. `translate_event` converts each `AgentEvent` into `ServerMessage` frames.
   Channel-bearing requests (`PermissionRequest`, `AskUser`, `GoalProposed`,
   `WorkspacePermissionRequest`) receive a `request_id` stored on `Session`.
6. `session.broadcast(msg)` fans out every wire event to attached clients so
   each frontend updates its display store.

## Web UI

`mew-web-bridge` serves the compiled React bundle from embedded
`mew-web-ui/dist/` assets and proxies browser WebSocket connections to the
daemon's Unix socket. `mew-web-ui` connects through `MewClient` (`@mew/web-client`),
stores session state in Zustand (`SessionState` via `bridgeClientToStore`), and
renders sessions through `components/session-rail.tsx`, `components/model-pill.tsx`,
and permission/question modals. See
[Web UI Development](/docs/development/dev-web/) for build recipes and the
event checklist.

## The turn loop

`turn_loop` (`crates/mew-agent/src/turn.rs`) executes each turn:

1. Build tool definitions from the agent's active tool map.
2. Clone history messages, run `on_chat_message`, and strip empty text parts.
3. Run context compaction when estimated tokens cross the model's threshold.
4. Construct the provider `Request` (system prompt, messages, tools, reasoning).
5. Call `provider.stream(req)` to obtain an `EventStream`.
6. Consume streaming events (`PartStart`, `PartDelta`, `PartEnd`, `MessageEnd`).
7. If tool calls are pending, run `execute_pending_tool_calls` sequentially and
   loop back to step 1; otherwise end the turn.

During tool execution, channel-bearing events block on a `oneshot::Sender`
until an attached client responds or the turn cancels.

## Hashline edits

`crates/mew-hashline` implements line-anchored edits:

1. `read` records a snapshot and emits numbered lines with a `[path#hash]` header.
2. The model invokes `edit_hashline` with section headers and operations
   (`SWAP`, `DEL`, `INS.*`, `SWAP.BLK`, `DEL.BLK`, `INS.BLK.POST`, `REM`, `MV`).
3. `Patcher` prepares every section in memory (hash validation, tree-sitter
   block resolution, seen-line bounds check, LF normalization).
4. On hash drift, `Patcher` attempts 3-way-merge recovery using the in-memory
   `SnapshotStore`.
5. Once all sections prepare cleanly, `Patcher` commits writes, deletes, and
   moves through the `HashlineFs` trait.

See [Hashline Internals](/docs/development/dev-hashline/) for details.

## AgentEvent variants

`AgentEvent` (`crates/mew-agent/src/lib.rs`) carries streaming, tool, and
lifecycle updates from the agent to `translate_event`:

| Variant | Purpose |
|---------|---------|
| `Provider(ProviderEvent)` | Raw provider stream event |
| `ToolStart` / `ToolEnd` | Top-level tool execution boundaries |
| `NestedToolStart` / `NestedToolEnd` | Nested tool calls inside a Python cell |
| `ToolProgress` | Intermediate tool output chunk |
| `PartUpdated` | Updated message part content or tool state |
| `PermissionRequest` / `SubagentPermissionRequest` | Tool approval request (with `oneshot::Sender`) |
| `WorkspacePermissionRequest` | Outside-workspace path approval (with `oneshot::Sender`) |
| `AskUser` / `SubagentAskUser` | User question prompt (with `oneshot::Sender`) |
| `GoalProposed` | Goal approval prompt (with `oneshot::Sender`) |
| `SubagentStart` / `SubagentProgress` / `SubagentStatus` / `SubagentEnd` | Subagent lifecycle and progress |
| `ChildGuidanceContinuation` | Accepted guidance continued a child run |
| `TodosUpdated` / `FlaggedFilesChanged` / `FileDelta` | Sidebar todos, flagged files, and diff stats |
| `JobUpdate` | Background shell job state update |
| `Error(String)` | Terminal turn error |

## Sessions and multisession

The daemon creates, stores, and runs every session. Frontends send `NewSession`, `AttachSession`, or
`ListSessions`. `SessionManager::create` generates a `sess_`-prefixed ULID and
persists `session.jsonl`, `meta.json`, `policy.json`, and `todos.json`.

`SessionManager` holds active sessions in memory (`SessionManager.active`) and
reloads unloaded sessions from `mew_session::session_dir()` on demand. Empty
sessions (zero messages) are omitted from `ListSessions`.

### One session, many clients

Each WebSocket connection binds to one session, while a session can have any
number of attached clients (`Session.clients`). `Session::broadcast` forwards
live events (`UserMessage`, provider deltas, tool events, `RequestResolved`) to
every attached client, while `SessionHistoryPage` goes only to the newly
attached client.

### Session switching and ownership

A connection switches sessions by sending `NewSession { cwd, client_kind }` or
`AttachSession { session_id, client_kind }`. `ListSessions`, `DeleteSession`,
`DeleteSessionConfirmed`, and `RenameSession` list, delete, and rename saved
sessions.

| State | Owner |
|-------|-------|
| Canonical message history | `Agent.messages` and `mew_session::Writer` JSONL |
| Session metadata | `meta.json` |
| Effective model, persona, thinking, permission | `policy.json` (`SessionPolicy`) |
| Active session registry and load lock | `SessionManager.active` / `SessionManager.loading` |
| Attached clients and pending prompts | `Session.clients`, `pending_permissions`, `pending_ask_user` |
| Display state | Frontend (`App`, `ClientEngine`, Zustand `SessionState`) |

### Lifecycle

```text
Client connects
  -> NewSession { cwd, client_kind } or AttachSession { session_id, client_kind }
  -> SessionManager creates or reloads session
  -> SessionReady + SessionHistoryPage (to attaching client)
  -> SubmitPrompt / Prompt -> events broadcast to all attached clients
Client disconnects
  -> detach_client
  -> check every 30s after last client leaves; unload when idle
```

Subagent sessions (`meta.depth != 0`) are hidden from `ListSessions` and cannot
be attached directly.

## Scope and isolation

Mew is a single-user daemon supporting multiple concurrent sessions. Sessions
run under the daemon's OS user and share `config.toml`, credentials, the MCP
server pool, and the hook dispatcher.

Each session isolates its own conversation history, `cwd`, `workspace_roots`,
`policy.json` (model, provider, persona, thinking variant, permission mode),
and `turn_lock`. Serving untrusted multi-user traffic would require separate
user accounts, config isolation, and per-tenant process boundaries.

## Continuing a session in another client

TUI, desktop, browser, and iOS clients can attach to the same daemon session.
Detached sessions continue running active turns and background jobs while the
daemon checks idle runtimes every 30 seconds. See
[Session handover](/docs/development/handover/) and
[Daemon Protocol](/docs/development/dev-protocol/).
