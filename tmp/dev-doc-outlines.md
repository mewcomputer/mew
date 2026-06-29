# Mew Documentation Expansion Outlines

This file contains detailed, section-by-section outlines for expanding each of the
6 skeleton dev docs. Each outline includes specific details, real code snippets with
file paths, and diagrams/tables that should appear in the final documentation.

---

# 1. dev-architecture.md

## Overview
The doc should teach how a keystroke becomes a provider stream, the exact event flow
with channel names, how tool calls are collected and executed, how the display store vs
API history store work, and how streaming markdown works with cache invalidation.

## Section-by-Section Outline

### 1.1 The Three-Layer Pipeline (expanded)

Start with the existing crate map, but expand it with a data-flow diagram showing the
real channel names:

```
Keyboard ──▶ crossterm EventStream ──▶ EventLoop (mpsc::channel(256))
                                        │
                                        ├─ Event::Input(crossterm::Event)
                                        ├─ Event::Agent(AgentEvent)
                                        ├─ Event::Tick (60fps)
                                        └─ Event::Quit
                                              │
              ┌───────────────────────────────┘
              ▼
     handle_input_event() ──▶ Action::Submit(text)
              │
              ▼
     agent.run_with_parts(enriched, attachments, token)
        returns mpsc::Receiver<AgentEvent>
              │
              ▼
     event_loop.forward_agent_events(agent_rx)
        spawns tokio task pumping AgentEvent → EventLoop tx
              │
              ▼
     app.handle_agent_event(event) ──▶ draw()
```

Include the crate map table from the existing doc but add columns for key types:

| Crate | Purpose | Key Types |
|-------|---------|-----------|
| `mew-tui` | Event loop, ratatui UI, App state | `Event`, `EventLoop`, `App`, `Action` |
| `mew-agent` | Conversation state, tool execution | `Agent`, `AgentEvent`, `turn_loop` |
| `mew-provider` | Provider trait + event stream | `Provider`, `ProviderEvent`, `EventStream` |
| `mew-tools` | Tool trait + built-ins | `Tool`, `ToolCtx`, `ToolOutput`, `Sensitivity` |
| `mew-protocol` | Wire message types | `ClientMessage`, `ServerMessage` |
| `mew-daemon` | WebSocket server, session ownership | `DaemonServer`, `Session`, `SessionManager` |

### 1.2 Startup: Building the Agent

Show the `run_tui` function's agent construction pipeline. The key sequence:

1. **Resolve model**: `resolve_model(cfg, cat, provider_flag, model_flag)` — checks
   catalog, falls back to `cfg.default_model`, then `"deepseek-v4-flash"`.

2. **Build provider**: `build_provider(cfg, cat, provider_id, model_id, raw)` — matches
   `shape` string (`"openai"` or `"anthropic"`) to the right adapter. For router
   providers, wraps small+big behind `Routed`.

   Code snippet from `crates/mew/src/main.rs:3751-3839`:
   ```rust
   match shape.as_str() {
       "openai" => {
           let mut adapter = OpenAIAdapter::new(provider_id.to_string(), base_url, model, creds);
           if raw { adapter.set_dump(true); }
           Ok(Arc::new(adapter))
       }
       "anthropic" => {
           let mut adapter = AnthropicAdapter::new(provider_id.to_string(), base_url, model, creds);
           if raw { adapter.set_dump(true); }
           Ok(Arc::new(adapter))
       }
       _ => anyhow::bail!("unsupported shape {} for provider {}", shape, provider_id),
   }
   ```

3. **Build tools**: `build_tools(skills, skill_filter, personas, pending_persona_switch)`
   returns `Vec<Arc<dyn Tool>>`. Show the tool list from `main.rs:699-734`:
   ```rust
   let mut tools: Vec<Arc<dyn mew_tools::Tool>> = vec![
       Arc::new(Read),
       Arc::new(Write),
       Arc::new(Edit),
       Arc::new(Bash),
       Arc::new(Glob),
       Arc::new(Grep),
       Arc::new(Echo),
       Arc::new(ExitTool),
       Arc::new(ProgressUpdate),
       Arc::new(AskUser),
       Arc::new(ShellBackground),
       Arc::new(ShellMonitor),
       Arc::new(JobStatus),
       Arc::new(JobBlock),
       Arc::new(JobCancel),
       Arc::new(TodoCreate),
       Arc::new(TodoUpdate),
       Arc::new(TodoComplete),
       Arc::new(TodoDelete),
       Arc::new(TodoListTool),
   ];
   ```
   Note conditional registration: `Skill` tool only when skills exist,
   `SwitchPersonaTool` only when personas exist.

4. **Load MCP tools**: `connect_mcp_servers(&mcp_configs)` returns
   `(Vec<Arc<dyn Tool>>, Vec<Arc<McpClient>>, Vec<(String, bool, usize)>)`.
   Tools are appended via `tools.extend(mcp_tools)`.

5. **Build permission engine**: `build_permission_engine(cfg, mode)` from
   `main.rs:845-865`.

6. **Construct agent**: `Agent::new(provider, dispatcher, session_writer, tools, None)`,
   then set fields: `flagged_files`, `secrets`, `permission_engine`, `workspace_roots`,
   `subagent_runner`, context files, skills, pricing from catalog.

### 1.3 The Event Loop

Show `EventLoop` from `crates/mew-tui/src/events.rs:26-88`. The key:

- Channel: `mpsc::channel(256)` — capacity 256.
- Three spawn tasks:
  - **Crossterm reader**: reads `EventStream::next()` and forwards as `Event::Input`.
  - **Tick generator**: `tokio::time::interval(Duration::from_millis(16))` → 60fps.
  - **Agent forwarder** (per-prompt): `forward_agent_events` spawns a task that pumps
    `mpsc::Receiver<AgentEvent>` → `Event::Agent`.

Code snippet from `events.rs:31-34`:
```rust
pub fn new() -> (Self, mpsc::Receiver<Event>) {
    let (tx, rx) = mpsc::channel(256);
    (Self { tx }, rx)
}
```

And `forward_agent_events` from `events.rs:78-87`:
```rust
pub fn forward_agent_events(&self, mut agent_rx: mpsc::Receiver<mew_agent::AgentEvent>) {
    let tx = self.tx.clone();
    tokio::spawn(async move {
        while let Some(event) = agent_rx.recv().await {
            if tx.send(Event::Agent(event)).await.is_err() {
                break;
            }
        }
    });
}
```

### 1.4 The Main Loop

Show the main loop structure from `run_tui` (`main.rs:2283-2835`):

1. **Drain plugin UI updates** before each render.
2. **Render**: `terminal.draw(|f| { mew_tui::ui::draw(f, &mut app); })` — but skip
   when idle: `if !last_event_was_tick || app.needs_redraw()`.
3. **Wait for event**: `event_rx.recv().await`.
4. **Process first event**: match on `Event::Input`, `Event::Agent`, `Event::Tick`, `Event::Quit`.
   - `Event::Input` → `handle_input_event()` → `Action::Submit(text)` →
     `agent.run_with_parts(enriched, attachments, None)` → `event_loop.forward_agent_events(agent_rx)`.
5. **Drain loop** (`main.rs:2837-3030`): coalesces rapid input. Key detail:
   `STREAMING_DRAIN_LIMIT: u32 = 4` — caps agent events at 4 per drain batch during
   streaming so text appears incrementally instead of all at once.

   Code snippet:
   ```rust
   let mut agent_drain_count = 0u32;
   const STREAMING_DRAIN_LIMIT: u32 = 4;
   'drain: while let Ok(event) = event_rx.try_recv() {
       // ... process event ...
       agent_drain_count += 1;
       if agent_drain_count >= STREAMING_DRAIN_LIMIT {
           break 'drain;
       }
   }
   ```

### 1.5 How a Keystroke Becomes a Provider Stream

Detailed walkthrough:

1. User types text and presses Enter → crossterm fires `Event::Key(KeyEvent{code: Enter})`.
2. `handle_input_event` → `handle_key_event` → `handle_normal_key` detects Enter,
   calls `app.submit_input()`, returns `Action::Submit(text)`.
3. In the main loop, `Action::Submit(text)` triggers:
   - `agent.dispatcher.on_user_input(text).await` — plugin hook.
   - `process_mentions(&text, &cwd, &mut app.context_files)` — resolves `@file` mentions.
   - `app.messages.push(user_message(display, attachments))` — adds to display store.
   - `app.streaming = true`.
   - `agent.run_with_parts(enriched, attachments, None)` — starts the agent.
   - `event_loop.forward_agent_events(agent_rx)` — pumps events.

4. Inside `Agent::run_loop` (`turn.rs:17-69`): constructs a user `Message` with
   `Part::Text` and any `Part::File` attachments, appends to `self.messages`
   (the API history store), then calls `turn_loop(ev_tx)`.

5. Inside `turn_loop` (`turn.rs:71-506`): each iteration is one LLM turn:
   - Build tool definitions from `self.tools` (filtered by persona allowlist/denylist).
   - Clone `self.messages` for the request (the agent never sends its live store).
   - Apply `on_chat_message` hook to each message.
   - Strip empty text parts from assistant messages.
   - Check for compaction (forced or auto-threshold).
   - Build the `Request` with system prompt, messages, tools, reasoning config, params.
   - Call `self.provider.stream(req).await` → returns `EventStream`.
   - Stream events in a `tokio::select!` loop with cancellation support.

### 1.6 The Display Store vs API History Store

This is a critical distinction:

**API History Store** (`Agent::messages: Arc<Mutex<Vec<Message>>>`):
- The actual conversation sent to the provider each turn.
- Each turn clones it, applies hooks, strips empties, compacts if needed.
- Appends user messages, assistant messages (with tool calls), tool results.

**Display Store** (`App::messages: Vec<Message>`):
- What the TUI renders.
- All parts from a multi-turn agentic loop (text → tool calls → follow-up text) are
  merged into one assistant message display entry.
- `handle_agent_event` appends parts to the last assistant message, or creates one
  if none exists.

Show the merge logic from `app.rs:2062-2092`:
```rust
AgentEvent::Provider(ProviderEvent::PartStart { part }) => {
    // Append part to the last assistant message, or create one.
    if let Some(msg) = self.messages.last_mut() {
        if msg.role == Role::Assistant {
            msg.parts.push(part);
            return;
        }
    }
    // No assistant message exists; create one.
    self.messages.push(Message {
        id: ulid::Ulid::new(),
        session_id: ulid::Ulid::new(),
        role: Role::Assistant,
        parts: vec![part],
        // ...
    });
}
```

### 1.7 How Tool Calls Are Collected and Executed

After `MessageEnd { finish: Finish::ToolUse, .. }`:

1. `turn_loop` calls `self.pending_tool_calls(msg)` to extract pending tool calls.
2. If empty → turn is done (check for persona switch, emit `on_turn_end`).
3. If non-empty → `self.execute_pending_tool_calls(&pending, &mut assistant_msg, &ev_tx)`:
   - For each pending tool call:
     - Check permission (sensitivity → permission engine → prompt if needed).
     - Emit `AgentEvent::ToolStart { call_id }`.
     - Execute tool: `tool.execute(ctx, input).await`.
     - Emit `AgentEvent::PartUpdated { part_id, part }` with the result.
     - Emit `AgentEvent::ToolEnd { call_id, success }`.
   - Collect results into a new `Message` with `Part::ToolResult` parts.
4. Append the tool-result message to `self.messages`.
5. Loop back to `turn_loop` top → next LLM turn sees the tool results.

### 1.8 Streaming Markdown with Cache Invalidation

The TUI uses incremental markdown rendering via `ratatui-mdstream`:

- `App::md_stream: Option<mdstream::MdStream>` — active stream during streaming.
- `App::md_state: mdstream::DocumentState` — incremental render state.
- `App::rendered_md_cache: HashMap<MessageId, (u16, String, Rc<Vec<Line<'static>>>)>`
  — cached rendered lines keyed by message ID, with the width they were rendered at.

**During streaming** (`handle_agent_event`, `PartDelta`):
```rust
if is_text_delta {
    if let Some(ref mut stream) = self.md_stream {
        let update = stream.append(&delta);
        self.md_state.apply(update);
    }
}
```
Only the **last** `Part::Text` in the active message uses `render_streaming(md_state)`.
Earlier text parts (preceding tool calls) use the static cached path.

**On MessageEnd**:
```rust
if let Some(mut stream) = self.md_stream.take() {
    let update = stream.finalize();
    self.md_state.apply(update);
}
if let Some(msg) = self.messages.last() {
    self.pending_md_rerender = Some(msg.id);
}
```
`pending_md_rerender` triggers a full re-render from `tp.text` on the next frame —
the finalized text may differ slightly from the incremental stream (e.g. code fence
completion). The cache is invalidated by width change:
`if width != self.last_md_width { cache.clear(); }`.

### 1.9 Draw Orchestration

Show the `draw()` function from `ui/mod.rs:48-202`:

1. Split into main area + sidebar (if width permits).
2. If no messages → landing screen with centered input.
3. Otherwise: vertical layout:
   - `Constraint::Min(1)` — chat area (`chat::draw_chat`)
   - `Constraint::Length(1)` — divider
   - `Constraint::Length(slash_height)` — slash autocomplete
   - `Constraint::Length(question_height)` — input or ask-user overlay
   - `Constraint::Length(1)` — status line
4. Render overlays on top: alerts, permission modals, persona confirm, command palette.

### 1.10 The AgentEvent Enum

Include the full `AgentEvent` enum from `crates/mew-agent/src/lib.rs:31-112`:

```rust
pub enum AgentEvent {
    Provider(ProviderEvent),
    PermissionRequest { call: HookToolCall, tx: oneshot::Sender<PermissionDecision> },
    ToolStart { call_id: String },
    ToolEnd { call_id: String, success: bool },
    PartUpdated { part_id: PartId, part: Part },
    ToolProgress { call_id: String, chunk: String },
    Error(String),
    WorkspacePermissionRequest { path: PathBuf, tx: oneshot::Sender<PermissionDecision> },
    SubagentStart { parent_call_id: String, name: String, child_session_id: String, display_name: Option<String> },
    SubagentProgress { parent_call_id: String, child_event: Box<AgentEvent> },
    SubagentStatus { parent_call_id: String, tool_name: String, message: String },
    SubagentEnd { parent_call_id: String, child_session_id: String, outcome: SubagentOutcome },
    SubagentPermissionRequest { parent_call_id: String, call: HookToolCall, tx: oneshot::Sender<PermissionDecision> },
    AskUser { call_id: String, questions: Vec<AskUserQuestion>, tx: oneshot::Sender<Vec<String>> },
    TodosUpdated { todos: Vec<Todo> },
    PersonaSwitchRequested { name: String },
    JobUpdate { job_id: String, command: String, state: String },
}
```

Note the two channel-bearing patterns:
- `oneshot::Sender<PermissionDecision>` — tool approval, workspace escape, subagent perms.
- `oneshot::Sender<Vec<String>>` — ask-user questions.

---

# 2. dev-providers.md

## Section-by-Section Outline

### 2.1 The Provider Trait (expanded)

Show the full trait from `crates/mew-provider/src/lib.rs:17-26`:
```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
}
```

Explain:
- `EventStream = Pin<Box<dyn Stream<Item = ProviderEvent> + Send>>` — a pinned boxed
  async stream. The adapter spawns a tokio task that feeds events into an
  `mpsc::channel(128)`, and the receiver is boxed into the stream.
- `list_models` has a default empty impl — providers that support model listing override it.

### 2.2 The Request Struct

Show `Request` from `lib.rs:54-70`:
```rust
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub system: String,
    pub reasoning: Option<ReasoningConfig>,
    pub params: Option<ChatParams>,
    pub headers: http::HeaderMap,
}
```

Explain each field and where it comes from:
- `model`: filled by the adapter (not the agent — the adapter knows its own model ID).
- `messages`: cloned from agent's `self.messages`, hooks applied.
- `tools`: filtered from `self.tools` by persona allowlist/denylist.
- `system`: rebuilt every turn from persona + context + skills.
- `reasoning`: `ReasoningConfig { params: Map }` — shallow-merged into request body.
- `params`: from `on_chat_params` hook — temperature, top_p, max_tokens, tool_choice.
- `headers`: from `on_chat_headers` hook.

### 2.3 ProviderEvent Variants

Show the full enum from `lib.rs:99-125`:
```rust
pub enum ProviderEvent {
    PartStart { part: Part },
    PartDelta { part_id: PartId, field: &'static str, delta: String },
    PartEnd { part_id: PartId },
    MessageEnd { finish: Finish, usage: Tokens, cost: f64 },
    RetryWait { attempt: u32, max_attempts: u32, delay_secs: u64, reason: String },
    Error(mew_message::MessageError),
}
```

Explain the `PartDelta` `field` values:
- `"text"` — text content for TextPart/ReasoningPart.
- `"arguments"` — streaming JSON args for ToolCallPart.
- `"call_id"` — tool call ID.
- `"tool_name"` — tool function name.
- `"signature"` — reasoning signature (Anthropic).

Note: `field` is `&'static str` — works for Rust adapters but requires `Box::leak`
on the daemon client side (see protocol doc).

### 2.4 The OpenAI Shape Adapter

Show the adapter struct from `crates/mew-provider-openai/src/lib.rs:15-22`:
```rust
struct Adapter {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    client: reqwest::Client,
    dump: bool,
}
```

**`stream()` method** (`lib.rs:47-116`):
1. Build request body via `build_request_body(&req)`.
2. POST to `{base_url}/chat/completions` with `Accept: text/event-stream`.
3. Retry loop: `RetryPolicy::default()` — 429 gets exponential backoff (1s, 2s, 4s, 8s),
   5xx gets one retry. Emit `ProviderEvent::RetryWait` before sleeping.
4. On success: spawn `read_stream` task, return `Box::pin(rx)`.

```rust
let (tx, rx) = mpsc::channel(128);
tokio::spawn(async move {
    Self::read_stream(dump, resp, tx).await;
});
Ok(Box::pin(rx))
```

**`read_stream()`** (`lib.rs:345-554`):
Uses `eventsource()` on the response byte stream. Maintains state:
- `current_text_part: Option<TextPart>`
- `current_reasoning_part: Option<ReasoningPart>`
- `current_tool_calls: HashMap<u32, ToolCallAccumulator>`

Process each SSE chunk:
- `delta.role == "assistant"` → start a new text part.
- `delta.content` → emit `PartDelta { field: "text" }`.
- `delta.reasoning` → start/append reasoning part.
- `delta.tool_calls` → accumulate by `index` (OpenAI streams tool calls in fragments).
  Emit `PartStart` on first fragment, `PartDelta { field: "call_id"|"tool_name"|"arguments" }`
  on subsequent fragments.
- `[DONE]` or `finish_reason` → `finalize_all` emits `PartEnd` for open parts + `MessageEnd`.

**`build_request_body()`** (`lib.rs:172-242`):
- Converts `Message`/`Part` → OpenAI wire format via `build_wire_message()`.
- System prompt prepended as `{"role": "system", "content": system}`.
- Tools wrapped in `{"type": "function", "function": {...}}`.
- Reasoning params shallow-merged into body.
- Chat params (temperature, top_p, max_tokens, tool_choice) conditionally inserted.

### 2.5 The Anthropic Shape Adapter

Show the struct from `crates/mew-provider-anthropic/src/lib.rs:14-21`:
```rust
struct Adapter {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    client: reqwest::Client,
    dump: bool,
}
```

**`read_stream()`** (`lib.rs:353-460`):
Anthropic uses named SSE events rather than anonymous chunks. Reads line-by-line:
```rust
let reader = tokio::io::BufReader::new(tokio_util::io::StreamReader::new(stream));
let mut lines = reader.lines();
let mut current_event = String::new();
```

Event dispatch:
- `event: content_block_start` → `handle_content_block_start()` — creates text/reasoning/tool_use parts.
- `event: content_block_delta` → `handle_content_block_delta()` — appends content.
- `event: content_block_stop` → `handle_content_block_stop()` — emits `PartEnd`.
- `event: message_delta` → `handle_message_delta()` — emits `MessageEnd` with finish reason.
- `event: message_stop` → stream complete.
- `event: error` → emit `ProviderEvent::Error`.

Key difference from OpenAI: Anthropic sends content blocks as discrete units
(`text`, `thinking`, `tool_use`) with start/delta/stop events, whereas OpenAI
sends deltas in a single `choices[0].delta` object.

### 2.6 The Router

Show `Router` from `crates/mew-provider-router/src/lib.rs:9-46`:
```rust
pub struct Router {
    small: Arc<dyn Provider>,
    big: Arc<dyn Provider>,
    turn_threshold: usize,  // default: 3
}
```

**Selection logic** (`select()`, `lib.rs:31-45`):
```rust
fn select(&self, req: &Request) -> &Arc<dyn Provider> {
    let has_tool_results = req.messages.iter()
        .any(|m| m.parts.iter().any(|p| matches!(p, Part::ToolResult(_))));
    let is_long = req.messages.len() > self.turn_threshold;
    if has_tool_results || is_long {
        &self.big
    } else {
        &self.small
    }
}
```

- **Tool results present** → use big model (agentic work needs the capable model).
- **Conversation length > threshold** → use big model (context complexity grows).
- Otherwise → use small model (cheap, fast for simple turns).

**`Routed` wrapper** (`lib.rs:220-251`):
```rust
pub struct Routed {
    inner: Router,
    pub display_model: String,
    pub display_provider: String,
}
```
Delegates `stream()` to `inner.stream()` but exposes `display_model`/`display_provider`
so the TUI status line shows the big model (what the user chose) even when the small
model is handling a simple turn.

### 2.7 Retry and Error Classification

Show `RetryPolicy` from `lib.rs:141-180`:
- 429: exponential backoff `initial_backoff * 2^attempt`, capped at `max_backoff`.
  Max 4 retries (default).
- 5xx: one retry at `initial_backoff` (default 1s). Configurable via `retry_5xx`.
- 4xx: no retry.

Show `classify_error()` from `lib.rs:182-202`:
- 401/403 → `ErrorKind::ProviderAuth`
- 429 → `ErrorKind::ProviderRateLimit`
- 500-599 → `ErrorKind::ProviderOverload`
- 400-499 → `ErrorKind::ProviderApi`
- Other → `ErrorKind::Unknown`

### 2.8 The Fake Provider

Show `FakeProvider` from `crates/mew-provider-fake/src/lib.rs:10-116`:
```rust
pub struct FakeProvider {
    script: Vec<ProviderEvent>,
}
```

Two constructors:
- `text_response(text)` — chunks text into 4-char `PartDelta`s, ends with `Finish::Stop`.
- `tool_call(name, id, input)` — emits `PartStart(ToolCall)` + `PartEnd` + `MessageEnd(ToolUse)`.

The `stream()` method uses `futures::stream::unfold` with a 10ms `sleep` between events:
```rust
let stream = futures::stream::unfold(script.into_iter(), |mut iter| async move {
    if let Some(event) = iter.next() {
        sleep(Duration::from_millis(10)).await;
        Some((event, iter))
    } else {
        None
    }
});
```
The delay is critical: it lets cancellation tests catch mid-stream state.

### 2.9 Adding a New Provider (checklist)

1. Create `mew-provider-<name>` crate, depend on `mew-provider`.
2. Implement `Provider` trait — parse SSE into `ProviderEvent`s.
3. Register in `build_provider` (`main.rs:3821-3838`) with a match arm.
4. Add to config defaults in `mew-config` if it should be available out of the box.
5. Add catalog entries if the provider has models in models.dev.

---

# 3. dev-tools.md

## Section-by-Section Outline

### 3.1 The Tool Trait (expanded)

Show the full trait from `crates/mew-tools/src/lib.rs:145-153`:
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &Value;
    fn sensitivity(&self) -> Sensitivity;
    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError>;
}
```

### 3.2 Sensitivity Levels

Show the enum from `lib.rs:13-18`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    ReadOnly,
    Mutating,
    Dangerous,
}
```

Expanded table:

| Level | Default Behavior | Examples |
|-------|-----------------|----------|
| `ReadOnly` | Auto-allowed (no prompt) | `read`, `glob`, `grep`, `job_status` |
| `Mutating` | Prompts the user (unless permission mode overrides) | `write`, `edit`, `bash`, all MCP tools |
| `Dangerous` | Prompts the user (highest urgency) | (reserved; currently no built-in tools use this) |

### 3.3 ToolCtx and ToolCtxShared

Show the two structs from `lib.rs:23-112`:

```rust
#[derive(Clone)]
pub struct ToolCtxShared {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub dispatcher: Option<Arc<dyn mew_hooks::Dispatcher>>,
    pub secrets: Arc<SecretSet>,
}

pub struct ToolCtx {
    pub call_id: String,
    pub cancel: CancellationToken,
    pub progress_tx: mpsc::Sender<ToolProgress>,
    pub shared: Arc<ToolCtxShared>,
}
```

`ToolCtx` implements `Deref<Target = ToolCtxShared>` so tools access `ctx.session_id`,
`ctx.cwd`, `ctx.secrets` directly.

Test helpers:
- `ToolCtx::test_new(cwd)` — minimal context with empty shared state (requires `test-utils` feature).
- `ToolCtx::test_with_secrets(cwd, secrets)` — for testing secret redaction.

### 3.4 ToolOutput and ToolProgress

`ToolOutput` is defined in `mew-hooks` (referenced as `mew_hooks::ToolOutput`). Show
how tools produce output — the `ToolProgress` enum from `lib.rs:129-133`:
```rust
#[derive(Debug, Clone)]
pub enum ToolProgress {
    OutputChunk(String),
    Metadata(Value),
}
```
Tools send `ToolProgress::OutputChunk(chunk)` via `ctx.progress_tx` for live streaming
output (e.g. Bash sends stdout chunks as they arrive).

### 3.5 ToolError

Show from `lib.rs:135-143`:
```rust
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("cancelled")]
    Cancelled,
}
```

### 3.6 A Real Tool Implementation: Read

Show the full `Read` tool from `crates/mew-tools/src/tools/read.rs:1-112`. Key points:
- Zero-arg struct (`pub struct Read;`)
- Schema uses `OnceLock` for static initialization (avoids re-allocating JSON every call)
- Sensitivity: `ReadOnly`
- Execute: joins path to `ctx.cwd`, checks file size (10MB max), reads, detects binary
  via null byte, supports offset/limit pagination, strips secrets from output.

Code snippet for the schema pattern:
```rust
fn schema(&self) -> &Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "..." },
                "offset": { "type": "integer", "description": "..." },
                "limit": { "type": "integer", "description": "..." }
            },
            "required": ["path"]
        })
    })
}
```

Code snippet for secret redaction:
```rust
let (content, redacted) = crate::secrets::redact_secret_words(&content, &ctx.secrets);
let content = crate::secrets::annotate_redaction(content, redacted);
```

### 3.7 The build_tools Function

Show `build_tools()` from `main.rs:693-734`. Explain:
- Core tools always registered.
- `Skill` tool only when `!skills.is_empty()`.
- `SwitchPersonaTool` only when `!personas.is_empty()` (otherwise it'd be a dead-end).
- `FlagImportant` added after `build_tools` in `build_session_agent`/`run_tui`.
- MCP tools added via `tools.extend(mcp_tools)`.
- Subagent tools (`subagent_start`, `subagent_wait`) inserted when subagent defs exist.

### 3.8 MCP Tool Wrapping

Show `McpTool` from `crates/mew-mcp/src/lib.rs:700-770`:

```rust
struct McpTool {
    qualified_name: String,
    description: String,
    schema: Value,
    tool_name: String,
    client: Arc<McpClient>,
}
```

Key design decisions:
- **Qualified name**: `server_name__tool_name` (double underscore separator). The agent
  registry uses `name()` as the key, so this avoids collisions across MCP servers.
- **Always `Mutating` sensitivity**: MCP tools can have arbitrary side effects, so
  they always prompt.
- **Execute**: calls `client.call_tool(&self.tool_name, input)`, maps the result to
  `ToolOutput` (text → output if success, → error if `is_error`).

Show the `connect_mcp_servers` flow: loads `mcp.json`, connects to each server
(via stdio or HTTP), initializes the MCP handshake, lists tools, wraps each as `McpTool`.

### 3.9 Secret Redaction

Show `SecretSet` from `lib.rs:114-127`:
```rust
#[derive(Debug, Clone, Default)]
pub struct SecretSet {
    pub words: Vec<String>,    // substrings to redact
    pub globs: Vec<String>,    // secret file patterns to drop entirely
}
```

Two layers of defense:
1. **Permission engine pre-check**: checks if a file path matches a secret glob → denies
   before the tool even runs.
2. **In-tool redaction**: even when approved, `redact_secret_words` replaces known secret
   values with `[REDACTED]` while preserving file structure.

### 3.10 Adding a New Tool (expanded checklist)

1. Implement `Tool` trait in `crates/mew-tools/src/tools/<name>.rs`.
2. Register in `build_tools()` (`main.rs:699-720`) — add `Arc::new(MyTool::new())`.
3. Test with `ToolCtx::test_new(cwd)` — write unit tests in the tool file.
4. If the tool needs session-shared state, add it to `ToolCtxShared`.

---

# 4. dev-protocol.md

## Section-by-Section Outline

### 4.1 Wire Format Overview

The daemon communicates with frontends over WebSocket using JSON text frames. The
protocol is defined in `mew-protocol` with `encode_json`/`decode_json` functions:
```rust
fn encode_json<T: Serialize>(msg: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(msg)
}
fn decode_json<T: DeserializeOwned>(text: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(text)
}
```

Messages are tagged enums with `#[serde(tag = "type")]`. Each variant's tag is its
PascalCase name (e.g. `"SessionReady"`, `"Prompt"`).

### 4.2 ClientMessage (expanded)

Show the full enum from `crates/mew-protocol/src/lib.rs:23-84`:
```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    NewSession { cwd: Option<String> },
    AttachSession { session_id: String },
    ListSessions,
    Prompt { text: String, #[serde(default)] attachments: Vec<Attachment> },
    Cancel,
    PermissionResponse { request_id: u64, decision: PermissionDecision },
    AskUserResponse { request_id: u64, answers: Vec<String> },
    SlashCommand { command: String },
    ListModels,
    SwitchModel { provider: String, model: String },
    SetThinkingVariant { variant: String },
}
```

Explain each with a concrete JSON example:
```json
{"type": "Prompt", "text": "hello", "attachments": []}
{"type": "PermissionResponse", "request_id": 3, "decision": "allow_once"}
{"type": "SwitchModel", "provider": "deepseek", "model": "deepseek-v4-flash"}
```

### 4.3 ServerMessage (expanded)

Show the full enum from `lib.rs:188-325`. Group by category:

**Session lifecycle**: `SessionReady`, `Error`, `SessionList`, `SessionHistory`,
`SessionCleared`, `SessionTitleChanged`.

**Streaming**: `Provider { event: ProviderEventWire }` — raw provider events relayed.
`ToolStart`, `ToolEnd`, `ToolProgress`, `PartUpdated`, `ErrorEvent`.

**Request/response pairs** (replaces oneshot channels): `PermissionRequest`,
`WorkspacePermissionRequest`, `AskUserRequest` — all carry `request_id: u64`.
`RequestResolved` — broadcast when any client resolves a pending request.

**Subagent**: `SubagentStart`, `SubagentStatus`, `SubagentEnd`, `SubagentPermissionRequest`.

**Session-level**: `TodosUpdated`, `PersonaSwitchRequested`, `JobUpdate`, `SlashResult`.

**Model management**: `ModelList`, `ModelSwitched`, `ThinkingVariantChanged`.

### 4.4 PermissionDecision Wire Type

Show from `lib.rs:147-175`:
```rust
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
}
```

Note: `mew_hooks::PermissionDecision::Prompt` collapses to `Deny` on the wire —
frontends never send `Prompt` (it's daemon-internal).

### 4.5 The Session Model

Show `Session` from `crates/mew-daemon/src/session.rs:56-76`:
```rust
pub struct Session {
    pub id: String,
    pub agent: Mutex<Agent>,
    pub turn_lock: Mutex<()>,
    pub clients: Mutex<Vec<(u64, mpsc::UnboundedSender<ServerMessage>)>>,
    pub pending_permissions: Mutex<HashMap<u64, oneshot::Sender<PermissionDecision>>>,
    pub pending_ask_user: Mutex<HashMap<u64, oneshot::Sender<Vec<String>>>>,
    pub next_id: AtomicU64,
    pub current_turn_cancel: Mutex<Option<CancellationToken>>,
    pub model: Mutex<Option<String>>,
    pub provider: Mutex<Option<String>>,
}
```

Key design points:
- **Session is owned by the daemon**, not by any connection. Multiple clients can attach.
- **Broadcast**: `session.broadcast(msg)` sends to all attached clients, dropping failed senders:
  ```rust
  pub async fn broadcast(&self, msg: ServerMessage) {
      let mut clients = self.clients.lock().await;
      clients.retain(|(_, sender)| sender.send(msg.clone()).is_ok());
  }
  ```
- **Turn serialization**: `turn_lock: Mutex<()>` ensures only one turn runs at a time.
  A fresh `CancellationToken` is created per turn so cancelling one turn doesn't poison
  future turns.
- **Request ID pairing**: `next_id: AtomicU64` generates monotonically increasing IDs
  for both client IDs and request IDs.

### 4.6 SessionManager

Show `SessionManager` from `session.rs:148-330`:
- `create(cwd)` — creates a new session with `sess_<ULID>` ID.
- `attach(session_id)` — fast path: already active. Slow path: resume from disk via
  `mew_session::Reader::load_from()`, with a per-session load lock to prevent TOCTOU.
- `list()` — merges active sessions + idle sessions from disk.
- `remove(session_id)` — removes from active map when last client detaches.

### 4.7 handle_connection

Show the connection handler from `crates/mew-daemon/src/lib.rs:259-580`. Key flow:

1. WebSocket handshake → split into `ws_tx`/`ws_rx`.
2. Create `client_tx: mpsc::UnboundedSender<ServerMessage>` — a per-connection outbound channel.
3. Spawn a writer task that owns `ws_tx` and drains `client_rx`.
4. Main read loop: decode `ClientMessage`, dispatch:

   - `NewSession` → `session_manager.create()`, `session.attach_client(client_tx)`,
     reply `SessionReady`.
   - `AttachSession` → `session_manager.attach()`, `session.attach_client()`, reply
     `SessionReady` + `SessionHistory` (only if first client).
   - `Prompt` → spawn task: acquire `turn_lock`, create `CancellationToken`, clone agent,
     `agent.run_with_parts()`, `forward_events(rx, session)`.
   - `Cancel` → `session.cancel_turn()` — cancels token + drains pending.
   - `PermissionResponse` → look up `pending_permissions[request_id]`, send decision,
     broadcast `RequestResolved`.
   - `SlashCommand` → spawn task with `turn_lock`, handle `/clear` and `/compact`.
   - `SwitchModel` → spawn task with `turn_lock`, call switcher, broadcast `ModelSwitched`.
   - `SetThinkingVariant` → spawn task with `turn_lock`, call thinking setter.

5. Cleanup on disconnect: `session.detach_client(cid)`, if last client: cancel turn,
   `session_manager.remove()`.

### 4.8 Event Translation: AgentEvent → ServerMessage

Show `translate_event()` from `lib.rs:603-752`. This converts channel-bearing
`AgentEvent`s into wire messages with request IDs:

```rust
AgentEvent::PermissionRequest { call, tx } => {
    let id = session.next_request_id();
    session.pending_permissions.lock().await.insert(id, tx);
    vec![ServerMessage::PermissionRequest {
        request_id: id,
        tool_name: call.tool_name,
        input: call.input,
    }]
}
```

The `oneshot::Sender` is stashed in `session.pending_permissions`. When a client sends
`PermissionResponse { request_id, decision }`, the handler looks up the sender and
resolves it — unblocking the agent.

### 4.9 Event Translation: ServerMessage → AgentEvent (Client Side)

Show `translate_server_message()` from `crates/mew-daemon/src/client.rs:193-461`.
This reconstructs channel-bearing `AgentEvent`s from wire messages:

```rust
ServerMessage::PermissionRequest { request_id, tool_name, input } => {
    let (tx, rx) = oneshot::channel();
    state.pending_permissions.lock().await.insert(*request_id, rx);
    spawn_permission_forwarder(*request_id, state);
    vec![AgentEvent::PermissionRequest {
        call: HookToolCall { tool_name: tool_name.clone(), call_id: String::new(), input: input.clone() },
        tx,
    }]
}
```

The `spawn_permission_forwarder` task waits for the TUI's decision and forwards it
back as `ClientMessage::PermissionResponse`.

Note the `wire_to_provider_event` function (`client.rs:492-532`) that reconstructs
`ProviderEvent` from `ProviderEventWire` — the `field: &'static str` is leaked via
`Box::leak` (acceptable for short-lived streaming events).

### 4.10 Connection Lifecycle Diagram

```
Client connects
  → NewSession/AttachSession
  → SessionReady { session_id, model, provider }
  → [if first client on attach] SessionHistory { messages }
  → Prompt { text }
    → Provider events stream (PartStart, PartDelta*, PartEnd, MessageEnd)
    → [if ToolUse] ToolStart → ToolProgress* → PartUpdated → ToolEnd
    → [if PermissionRequest] PermissionRequest → PermissionResponse → RequestResolved
    → [if more turns] loop back to Prompt
  → [optional] Cancel
Client disconnects
  → detach_client(cid)
  → if last client: cancel_turn() + remove(session_id)
```

### 4.11 Adding a New Message Type (expanded checklist)

1. Add variant to `ClientMessage` or `ServerMessage` in `mew-protocol/src/lib.rs`.
2. Add a roundtrip test in the protocol test module (`lib.rs:433+`).
3. Handle in `handle_connection` (`mew-daemon/src/lib.rs`).
4. Update `translate_event` (daemon→wire) or `translate_server_message` (wire→TUI).
5. Add TypeScript type + dispatch to `mew-web-client/src/index.ts`.
6. Wire store action in `mew-web-ui/src/stores/session.ts`.

---

# 5. dev-testing.md

## Section-by-Section Outline

### 5.1 Commands (expanded)

Show the full command set from `justfile`:

```sh
just ci           # fmt + clippy + test-all + e2e (full CI gate)
cargo test --all  # all Rust tests
cargo test -p mew-tui    # one crate
cargo test test_text_turn  # one test by name
just test-v      # verbose output
just test-all    # Rust + TypeScript tests
just test-js     # mew-web-client TS tests only
just e2e         # build-web + bin_e2e subprocess test
just fmt         # format
just clippy      # lint (-D warnings)
just record      # record provider test fixtures (MEW_RECORD=1)
```

Explain the CI pipeline: `ci: fmt clippy test-all e2e` — format check, lint gate,
all tests (Rust + JS), then the subprocess e2e test.

### 5.2 Test Tiers (expanded)

**Tier 1: Foundation**
- `mew-provider-fake`: shape verification, part_id consistency, streaming semantics,
  multi-byte text round-trip, cancellation timing.
- `mew-protocol`: exhaustive roundtrip for every `ClientMessage`/`ServerMessage` variant,
  negative tests (malformed JSON, missing fields, wrong types, unknown variants).
- `mew-daemon` e2e: full daemon lifecycle over real Unix sockets.

**Tier 2: Behavior**
- `mew-tools` integration: composed tool scenarios.
- Agent escape-tier integration: workspace sandbox enforcement.
- Regression tests: error messages include file paths.

**Tier 3: Polish**
- `mew-daemon` concurrency: stress tests under contention.
- `mew-session` roundtrip: JSONL write/load.
- `mew-personas` discovery.
- `mew-subagents` loader.

### 5.3 The Fake Provider as a Test Tool

Show `FakeProvider::text_response()` and `FakeProvider::tool_call()` from
`crates/mew-provider-fake/src/lib.rs:19-94`.

Key test patterns:
- `text_response("hello")` → `PartStart → PartDelta(4-char chunks) → PartEnd → MessageEnd(Stop)`.
- `tool_call("bash", "call_1", json!({"command": "ls"}))` →
  `PartStart(ToolCall) → PartEnd → MessageEnd(ToolUse)`.
- 10ms delay between events — critical for cancellation tests.
- `text_response("")` → only 3 events (PartStart + PartEnd + MessageEnd), no deltas.

Show the test helper `drain()` from `lib.rs:126-133`:
```rust
async fn drain(stream: EventStream) -> Vec<ProviderEvent> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.collect::<Vec<_>>(),
    )
    .await
    .expect("fake provider stream did not terminate within 5s")
}
```

### 5.4 Daemon E2E Tests

Show the test harness from `crates/mew-daemon/tests/e2e.rs`:

**`spawn_daemon()`** (`e2e.rs:33-54`):
```rust
async fn spawn_daemon<F>(agent_factory: F) -> (TempDir, String)
where F: Fn(AgentBuildParams) -> Result<(Agent, Option<String>, Option<String>)> + Send + Sync + 'static,
{
    let dir = tempfile::tempdir().expect("create tempdir");
    let socket_path = dir.path().join("mew.sock");
    let session_dir = dir.path().join("sessions");
    let builder: mew_daemon::AgentBuilder = Arc::new(agent_factory);
    let server = DaemonServer::with_session_dir(builder, session_dir);
    tokio::spawn(async move { let _ = server.run(&socket_str).await; });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (dir, socket_str)
}
```

**`make_text_agent_factory()`** (`e2e.rs:128-151`):
Builds an agent backed by `FakeProvider` with no tools. The `script_fn` closure lets
each test inject a specific event script per connection.

**Key test patterns**:
- `recv_until(pred)` — collects messages until predicate matches or 5s timeout.
- `recv_one_matching(pred)` — convenience wrapper.
- `send(msg)` — encodes + sends a `ClientMessage`.
- Tests use raw WebSocket over `UnixStream` (not `DaemonClient`) to assert on wire shape.

Show representative tests:
- `new_session_returns_session_ready` — basic lifecycle.
- `prompt_streams_text_response_events` — full streaming round-trip, reassembles deltas.
- `prompt_without_new_session_returns_error` — error handling.
- `invalid_json_returns_server_error` — malformed input.
- `cancel_during_stream_does_not_panic` — cancellation safety.
- `tool_call_response_emits_tool_use_finish` — tool call relay.
- `part_id_consistent_across_part_start_and_part_end` — PartId contract.
- `fresh_agent_per_connection` — isolation.

### 5.5 Concurrency Tests

Show from `crates/mew-daemon/tests/concurrency.rs`:

**`TurnRotatingProvider`** (`concurrency.rs:31-64`):
A provider that hands out a different script on each `stream()` call, so successive
turns produce distinct `PartId`s. Falls back to `text_response("(no script)")` when
scripts are exhausted.

Key tests:
- `five_concurrent_connections_all_get_distinct_sessions` — 5 parallel connections,
  each gets a unique session_id.
- `concurrent_prompts_on_same_connection_serialize` — 3 rapid prompts produce 3 distinct
  PartStart IDs (turns are serialized, not interleaved).
- `concurrent_prompts_across_connections_do_not_cross_contaminate` — part IDs are disjoint
  across connections.
- `prompt_during_in_flight_turn_is_serialized` — back-to-back prompts don't interleave events.
- `rapid_fire_cancel_does_not_crash_daemon` — 20 cancel messages, daemon survives.
- `slash_command_during_in_flight_turn_does_not_block_stream` — `/clear` during stream,
  both MessageEnd and SlashResult arrive.

### 5.6 Tools Integration Tests

Show from `crates/mew-tools/tests/integration.rs`:

**Test helper**: `ctx(dir)` → `ToolCtx::test_new(dir.path().to_path_buf())`.

Key patterns:
- **Composed scenarios**: write→read round-trip, write→edit→bash verification,
  glob→grep composition.
- **Cross-tool verification**: after `edit`, use `bash cat` to verify the file changed.
- **Extension filtering**: grep with `glob: "*.rs"` excludes `.txt` files.
- **Error surfacing**: `exit 42` surfaces exit code in error output.

Show the write→edit→bash test (`integration.rs:51-87`):
```rust
#[tokio::test]
async fn write_then_edit_then_bash_cat_verifies_edit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    let write = Write;
    write.execute(ctx(&dir), json!({"path": path.to_string_lossy(), "content": "version=1\n"}))
        .await.unwrap();
    let edit = Edit;
    edit.execute(ctx(&dir), json!({"path": "data.txt", "old_string": "version=1", "new_string": "version=2"}))
        .await.unwrap();
    let bash = Bash;
    let result = bash.execute(ctx(&dir), json!({"command": "cat data.txt"}))
        .await.unwrap();
    assert_eq!(result.output.trim(), "version=2");
}
```

### 5.7 Writing Tests (expanded checklist)

- Use `#[tokio::test]` for async tests (default: current-thread runtime).
- Use `tempfile::tempdir()` for filesystem isolation.
- Use `DaemonServer::with_session_dir(builder, temp_session_dir)` for daemon test isolation.
- The `test-utils` feature on `mew-tools` exposes `ToolCtx::test_new()`.
- For provider tests, use `FakeProvider` with scripted events.
- For daemon tests, use raw WebSocket over `UnixStream` (not `DaemonClient`) to assert on wire shape.
- Always use `recv_until` with a 5s timeout to avoid CI hangs.

---

# 6. dev-web.md

## Section-by-Section Outline

### 6.1 Architecture (expanded)

Show the three-package architecture:

```
Browser ──ws://──▶ mew-web (bridge, Rust) ──ws+unix://──▶ mew daemon
                       │
                       └─ HTTP GET / ──▶ embedded React app (mew-web-ui/dist/)
```

| Package | Language | Purpose |
|---------|----------|---------|
| `mew-web-bridge` | Rust | TCP+WS relay to daemon Unix socket; serves embedded static assets |
| `mew-web-client` | TypeScript | Typed client for wire protocol; ESM + `.d.ts` types |
| `mew-web-ui` | TypeScript/React | React + TanStack Router app; Vite build to `dist/` |

### 6.2 The Bridge (mew-web-bridge)

Show from `crates/mew-web-bridge/src/main.rs`:

**Key design**: The bridge is a pure relay — it doesn't read or interpret the wire
protocol. Each browser connection opens a fresh daemon connection.

**Connection handling** (`main.rs:71-136`):
1. Peek at the HTTP request via `BufReader::fill_buf()` without consuming.
2. If `Upgrade: websocket` header → hand `BufReader` to `accept_async` for WS upgrade.
3. Otherwise → serve static HTTP from embedded assets.

**Static asset serving** (`main.rs:159-198`):
- Uses `include_dir!` to embed `mew-web-ui/dist/` at compile time.
- SPA fallback: unknown paths serve `index.html` so TanStack Router handles client-side routing.
- MIME type mapping covers all Vite-produced asset types.

**Daemon auto-spawn** (`main.rs:301-356`):
- Tries 3 locations for the `mew` binary: next to this binary, `mew` on PATH,
  `target/debug/mew` relative to CWD.
- Polls for the socket to appear (up to 4 seconds).
- Falls back gracefully — logs a warning and continues.

**Bidirectional relay** (`main.rs:259-299`):
```rust
async fn proxy<S>(browser: WebSocketStream<S>, daemon: WebSocketStream<UnixStream>) -> Result<()> {
    let (mut b_tx, mut b_rx) = browser.split();
    let (mut d_tx, mut d_rx) = daemon.split();
    let b_to_d = async { while let Some(msg) = b_rx.next().await { /* forward */ } };
    let d_to_b = async { while let Some(msg) = d_rx.next().await { /* forward */ } };
    tokio::select! { _ = b_to_d => {}, _ = d_to_b => {} }
    Ok(())
}
```

### 6.3 The TypeScript Client (mew-web-client)

Show the `MewClient` class from `mew-web-client/src/index.ts:424-814`:

**Connection**: `connect()` opens a WebSocket, spawns a background reader that
decodes `ServerMessage` and dispatches via `dispatch()`.

**Key methods**:
- `newSession()` → sends `ClientMessage::NewSession`, waits for `SessionReady`.
- `prompt(text)` → sends `ClientMessage::Prompt`, returns void (events come via `on("provider", ...)`).
- `cancel()` → sends `ClientMessage::Cancel`.
- `respondToPermission(requestId, decision)` → sends `ClientMessage::PermissionResponse`.
- `attachSession(sessionId)` → sends `ClientMessage::AttachSession`.
- `listSessions()` / `listModels()` / `switchModel()` / `setThinkingVariant()` — management methods.

**Event dispatch** (`index.ts:672-802`):
The `dispatch()` method matches on `msg.type` (PascalCase tag) and emits typed events
via `this.emit()`. Show the full dispatch table:

```typescript
switch (msg.type) {
    case "SessionReady":
        this.sessionId = msg.session_id;
        this.emit("session-ready", { session_id: msg.session_id, model: msg.model, provider: msg.provider });
        break;
    case "Provider":
        this.emit("provider", msg.event);
        break;
    case "ToolStart":
        this.emit("tool-start", { call_id: msg.call_id });
        break;
    case "PermissionRequest":
        this.emit("permission-request",
            { request_id: msg.request_id, tool_name: msg.tool_name, input: msg.input },
            (decision) => this.respondToPermission(msg.request_id, decision));
        break;
    // ... 20+ more cases
}
```

Note the permission pattern: `emit("permission-request", data, respondCallback)` —
the third argument is a callback the UI calls with the decision. The client wraps it
into a `ClientMessage::PermissionResponse`.

**Event interface**: `MewClientEvents` (`index.ts:335-404`) defines all event names
and their payload types. `on(name, handler)` / `off(name, handler)` for subscription.

### 6.4 The Zustand Store (session.ts)

Show the `SessionState` interface from `mew-web-ui/src/stores/session.ts:124-211`.
Group the state fields:

**Connection**: `connectionState`, `sessionId`.

**Messages**: `messages: ChatMessage[]`, `streamingPartId`, `streamingText`,
`streamingReasoningId`, `streamingReasoningText`.

**Tools**: `toolStates: Map<string, ToolDisplayState>`, `toolOutputs: Map<string, string>`.

**Permissions**: `pendingPermissions: PendingPermission[]`.

**Cost**: `totalInputTokens`, `totalOutputTokens`, `totalCost`.

**Model management**: `availableModels`, `currentModel`, `currentProvider`,
`currentThinkingVariant`.

**Sessions**: `availableSessions`, `sessionsLoading`, `sessionTitles`.

**Subagents**: `subagents: Map<string, SubagentInfo>`.

**Actions**: `onProviderEvent`, `onToolStart`, `onToolEnd`, `onToolProgress`,
`onPartUpdated`, `onPermissionRequest`, `resolvePermission`, `onError`, `onSlashResult`,
`setAvailableModels`, `setCurrentModel`, `onSessionHistory`, `onSessionCleared`, etc.

### 6.5 The Bridge Function: bridgeClientToStore

Show from `session.ts:735-809`:
```typescript
export function bridgeClientToStore(client: MewClient) {
    const store = useSessionStore;
    client.on("open", () => store.getState().setConnectionState("connected"));
    client.on("close", () => store.getState().setConnectionState("disconnected"));
    client.on("session-ready", (data) => {
        store.getState().setSessionId(data.session_id);
        if (data.model && data.provider) {
            store.getState().setCurrentModel(data.provider, data.model);
        }
    });
    client.on("provider", (ev) => store.getState().onProviderEvent(ev));
    client.on("tool-start", (data) => store.getState().onToolStart(data.call_id));
    client.on("tool-end", (data) => store.getState().onToolEnd(data.call_id, data.success));
    client.on("permission-request", (req, respond) => {
        store.getState().onPermissionRequest({
            requestId: req.request_id,
            toolName: req.tool_name,
            input: req.input,
        });
        permissionResponders.set(req.request_id, respond);
    });
    // ... 15+ more event wirings
}
```

The `permissionResponders` side-channel map (`session.ts:813`):
```typescript
export const permissionResponders = new Map<number, (decision: PermissionDecision) => void>();
```
When the UI calls `handlePermission(requestId, decision)`, it looks up the responder
and calls it — the client then sends `ClientMessage::PermissionResponse`.

### 6.6 App.tsx: Root Component

Show from `mew-web-ui/src/App.tsx`:

**Connection lifecycle** (`App.tsx:36-97`):
- `doConnect` with exponential backoff: `Math.min(1000 * 2 ** attempt, 30000)`.
- On connect: re-attach to previous session from `localStorage` (`mew.sessionId` key).
  If session no longer exists, create a new one.
- On unexpected disconnect: increment attempt, reconnect.
- On unmount: intentional disconnect, clear timer.

**Layout** (`App.tsx:126-152`):
```tsx
<ErrorBoundary title="App crashed">
  <div className="flex h-screen flex-col bg-background text-foreground">
    <TopBar connectionState={connectionState} client={clientRef.current} />
    <div className="flex flex-1 overflow-hidden">
      <SessionRail client={clientRef.current} collapsed={railCollapsed} />
      <main className="flex flex-1 flex-col overflow-hidden">
        <ChatSurface />
        <TodoPanel />
        <SubagentPanel />
        <AskUserCard />
        <InputArea onSend={handleSend} onCancel={handleCancel} connected={connected} />
      </main>
    </div>
    <StatusFooter />
    <PermissionToast onResolve={handlePermission} />
  </div>
</ErrorBoundary>
```

**Send flow** (`App.tsx:107-111`):
```typescript
const handleSend = (text: string) => {
    const store = useSessionStore.getState();
    store.addUserMessage(text);
    clientRef.current?.prompt(text);
};
```

**Permission resolution** (`App.tsx:117-124`):
```typescript
const handlePermission = (requestId: number, decision: "allow_once" | "allow_session" | "deny") => {
    const respond = permissionResponders.get(requestId);
    if (respond) {
        respond(decision);
        permissionResponders.delete(requestId);
    }
    useSessionStore.getState().resolvePermission(requestId);
};
```

### 6.7 Dev Workflow

```sh
just dev-ui           # Vite dev server with HMR (proxies WS to bridge)
just dev-web --spawn false  # start bridge separately, Vite proxies to it
just build-web        # build everything (Rust + TS), embed dist into bridge
just clean-web        # remove build artifacts + Vite cache
```

In dev mode, Vite serves on `:5173` and proxies `/ws` to the bridge at `127.0.0.1:9847`.
The WS URL is computed dynamically (`App.tsx:16-21`):
```typescript
const WS_URL = (() => {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    return `${proto}://${location.host}/ws`;
})();
```

### 6.8 Adding a New Wire Event End-to-End (expanded)

1. **Protocol**: Add `ClientMessage` or `ServerMessage` variant + roundtrip test in
   `mew-protocol/src/lib.rs`.
2. **Daemon**: Handle in `handle_connection`, update `translate_event`.
3. **TS client**: Add type to `ServerMessage`, add dispatch case in `dispatch()`,
   add event name to `MewClientEvents`, add emit call.
4. **Store**: Add state field to `SessionState`, add action, wire in `bridgeClientToStore`.
5. **Component**: Use the new state in a React component.

### 6.9 Key Components Table (expanded)

| Component | Purpose | Key State |
|-----------|---------|------------|
| `App.tsx` | Root layout, connection lifecycle, session reattach | `connectionState`, `clientRef` |
| `ChatSurface.tsx` | Message list + streaming text | `messages`, `streamingText` |
| `InputArea.tsx` | Prompt input, Cmd/Ctrl+Enter to send | (local state) |
| `TopBar.tsx` | Model picker, connection status, session menu | `availableModels`, `currentModel` |
| `PermissionToast.tsx` | Permission approval modal | `pendingPermissions` |
| `SessionRail.tsx` | Session list sidebar | `availableSessions` |
| `SubagentPanel.tsx` | Running subagent display | `subagents` |
| `AskUserCard.tsx` | Ask-user question overlay | `pendingAskUser` |
| `TodoPanel.tsx` | Todo list sidebar | `todos` |
| `StatusFooter.tsx` | Cost + token footer | `totalInputTokens`, `totalCost` |
