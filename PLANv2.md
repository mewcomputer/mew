# mew — implementation plan (rust)

a terminal agent harness in rust, similar to claude code. tui built on ratatui + crossterm. provider-agnostic via models.dev catalog. openai-shape and anthropic-shape adapters out of the gate, targeting opencode zen and z.ai's coding plan respectively. designed to grow into an acp (agent client protocol, zed-flavored) frontend in both directions.

this document is the source of truth for scope and architecture. milestones are sequenced; do not skip ahead. each milestone has an explicit "done when" — meet it before moving on.

---

## non-goals, at the moment (read first)

- not a claude code clone in features. parity with the tool-use loop, tui polish, and permission model is the bar. fancy session branching, ide integrations beyond acp, and team/cloud features are out of scope.
- terminal only, for now. web version is a maybe but not planned for v1. design with a clean separation between core and tui to keep that door open (likely already doing this for ACP).
- tools are either built-in (in-tree rust) or mcp. a general plugin system (hooks beyond tool registration) is deferred to m7. hook *points* are defined in the agent loop from m0 onward so adding the runtime later isn't a refactor.
- no multi-tenant server mode. single user, local-first. acp is the only remoting story.
- no telemetry, ever. no analytics, no crash reporting phoning home, no usage pings. logs are local-only. if this changes it requires explicit user opt-in and a config flag, never a default.
- image *input* is supported, voice input and tts output is a maybe but probably not. focus on text and images having the best experience.

---

## architecture

three layers, hard boundaries. even when everything runs in one process, the boundaries are real rust traits and the wire format between them is the canonical message/event types. this is what lets the tui later talk to a remote agent over acp without a rewrite.

```
┌─────────────────────────────────────────────────┐
│ tui (ratatui + crossterm)                       │  crates/mew-tui (bin)
│   subscribes to agent event stream              │
│   sends user prompts + permission decisions     │
└──────────────────┬──────────────────────────────┘
                   │ canonical Event stream (Stream<Item=Event>)
                   │ in-process today, acp jsonrpc later
┌──────────────────▼──────────────────────────────┐
│ agent core                                      │  crates/mew-agent
│   - conversation state (parts-based messages)   │
│   - tool registry + permission gate             │
│   - context/compaction                          │
│   - router (auto mode)                          │
└──────────────────┬──────────────────────────────┘
                   │ Provider trait
┌──────────────────▼──────────────────────────────┐
│ provider adapters                               │  crates/mew-provider-*
│   sse parsing, tool-call accumulation,          │
│   translation to canonical events               │
└─────────────────────────────────────────────────┘
```

### workspace layout

cargo workspace, one crate per concern. small crates compile independently and let you publish/embed pieces if mew ever grows beyond a single binary.

```
Cargo.toml                # workspace root
crates/
  mew/                    # the binary (cmd/mew). thin; wires everything.
  mew-message/            # canonical Message + Part types, serde, events
  mew-provider/           # Provider trait + Event types
  mew-provider-openai/    # openai-shape adapter
  mew-provider-anthropic/ # anthropic-shape adapter
  mew-provider-router/    # auto-mode router (m5)
  mew-provider-fake/      # scripted provider for tests
  mew-agent/              # the loop, state, compaction
  mew-tools/              # built-in tools + Tool trait + permission gate
  mew-mcp/                # mcp client (stdio + http)
  mew-acp/                # acp client and server (m6), generic over Transport
  mew-acp-iroh/           # iroh transport for acp (m8, feature-flagged)
  mew-config/             # config + creds (keychain + json fallback)
  mew-session/            # jsonl session persistence
  mew-catalog/            # models.dev catalog loader + cache
  mew-context/            # AGENTS.md / CLAUDE.md project context loader
  mew-skills/             # SKILL.md discovery + loading
  mew-subagents/          # subagent definitions, execution model (m9)
  mew-plugins/            # native plugin format: discovery, install, registration (m10)
  mew-plugins-cc/         # claude code plugin compatibility adapter (m11, optional)
  mew-hooks/              # Dispatcher trait; nop impl through m6, runtime in m7
  mew-tui/                # tui rendering + input (the bulk of the binary lives here as a lib)
```

`mew` (binary) is intentionally tiny — it parses argv, loads config, picks a runtime mode (`run` / interactive / `acp`), and wires the layers. all logic is in libraries so they're independently testable and reusable.

### runtime: tokio

one tokio runtime per process, multi-threaded. async everywhere except pure-cpu code (parsing, serde). reasoning:

- streaming sse, subprocess i/o, tui input events, and provider requests are all i/o-bound and naturally concurrent
- mcp stdio servers are subprocesses you must drive concurrently with the agent loop
- acp jsonrpc is async by nature
- ratatui is sync-rendering but you read crossterm events into a channel and the render loop drives off async

dependencies pinned, not floated. workspace `Cargo.toml` declares versions; member crates use `workspace = true`. every dep gets a one-line justification in a comment when added; pruning unused deps is part of merge review.

---

## canonical message and event types

cribbed in shape from opencode's MessageV2, native rust idiom. messages are metadata + an ordered list of parts. parts are a tagged enum dispatched on a `type` field via serde's internal tagging. tool calls and tool results are separate parts (do not fuse them) so a tool can render as "running" before its result exists.

### the simplification rust gets you

in go, the discriminated union required a custom `UnmarshalJSON` on `Vec<Part>` that peeked at the type tag and dispatched. in rust this is one derive:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text(TextPart),
    Reasoning(ReasoningPart),
    File(FilePart),
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
    Compaction(CompactionPart),
}
```

this is one of the main reasons rust pulls its weight on this project. the rest of the type system follows the same pattern.

### messages

```rust
use serde::{Serialize, Deserialize};
use ulid::Ulid;

pub type MessageId = Ulid;
pub type SessionId = Ulid;
pub type PartId = Ulid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role { User, Assistant }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    pub role: Role,
    pub parts: Vec<Part>,
    pub time: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant: Option<AssistantMeta>, // None on user messages
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Time {
    pub created: i64, // unix ms
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMeta {
    pub provider_id: String,
    pub model_id: String,
    pub cost: f64,
    pub tokens: Tokens,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<Finish>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<MessageError>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u32,
    pub output: u32,
    pub reasoning: u32,
    pub cache_read: u32,
    pub cache_write: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish { Stop, Length, ToolUse, Error }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    ProviderAuth,
    ProviderRateLimit,
    ProviderOverload,
    ProviderApi,
    ContextOverflow,
    Aborted,
    ToolExec,
    ToolTimeout,
    McpTransport,
    AcpProtocol,
    Network,
    Unknown,
}
```

### parts

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text(TextPart),
    Reasoning(ReasoningPart),
    File(FilePart),
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
    Compaction(CompactionPart),
}

impl Part {
    pub fn id(&self) -> PartId { /* match arm returning self.id of the inner */ }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartBase {
    pub id: PartId,
    pub message_id: MessageId,
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    #[serde(flatten)] pub base: PartBase,
    pub text: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningPart {
    #[serde(flatten)] pub base: PartBase,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>, // anthropic signed reasoning, opaque
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePart {
    #[serde(flatten)] pub base: PartBase,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub url: String, // file://, data:, or https://
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallPart {
    #[serde(flatten)] pub base: PartBase,
    pub tool_name: String,
    pub call_id: String, // provider's id, for threading results
    pub state: ToolState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultPart {
    #[serde(flatten)] pub base: PartBase,
    pub call_id: String,
    // result lives on the matching ToolCallPart's terminal state.
    // this part is an ordered marker for "result delivered."
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPart {
    #[serde(flatten)] pub base: PartBase,
    pub auto: bool,
    #[serde(default)]
    pub overflow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_start_id: Option<MessageId>,
}
```

### tool state machine

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolState {
    Pending(ToolStatePending),
    Running(ToolStateRunning),
    Completed(ToolStateCompleted),
    Error(ToolStateError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatePending {
    pub input: serde_json::Value, // partial; args still streaming
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStateRunning {
    pub input: serde_json::Value,
    #[serde(default)] pub output: String, // appended-to as bash etc. streams
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStateCompleted {
    pub input: serde_json::Value,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub time: ToolTime, // start + end
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStateError {
    pub input: serde_json::Value,
    pub error: String,
    pub time: ToolTime,
}
```

state transitions: pending → running → (completed | error). always mutate the same `ToolCallPart.state`. agent core emits `Event::PartUpdated` after each transition.

### events (the wire format between layers)

```rust
#[derive(Debug, Clone)]
pub enum ProviderEvent {
    PartStart { part: Part },
    PartDelta { part_id: PartId, field: &'static str, delta: String },
    PartEnd   { part_id: PartId },
    MessageEnd { finish: Finish, usage: Tokens, cost: f64 },
    Error(MessageError),
}
```

agent core emits a superset to the tui — adds permission requests, tool execution events, session lifecycle. that type lives in `mew-agent`, not `mew-provider`.

```rust
// in mew-agent
#[derive(Debug, Clone)]
pub enum AgentEvent {
    Provider(ProviderEvent),
    PermissionRequest { call: ToolCall, tx: oneshot::Sender<PermissionDecision> },
    ToolStart { call_id: String },
    ToolEnd   { call_id: String, success: bool },
    SessionUpdate(SessionUpdate),
}
```

note the `oneshot::Sender` pattern for permission requests: the tui receives the event, presents the prompt, and sends the user's decision back through the channel. clean async dialogue without polluting the agent state.

---

## provider trait

```rust
use futures::Stream;
use std::pin::Pin;

pub type EventStream = Pin<Box<dyn Stream<Item = ProviderEvent> + Send>>;

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError>;
}

pub struct Request {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub system: String,
    // provider-specific knobs are typed Options on construction, never a HashMap<String, Value>
}

pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value, // jsonschema
}
```

adapters fully consume the sse stream and emit canonical events. they own:

- sse / chunked-json parsing (use `eventsource-stream` for sse, manual chunking for anthropic's named events)
- accumulating tool-call argument deltas (openai streams `arguments` as a string-concatenation across chunks; anthropic streams `input_json_delta` blocks under `tool_use` content)
- mapping provider finish reasons to `Finish`
- mapping http and parse errors to `ErrorKind`
- never leaking provider-specific types past the `EventStream`

http: `reqwest` with `rustls-tls`, no openssl. one client per provider, kept-alive.

### image input

`FilePart` carries images alongside text files; `mime` determines rendering. capability comes from the catalog: each model has a `vision: bool` derived from models.dev. when a user attaches an image and the active model has `vision: false`, the agent core rejects the turn before sending with a clear error rather than letting the provider 400.

adapter responsibilities:

- **openai shape**: encode `FilePart` with image mime as `{type: "image_url", image_url: {url: "data:..."}}` content blocks alongside text content in the user message.
- **anthropic shape**: encode as `{type: "image", source: {type: "base64", media_type: ..., data: ...}}` blocks.

source resolution: `FilePart.url` may be `file://` (read from disk and base64), `data:` (pass through), or `https://` (pass through if provider accepts urls, else fetch and base64; openai accepts urls, anthropic always inlines).

size limits: cap at 10mb per image after base64 encoding. larger gets rejected at the agent core, never sent. images are *not* compacted; if context pressure requires dropping turns, drop them whole including their images, never strip images from a kept turn.

image input lands in m1 (so both shapes are exercised), not m0. m0 covers text-only round-trips.

---

## the agent loop

```text
loop:
  hooks.on_chat_message(msg).await        ← may rewrite outbound message
  hooks.on_chat_params(params).await      ← may rewrite temp/topP/maxTokens
  hooks.on_chat_headers(headers).await    ← may inject http headers
  let stream = provider.stream(req).await?
  while let Some(ev) = stream.next().await:
    update in-memory message
    hooks.on_event(&ev).await             ← observers only, no rewrite
    forward (translated) event to tui
  on MessageEnd:
    if assistant message has tool_call parts in pending/running terminal state:
      for each:
        decision = check permissions
        decision = hooks.on_permission_ask(&call, decision).await   ← override
        if decision == prompt: ask user via oneshot channel
        input = hooks.on_tool_execute_before(&call, input).await    ← rewrite args
        execute tool (built-in or mcp)
        output = hooks.on_tool_execute_after(&call, output).await   ← rewrite result
        update ToolCallPart.state
        emit PartUpdated
      append tool results as a new user message (anthropic shape) or
        role: tool messages (openai shape) — handled by adapter, agent
        works in canonical shape only
      goto loop
    else:
      done
```

every `hooks.*` call is a no-op in the default `NopDispatcher` shipped through m6. the dispatcher trait is wired through the loop from m0 so adding a real runtime in m7 doesn't touch loop code.

context management: track total tokens against the model's context window (from catalog). when above threshold (start 80%), insert a `CompactionPart` and summarize prior turns via a cheap model call. preserve the system prompt, the most recent k turns verbatim, and a synthesized summary of the rest.

abort: a `tokio_util::sync::CancellationToken` propagates through the loop. on cancel mid-stream, the in-progress message is marked `ErrorKind::Aborted` and persisted with whatever was streamed so far.

### hook points (defined now, runtime deferred to m7)

```rust
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync {
    async fn on_event(&self, ev: &AgentEvent);

    async fn on_chat_message(&self, msg: Message) -> Message;
    async fn on_chat_params(&self, p: ChatParams) -> ChatParams;
    async fn on_chat_headers(&self, h: HeaderMap) -> HeaderMap;
    async fn on_tool_execute_before(&self, call: &ToolCall, input: Value) -> Value;
    async fn on_tool_execute_after(&self, call: &ToolCall, output: ToolOutput) -> ToolOutput;
    async fn on_permission_ask(&self, call: &ToolCall, current: PermissionDecision) -> PermissionDecision;
    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String>;
}

pub struct NopDispatcher;

#[async_trait::async_trait]
impl Dispatcher for NopDispatcher {
    async fn on_event(&self, _: &AgentEvent) {}
    async fn on_chat_message(&self, msg: Message) -> Message { msg }
    // etc.
}
```

omitted from opencode's set on purpose:

- their `tool` hook (plugin registers a tool) — already covered by mcp and the in-tree `Tool` trait. plugins that want tools implement `Tool` directly.
- their `auth` hook — defer until mew supports oauth flows; byok covers everything until then.
- their `command.execute.before` — slash commands aren't a v1 plugin concern beyond fixed built-ins.

---

## tools

### Tool trait

```rust
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;
    fn sensitivity(&self) -> Sensitivity;

    async fn execute(
        &self,
        ctx: ToolCtx,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError>;
}

pub struct ToolCtx {
    pub session_id: SessionId,
    pub call_id: String,
    pub cancel: CancellationToken,
    pub progress_tx: mpsc::Sender<ToolProgress>, // for streaming output (e.g. bash)
    pub cwd: PathBuf,
}

pub enum ToolProgress {
    OutputChunk(String),
    Metadata(serde_json::Value),
}

pub enum Sensitivity { ReadOnly, Mutating, Dangerous }
```

### built-in tool set (m3)

minimum viable, modeled on claude code's:

- `read` — read file (path, optional offset/limit). returns text + metadata. ReadOnly.
- `write` — create or overwrite file (path, content). Mutating, always permission-gated.
- `edit` — find/replace in file (path, old_string, new_string). exact match required, fails on ambiguity. Mutating.
- `bash` — execute shell command (command, optional timeout). Dangerous, always permission-gated. uses `tokio::process::Command`. captures stdout+stderr concurrently via `tokio::io::AsyncBufReadExt`. streams chunks through `progress_tx` for live tui display. truncates the captured-for-session output at 30k chars (configurable); the live stream is uncapped.
- `glob` — pattern match files (pattern, optional path). uses `globset` + walking. ReadOnly. returns sorted by mtime desc.
- `grep` — content search (pattern, optional path, optional glob). prefers `ripgrep` binary if on path, falls back to `grep` crate. ReadOnly.

### permission model

three decisions: `AllowOnce`, `AllowSession`, `Deny`. plus rule-based pre-allows from config:

```toml
[[permissions.rules]]
tool = "bash"
match.command_prefix = "git status"
decision = "allow"

[[permissions.rules]]
tool = "read"
match.path_glob = "**/*.rs"
decision = "allow"

[[permissions.rules]]
tool = "write"
match.path_glob = "/etc/**"
decision = "deny"
```

evaluation order: explicit deny rules → explicit allow rules → session allows → prompt user. read-only tools default to allow; mutating and dangerous prompt unless allowed by rule.

path matching: `globset` for glob patterns. all paths normalized to absolute before matching.

### mcp client (m4)

stdio and streamable-http transports. servers configured in `config.toml`:

```toml
[mcp.servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]

[mcp.servers.github]
transport = "http"
url = "https://..."
headers = { Authorization = "Bearer ..." }
```

mcp tools registered alongside built-ins, namespaced as `mcp__<server>__<tool>`. mcp tool calls flow through the same permission gate. sensitivity defaults to `Mutating` unless the server declares otherwise.

implementation notes specific to rust:

- `rmcp` (the official rust mcp sdk) is the right starting point if it's mature enough by m4. evaluate at m4 start. if not, hand-rolled jsonrpc over `tokio::process::Child` stdio + `reqwest` for http is a few hundred lines.
- mcp servers as subprocesses: own them with a per-session task. on session end, send shutdown notification, wait briefly, then `kill_on_drop` cleans up. orphan prevention is non-negotiable.

---

## providers and the catalog

### models.dev integration

at startup, fetch `https://models.dev/api.json` and cache locally with etag, refresh on 24h. this gives:

- list of models with `provider`, `id`, `context_window`, `max_output`, `tool_call: bool`, `vision: bool`, `reasoning: bool`, pricing.
- per-model "shape": which adapter to use.

mew's config selects a model by id. catalog lookup determines the adapter and capabilities. unknown models can be configured with explicit `shape = "openai" | "anthropic"` and `context_window` overrides.

### opencode zen (openai-shape)

base url: `https://opencode.ai/zen/v1` (verify on first connect). standard openai chat completions with tool calling. byok via `OPENCODE_ZEN_API_KEY`.

### z.ai coding plan (anthropic-shape)

base url: `https://api.z.ai/api/anthropic`. anthropic messages api shape. byok via `ZAI_API_KEY`. coding plan currently centers on glm-4.6.

### auto mode (m5)

a `Provider` impl that wraps multiple inner providers and routes per request. v1 heuristic:

- if last message has tool results, or message count > N → "big" model
- if conversation has only short user prompts and no tool use → "small" model
- explicit hint via system metadata can override

```toml
[providers.auto]
kind = "router"
small = "z-ai/glm-4.5-air"
big = "z-ai/glm-4.6"
```

router is itself a `Provider` — the tui and agent core never know they're talking to a router.

---

## config and credentials

### locations

- `directories::ProjectDirs::from("ai", "mew", "mew").config_dir()` for config:
  - linux: `~/.config/mew/`
  - macos: `~/Library/Application Support/ai.mew.mew/`
  - windows: `%AppData%\mew\mew\config\`
- `config.toml` — non-secret config (toml chosen over json: comments, multi-line strings, friendlier for hand editing)
- `credentials.json` — fallback for secrets if keychain unavailable, mode 0600 (use `nix::sys::stat` on unix, accept windows is best-effort)
- `sessions/` — jsonl per session
- `logs/` — structured tracing output

### keychain

use `keyring` crate (cross-platform: macos keychain, windows credential manager, linux secret service / kwallet). on first run, probe with sentinel write/read/delete. if it fails (common on headless linux), fall back to `credentials.json` and *log a warning at startup*. never silently store secrets in plaintext.

credentials keyed by provider id:

```toml
[providers.opencode-zen]
shape = "openai"
base_url = "https://opencode.ai/zen/v1"
credential_ref = "opencode-zen"

[providers.z-ai]
shape = "anthropic"
base_url = "https://api.z.ai/api/anthropic"
credential_ref = "z-ai"

default_model = "z-ai/glm-4.6"
```

`credential_ref` resolves in order: keychain → `credentials.json` → env var `MEW_CRED_<REF_UPPERCASED>`.

cli for cred management: `mew auth set <provider_id>` (prompts for key, writes to keychain or fallback), `mew auth list`, `mew auth remove <provider_id>`.

---

## sessions

### format

one jsonl file per session: `sessions/<session_id>.jsonl`. one line per `Message`. resume by reading the file. session id is a ulid; sessions listable/sortable by creation time.

a small `sessions/index.json` tracks `{id, created, title, last_model}` for fast tui listing. rebuilt on demand if missing.

### structured logging

separate from session files. use `tracing` + `tracing-subscriber` with json output to `logs/<session_id>.jsonl`. log every:

- provider request (sanitized — strip api keys from echoed headers)
- provider response chunk metadata (not full content; that's in the session)
- tool execution start/end with duration
- permission decisions
- errors with backtrace where caught via `anyhow`

debugging without these is hell. they're not optional. `RUST_LOG` controls verbosity; default `info`.

---

## acp (m6)

zed's agent client protocol. jsonrpc 2.0 over stdio. spec: <https://agentclientprotocol.com>.

### two modes

**(a) mew tui as acp client.** tui spawns claude-code-acp (or any acp agent) as a subprocess and drives it. agent core bypassed; tui talks acp directly. enabled with `mew --acp-agent claude-code`.

implementation: the tui already consumes a canonical event stream. an `AcpClient` translates acp `session/update` notifications into those events. user input becomes acp `session/prompt`. permission requests come back as acp `session/request_permission`, render the same approval ui, send response.

**(b) mew agent core as acp server.** `mew acp` subcommand starts mew's agent core speaking acp over stdio. zed/nvim/etc. drive mew. translate inbound acp methods to agent operations; emit acp notifications for our event stream.

both modes share translator code in `mew-acp`.

### slash commands and prompts

acp 2025 added slash commands via the agent. mew's built-ins (`/compact`, `/clear`, `/cost`) exposed via the slash-command extension when running as a server.

### jsonrpc plumbing

`jsonrpsee` is overkill for stdio jsonrpc — use it only if its server traits are useful for the acp server mode. for the client side, hand-rolled jsonrpc (a `LineCodec` over stdio + a request/response correlator) is roughly 200 lines and gives better control. evaluate at m6 start.

---

## milestones, sequenced

each milestone has a "done when" gate. do not advance past a gate unless all bullets are satisfied.

### m0: skeleton + openai adapter + opencode zen

- `mew-message`: types above, serde derives, round-trip tests with table-driven fixtures (text-only, with reasoning, with tool calls in each state, multi-part). proptest-based fuzz round-trip.
- `mew-provider`: trait + event types.
- `mew-provider-openai`: sse parsing via `eventsource-stream`, tool-call argument accumulation across chunks, mapping to canonical events. tests use captured sse fixtures (record real responses to `testdata/`, replay in tests).
- `mew-provider-fake`: scripted provider for agent-loop tests.
- `mew-agent`: minimal loop. one fake echo tool registered for testing.
- `mew-hooks`: `Dispatcher` trait + `NopDispatcher`. agent core takes `Arc<dyn Dispatcher>` in its constructor; wire the nop. hook calls placed at every point shown in the loop pseudocode.
- `mew-config`: config loading, env var creds only at this stage.
- `mew-context`: project context file loader. on session start, walk from cwd up to git root (or $HOME), collect `AGENTS.md`, `CLAUDE.md`, `.mew/AGENTS.md` along the way; also load `~/.config/mew/AGENTS.md`. concatenate in order (most-general first), prepend to system prompt with `<context source="<path>">...</context>` framing. all four common filenames supported for cross-tool interop.
- `mew-session`: jsonl writer, no resume yet.
- `mew run "<prompt>"`: non-interactive. streams text to stdout, executes the fake tool when called, feeds results back, terminates on stop. exits 0.

done when: `MEW_CRED_OPENCODE_ZEN=... mew run "list three primes then call the echo tool with input='hi'"` produces correct streaming output, the echo tool runs, the session file under `sessions/` round-trips through `mew-message` types cleanly, and provider tests pass against recorded fixtures.

### m1: anthropic adapter + z.ai

- `mew-provider-anthropic`: sse parsing for anthropic messages api, including `input_json_delta` accumulation, `thinking`/`reasoning` blocks → `ReasoningPart`, signed reasoning round-trip.
- `mew-catalog`: reads models.dev, picks adapter by model id.
- multi-provider config; `mew run --model <id>` chooses.
- canonical message ↔ wire format translation lives in adapters; agent core never sees provider-specific data.
- image input support per "image input" section. both adapters encode `FilePart` images correctly. catalog `vision` flag gates pre-send.
- rate limit handling: on 429 from either provider, retry with exponential backoff (start 1s, cap 30s, max 4 attempts). on other 5xx, retry once. on persistent failure, emit `ProviderEvent::Error` with `ErrorKind::ProviderRateLimit` or `ProviderApi`; agent core marks the turn errored, keeps the partial assistant message, lets the tui prompt the user for retry. never silently drop a turn.

done when: same `mew run` works against `z-ai/glm-4.6` and against opencode zen, switched only by `--model`. captured anthropic fixture replays correctly including a tool-call turn with reasoning. image input works end-to-end on both shapes against a vision-capable model. a synthetic 429 in fixture replay triggers correct backoff.

### m2: ratatui tui

- `mew-tui` library + `mew` interactive mode (`mew` with no subcommand).
- ratatui-based rendering, crossterm input. event loop reads `crossterm::event::EventStream`, agent events, and tick events through `tokio::select!`.
- streaming token rendering, tool-call display reflecting state machine (spinner on running, output collapsed by default on completed, red on error).
- approval prompts: modal overlay, three options (allow once, allow session, deny). keyboard-driven. driven by the `oneshot::Sender` pattern from `AgentEvent::PermissionRequest`.
- input editor with multiline support, history (up/down). use `tui-textarea` or hand-roll. default to readline-style keys. one chosen, stick with it.
- status line: model, session token usage, session cost.
- keychain creds via `mew auth set` cli plus json fallback.
- **slash commands.** built-in command registry, dispatched when input starts with `/`. v1 set: `/help`, `/clear` (start new session), `/compact` (force compaction), `/cost` (full breakdown), `/model <id>` (switch mid-session), `/sessions` (list resumable), `/resume <id>`, `/quit`. registry is a `HashMap<String, Box<dyn Command>>` so plugins (m7) can add commands later. unknown `/foo` falls through to the model as literal text — don't error.
- **@-mentions for files.** typing `@` opens a fuzzy file picker rooted at the project (or git root). selecting inserts `@path/to/file` into the prompt; on submit, agent core reads the file and attaches as `FilePart` (text mime → text content, image mime → image content per image-input rules). respects `.gitignore` via the `ignore` crate. files over 1mb prompt for confirmation.
- **interrupt / cancellation.** ctrl-c during a streaming turn cancels via the cancellation token, marks the in-progress assistant message `Aborted`, preserves what was streamed, returns input focus. ctrl-c with no active turn clears non-empty input, otherwise prompts "exit? (y/n)". double ctrl-c within 1s exits unconditionally. ctrl-d on empty input exits.
- **bash output streaming.** bash tool streams stdout/stderr to the tui via `progress_tx` as bytes arrive. tui renders a live-updating tool block. `ToolStateRunning.output` appended-to; `Event::PartUpdated` emitted on a 50ms debounce to avoid flooding. truncation cap (30k chars default) applies to the final captured output written to session, not the live stream.
- **diff display for edits.** when `edit` runs (and `write` to existing file), tui renders unified diff: additions green, removals red, context dim. `similar` crate. raw before/after still in `ToolState.output` for session record; diff is purely render.
- **cost surfacing.** session cost = sum of per-message costs (input × input_price + output × output_price + cache adjustments, prices from catalog). status line shows running total. `/cost` shows full breakdown: per-model totals if session crossed models, per-turn list, cache hit ratio. costs persisted on each `Message` so resume reconstructs accurately. if catalog pricing missing for a model, mark cost as `None` and note "cost unavailable" — never silently zero.

done when: a user can `mew auth set z-ai`, then `mew`, hold a multi-turn conversation with tool calls and approvals, see streaming output and tool execution states update live, attach files via `@`, run a long bash command and watch its output stream, ctrl-c a long-running turn cleanly, run `/cost` and see a sane breakdown, and resume the session next launch via `mew --session <id>` or `mew --resume`.

### m3: built-in tools, permission model, skills

- read, write, edit, bash, glob, grep, all implementing `Tool`.
- permission rules in config, evaluated in documented order.
- session-allow tracked in memory only; never persisted.
- bash tool: configurable timeout (default 2 min), output truncation (default 30k), env passthrough policy.
- `mew-skills`: discovery + loading. on session start, scan in this order, load all matches:
  - `<project>/.mew/skills/<name>/SKILL.md`
  - `<project>/.opencode/skills/<name>/SKILL.md`
  - `<project>/.claude/skills/<name>/SKILL.md`
  - `<project>/.agents/skills/<name>/SKILL.md`
  - `~/.config/mew/skills/<name>/SKILL.md`
  - `~/.config/opencode/skills/<name>/SKILL.md`
  - `~/.claude/skills/<name>/SKILL.md`
  - `~/.agents/skills/<name>/SKILL.md`

  walk project paths from cwd up to git worktree root. parse yaml frontmatter (`name` required, `description` required, `license`/`compatibility`/`metadata` optional) via `serde_yaml`. validate name against `^[a-z0-9]+(-[a-z0-9]+)*$`, 1-64 chars, must equal directory name. duplicate names: project beats global; within a tier, mew's own paths beat compat paths. log conflicts.
- `skill` built-in tool: takes `name`, returns the full markdown body. listing of available `<name, description>` pairs injected into the tool's description as xml.
- skill permissions in config, sharing rule engine with tools but discriminated by kind:
  ```toml
  [[permissions.skills]]
  match.name_glob = "internal-*"
  decision = "deny"

  [[permissions.skills]]
  match.name_glob = "experimental-*"
  decision = "ask"

  [[permissions.skills]]
  match.name_glob = "*"
  decision = "allow"
  ```
  evaluated top-to-bottom, first match wins. `ask` uses the same approval ui as tool permissions.

done when: a user can ask "find all rust files importing reqwest and add a comment to each", mew uses glob+grep+edit, prompts for each edit (or batches under session-allow rule), and the session file accurately reflects the tool state machine throughout. a `git-release` skill at `.mew/skills/git-release/SKILL.md` shows up in the `skill` tool's listing, can be loaded by the model, and a `deny` rule for `internal-*` hides matching skills from the listing entirely.

### m4: mcp client

- stdio transport (subprocess + jsonrpc over stdio).
- streamable-http transport.
- server lifecycle: spawn on first use, keep alive for session, shut down on session end.
- tool namespacing `mcp__<server>__<tool>`, registered at agent startup after server handshake.
- mcp tool calls flow through the same permission gate. sensitivity defaults to `Mutating` unless server declares otherwise.

done when: configuring an mcp server in `config.toml` makes its tools available to the model, a tool call against it works end-to-end, and killing the session cleans up subprocesses without orphans (verify via `ps` or `pgrep` in a smoke test).

### m5: router / auto mode

- `mew-provider-router`: implements `Provider`, dispatches per-request.
- v1 heuristic as described.
- config schema for routers.
- `mew run --model auto` and tui model picker support.

done when: `--model auto` selects small for short single-turn prompts and big when tools are present, choice visible in logs and status line.

### m6: acp, both directions

- `mew-acp`: jsonrpc framing, message types, capability negotiation.
- transport abstracted behind a trait: `acp::Transport: AsyncRead + AsyncWrite + Send + 'static`. m6 ships one impl: `StdioTransport`. additional transports (m8) plug in here without touching protocol code.
- client mode: `mew --acp-agent <cmd>` spawns the agent, tui uses acp instead of internal agent core. transport is stdio of the spawned subprocess.
- server mode: `mew acp` exposes agent core over stdio.
- slash commands surfaced via acp.

done when: `mew --acp-agent "npx @zed-industries/claude-code-acp"` gives a working tui driven by claude code, *and* zed (or a minimal acp client used in tests) can drive `mew acp` for a multi-turn conversation including tool approvals. the protocol code in `mew-acp` is generic over `Transport` so m8 can add iroh without modifications to message handling.

### m7: plugin runtime (deferred — do not start until m6 ships)

implements `Dispatcher` with a real plugin runtime. hook *points* and *trait* are already in place from m0; this milestone is purely the execution substrate.

leading candidate: **wasmtime + the wasm component model**. reasoning specific to rust:

- wasmtime is the reference wasm runtime, native rust, first-class.
- the component model + wit gives you typed plugin interfaces (no manual ffi shimming). plugin authors write rust/js/python/go and compile to wasm components; mew imports them as if they were rust traits.
- wasi sandboxing covers filesystem, network, env vars by default — capability-based, you grant exactly what plugins need.
- this is meaningfully better tooling than anything in the go world.

second choice: subprocess + jsonrpc (mcp-style), reusing `mew-mcp` plumbing. simpler, slower, less expressive. only fall back to this if wasmtime + components has unfixable rough edges at m7 start.

scope:

- pick the runtime, document the decision in this file under "decisions made".
- implement `Dispatcher` backed by the runtime. preserve the contract: errors logged, never propagated; mutating hooks fall back to input on failure.
- plugin loading from `~/.config/mew/plugins/` and `<project>/.mew/plugins/` in that order, all loaded, hooks run in load order.
- a host api exposed via wit covering: read session messages, log, fetch http (gated), read config values. no filesystem or shell access by default; plugins needing those use mcp.
- a sample plugin in-tree demonstrating each hook (rust, compiled to wasm component).
- docs for plugin authors: which hooks exist, host api surface, examples in at least two source languages.

done when: a plugin loaded from `~/.config/mew/plugins/` can (a) inject a custom http header into provider requests, (b) deny a `bash` permission for commands matching a regex, (c) rewrite a tool's output to redact secrets — all observed end-to-end with mew running against a real provider, and the same plugin works unchanged when written in at least two languages compiling to wasm.

### m8: remote agent over iroh (stretch — feature-flagged)

run the agent on one machine, the tui on another. acp is the wire protocol; iroh is the transport. lets you put the agent on a beefy desktop or homelab box and drive it from a laptop, no port forwarding, no vpn, no public ip.

new crate: `mew-acp-iroh`, gated behind a `iroh` cargo feature. default builds do not include iroh or its transitive deps. nothing in the default binary changes.

scope:

- `IrohTransport` implementing `acp::Transport` over an iroh quic bidirectional stream. drops directly into the existing acp client and server.
- server: `mew acp --listen iroh` starts an iroh node, prints a connection ticket and a pairing code, awaits one client.
- client: `mew --acp-agent iroh://<ticket>` dials the node, prompts for the pairing code, runs pake handshake, runs acp on the resulting authenticated stream.

#### pairing and authentication

iroh gives you a quic connection encrypted to a known node id. that proves *which node* you reached. it does not prove *the human at each end agreed*. since the agent can run `bash` and write files, ticket-based-only access is unsafe — a leaked ticket is shell access.

solution: short numeric pairing code + pake. ux modeled on magic-wormhole.

```
server side:
  $ mew acp --listen iroh
  iroh node id: nodeabc123def456...
  ticket: <opaque ticket containing node id and relay hints>
  pairing code: 472-819
  (code expires in 60s; press <enter> to regenerate)
  waiting for client...

client side:
  $ mew --acp-agent iroh://<ticket>
  connecting to nodeabc123def456...
  pairing code: _
  > 472-819
  paired. starting session.
```

protocol:

- pairing code is 6 digits, displayed and accepted with optional hyphens (`472-819` or `472819` both work).
- both sides run **spake2** using the code as the shared low-entropy secret. neither side ever transmits the code. crate: `spake2`.
- spake2 yields a session key. server immediately sends a small encrypted "hello" via aead (chacha20poly1305) using that key; client decrypts and sends ack. failure on either side aborts the connection with an artificial 1-second delay before closing — prevents online brute force.
- code lifetime: 60 seconds, single-use. expired or used codes regenerate automatically.
- on success, the acp message stream runs over the iroh connection with the session key authenticating frames via aead.
- never reuse the iroh ticket as the secret. ticket = routing address, code = authorization. they ride different channels.

#### trust on first use

after a successful pairing, optionally persist the long-term derived key, scoped to (local node id, remote node id). subsequent connections from the same client to the same server skip the code prompt and use the saved key directly.

- prompt on first pairing: "remember this pairing? [y/N]"
- saved keys live in `~/.config/mew/acp-pairings.json` (mode 0600), keyed by remote node id.
- `mew acp pair list` shows saved pairings.
- `mew acp pair forget <node_id>` removes one.
- mismatch (saved key fails to authenticate) aborts the connection with a clear "remote key changed; remove with `pair forget` if expected" message.

#### permission ux for remote agents

when the remote agent requests permission for `bash` etc., the tui prompt makes the destination explicit:

```
permission request from remote agent (mew@nodeabc123)
  bash: rm -rf ./build
  this command will run on the REMOTE machine.
  [a]llow once  [s]ession  [d]eny
```

never elide the remote/local distinction. users approving a `bash` thinking it's their laptop when it's actually a remote box is the obvious footgun.

#### remote filesystem semantics

when mew runs as a remote acp server, `@-mention` of files refers to the *remote* filesystem. the tui's fuzzy file picker queries the remote agent over acp. acp has filesystem extensions (`fs/list`, `fs/read`); use them. if the spec lacks what you need at m8 time, define mew-specific extension methods under a `mew/` namespace and document them.

#### dependency weight

iroh + quinn + the rest is meaningful binary size (~few mb) and compile-time cost. feature-flagged at the crate level so default builds stay lean.

```toml
# crates/mew/Cargo.toml
[features]
default = []
iroh = ["dep:mew-acp-iroh"]
```

ci builds both with and without the feature.

#### what's explicitly out of scope at m8

- multi-client to one server (only one paired client at a time; later if needed).
- agent-to-agent communication. mew-as-server speaks to mew-as-client only.
- iroh's pubsub or doc primitives. transport-only use of iroh.
- discovery beyond what's in the ticket. no global node lookup, no dns over iroh, etc.

done when: on two machines (or two terminals on the same machine), `mew acp --listen iroh` on side a and `mew --acp-agent iroh://<ticket>` on side b complete a pairing, the client tui drives a multi-turn conversation including tool approvals against the agent core running on side a, files referenced by `@` resolve to side a's filesystem, and a saved pairing lets a subsequent reconnect skip the code prompt. an attacker presented with the ticket but not the code cannot complete pairing within reasonable attempt counts.

### m9: subagents

undefer this. `mew-message` already has space for a `SubtaskPart` (cribbed from opencode). m9 implements the execution model.

a subagent is a spawned sub-session: own conversation state, own model selection, own (inherited or restricted) tool set, own system prompt. the parent agent invokes a subagent via a built-in `subagent` tool, providing a name (referencing a defined subagent) and a prompt. the subagent runs to completion (with its own potentially nested tool calls) and returns a final text result that becomes the tool result back in the parent's conversation.

subagent definitions live in markdown files with yaml frontmatter:

```markdown
---
name: code-reviewer
description: Reviews code changes for issues
model: claude-sonnet-4-5  # or "inherit" for parent's model
tools: [read, glob, grep]  # restrict tool set; omit for full inherit
isolation: none            # or "worktree" for git worktree isolation
max_turns: 20
---

You are a code reviewer. Examine the provided changes and...
```

discovery: same multi-path approach as skills. `<project>/.mew/agents/<name>.md`, `<project>/.opencode/agents/<name>.md`, `<project>/.claude/agents/<name>.md`, plus globals.

scope:

- `mew-agent` gains a notion of nested sessions. each subagent invocation creates a child session sharing the parent's session id as a prefix, with its own jsonl file for transcript.
- the `subagent` built-in tool: takes `name` (string) and `prompt` (string), returns the subagent's final text response.
- tool inheritance: if subagent definition lists `tools`, only those are available; otherwise all parent tools available. permissions are *not* inherited — subagents prompt for their own approvals. (alternative: inherit session-allow rules from the parent. decide at m9 start; default to non-inheritance for safety.)
- model selection: explicit model in frontmatter wins; "inherit" or omitted means parent's current model.
- `isolation: worktree` (optional, deferred sub-feature within m9): create a git worktree, run the subagent's tool calls (especially writes) inside the worktree's cwd, abandon or merge the worktree at completion. only meaningful when the project is a git repo.
- max_turns enforces a hard cap; if hit, subagent returns "max turns exceeded" with whatever output it has.
- the parent's tui shows subagent execution as a collapsible nested block (similar to a tool call but with internal turn structure). cost and tokens roll up.

done when: a `code-reviewer` subagent defined at `.mew/agents/code-reviewer.md` can be invoked by the parent agent (either via direct user request "use the code-reviewer agent on this diff" or autonomously when the parent decides to delegate), runs in its own session with its restricted toolset, returns a result that becomes a tool result in the parent's conversation, and the full nested transcript is queryable via `mew sessions show <id>`. subagent execution surviving across `--resume` requires the child sessions to be saved alongside the parent.

### m10: native plugin format

mew gets its own plugin packaging story. a plugin is a directory bundling any combination of: skills, subagents, slash commands, hooks (for m7+), mcp server definitions. this is mostly *organizational* — the components themselves already work; m10 wraps them in a discoverable, installable package.

format chosen explicitly to be a **superset of claude code's** plugin layout, so m11's adapter is thin. cc's structure:

```
plugin-name/
├── .claude-plugin/plugin.json   ← cc's manifest location
├── commands/*.md                ← markdown slash commands
├── agents/*.md                  ← subagent definitions
├── skills/*/SKILL.md            ← skills
├── hooks/hooks.json             ← shell-based hooks
└── .mcp.json                    ← mcp server defs
```

mew's structure:

```
plugin-name/
├── mew-plugin.toml              ← mew's manifest (toml; cc's plugin.json also accepted)
├── commands/*.md                ← same as cc
├── agents/*.md                  ← same as cc
├── skills/*/SKILL.md            ← same as cc
├── hooks/                       ← may contain hooks.json (cc-style shell) OR *.wasm (mew-native)
│   ├── hooks.json
│   └── *.wasm
└── mcp-servers.toml             ← mew's mcp config (.mcp.json also accepted)
```

scope:

- `mew-plugin.toml` schema: name, version, description, author, license, optional component path overrides.
- discovery: `~/.config/mew/plugins/<name>/` and `<project>/.mew/plugins/<name>/`. each directory is a plugin if it contains either `mew-plugin.toml` or `.claude-plugin/plugin.json`.
- on session start: load all plugins; register their components (skills into the skills registry, agents into the subagents registry, slash commands into the command registry, mcp servers into the mcp client config, hooks into the dispatcher).
- markdown slash commands: a new `Command` impl `MarkdownCommand` that, when invoked, expands the markdown body (with argument substitution) into a user prompt sent to the model. cc-compatible.
- enable/disable: `mew plugin list`, `mew plugin enable <name>`, `mew plugin disable <name>`. disabled plugins skip registration. state persisted in config.
- install: `mew plugin install <git-url>` clones into `~/.config/mew/plugins/<name>/`. `mew plugin update <name>` git-pulls. nothing fancier; trust the user's git.

done when: a single git repo containing `mew-plugin.toml`, `commands/foo.md`, `skills/bar/SKILL.md`, and `agents/baz.md` can be installed via `mew plugin install <url>`, all components surface correctly (the slash command shows in `/help`, the skill in the `skill` tool listing, the subagent in subagent invocations), and `mew plugin disable` removes them from registration without uninstalling files.

### m11: claude code plugin compatibility (stretch)

an adapter that loads claude code plugins as if they were native mew plugins. given m10's superset format, this is largely a thin translation layer.

scope:

- recognize cc plugins by presence of `.claude-plugin/plugin.json`. m10's loader already accepts this as a plugin marker.
- read `plugin.json` and translate fields to mew's manifest internally.
- markdown commands and agents: identical format, no translation needed.
- skills: identical format, no translation needed (mew-skills already handles `.claude/skills/` paths).
- `.mcp.json` parser: convert cc's mcp server definition format to mew's mcp config. mostly a 1:1 field rename.
- `hooks/hooks.json` adapter: cc hooks are shell commands keyed by event name (PreToolUse, PostToolUse, etc.). a `ShellHookDispatcher` impl runs these via subprocess on matching events. cc's shell-script-with-stdin protocol gets reproduced. `${CLAUDE_PLUGIN_ROOT}` env var set to the plugin directory. event mapping:

  | cc event | mew hook |
  |---|---|
  | `PreToolUse` | `on_tool_execute_before` |
  | `PostToolUse` | `on_tool_execute_after` |
  | `UserPromptSubmit` | `on_chat_message` |
  | `SessionStart` / `SessionEnd` | session lifecycle events on `on_event` |
  | `PreCompact` | new event; emit before compaction runs |
  | `Stop` / `SubagentStop` | `on_event` filtered to the right kind |
  | `Notification` | tui-side, surface via the notification subsystem |

- tool name mapping: cc's tool names (`Write`, `Edit`, `Bash`, `Read`) map to mew's lowercase equivalents in matchers. document the mapping and the limits.

what's explicitly best-effort, not guaranteed:

- plugins that depend on cc-specific model strings (`claude-sonnet-4-5`) fail or require model substitution config.
- plugins that depend on cc-specific environment variables beyond `${CLAUDE_PLUGIN_ROOT}` may break.
- plugins that depend on internal cc behaviors (specific prompt formats, specific file paths) won't work.

set the bar: "cc plugins that are essentially packaged skills/agents/commands/mcp/hooks work; ones that depend on cc internals don't." we don't promise full compat. document a known-incompat list as plugins are tested.

done when: a representative sample of three popular cc plugins (one skill-heavy, one agent-heavy, one hook-heavy — pick from `plugins.claude.ai` top installs at m11 time) install via `mew plugin install <url>` and function correctly under mew. failures on more cc-internal-dependent plugins are documented but don't block the milestone.

---

## decisions made (don't relitigate)

- workspace of crates, not one big crate. small crates compile faster and let you publish/embed parts later.
- tokio multi-threaded runtime, async everywhere except pure-cpu code.
- ratatui + crossterm for tui. no other tui libs.
- reqwest + rustls (no openssl). serde + serde_json for json. serde_yaml for skill frontmatter. toml for config.
- creds: keychain first (`keyring` crate), json fallback, env var override. warn loudly on fallback.
- session format: jsonl of canonical messages, one file per session.
- streaming: `Provider::stream` returns a `Stream<Item = ProviderEvent>`. no non-streaming variant.
- tool calls and tool results are separate parts, never fused.
- discriminated unions via tagged `enum` + serde. no manual deserializer plumbing.
- no provider-specific types past the adapter boundary, ever.
- router is a `Provider`. tui never knows.
- structured logging: `tracing` + `tracing-subscriber` jsonl per session, separate from session files.
- error handling: `thiserror` for library crates (typed errors), `anyhow` only at the binary boundary. no `Box<dyn Error>` floating around. every error in the codebase maps to one `ErrorKind` variant when crossing into a `Message`.
- hook points wired through the agent loop from m0. nop impl through m6. plugin runtime decided at m7 start, leaning hard toward wasmtime + components.
- project context files: load `AGENTS.md`, `CLAUDE.md`, `.mew/AGENTS.md` walking up from cwd to git root, plus global. concatenate into system prompt. cross-tool filename support is a hard requirement.
- skills: opencode-compatible frontmatter and discovery. on-demand via the `skill` built-in tool, never bulk-injected. multi-path discovery covers `.mew/`, `.opencode/`, `.claude/`, `.agents/`. permission rules share the engine with tool permissions.
- acp transport is pluggable behind a `Transport: AsyncRead + AsyncWrite` trait. m6 ships stdio. m8 adds iroh, feature-flagged. acp protocol code never knows or cares which transport it's running on.
- iroh remote agents authenticate via spake2 pake with a 6-digit pairing code, plus optional trust-on-first-use for repeat connections. iroh node identity is *routing*, not authorization — pairing is the trust anchor. pairing code never transmitted; spake2 derives session keys from it. failed pairings delay 1s before close to defeat online brute force.
- subagents are nested sessions invoked via a built-in `subagent` tool. permissions do *not* inherit from parent — child prompts for its own approvals. tool sets are restrictable in the subagent definition; model is selectable or inheritable.
- mew's native plugin format is a superset of claude code's. components live at plugin root (`commands/`, `agents/`, `skills/`, `hooks/`, mcp config). manifest is `mew-plugin.toml`; `.claude-plugin/plugin.json` also accepted. designed this way so cc compat is a thin adapter, not a parallel system.
- cc plugin compatibility is best-effort, not guaranteed. plugins that are essentially packaged skills/agents/commands/mcp/hooks work; plugins depending on cc-specific internals (model names, env vars beyond `${CLAUDE_PLUGIN_ROOT}`, internal behaviors) are unsupported. document the known-incompat list as plugins get tested.
- cancellation: `tokio_util::sync::CancellationToken` threaded through the loop, tools, and provider streams.
- ulids for ids (`ulid` crate). sortable by creation, random enough.

---

## testing strategy

three tiers, in this order of importance.

**1. unit tests, in every crate.** standard cargo test. `mew-message` has table-driven round-trip tests against handwritten json fixtures covering every part type and every tool state, plus a `proptest` fuzz round-trip (generate arbitrary `Message`, serialize, deserialize, assert equal). `mew-skills` tests frontmatter parsing and the name regex. permission rule evaluation gets exhaustive coverage — this is exactly the code where a subtle bug turns into "agent runs `rm -rf` on a denied path."

**2. recorded provider fixtures.** for each provider adapter, capture real sse streams to `testdata/<scenario>.sse` and replay through the adapter in tests. scenarios per adapter, minimum:

- text-only single turn
- multi-turn with tool call and result
- streaming with reasoning blocks (anthropic)
- streaming tool call with arguments split across many chunks (openai)
- 429 mid-stream
- malformed chunk mid-stream
- abort (stream cut off mid-message)
- error response (auth fail, context overflow)

fixtures checked in. recording is `MEW_RECORD=1 cargo test -p mew-provider-openai` that hits the real provider with a known prompt and writes the fixture. extending the suite means adding new fixtures the same way.

**3. agent-loop tests with the fake provider.** the `Provider` trait makes this trivial: `mew-provider-fake` returns a scripted event stream. test the loop's handling of tool execution, permission flows, abort during tool execution, compaction trigger, hooks dispatch order, multi-turn tool loops. also useful for `mew run --model fake` in development.

**no end-to-end tests against real providers in ci.** flaky and costs money. recorded fixtures are the contract. a `cargo xtask smoke` target runs a single live turn against opencode zen and z.ai for pre-release sanity, gated on env-var creds, never run by ci.

logging in tests: install a `tracing` subscriber that writes to a per-test buffer; assert on log output where helpful. default to discarding.

---

## error taxonomy

`ErrorKind` is a closed enum on `MessageError`. additions require updating this list:

| kind | meaning | retry? |
|---|---|---|
| `provider_auth` | api key missing/invalid/revoked | no, surface to user |
| `provider_rate_limit` | 429 | yes, backoff per m1 spec |
| `provider_overload` | 5xx, server-side capacity | yes, single retry |
| `provider_api` | 4xx other than auth/429, malformed response | no, mark errored |
| `context_overflow` | request exceeded model context window | no, trigger compaction and retry once at the loop |
| `aborted` | cancellation token fired (user ctrl-c, shutdown) | no |
| `tool_exec` | tool returned `Err` for an unrecoverable reason | no, return result to model |
| `tool_timeout` | tool exceeded its timeout | no, return result to model |
| `mcp_transport` | mcp server crashed, stdio closed, http unreachable | yes for transient, mark server unavailable on persistent failure |
| `acp_protocol` | malformed acp message, capability mismatch | no |
| `network` | dns failure, connection refused, tls error | yes, single retry |
| `unknown` | catch-all; bug if you see one in production | no |

every error in the codebase maps to one of these when crossing into a `Message`. adapters do the mapping at the boundary. internal crates use their own `thiserror` types and convert at the boundary. `unknown` is a code smell — finding one in logs is a signal to add a more specific kind.

---

## appendix: what the model sees

the actual message array sent to the provider, layered:

```
1. system prompt
   ├── mew's base system prompt (identity, tool-use protocol)
   ├── catalog-derived model guidance if any
   ├── concatenated AGENTS.md / CLAUDE.md / .mew/AGENTS.md content,
   │     each wrapped: <context source="<path>">...</context>
   │     order: ~/.config/mew/AGENTS.md → git-root → cwd
   └── tool definitions block (provider-specific encoding)
       includes the `skill` tool with its dynamic listing:
       <available_skills>
         <skill><name>...</name><description>...</description></skill>
         ...
       </available_skills>

2. conversation messages, in order, each as a Message:
   - user messages: parts include text, file attachments (from @-mentions), images
   - assistant messages: parts include text, reasoning, tool_call (with terminal state), tool_result markers
   - on the wire: anthropic shape preserves blocks 1:1; openai shape splits
     assistant tool_use into the assistant message's tool_calls field and
     synthesizes role: tool messages for tool results

3. if a CompactionPart exists in history, everything before it is replaced
   by a synthesized summary message:
   - role: user (anthropic) or system (openai shape)
   - content: "Summary of earlier conversation: <generated summary>"
   - the CompactionPart itself is not sent; it's a session-record artifact

4. skill bodies are NOT in the system prompt. they're returned as tool
   results when the model calls the `skill` tool. each loaded skill becomes
   a tool_result part in the conversation, just like any other tool output.

5. file attachments via @-mentions are NOT in the system prompt either.
   they're FilePart entries in the user message that referenced them.
```

invariants:

- system prompt is rebuilt from scratch every turn. don't mutate it incrementally; it's deterministic from (mew version, model, project context files, available skills, available tools).
- skill listings reflect *current* permission state. a skill matched by a `deny` rule is omitted from the listing, not just rejected at load time.
- compaction replaces messages but never touches the system prompt or the most recent k turns (default k=4, configurable).

if any of this drifts during implementation, fix this appendix or the implementation, not both.

---

## open questions deferred until they matter

- session branching / fork semantics. probably needed eventually; punt past m6.
- cost tracking accuracy when models.dev pricing differs from provider's actual billing. log both, reconcile later.
- multi-cwd / project sandboxing. bash and edit currently respect a single root cwd per session. revisit when an actual user wants more.
- prompt caching (anthropic). add when measured to matter.
- windows support. plan currently assumes posix (paths, signals, bash). if windows ever happens it's a separate effort; flagging here so the assumption is explicit.

---

## first thing to build

`mew-message`, with types and round-trip tests (table-driven + proptest fuzz). a passing test suite there is the foundation everything else assumes works. then `mew-provider-openai` against captured fixtures. then wire to opencode zen for real. don't write tui code until m1 is green.
