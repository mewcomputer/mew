## M0: Skeleton + OpenAI adapter + Opencode providers

- rust project skeleton with workspace crates
- `mew-provider-openai` adapter (SSE streaming, delta-based)
- two built-in providers: `opencode-zen` and `opencode-go`
- clap CLI with `run` and `chat` subcommands
- `mew-message` canonical message/part types
- `mew-catalog` model catalog with 24h cache
- `mew-config` config loading + credential resolution
- `mew-session` JSONL session persistence
- `mew-hooks` Dispatcher trait

## M1: Anthropic adapter + provider routing + tests

- `mew-provider-anthropic` adapter (SSE, content-block events)
- `z-ai` built-in provider (anthropic-shape, glm-5.1)
- model routing: minimax models go through anthropic adapter
- catalog parsing fix (array format)
- retry policy tests, image wire-format tests, adapter tests

## M2: Built-in tools + permission engine

- `mew-tools` crate with Read, Write, Edit, Bash, Glob, Grep, Echo
- `PermissionEngine` with declarative rules matching
- `Tool` trait with sensitivity levels (ReadOnly, Mutating, Dangerous)
- minimal interactive mode (crossterm line input)

## M3: Ratatui TUI

- full ratatui event loop with `App` state
- streaming text display, tool call state tracking
- permission prompt modal, model picker
- status line with model/provider/cost
- context sidebar on wide screens
- ctrl+p command palette, model switching
- model discovery by querying each provider API
- picker scrolling + scrollbar

## M3.5 (uncommitted): TUI overhaul — markdown, tool display, keyboard UX

- ratatui-mdstream markdown rendering with syntect highlighting
- cached markdown rendering (per message-id, re-rendered on width change)
- streaming markdown support via `render_streaming(md_state)`
- tool call blocks with colored backgrounds and half-block edges
- bash output collapse/expand (ctrl+o)
- ansi-to-tui rendering for tool output
- diff display with green/red coloring
- scroll indicators (↑/↓), scrollbar, ctrl+home/end for scroll-to-top/bottom
- esc-to-cancel agent (double-esc with pending state)
- slash autocomplete with keyboard navigation (tab/arrows)
- input editing: ctrl+a/e/f/b, ctrl+u/w, alt+backspace/cmd+backspace
- word-level cursor movement (alt+left/right)
- ctrl+l to jump to bottom
- image file attachments via @mentions (png/jpg/gif/webp)
- streaming drain coalescing (max 4 agent events per frame)
- model attribution in assistant messages
- context window display in status line
- bracket paste mode

## M4 (uncommitted): MCP integration + state persistence + agent improvements

- new `mew-mcp` crate: MCP client with HTTP (streamable) and stdio transports
- `McpClient::connect_http()` / `connect_stdio()` with initialize handshake
- `McpTool` wraps MCP tools into `mew_tools::Tool` with `mcp__<server>__<tool>` namespace
- `load_mcp_configs()` reads Claude Code-compatible `mcp.json`
- `connect_mcp_servers()` wires MCP tools into agent startup
- four HTTP MCP servers configured: context7, grep, deepwiki, alphavantage
- `State` persistence: last-used model/provider saved to `state.toml`
- `load_state`/`save_state` in `mew-config` with test coverage
- `ToolOutput.diff` field + `similar` dep for diff computation
- `AgentEvent::ToolProgress` for live tool output streaming
- `run_with_parts()` for file attachment support in agent

---

## 2026-05-02: MCP client implementation review

Reviewed `crates/mew-mcp/src/lib.rs` against the MCP `2025-11-25` spec. Key findings:

- Initialize handshake is correct but `MCP-Protocol-Version` header is hardcoded (should reflect negotiated version) and `MCP-Session-Id` is not handled at all.
- SSE parsing works for the request-response pattern but ignores `event:`, `id:`, `retry:` fields and doesn't handle server-to-client requests on the stream.
- tools/list pagination is correct; edge case with empty-string `nextCursor` could infinite-loop.
- isError handling is correct; text is properly routed to error/output fields.
- HTTP headers are correct (`Accept`, `MCP-Protocol-Version`).
- Stdio transport has no timeouts, shutdown goes straight to kill (skipping graceful close), and failed init orphans the child process.
- Transport trait has inconsistent error handling between `call` and `notify`, and `call_tool` only extracts text content (drops image/audio/resource/structuredContent results).
- All MCP tools hardcoded to `Sensitivity::Mutating`.

Decided to document findings rather than fix them immediately — these are spec-compliance notes for future improvement.

---

## 2026-05-03: M0–M4 audit pass + gap fixes

Systematic audit of M0–M4 against PLAN.md. Fixed all blocking gaps and most partials:

**M0 gaps fixed:**
- `mew-message`: 23 tests (22 table-driven round-trip + proptest fuzz 100 cases). Added `PartialEq` to all types.
- `mew-provider-openai` + `mew-provider-anthropic`: wiremock SSE fixture replay tests. Fixed broken fixture files (extra `}` in message_delta, malformed JSON escaping in tool-call fixture).
- Anthropic `signature_delta` inbound parsing: added `signature` field to `Delta` struct in `handle_content_block_delta`, wired in agent's `apply_delta`.

**M2 gaps fixed:**
- `mew-skills` crate: full implementation with 8-path discovery (`.mew/skills`, `.opencode/skills`, `.claude/skills`, `.agents/skills` × project + global), YAML frontmatter parsing via `serde_yaml`, name validation (`^[a-z0-9]+(-[a-z0-9]+)*$`, max 64), duplicate resolution (project beats global, first within tier wins). 8 tests.
- `skill` built-in tool: loads skill bodies on demand, registered in `build_tools()`. Skills listed in system prompt as `<available_skills>` XML block.
- Skill permissions: `[[permissions.skills]]` TOML config with `name_glob` matching. `PermissionEngine::check_skill()` evaluates top-to-bottom, first match wins.
- `RuleDecision::Ask` now wired in permission engine as step 2.5 (between Allow and Session allow) — forces a prompt even for ReadOnly tools.

**M0/M1 partials fixed:**
- 3 missing `Dispatcher` hooks: `on_chat_message` called before each turn, `on_event` called after each provider event, `on_shell_env` called in bash tool before subprocess spawn.
- `ToolCtx.dispatcher` added, passed from agent to tools.
- Vision gate: `Agent.supports_vision` flag from catalog, rejects image attachments pre-send.
- Bash env passthrough: calls `on_shell_env` with current env vars, applies filtered result to subprocess.

---

## 2026-05-03: M3 TUI enhancements

All items from the M3 "done when" gate that were missing or partial:

- **Cost computation**: Agent-side per-token cost from catalog pricing (`usage / 1M * price`), refreshed on model switch. `Agent.input_price/output_price/etc` fields.
- **50ms debounce**: Bash output chunks batched via `tokio::time::interval` before forwarding to TUI. Buffer flushes on tick or channel close.
- **Double-ctrl-c**: First ctrl-c during streaming shows "ctrl-c again to quit" (1s window, red). Second exits unconditionally. Status line integration.
- **Multiline input**: Alt+Enter inserts newline. Cursor moves line-wise (home/end). Input area height: `min(line_count, 12) + 2`. Dynamic layout constraint.
- **Slash commands**: `/model <id>` (switch), `/model` solo → opens model picker, `/sessions` (lists .jsonl files by modified time), `/compact` (force compaction next turn), `/resume <id>` (loads session from disk via `mew_session::Reader`). All 8 slash commands implemented.
- **@-mention file picker**: typing `@` at word boundary opens dropdown via existing `Picker`. Walks cwd with `ignore` crate (.gitignore filtered), max depth 4, 1MB file size limit, 50 results sorted shortest-first. Kind "file" routes to `InsertAtMention` action.
- **TUI retry prompt**: `ProviderEvent::RetryWait` emitted by both adapters before each sleep with attempt count, max, delay, and reason (`classify_reason` maps HTTP status to "rate limited"/"server overloaded"). TUI status line shows light-blue countdown.

- **Session resume**: `mew_session::Reader::load(session_id)` reads JSONL. `/resume <id>` loads messages into agent + TUI display, updates session_id.
- **Context compaction**: `/compact` sets `agent.force_compact` for next turn. Auto-compaction at 95% context window (configurable `compaction_threshold`). Keeps last 4 turns, inserts synthetic summary, drops older messages from request. `estimated_tokens()` heuristic (chars / 4).

- **Agent refactor**: split `lib.rs` (1625 lines) into `agent.rs` (139), `turn.rs` (289), `events.rs` (182), `tools.rs` (379), `tests.rs` (685).

---

## 2026-05-03: M5 — router / auto mode

- `mew-provider-router` crate: `Router` wraps small + big `Arc<dyn Provider>`, heuristic picks small for tool-free turns under threshold.
- `Routed` wrapper carries `display_model`/`display_provider` for TUI status line.
- `ProviderConfig` gains `kind` (default "direct"), `small`, `big` fields for TOML config.
- Config example: `[providers.auto] kind = "router" small = "z-ai/glm-4.5-air" big = "z-ai/glm-4.6"`
- 3 tests: simple turn → small, tool results → big, turn threshold → big.
- Wired into `build_provider()`: resolves inner models, sets pricing/vision from big model.

---

## 2026-05-03: M6 — ACP integration (groundwork)

- `mew-acp` crate: hand-rolled JSON-RPC 2.0 framing over stdio (newline-delimited). No external deps.
- `AcpClient`: spawns ACP agent subprocess, handles `initialize` + `session/new`, runs prompt turns via sequential read-after-write.
- Translates `session/update` notifications → `AgentEvent` (agent_message_chunk, tool_call, tool_call_update).
- CLI: `mew chat --acp-agent <cmd>` flag, `mew acp` subcommand (server stub).
- TUI integration and server mode pending.
