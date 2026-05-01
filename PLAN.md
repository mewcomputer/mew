# mew — implementation plan

a terminal agent harness in go, similar to claude code. tui built on bubbletea/lipgloss. provider-agnostic via models.dev catalog. openai-shape and anthropic-shape adapters out of the gate, targeting opencode zen and z.ai's coding plan respectively. designed to grow into an acp (agent client protocol, zed-flavored) frontend in both directions.

this document is the source of truth for scope and architecture. milestones are sequenced; do not skip ahead. each milestone has an explicit "done when" — meet it before moving on.

---

## non-goals (read first)

- not a claude code clone in features. parity with the tool-use loop, tui polish, and permission model is the bar. ide integrations beyond acp, and team/cloud features are out of scope.
- no multi-tenant server mode. single user, local-first. acp is the only remoting story.
- **no telemetry, ever.** no analytics, no crash reporting phoning home, no usage pings. logs are local-only. if this changes it requires explicit user opt-in and a config flag, never a default.
- no multimodal output (generated images, audio, tts). image *input* is supported (see image handling). voice input is out.

---

## architecture

three layers, hard boundaries between them. even when everything runs in one process, the boundaries are real go interfaces and the wire format between them is the canonical message/event types. this is what lets the tui later talk to a remote agent over acp without a rewrite.

```
┌─────────────────────────────────────────────────┐
│ tui (bubbletea/lipgloss)                        │  cmd/mew
│   subscribes to agent event stream              │
│   sends user prompts + permission decisions     │
└──────────────────┬──────────────────────────────┘
                   │ canonical Event stream
                   │ (in-process channel today,
                   │  acp jsonrpc later)
┌──────────────────▼──────────────────────────────┐
│ agent core                                      │  internal/agent
│   - conversation state (parts-based messages)   │
│   - tool registry + permission gate             │
│   - context/compaction                          │
│   - router (auto mode)                          │
└──────────────────┬──────────────────────────────┘
                   │ Provider interface
┌──────────────────▼──────────────────────────────┐
│ provider adapters                               │  internal/provider/{openai,anthropic}
│   sse parsing, tool-call accumulation,          │
│   translation to canonical events               │
└─────────────────────────────────────────────────┘
```

### module layout

```
cmd/mew/                  tui entry point + cli
internal/message/         canonical message + part types, json round-trip
internal/provider/        Provider interface, Event types
internal/provider/openai/   openai-shape adapter
internal/provider/anthropic/ anthropic-shape adapter
internal/provider/router/   auto-mode router (m5)
internal/agent/           the loop, conversation state, compaction
internal/tools/           built-in tools + registry + permission gate
internal/mcp/             mcp client (stdio + http)
internal/acp/             acp client and server (m6)
internal/config/          config + creds (keychain + json fallback)
internal/session/         jsonl session persistence
internal/catalog/         models.dev catalog loader + cache
internal/context/         AGENTS.md / CLAUDE.md / .mew/AGENTS.md project context loader
internal/skills/          SKILL.md discovery + loading; backs the `skill` built-in tool
internal/hooks/           Dispatcher interface; nop impl through m6, runtime in m7
```

tui code lives in `cmd/mew` rather than `internal/tui` because the bubbletea program *is* the application from the user's perspective. if it grows past a few files, split into `cmd/mew/ui/`.

---

## canonical message and event types

cribbed in shape from opencode's MessageV2, adapted for go. messages are metadata + an ordered list of parts. parts are a discriminated union dispatched on a `type` field. tool calls and tool results are separate parts (do not fuse them) so a tool can render as "running" before its result exists.

### messages

```go
package message

type Role string
const (
    RoleUser      Role = "user"
    RoleAssistant Role = "assistant"
)

type Message struct {
    ID        string         `json:"id"`         // ulid
    SessionID string         `json:"sessionID"`
    Role      Role           `json:"role"`
    Parts     Parts          `json:"parts"`
    Time      Time           `json:"time"`
    Assistant *AssistantMeta `json:"assistant,omitempty"` // nil on user
}

type Time struct {
    Created   int64 `json:"created"`             // unix ms
    Completed int64 `json:"completed,omitempty"`
}

type AssistantMeta struct {
    ProviderID string  `json:"providerID"`
    ModelID    string  `json:"modelID"`
    Cost       float64 `json:"cost"`
    Tokens     Tokens  `json:"tokens"`
    Finish     string  `json:"finish,omitempty"` // stop|length|tool_use|error
    Error      *Error  `json:"error,omitempty"`
}

type Tokens struct {
    Input         int `json:"input"`
    Output        int `json:"output"`
    Reasoning     int `json:"reasoning"`
    CacheRead     int `json:"cacheRead"`
    CacheWrite    int `json:"cacheWrite"`
}

type Error struct {
    Kind    string `json:"kind"`    // provider_auth|context_overflow|aborted|api|unknown
    Message string `json:"message"`
}
```

### parts

```go
type Part interface {
    PartID() string
    partType() string // unexported sentinel
}

type partBase struct {
    ID        string `json:"id"`
    MessageID string `json:"messageID"`
    SessionID string `json:"sessionID"`
}
func (p partBase) PartID() string { return p.ID }

type TextPart struct {
    partBase
    Type      string `json:"type"` // "text"
    Text      string `json:"text"`
    Synthetic bool   `json:"synthetic,omitempty"`
}

type ReasoningPart struct {
    partBase
    Type      string `json:"type"` // "reasoning"
    Text      string `json:"text"`
    Signature string `json:"signature,omitempty"` // anthropic signed reasoning, opaque
}

type FilePart struct {
    partBase
    Type     string `json:"type"` // "file"
    Mime     string `json:"mime"`
    Filename string `json:"filename,omitempty"`
    URL      string `json:"url"` // file:// or data: or https://
}

type ToolCallPart struct {
    partBase
    Type     string    `json:"type"` // "tool_call"
    ToolName string    `json:"toolName"`
    CallID   string    `json:"callID"` // provider's id, for threading results
    State    ToolState `json:"state"`
}

type ToolResultPart struct {
    partBase
    Type   string `json:"type"` // "tool_result"
    CallID string `json:"callID"` // matches ToolCallPart.CallID
    // result is conveyed by the referenced ToolCallPart's terminal state.
    // this part exists so the message stream has an ordered marker for "result delivered".
}

type CompactionPart struct {
    partBase
    Type        string `json:"type"` // "compaction"
    Auto        bool   `json:"auto"`
    Overflow    bool   `json:"overflow,omitempty"`
    TailStartID string `json:"tailStartID,omitempty"`
}
```

### tool state machine

pending → running → (completed | error). every transition mutates the same `ToolCallPart.State`. the agent core emits `EventPartUpdated` after each transition.

```go
type ToolState struct {
    Status   ToolStatus     `json:"status"`
    Input    map[string]any `json:"input,omitempty"`
    Output   string         `json:"output,omitempty"`
    Error    string         `json:"error,omitempty"`
    Metadata map[string]any `json:"metadata,omitempty"`
    Time     ToolTime       `json:"time"`
}

type ToolStatus string
const (
    ToolPending   ToolStatus = "pending"   // call known, args still streaming
    ToolRunning   ToolStatus = "running"   // args complete, executing
    ToolCompleted ToolStatus = "completed"
    ToolError     ToolStatus = "error"
)

type ToolTime struct {
    Start int64 `json:"start"`
    End   int64 `json:"end,omitempty"`
}
```

### Parts unmarshaling

custom `UnmarshalJSON` on `Parts` that peeks at `"type"` and dispatches to the right concrete type. unknown types are an error in v1; if this becomes a forward-compat problem later, add an `UnknownPart` variant that preserves the raw json.

### events (the wire format between layers)

```go
package provider // shared by agent core too; lives here so adapters can produce them

type Event interface{ eventType() string }

type EventPartStart   struct{ Part message.Part }
type EventPartDelta   struct{ PartID, Field, Delta string }
type EventPartEnd     struct{ PartID string }
type EventMessageEnd  struct{
    Finish string
    Usage  message.Tokens
    Cost   float64
}
type EventError       struct{ Err error }
```

agent core emits a superset to the tui (adds permission requests, tool execution events). keep the agent-core-to-tui event type in `internal/agent`, not `internal/provider`.

---

## provider interface

```go
package provider

type Provider interface {
    Name() string
    Stream(ctx context.Context, req Request) (<-chan Event, error)
}

type Request struct {
    Model    string
    Messages []message.Message
    Tools    []ToolDef
    System   string
    // provider-specific knobs are *not* a map[string]any. each adapter
    // exposes typed Options if it needs them, set via functional options
    // on Request construction.
}

type ToolDef struct {
    Name        string
    Description string
    Schema      json.RawMessage // jsonschema
}
```

adapters fully consume the sse stream and emit canonical events. they are responsible for:

- parsing sse / chunked json
- accumulating tool-call argument deltas (openai streams `arguments` as a string-concatenation across chunks; anthropic streams `input_json_delta` blocks)
- mapping provider finish reasons to the canonical set: `stop|length|tool_use|error`
- mapping errors to `message.Error.Kind`
- never leaking provider-specific types past the channel

### image input

`FilePart` carries images as well as text files; mime determines the rendering. capability comes from the catalog: each model has a `vision: bool` derived from models.dev. when a user attaches an image and the active model has `vision: false`, the agent core rejects the turn before sending with a clear error rather than letting the provider 400.

adapter responsibilities:

- **openai shape**: encode `FilePart` with image mime as `{type: "image_url", image_url: {url: "data:..."}}` content blocks alongside text content in the user message.
- **anthropic shape**: encode as `{type: "image", source: {type: "base64", media_type: ..., data: ...}}` blocks.

source resolution: `FilePart.URL` may be `file://` (read from disk and base64), `data:` (pass through), or `https://` (pass through if provider accepts urls, else fetch and base64; openai accepts urls, anthropic in this codepath does not — anthropic path always inlines as base64).

size limits: cap at 10mb per image after base64 encoding. larger gets rejected at the agent core, never sent. images are *not* compacted; if context pressure requires dropping turns, drop them whole including their images, never strip images from a kept turn.

image input lands in m1 (so both shapes are exercised), not m0. m0 covers text-only round-trips.

---

## the agent loop

```
loop:
  hooks.dispatch(ChatMessage, msg)            ← may rewrite outbound message
  hooks.dispatch(ChatParams, params)          ← may rewrite temp/topP/maxTokens/etc
  hooks.dispatch(ChatHeaders, headers)        ← may inject http headers
  send conversation to provider, get event stream
  for each event:
    update in-memory message
    hooks.dispatch(Event, event)              ← observers only, no rewrite
    forward (translated) event to tui
  on EventMessageEnd:
    if assistant message has tool_call parts in pending/running terminal state:
      for each:
        decision = check permissions
        decision = hooks.dispatch(PermissionAsk, call, decision)  ← plugin override
        if decision == prompt: ask user
        input = hooks.dispatch(ToolExecuteBefore, call, input)    ← rewrite args
        execute tool (built-in or mcp)
        output = hooks.dispatch(ToolExecuteAfter, call, output)   ← rewrite result
        update ToolCallPart.State to completed/error
        emit EventPartUpdated
      append tool results as a new user message (anthropic-shape) or
        role:tool messages (openai-shape) — handled by adapter, agent
        works in canonical shape only
      goto loop
    else:
      done
```

every `hooks.dispatch` call is a no-op in the default `nopDispatcher` shipped through m6. the dispatcher interface is wired through the loop from m0 so adding a real runtime in m7 doesn't touch loop code.

context management: track total tokens against the model's context window (from models.dev catalog). when above a threshold (start at 80%), insert a `CompactionPart` and summarize prior turns via a cheap model call. preserve the system prompt, the most recent k turns verbatim, and a synthesized summary of the rest.

abort: ctx cancellation propagates everywhere. on abort mid-stream, mark the in-progress message with `Error.Kind = "aborted"` and persist it.

### hook points (defined now, runtime deferred to m7)

```go
package hooks

type Dispatcher interface {
    // Observe-only. Errors logged, never propagated.
    OnEvent(ctx context.Context, ev agent.Event)

    // Mutating. Each returns the (possibly modified) value.
    // Errors fall back to the input unchanged and are logged.
    OnChatMessage(ctx context.Context, msg message.Message) message.Message
    OnChatParams(ctx context.Context, p ChatParams) ChatParams
    OnChatHeaders(ctx context.Context, h http.Header) http.Header
    OnToolExecuteBefore(ctx context.Context, call ToolCall, input map[string]any) map[string]any
    OnToolExecuteAfter(ctx context.Context, call ToolCall, output ToolOutput) ToolOutput
    OnPermissionAsk(ctx context.Context, call ToolCall, current PermissionDecision) PermissionDecision
    OnShellEnv(ctx context.Context, env map[string]string) map[string]string
}

type ChatParams struct {
    Temperature *float64
    TopP        *float64
    MaxTokens   *int
    // extend as providers grow; keep typed, no map[string]any
}
```

ship `nopDispatcher` from m0. agent core takes a `Dispatcher` in its constructor; `cmd/mew` wires the nop until m7.

omitted from opencode's set, on purpose:

- their `tool` hook (plugin registers a tool) — already covered by mcp and the in-tree `Tool` interface. plugins that want to add tools should implement `Tool` directly, registered via the same registry.
- their `auth` hook — defer until mew supports oauth flows; byok covers everything until then.
- their `command.execute.before` — slash commands aren't a v1 concern beyond the fixed built-ins. revisit when we have user-defined commands.

---

## tools

### built-in tool set (m3)

minimum viable, modeled on claude code's:

- `read` — read file (path, optional offset/limit). returns text + metadata.
- `write` — create or overwrite file (path, content). always permission-gated.
- `edit` — find/replace in file (path, old_string, new_string). exact match required, fails on ambiguity. always permission-gated.
- `bash` — execute shell command (command, optional timeout). always permission-gated. captures stdout+stderr, truncates over n bytes.
- `glob` — pattern match files (pattern, optional path). returns sorted by mtime desc.
- `grep` — content search (pattern, optional path, optional glob). uses ripgrep if available, falls back to go regexp.

each tool is a go type implementing:

```go
type Tool interface {
    Name() string
    Description() string
    Schema() json.RawMessage
    Execute(ctx context.Context, input map[string]any) (Result, error)
    // permission classification
    Sensitivity() Sensitivity // ReadOnly | Mutating | Dangerous
}
```

### permission model

three decisions: `allow_once`, `allow_session`, `deny`. plus rule-based pre-allows from config:

```json
{
  "permissions": {
    "rules": [
      {"tool": "bash", "match": {"command_prefix": "git status"}, "decision": "allow"},
      {"tool": "read", "match": {"path_glob": "**/*.go"}, "decision": "allow"},
      {"tool": "write", "match": {"path_glob": "/etc/**"}, "decision": "deny"}
    ]
  }
}
```

evaluation order: explicit deny rules → explicit allow rules → session allows → prompt user. read-only tools default to allow; mutating and dangerous prompt unless allowed by rule.

path matching uses `doublestar` for `**` support. all paths normalized to absolute before matching.

### mcp client (m4)

stdio and streamable-http transports. servers configured in `config.json`:

```json
{
  "mcp": {
    "servers": {
      "filesystem": {"transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]},
      "github": {"transport": "http", "url": "https://...", "headers": {"Authorization": "Bearer ..."}}
    }
  }
}
```

mcp tools are registered alongside built-ins, namespaced as `mcp__<server>__<tool>` to avoid collisions. mcp tool calls go through the same permission gate.

mcp is also a tool *source* for search: there's a meta-tool `mcp_search` that lists available servers/tools from a registry (later — punt past m4).

---

## providers and the catalog

### models.dev integration

at startup, fetch `https://models.dev/api.json` and cache locally (with etag, refresh on 24h). this gives:

- list of models with `provider`, `id`, `context_window`, `max_output`, `tool_call: bool`, `reasoning: bool`, pricing.
- per-model "shape": which adapter to use.

mew's config selects a model by id. catalog lookup determines the adapter and capabilities. if a user configures a model not in the catalog, allow it with explicit `shape: "openai"|"anthropic"` and `context_window` overrides in config.

### opencode zen (openai-shape)

base url: `https://opencode.ai/zen/v1` (or whatever opencode zen exposes — verify on first connect). uses standard openai chat completions with tool calling. byok via `OPENCODE_ZEN_API_KEY`.

### z.ai coding plan (anthropic-shape)

base url: `https://api.z.ai/api/anthropic`. uses the anthropic messages api shape. byok via `ZAI_API_KEY`. the coding plan currently centers on glm-4.6.

### auto mode (m5)

a `Provider` implementation that wraps multiple inner providers and routes per request. v1 heuristic:

- if last message has tool results or message count > N → "big" model
- if conversation has only short user prompts and no tool use → "small" model
- explicit hint via system metadata can override

router config:

```json
{
  "providers": {
    "auto": {
      "kind": "router",
      "small": "z-ai/glm-4.5-air",
      "big": "z-ai/glm-4.6"
    }
  }
}
```

router is itself a provider — the tui and agent core never know they're talking to a router.

---

## config and credentials

### locations

- `os.UserConfigDir() / mew/config.json` — non-secret config
- `os.UserConfigDir() / mew/credentials.json` — fallback for secrets if keychain unavailable, mode 0600
- `os.UserConfigDir() / mew/sessions/` — jsonl per session
- `os.UserConfigDir() / mew/logs/` — structured slog jsonl

on linux this resolves to `~/.config/mew/`, mac `~/Library/Application Support/mew/`, windows `%AppData%\mew\`.

### keychain

use `github.com/zalando/go-keyring`. on first run, probe with sentinel write/read/delete. if it fails (common on headless linux), fall back to `credentials.json` and *log a warning at startup*. never silently store secrets in plaintext.

credentials are keyed by provider id:

```json
// config.json
{
  "providers": {
    "opencode-zen": {"shape": "openai", "baseURL": "https://opencode.ai/zen/v1", "credentialRef": "opencode-zen"},
    "z-ai": {"shape": "anthropic", "baseURL": "https://api.z.ai/api/anthropic", "credentialRef": "z-ai"}
  },
  "defaultModel": "z-ai/glm-4.6"
}
```

`credentialRef` is looked up first in keychain, then in `credentials.json`, then in env vars (`MEW_CRED_<REF_UPPERCASED>`).

cli for cred management: `mew auth set <providerID>` (prompts for key, writes to keychain or fallback), `mew auth list`, `mew auth remove <providerID>`.

---

## sessions

### format

one jsonl file per session: `sessions/<sessionID>.jsonl`. one line per `Message`. resume by reading the file. session id is a ulid; sessions are listable/sortable by creation time.

a small index file `sessions/index.json` tracks `{id, created, title, lastModel}` for fast listing in the tui. rebuilt on demand if missing.

copy all lines over on message branch.

### structured logging

separate from session files. `logs/<sessionID>.jsonl` with `slog` json output. logs every:

- provider request (sanitized — strip api keys from any echoed headers)
- provider response chunk metadata (not full content; that's in the session)
- tool execution start/end with duration
- permission decisions
- errors with stack

debugging without these is hell. they're not optional.

---

## acp (m6)

zed's agent client protocol. jsonrpc 2.0 over stdio. spec: <https://agentclientprotocol.com>.

### two modes

**(a) mew tui as acp client.** mew's tui spawns claude-code-acp (or any acp agent) as a subprocess and drives it. agent core is bypassed; the tui talks acp directly. enabled with `mew --acp-agent claude-code` or similar.

implementation: the tui already consumes a canonical event stream. write an adapter that translates acp `session/update` notifications into those events. translate user input into acp `session/prompt` requests. permission requests come back as acp `session/request_permission`, render the same approval ui, send the response.

**(b) mew agent core as acp server.** `mew acp` subcommand starts mew's agent core speaking acp over stdio. zed/nvim/etc. can then drive mew. translate inbound acp methods to agent-core operations; emit acp notifications for our event stream.

both modes share the same translator code in `internal/acp/`.

### slash commands and prompts

acp 2025 added slash commands via the agent. mew's built-in commands (e.g. `/compact`, `/clear`, `/cost`) should be exposed via acp's slash-command extension when running as a server.

---

## milestones, sequenced

each milestone has a "done when" gate. do not advance past a gate unless all bullets are satisfied.

### m0: skeleton + openai adapter + opencode zen

- `internal/message`: types above, custom `UnmarshalJSON` for `Parts`, round-trip tests with table-driven fixtures (text-only, with reasoning, with tool calls in each state, multi-part).
- `internal/provider`: interface + event types.
- `internal/provider/openai`: sse parsing, tool-call argument accumulation across chunks, mapping to canonical events. unit tests using captured sse fixtures (record real responses to `testdata/`, replay in tests).
- `internal/agent`: minimal loop. one fake echo tool registered in-tree for testing.
- `internal/hooks`: `Dispatcher` interface + `nopDispatcher`. agent core takes a `Dispatcher` in its constructor; wire the nop. dispatch calls placed at every hook point shown in the loop pseudocode, even though they no-op.
- `internal/config`: config loading, env var creds only at this stage.
- `internal/context`: project context file loader. on session start, walk from cwd up to git root (or $HOME), collect `AGENTS.md`, `CLAUDE.md`, `.mew/AGENTS.md` along the way; also load `~/.config/mew/AGENTS.md` if present. concatenate in order (most-general first), prepend to system prompt with clear `<context source="path">...</context>` framing. all four filenames supported for cross-tool interop — users coming from claude code, cursor, or opencode should not have to rename anything.
- `internal/session`: jsonl writer, no resume yet.
- `cmd/mew run "<prompt>"`: non-interactive. streams text to stdout, executes the fake tool when called, feeds results back, terminates on stop. exits 0.

done when: `MEW_CRED_OPENCODE_ZEN=... mew run "list three primes then call the echo tool with input='hi'"` produces correct streaming output, the echo tool runs, the session file under `sessions/` round-trips through `internal/message` types cleanly, and provider unit tests pass against recorded fixtures.

### m1: anthropic adapter + z.ai

- `internal/provider/anthropic`: sse parsing for anthropic messages api, including `input_json_delta` accumulation, `thinking`/`reasoning` blocks → `ReasoningPart`, signed reasoning round-trip.
- catalog loader (`internal/catalog`) reads models.dev, picks adapter by model id.
- multi-provider config; `mew run --model <id>` chooses.
- canonical message <-> wire format translation for both shapes lives in adapter; agent core never sees provider-specific data.
- image input support per "image input" section. both adapters encode `FilePart` images correctly. catalog `vision` flag gates pre-send.
- rate limit handling: on 429 from either provider, retry with exponential backoff (start 1s, cap 30s, max 4 attempts). on other 5xx, retry once. on persistent failure, emit `EventError` with `Error.Kind = "api"` and a human-readable message; the agent core marks the turn errored, keeps the partial assistant message in the session, and lets the tui prompt the user for retry. never silently drop a turn.

done when: same `mew run` works against `z-ai/glm-4.6` and against opencode zen, switched only by `--model`, and a captured anthropic fixture replays correctly in unit tests including a tool-call turn with reasoning. image input works end-to-end on both shapes against a vision-capable model. a synthetic 429 in the fixture replay triggers correct backoff behavior.

### m2: bubbletea tui

- `cmd/mew` interactive mode (`mew` with no subcommand).
- streaming token rendering, tool-call display reflecting state machine (spinner on running, output collapsed by default on completed, red on error).
- approval prompts: modal-ish overlay, three options (allow once, allow session, deny). keyboard-driven.
- input editor with multiline support, history (up/down), basic vim-ish or readline keys (pick one and stick to it; default to readline).
- status line: model name, session token usage, session cost.
- keychain creds via `mew auth set` cli plus the json fallback.
- **slash commands.** built-in command registry, dispatched when input starts with `/`. v1 set: `/help`, `/clear` (start new session), `/compact` (force compaction now), `/cost` (full breakdown for current session), `/model <id>` (switch model mid-session), `/sessions` (list resumable), `/resume <id>`, `/quit`. registry is a `map[string]Command` with a `Command` interface so user-defined commands can land later via plugins (m7) without restructuring. unknown `/foo` falls through to the model as literal text — don't error.
- **@-mentions for files.** typing `@` opens a fuzzy file picker rooted at the project directory (or git root). selecting a file inserts a token like `@path/to/file.go` into the prompt; on submit, the agent core reads the file and attaches it as a `FilePart` (text mime → text content, image mime → image content per image-input rules) before sending. respects `.gitignore`. files over 1mb prompt for confirmation in the tui before attaching.
- **interrupt / cancellation.** ctrl-c during a streaming turn cancels the current request via ctx, marks the in-progress assistant message with `Error.Kind = "aborted"`, preserves whatever was streamed so far, and returns input focus to the user. ctrl-c with no active turn clears the input buffer if non-empty, otherwise prompts "exit? (y/n)". double ctrl-c within 1 second exits unconditionally. ctrl-d on empty input exits.
- **bash output streaming.** the `bash` tool streams stdout/stderr to the tui as it produces them, not after completion. the tui renders a live-updating tool block. the `ToolStateRunning.Output` field is appended-to as bytes arrive; `EventPartUpdated` emitted on a small debounce (~50ms) to avoid flooding the renderer. the truncation cap (30k chars default) applies to the final captured output written to session, not the live stream.
- **diff display for edits.** when the `edit` tool runs (and `write` to an existing file), the tui renders a unified diff of the change rather than just the path. additions green, removals red, context dim. uses `github.com/sergi/go-diff` or equivalent. the raw before/after still goes into `ToolState.Output` for the session record; diff is purely a render concern.
- **cost surfacing.** session cost is the sum of per-message costs computed by adapters (input_tokens × input_price + output_tokens × output_price + cache_read × cache_price etc., prices from catalog). status line shows running total. `/cost` command shows full breakdown: per-model totals if the session crossed models, per-turn list, cache hit ratio. costs persisted on each `Message` so resume reconstructs accurately. if catalog pricing is missing for a model, mark cost as `null` for those messages and note "cost unavailable" — never silently zero.
- **branching**: if the user goes back to a prior message and submits a new turn, create a new branch in the session from that point.

done when: a user can `mew auth set z-ai`, then `mew`, hold a multi-turn conversation with tool calls and approvals, see streaming output and tool execution states update live, attach files via `@`, run a long bash command and watch its output stream, ctrl-c a long-running turn cleanly, run `/cost` and see a sane breakdown, and resume the session next launch via `mew --session <id>` or `mew --resume`.

### m3: built-in tools, permission model, skills

- read, write, edit, bash, glob, grep, all implementing the `Tool` interface.
- permission rules in config, evaluated in documented order.
- session-allow tracked in memory only; never persisted.
- bash tool: configurable timeout (default 2 min), output truncation (default 30k chars), env passthrough policy.
- `internal/skills`: discovery + loading. on session start, scan in this order, load all matches:
  - `<project>/.mew/skills/<name>/SKILL.md`
  - `<project>/.opencode/skills/<name>/SKILL.md`
  - `<project>/.claude/skills/<name>/SKILL.md`
  - `<project>/.agents/skills/<name>/SKILL.md`
  - `~/.config/mew/skills/<name>/SKILL.md`
  - `~/.config/opencode/skills/<name>/SKILL.md`
  - `~/.claude/skills/<name>/SKILL.md`
  - `~/.agents/skills/<name>/SKILL.md`

  walk the project paths from cwd up to git worktree root. parse yaml frontmatter (`name` required, `description` required, `license`/`compatibility`/`metadata` optional). validate `name` against `^[a-z0-9]+(-[a-z0-9]+)*$`, 1-64 chars, must equal directory name. duplicate names: project beats global; within a tier, mew's own paths beat the compat paths. log conflicts.
- `skill` built-in tool: takes `name`, returns the full markdown body. listing of available `<name, description>` pairs is injected into the tool's description as xml so the model can discover what to load.
- skill permissions in config, sharing the rule engine with tools but discriminated by kind:
  ```json
  {"permissions": {"skills": [
    {"match": {"name_glob": "internal-*"}, "decision": "deny"},
    {"match": {"name_glob": "experimental-*"}, "decision": "ask"},
    {"match": {"name_glob": "*"}, "decision": "allow"}
  ]}}
  ```
  evaluated top-to-bottom, first match wins. `ask` triggers the same approval ui as tool permissions.

done when: a user can ask "find all go files importing net/http and add a comment to each", mew uses glob+grep+edit, prompts for each edit (or batches them under a session-allow rule), and the session file accurately reflects the tool state machine throughout. additionally: a `git-release` skill placed at `.mew/skills/git-release/SKILL.md` shows up in the `skill` tool's listing, can be loaded by the model, and a `deny` rule for `internal-*` hides matching skills from the listing entirely.

### m4: mcp client

- stdio transport (subprocess + jsonrpc over its stdio).
- streamable-http transport.
- server lifecycle: spawn on first use, keep alive for session, shut down on session end.
- tool namespacing `mcp__<server>__<tool>`, registered at agent startup after server handshake.
- mcp tool calls flow through the same permission gate. sensitivity defaults to `Mutating` unless the server declares otherwise.

done when: configuring an mcp server in `config.json` makes its tools available to the model, a tool call against it works end-to-end, and killing the session cleans up subprocesses without orphans.

### m5: router / auto mode

- `internal/provider/router`: implements `Provider`, dispatches per-request.
- v1 heuristic as described.
- config schema for routers.
- `mew run --model auto` and tui model picker support.

done when: `--model auto` selects a small model for short single-turn prompts and the big model when tools are present, and the choice is visible in logs and the status line.

### m6: acp, both directions

- `internal/acp`: jsonrpc framing, message types, capability negotiation.
- client mode: `mew --acp-agent <cmd>` spawns the agent, tui uses acp instead of internal agent core.
- server mode: `mew acp` exposes agent core over stdio.
- slash commands surfaced via acp.

done when: `mew --acp-agent "npx @zed-industries/claude-code-acp"` gives a working tui driven by claude code, *and* zed (or a minimal acp client used in tests) can drive `mew acp` for a multi-turn conversation including tool approvals.

### m7: plugin runtime (deferred — do not start until m6 ships)

implements `hooks.Dispatcher` with a real plugin runtime. the hook *points* and *interface* are already in place from m0; this milestone is purely about the execution substrate.

runtime decision is intentionally **not made** in this plan. evaluate when starting m7 against then-current options. the leading candidate as of writing is **wasm via extism** because it's polyglot (rust, go, js, python, c, etc. all compile to wasm), properly sandboxed by default, and small to integrate (~hundreds of lines). subprocess+jsonrpc (mcp-style) is the second choice — already-proven plumbing from m4, but slow per-hook and harder to control.

scope:

- pick a runtime, document the decision in this file under "decisions made."
- implement `Dispatcher` backed by the runtime. preserve the contract: errors logged, never propagated; mutating hooks fall back to input on failure.
- plugin loading from `~/.config/mew/plugins/` and `<project>/.mew/plugins/` in that order, all loaded, hooks run in load order (matches opencode's model).
- a host api exposed to plugins covering: read session messages, log, fetch http (gated), read config values. no filesystem or shell access by default; plugins needing those must use mcp.
- a sample plugin in-tree demonstrating each hook.
- docs for plugin authors: which hooks exist, host api surface, examples.

done when: a plugin loaded from `~/.config/mew/plugins/` can (a) inject a custom http header into provider requests, (b) deny a `bash` permission for commands matching a regex, (c) rewrite a tool's output to redact secrets, all observed end-to-end with mew running against a real provider, and the same plugin works unchanged across at least two plugin-author languages if the runtime is wasm.

---

## decisions made (don't relitigate)

- creds: keychain first, json fallback, env var override. warn loudly on fallback.
- session format: jsonl of canonical messages, one file per session.
- streaming: provider interface is stream-first. no non-streaming variant.
- tool calls and tool results are separate parts, never fused.
- discriminated unions via `Part` interface + custom unmarshaler. unknown types error in v1.
- no provider-specific types past the adapter boundary, ever.
- router is a `Provider`. tui never knows.
- bubbletea + lipgloss. no other tui libs.
- go-keyring for keychain. doublestar for path globs. ripgrep optional with regexp fallback.
- structured slog jsonl logs per session, separate from session files.
- hook points (`hooks.Dispatcher` calls) wired through the agent loop from m0. nop impl through m6. plugin runtime decided at m7 start, not now. opencode's hook taxonomy is the reference; mew omits their `tool`, `auth`, and `command.execute.before` hooks (covered elsewhere or deferred).
- project context files: load `AGENTS.md`, `CLAUDE.md`, `.mew/AGENTS.md` walking up from cwd to git root, plus `~/.config/mew/AGENTS.md`. concatenate into system prompt. cross-tool filename support is a hard requirement, not a maybe.
- skills: opencode-compatible frontmatter and discovery. on-demand loading via a built-in `skill` tool, never bulk-injected into context. multi-path discovery covers `.mew/`, `.opencode/`, `.claude/`, `.agents/` (project and global). permission rules share the engine with tool permissions.

---

## testing strategy

three tiers, in this order of importance:

**1. unit tests, in every package.** standard go testing. `internal/message` has table-driven round-trip tests against handwritten json fixtures covering every part type and every tool state. `internal/skills` tests frontmatter parsing and the name regex. permission rule evaluation gets exhaustive coverage — this is exactly the code that a subtle bug turns into "agent runs `rm -rf` on a denied path."

**2. recorded provider fixtures.** for each provider adapter, capture real sse streams to `testdata/<scenario>.sse` (or `.jsonl` for chunked json) and replay them through the adapter in tests. scenarios per adapter, minimum:

- text-only single turn
- multi-turn with tool call and result
- streaming with reasoning blocks (anthropic)
- streaming tool call with arguments split across many chunks (openai)
- 429 mid-stream
- malformed chunk mid-stream
- abort (stream cut off mid-message)
- error response (auth fail, context overflow)

these fixtures are checked in. recording is a one-time `MEW_RECORD=1 go test ./internal/provider/openai/...` that hits the real provider with a known prompt and writes the fixture. agents extending the suite add new fixtures the same way.

**3. agent-loop tests with a fake provider.** the `Provider` interface makes this trivial: write a `FakeProvider` that returns a scripted event stream. test the loop's handling of: tool execution, permission flows, abort during tool execution, compaction trigger, hooks dispatch order, multi-turn tool loops. fake provider lives in `internal/provider/fake/` and is also useful for the `--model fake` cli flag in development.

**no end-to-end tests against real providers in ci.** they're flaky and cost money. the recorded fixtures are the contract. a separate `make smoke` target runs a single live turn against opencode zen and z.ai for pre-release sanity, gated on env-var creds, never run by ci.

logging: tests should not produce log output. wire `slog` to a discard handler in test setup, override per-test when assertions need to inspect logs.

---

## error taxonomy

`message.Error.Kind` is a closed enum. additions require updating this list:

| kind | meaning | retry? |
|---|---|---|
| `provider_auth` | api key missing/invalid/revoked | no, surface to user |
| `provider_rate_limit` | 429 | yes, backoff per m1 spec |
| `provider_overload` | 5xx, server-side capacity | yes, single retry |
| `provider_api` | 4xx other than auth/429, malformed response | no, mark errored |
| `context_overflow` | request exceeded model context window | no, trigger compaction and retry once at the loop level |
| `aborted` | ctx cancelled (user ctrl-c, shutdown) | no |
| `tool_exec` | tool panicked or returned unrecoverable error | no, return result to model |
| `tool_timeout` | tool exceeded its timeout | no, return result to model |
| `mcp_transport` | mcp server crashed, stdio closed, http unreachable | yes for transient, mark server unavailable on persistent failure |
| `acp_protocol` | malformed acp message, capability mismatch | no |
| `network` | dns failure, connection refused, tls error | yes, single retry |
| `unknown` | catch-all; bug if you see one in production | no |

every error in the codebase maps to one of these. adapters do the mapping at the boundary; agent core, tui, and acp layers consume the canonical kinds only. `unknown` is a code smell — finding one in logs is a signal to add a more specific kind.

---

## appendix: what the model sees

the actual message array sent to the provider, layered:

```
1. system prompt
   ├── mew's base system prompt (identity, tool-use protocol)
   ├── catalog-derived model guidance if any (e.g. anthropic-specific blocks)
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
   - assistant messages: parts include text, reasoning, tool_call (with their
     terminal state), tool_result markers
   - on the wire: anthropic shape preserves blocks 1:1; openai shape splits
     assistant tool_use into the assistant message's tool_calls field and
     synthesizes role:"tool" messages for tool results

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
- multi-cwd / project sandboxing. the bash and edit tools currently respect a single root cwd per session. revisit when an actual user wants more.
- prompt caching (anthropic). add when measured to matter.
- subagents / sub-tasks (opencode's `SubtaskPart`). not in scope until there's a real use case.

---

## first thing to build

`internal/message`, with types and round-trip tests. a passing test suite there is the foundation everything else assumes works. then the openai adapter against captured fixtures. then wire to opencode zen for real. don't write tui code until m1 is green.
