---
title: Architecture
description: Internal architecture and crate structure for mew contributors.
---

mew is a three-layer pipeline: **TUI → Agent → Provider**. This doc walks
through how a keystroke becomes a provider stream, how tool calls are
collected and executed, and how the display and API stores relate.

## The pipeline

```
Keyboard → crossterm EventStream → EventLoop (mpsc::channel(256))
                                      │
                                      ├─ Event::Input(crossterm::Event)
                                      ├─ Event::Agent(AgentEvent)
                                      ├─ Event::Tick (60fps)
                                      └─ Event::Quit
                                            │
                  handle_input_event() → Action::Submit(text)
                                            │
                  agent.run_with_parts(prompt, attachments, token)
                      returns mpsc::Receiver<AgentEvent>
                                            │
                  event_loop.forward_agent_events(agent_rx)
                      spawns tokio task pumping AgentEvent → EventLoop
                                            │
                  app.handle_agent_event(event) → draw()
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
| `mew-config` | config.toml + credentials + permissions | `Config`, `PermissionEngine` |
| `mew-session` | JSONL session persistence | `Writer`, `Reader`, `Meta` |
| `mew-catalog` | models.dev catalog with 24h cache | `Catalog`, `Model`, `ThinkingVariant` |
| `mew-mcp` | MCP server client + McpTool wrapper | `McpClient`, `McpTool` |
| `mew-hooks-runtime` | Subprocess plugin dispatcher | `SubprocessDispatcher`, `PluginHost` |
| `ratatui-mdstream` | Streaming markdown to ratatui Lines | `MdStream`, `DocumentState` |

## Startup: building the agent

The `run_tui` function in `main.rs` constructs the agent through a pipeline:

1. **Resolve model** from CLI flags, config, or state. Falls back to
   `"deepseek-v4-flash"`.

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

## The event loop

`EventLoop` (`events.rs`) is a thin wrapper around `mpsc::channel(256)`:

```rust
pub struct EventLoop {
    tx: mpsc::Sender<Event>,
}
```

Three tokio tasks feed events into the channel:

- **Crossterm reader**: reads keyboard/mouse events and forwards as `Event::Input`.
- **Tick generator**: fires every 16ms (60fps) as `Event::Tick`. Skipped
  when idle (see `tick_interval_ms` for adaptive polling).
- **Agent forwarder**: per-prompt. `forward_agent_events` spawns a task
  that pumps `mpsc::Receiver<AgentEvent>` into `Event::Agent`:

```rust
pub fn forward_agent_events(&self, mut agent_rx: Receiver<AgentEvent>) {
    let tx = self.tx.clone();
    tokio::spawn(async move {
        while let Some(event) = agent_rx.recv().await {
            if tx.send(Event::Agent(event)).await.is_err() { break; }
        }
    });
}
```

## The main loop

The TUI main loop (`run_tui` in `main.rs`):

1. **Render**: `terminal.draw(|f| mew_tui::ui::draw(f, &mut app))`. Skipped
   when idle: `if !last_event_was_tick || app.needs_redraw()`.
2. **Wait for event**: `event_rx.recv().await`.
3. **Process**: match on `Event::Input` / `Event::Agent` / `Event::Tick` / `Event::Quit`.
4. **Drain loop**: after processing the first event, coalesces rapid events
   via `try_recv()`. Capped at `STREAMING_DRAIN_LIMIT = 4` agent events per
   frame so streaming text appears incrementally instead of all at once.

## How a keystroke becomes a provider stream

1. User types text and presses Enter. Crossterm fires `Event::Key(Enter)`.
2. `handle_input_event` calls `app.submit_input()`, returns `Action::Submit(text)`.
3. The main loop calls `agent.run_with_parts(prompt, attachments, token)`.
4. `run_with_parts` spawns a tokio task running `run_loop`, which calls
   `turn_loop`. Returns `mpsc::Receiver<AgentEvent>` immediately.
5. `event_loop.forward_agent_events(agent_rx)` pumps the receiver into the
   main event channel.
6. Each `AgentEvent::Provider(pe)` updates App state via `handle_agent_event`:
   `PartStart` creates a new part, `PartDelta` appends text, `MessageEnd`
   finalizes the stream.
7. `draw()` renders the updated state.

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

## Display store vs API history store

Two separate message stores exist:

- **`app.messages`** (display): what the TUI renders. All parts from a
  multi-turn agentic loop (text, tool calls, follow-up text) merge into
  one assistant message entry. Synthetic messages (alerts, cost reports)
  live here too.
- **`agent.messages`** (API history): what gets sent to the provider. Each
  provider turn produces a separate assistant `Message`. Tool calls and
  results are separate parts. This is the canonical conversation state
  persisted to disk.

The display store is rebuilt from the API history on session resume. The
streaming markdown cache (`rendered_md_cache`) maps message IDs to rendered
ratatui Lines, invalidated when the terminal width changes.

## Streaming markdown

`app.md_stream` / `app.md_state` track the currently-streaming text part:

- The **last** `Part::Text` in the active message uses `render_streaming(md_state)`
  for incremental rendering.
- Earlier text parts (before tool calls in the same message) use the cached
  path: `render_markdown(tp.text, md_width, theme)`.
- On `MessageEnd`, the stream is finalized and `pending_md_rerender` triggers
  a full re-render from `tp.text` on the next frame.
- Cache invalidation: `rendered_md_cache` is cleared when `md_width` changes
  (terminal resize).

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

The agent communicates with the TUI through `AgentEvent`:

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
`NewSession` or `AttachSession`. A session ID is a ULID prefixed with `sess_`,
generated by `SessionManager::create` and persisted in the session directory.

### One session, many clients

A WebSocket connection is bound to exactly one session, but a session can have
many attached clients at the same time. `Session::attach_client` assigns each
client a monotonically increasing `client_id`. `Session::broadcast` sends every
server message to all attached clients, dropping any that have disconnected.

This means you can have the web UI and another frontend viewing the same
conversation simultaneously. All clients see `UserMessage`, tool progress, and
`RequestResolved`. `SessionHistory` is sent only to the client that just
attached; other clients already have the state.

### State ownership

| State | Owner |
|-------|-------|
| Canonical message history | Daemon `Agent.messages` + `mew_session::Writer` JSONL |
| Session metadata (title, summary, timestamps) | `meta.json` on disk |
| Active session registry | `SessionManager.active` in the daemon |
| Attached client list | `Session.clients` in the daemon |
| Pending permission / ask-user requests | `Session.pending_permissions` / `pending_ask_user` |
| Display/render state | Each frontend (TUI `App`, web UI store) |
| Model/provider per session | `Session.model` / `Session.provider` |

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
append to the same file.

### Current limitations

There is no explicit handover protocol. Moving from the TUI to the web UI is
possible, but it is not a first-class gesture:

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
`docs/HANDOVER.md` in the repo for the target design.
