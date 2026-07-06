---
title: Architecture
description: Internal architecture and crate structure for mew contributors.
---

mew is a multi-frontend agent server. The canonical runtime architecture is
**Frontend → Daemon → Agent → Provider**. A frontend can be the built-in TUI,
the React web UI, or any other client that speaks the daemon wire protocol.
The TUI can also run standalone (embedding the agent directly), but in daemon
mode it is just another frontend.

This doc walks through how input reaches a provider stream, how the daemon
owns sessions, how tool calls are collected and executed, and how the agent
relates to the rest of the system. For the ratatui event loop, display store,
and streaming markdown implementation, see
[TUI Architecture](/docs/development/dev-tui/).

## The pipeline

```
Browser/Frontend → WebSocket → mew-daemon (Unix socket or TCP)
                                      │
                         SessionManager::create / attach
                                      │
                         Session { agent, clients, turn_lock }
                                      │
                         agent.run_with_parts(...) → AgentEvent
                                      │
                         translate_event → ServerMessage
                                      │
                         session.broadcast → all attached clients
```

The web UI reaches the daemon through `mew-web-bridge`, a small TCP+WS
bridge that also serves the built React app:

```
Browser ──ws/http──▶ mew-web-bridge (127.0.0.1:9847)
                         │
                         └── unix ws ──▶ mew-daemon
```

## Crate map

| Crate | Purpose | Key Types |
|-------|---------|-----------|
| `mew` | Binary entry point, CLI parsing | `Cli`, `Commands`, `build_provider` |
| `mew-tui` | Event loop, ratatui UI, App state | `Event`, `EventLoop`, `App`, `Action` |
| `mew-agent` | Conversation state, tool execution loop | `Agent`, `AgentEvent`, `turn_loop` |
| `mew-provider` | Provider trait + event stream | `Provider`, `ProviderEvent`, `EventStream` |
| `mew-tools` | Tool trait + built-ins | `Tool`, `ToolCtx`, `ToolOutput`, `Sensitivity` |
| `mew-hashline` | Line-anchored edits with hash staleness detection | `Patcher`, `SnapshotStore`, `Patch` |
| `mew-protocol` | Wire message types | `ClientMessage`, `ServerMessage` |
| `mew-daemon` | WebSocket server, session ownership | `DaemonServer`, `Session`, `SessionManager` |
| `mew-web-bridge` | TCP+WS bridge + static UI server | `handle_connection`, `proxy` |
| `mew-web-client` | TypeScript client for the wire protocol | `MewClient`, `MewClientEvents` |
| `mew-web-ui` | React chat UI | `App.tsx`, `SessionState`, `ChatSurface` |
| `mew-config` | config.toml + credentials + permissions | `Config`, `PermissionEngine` |
| `mew-session` | JSONL session persistence | `Writer`, `Reader`, `Meta` |
| `mew-catalog` | models.dev catalog with 24h cache | `Catalog`, `Model`, `ThinkingVariant` |
| `mew-mcp` | MCP server client + McpTool wrapper | `McpClient`, `McpTool` |
| `mew-hooks-runtime` | Subprocess plugin dispatcher | `SubprocessDispatcher`, `PluginHost` |
| `mew-context` | Discover `AGENTS.md` / `CLAUDE.md` from cwd up to git root | `ContextResolver` |
| `mew-skills` | Skill discovery + loading from `.mew/skills`, `.opencode/skills`, etc. | `Skill`, `SkillRegistry` |
| `mew-personas` | Switchable system prompts + model pinning + tool allowlists | `Persona`, `PersonaLoader` |
| `ratatui-mdstream` | Streaming markdown to ratatui Lines | `MdStream`, `DocumentState` |

## Startup: building the agent

In standalone mode the `run_tui` function in `main.rs` builds the agent
locally. In daemon mode the agent is built once per session by the daemon's
`AgentBuilder` closure. Frontends never construct an agent directly; they
connect to a session and send `Prompt` messages.

The builder pipeline is the same in both cases:

1. **Resolve model** from CLI flags, config, session state, or the persona
   pin. Falls back to `"deepseek-v4-flash"`.

2. **Build provider** via `build_provider(cfg, cat, provider_id, model_id, raw)`.
   Matches the provider's `shape` string to an adapter:

```rust
match shape.as_str() {
    "openai" => Ok(Arc::new(OpenAIAdapter::new(...))),
    "anthropic" => Ok(Arc::new(AnthropicAdapter::new(...))),
    _ => anyhow::bail!("unsupported shape"),
}
```

For router providers, wraps small + big models behind `Routed`.

3. **Build tools** via `build_tools()`. Returns a `Vec<Arc<dyn Tool>>`:

```rust
let mut tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(Read), Arc::new(Write), Arc::new(Edit), Arc::new(Bash),
    Arc::new(Glob), Arc::new(Grep), Arc::new(Echo), Arc::new(ExitTool),
    Arc::new(ProgressUpdate), Arc::new(AskUser),
    Arc::new(ShellBackground), Arc::new(ShellMonitor),
    Arc::new(JobStatus), Arc::new(JobBlock), Arc::new(JobCancel),
    Arc::new(TodoCreate), Arc::new(TodoUpdate),
    Arc::new(TodoComplete), Arc::new(TodoDelete), Arc::new(TodoListTool),
];
```

The `Skill` tool registers only when skills are discovered. The
`SwitchPersonaTool` registers only when personas exist.

4. **Load MCP servers** via `connect_mcp_servers()`. Each server's tools
   are wrapped as `McpTool` and appended via `tools.extend(mcp_tools)`.

5. **Build permission engine** from config + permission mode. Applies deny
   rules, ask rules, workspace escape tier, and permissive short-circuit.

6. **Construct agent** via `Agent::new(provider, dispatcher, writer, tools, session_id)`,
   then set catalog-derived fields: pricing, context window, vision support,
   workspace roots, subagent runner, flagged files, secrets.

The daemon passes a fresh `mew_session::Writer` for the session so every
message is persisted to that session's JSONL log as it is produced.

## How input becomes a provider stream

Each frontend has its own event loop and display store. The TUI's is covered
in [TUI Architecture](/docs/development/dev-tui/); here is the daemon-side
path shared by all frontends.

1. User submits a prompt in the web UI (or TUI connected via `--connect`).
2. The frontend sends `ClientMessage::Prompt { text, attachments }` over the
   WebSocket.
3. The daemon's `handle_connection` looks up the attached `Session` and
   acquires `session.turn_lock` to serialize turns.
4. It broadcasts `ServerMessage::UserMessage` to all attached clients so
   every frontend shows the prompt immediately.
5. It calls `agent.run_with_parts(text, vec![], Some(token))` and pumps the
   resulting `AgentEvent`s through `forward_events`.
6. `translate_event` converts each `AgentEvent` to one or more
   `ServerMessage`s. `Provider` events become wire provider events;
   channel-bearing events (`PermissionRequest`, `AskUser`) are assigned a
   request ID and stashed in `session.pending_permissions` /
   `pending_ask_user`.
7. `session.broadcast(msg)` sends the wire event to every attached client.
8. Each frontend updates its local display store and re-renders.

## Web UI

The web UI is a React app served by `mew-web-bridge`. It uses the
`mew-web-client` TypeScript library to speak the daemon wire protocol and
Zustand for local state. Key pieces:

- **`mew-web-bridge`** (Rust): listens on TCP for browser HTTP/WebSocket
  connections, proxies WebSocket frames to the daemon's Unix socket, and
  serves the built React app from embedded `mew-web-ui/dist/` assets.
- **`mew-web-client`** (TypeScript): typed `MewClient` class that manages the
  WebSocket, dispatches events, and provides promise-based request/response
  helpers.
- **`mew-web-ui`** (TypeScript/React): React app with TanStack Router. The
  `SessionState` Zustand store holds messages, streaming state, model
  selection, and the session list; `bridgeClientToStore` wires client events
  to store actions.

The web UI supports session switching through a sidebar rail, model switching
via the `ModelPill` component, and permission/ask-user modals that any attached
client can answer. See [Web UI Development](/docs/development/dev-web/) for
build commands, component inventory, and the end-to-end event checklist.

## The turn loop

`turn_loop` (`turn.rs`) is the core agent loop:

1. Build tool definitions from the agent's tool map.
2. Clone messages, apply `on_chat_message` hook, strip empty text parts.
3. Check if context compaction is needed (estimated tokens vs threshold).
4. Build the request: system prompt, messages, tools, reasoning config.
5. Call `provider.stream(req)` to get an `EventStream`.
6. Stream events until the stream ends or cancellation:
   - `PartStart`: create a new assistant message part.
   - `PartDelta`: append content to the current part.
   - `PartEnd`: finalize the part.
   - `MessageEnd`: record finish reason + usage.
7. After the stream ends: check for pending tool calls.
   - If no tool calls: end the turn.
   - If tool calls: execute them sequentially, then loop back for another
     provider turn.

Tool execution happens in `execute_pending_tool_calls`. Each tool runs
sequentially. Permission requests are emitted as `AgentEvent::PermissionRequest`
with an `oneshot::Sender`. The agent blocks until the user responds.

## Hashline edits

mew implements line-anchored file edits in `crates/mew-hashline`. The
high-level flow is:

1. `read` records a snapshot of the file content and returns a
   `[path#hash]` header plus numbered lines.
2. The model calls `edit_hashline` with one or more `[path#hash]` sections
   and operations like `SWAP`, `DEL`, `INS.*`, `SWAP.BLK`, `REM`, or `MV`.
3. `mew-hashline::Patcher` preflights every section in memory: it validates
   the hash, resolves block ops, checks seen-line bounds, and applies the
   edits to LF-normalized text.
4. If the live file has drifted, the patcher tries 3-way-merge recovery from
   the in-memory `SnapshotStore`.
5. Only when every section prepares successfully does the patcher commit
   writes, deletes, and moves.

The crate is filesystem-agnostic: it calls a small `HashlineFs` trait. The
`edit_hashline` tool provides a tokio-fs implementation. See
[Hashline Internals](/docs/development/dev-hashline/) for the full architecture and
extension points.

## AgentEvent variants

The agent communicates with the active frontend through `AgentEvent`. In
standalone TUI mode this is delivered directly to `App`. In daemon mode it is
translated to `ServerMessage` and broadcast to every attached client:

| Variant | Purpose |
|---------|---------|
| `Provider(ProviderEvent)` | Raw streaming event from the provider |
| `ToolStart { call_id }` | Tool execution started |
| `ToolEnd { call_id, success }` | Tool execution finished |
| `PartUpdated { part_id, part }` | A part's content/state changed |
| `PermissionRequest { call, tx }` | Request user approval (oneshot channel) |
| `SubagentStart { ... }` | A subagent was spawned |
| `SubagentStatus { ... }` | Subagent progress update |
| `SubagentEnd { ... }` | Subagent finished |
| `TodosUpdated { todos }` | Todo list changed |
| `PersonaSwitchRequested { name }` | `switch_persona` tool was called |
| `JobUpdate { ... }` | Background shell job state changed |
| `Error(String)` | Terminal error |
| `AskUser { questions, tx }` | Ask the user free-text questions |

Channel-bearing variants (`PermissionRequest`, `AskUser`) use `oneshot::Sender`
to receive the user's response. In daemon mode, these become ID-paired wire
requests via `ServerMessage::PermissionRequest` / `AskUserRequest`.

## Sessions and multisession

The daemon owns every session. Frontends connect over WebSocket and ask to
`NewSession`, `AttachSession`, or `ListSessions`. A session ID is a ULID
prefixed with `sess_`, generated by `SessionManager::create` and persisted in
the session directory.

Multi-session support is implemented today: `SessionManager` maintains an
`active` map of in-memory sessions and can resume idle sessions from disk on
demand. The web UI exposes this through a session rail (`SessionRail.tsx`) that
lists sessions, attaches to them, renames them, and deletes them.

### One session, many clients

A WebSocket connection is bound to exactly one session, but a session can have
many attached clients at the same time. `Session::attach_client` assigns each
client a monotonically increasing `client_id`. `Session::broadcast` sends every
server message to all attached clients, dropping any that have disconnected.

This means you can have the web UI and another frontend viewing the same
conversation simultaneously. All clients see `UserMessage`, tool progress, and
`RequestResolved`. `SessionHistory` is sent only to the client that just
attached; other clients already have the state.

### Active vs idle sessions

| State | Where it lives | How it is reached |
|-------|----------------|-------------------|
| Active | `SessionManager.active` | In memory, has at least one attached client |
| Idle | `~/.local/share/mew/sessions/<id>/` | On disk; loaded into `active` on `AttachSession` |
| Empty | Same as idle, but with a zero-byte `session.jsonl` | Auto-deleted on `ListSessions` |

`SessionManager::list()` returns both active and idle top-level sessions as
`SessionInfo` records, including model, provider, created/last-message times,
summary, and attached client count.

### Session switching

A single frontend connection can switch sessions by sending a new
`NewSession` or `AttachSession`. The web UI keeps `sessionId` in `localStorage`
and re-attaches on reload; the session rail lets the user jump between
sessions without reconnecting the WebSocket.

Wire messages that change or inspect sessions:

| Message | Purpose |
|---------|---------|
| `NewSession { cwd }` | Create a fresh session |
| `AttachSession { session_id }` | Attach to an active or idle session |
| `ListSessions` | List all top-level sessions |
| `DeleteSession { session_id }` | Delete a session from disk and memory |
| `RenameSession { session_id, title }` | Set a custom title |

### State ownership

| State | Owner |
|-------|-------|
| Canonical message history | Daemon `Agent.messages` + `mew_session::Writer` JSONL |
| Session metadata (title, summary, timestamps) | `meta.json` on disk |
| Active session registry | `SessionManager.active` in the daemon |
| Per-session load lock | `SessionManager.loading` |
| Attached client list | `Session.clients` in the daemon |
| Pending permission / ask-user requests | `Session.pending_permissions` / `pending_ask_user` |
| Display/render state | Each frontend (TUI `App`, web UI store) |
| Model/provider per session | `Session.model` / `Session.provider` |
| Permission mode per session | `Agent.permission_mode` |

### Lifecycle

```
Client connects
  → NewSession { cwd }  or  AttachSession { session_id }
  → SessionManager creates or resumes the session
  → SessionReady + SessionHistory (to attaching client)
  → Prompt and streaming events broadcast to all clients
Client disconnects
  → detach_client
  → if last client: cancel any in-flight turn, remove session from active
```

Idle sessions are loaded from disk on `AttachSession`. The JSONL log is replayed
through `Agent::load_messages`, and the writer is reopened so new messages
append to the same file. Subagent sessions (`meta.depth != 0`) are excluded from
`ListSessions` and cannot be attached directly.

## Multitenancy and isolation

mew is single-user, multi-session. The daemon runs as the local user and all
sessions share the same OS identity, config file, credential resolution, and
MCP server pool. There is no authentication or per-user sandboxing.

Session-level isolation exists for the things that matter to a conversation:

- **Message history**: each session has its own JSONL log and `Agent.messages`.
- **Model/provider**: `Session.model` / `Session.provider` are per-session, and
  `SwitchModel` only affects the attached session.
- **Permission mode**: `SetPermissionMode` is per-session.
- **Workspace roots**: each session's agent gets its own `workspace_roots`
  (defaulting to the `cwd` supplied in `NewSession` or the current directory).
- **Turn lock**: `session.turn_lock` serializes turns within a session, but
  different sessions run independently.

What is **not** isolated:

- **Config and credentials**: all sessions read the same `config.toml`,
  credentials, and provider defaults.
- **MCP servers**: MCP tools are registered once per daemon startup and shared
  across sessions.
- **Hooks/dispatcher**: the same dispatcher instance is used for every session.

This is enough for one person to run many independent conversations, but it is
not multi-tenant in the SaaS sense. A shared daemon would need user accounts,
namespaces, and per-session config overrides before it could safely serve
multiple people.

## Current limitations

Handover between frontends is partially supported but not seamless:

- The TUI in daemon mode (`mew chat --connect`) always calls `NewSession`; it
  cannot attach to an existing daemon session, so `/resume` is rejected.
- The web UI stores the last session ID in `localStorage` and calls
  `AttachSession` on reload. Two tabs with the same ID will share the session.
- There is no wire event for "another client attached/detached", so a frontend
  cannot show presence or claim input focus.
- The last client to disconnect cancels the current turn and unloads the
  session from memory. There is no way to keep a session warm while switching
  frontends.

See [Daemon Protocol](/docs/development/dev-protocol/) for the wire-level message types and
[Session Handover](/docs/development/handover/) for the target design.
