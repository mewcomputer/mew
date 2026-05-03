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
