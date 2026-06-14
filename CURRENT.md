## Milestones

### M0: Skeleton + OpenAI adapter
`mew-message` canonical types (23 tests, proptest), `mew-provider-openai` (SSE delta-based), `opencode-zen`/`opencode-go` built-in providers, `mew-catalog` (models.dev, 24h cache), `mew-config` (credential resolution, config.toml), `mew-session` (JSONL persistence), `mew-hooks` (Dispatcher trait), clap CLI with `run`/`chat`.
- Anthropic `signature_delta` parsing in `handle_content_block_delta`, vision gate (`supports_vision`).
- Wiremock SSE fixture tests (openai + anthropic), broken fixtures repaired.
- `mew-message` got `PartialEq` on all types, build wire-message image tests (tempdir instead of hardcoded `/tmp/test.png`).

### M1: Anthropic adapter + routing
`mew-provider-anthropic` (SSE content-block events), `z-ai` built-in provider (anthropic-shape, glm models), model routing, catalog parsing fix.

### M2: Tools + permissions + skills
`mew-tools` (Read, Write, Edit, Bash, Glob, Grep, Echo), `PermissionEngine` with declarative rules (`allow`/`ask`/`deny` per glob), `Tool` trait sensitivity levels, `RuleDecision::Ask` wired as step 2.5.
`mew-skills` crate: 8-path discovery (`.mew/skills`, `.opencode/skills`, `.claude/skills`, `.agents/skills` × project + global), YAML frontmatter, name validation, duplicate resolution (8 tests). Skills listed as `<available_skills>` in system prompt. `[[permissions.skills]]` config with `name_glob`.
Dispatcher hooks: `on_chat_message` (pre-turn), `on_event` (post-event), `on_shell_env` (bash tool). `ToolCtx.dispatcher` wired.

### M3: Ratatui TUI
Full event loop with `App` state, streaming text, tool call state tracking, permission modal, model picker, status line, context sidebar.
Markdown via ratatui-mdstream (syntect, cached per message-id, streaming `render_streaming`), tool blocks (colored bg, half-block edges), bash collapse/expand, ansi-to-tui, diff coloring, scrollbar/indicators.
Keyboard UX: esc-to-cancel, slash autocomplete, input editing (ctrl+a/e/f/b, ctrl+u/w, alt+left/right/backspace), multiline input, bracket paste, stream drain coalescing (max 4 events/frame).
Slash commands: `/model`, `/sessions`, `/compact`, `/resume`, `/help`, `/clear`, `/cost` — all 8.
@-mention file picker (walk cwd with `.gitignore` filtering, max depth 4, 1MB limit, 50 results), `InsertAtMention` action.
Cost computation (catalog pricing `usage/1M × price`), retry prompt (`RetryWait` event, HTTP status → reason mapping).
Session resume (`mew_session::Reader::load`), context compaction (auto at 95% window, forced via `/compact`, keeps last 4 turns + synthetic summary).
Agent refactor: `lib.rs` (1625 lines) → `agent.rs`/`turn.rs`/`events.rs`/`tools.rs`/`tests.rs`.

### M4: MCP integration + state
`mew-mcp` crate: HTTP (SSE streamable) and stdio transports, `McpClient::connect_http()`/`connect_stdio()`, initialize handshake, `McpTool` wrapping (`mcp__<server>__<tool>` namespace). Four HTTP MCP servers: context7, grep, deepwiki, alphavantage.
State persistence: last model/provider in `state.toml` (`load_state`/`save_state`). `ToolOutput.diff` + `similar` dep. `AgentEvent::ToolProgress` for live output streaming. Protocol version negotiation fix (accept ≤ max, `MCP-Protocol-Version` header extraction, `std::sync::Mutex` for `HttpTransport`).

### M5: Router / auto mode
`mew-provider-router`: `Router` wraps `small` + `big` `Arc<dyn Provider>`, heuristic picks small for tool-free turns under threshold. `ProviderConfig.kind = "router"`. 3 tests. Wired into `build_provider()`.

### M6: ACP integration
`mew-acp` crate: hand-rolled JSON-RPC 2.0 framing over `Transport` trait (`split() -> (Reader, Writer)`). `StdioTransport`, `DuplexTransport` for tests. 16 tests.
`AcpClient`: spawns subprocess, initialize + session/new, translates `session/update` → `AgentEvent`.
Server mode (`mew acp`): `run_server_on(transport)`, permission gating via `session/request_permission` (spec-compliant JSON-RPC request/response), slash command handling.
CLI: `mew chat --acp-agent <cmd>`, `mew acp`.

### M7: Plugin runtime
`mew-hooks-runtime`: `SubprocessDispatcher` (JSON-RPC subprocess plugins), `PluginLoader` (`.mew/plugins/`, `~/.config/mew/plugins/`), `DynamicTool` wrapper. `Dispatcher` extended with `init`/`shutdown`/`on_register_tools`. `PluginHost` trait. 6 integration tests + Rust sample plugin.

### M8: iroh transport
`mew-acp-iroh`: `IrohTransport<R,W>` impls `Transport`, `PairingTicket` (base64 JSON), `listen(agent)`/`connect(ticket, code)` with OPAQUE (Ristretto255 + TripleDh/SHA-512, 5 tests) + AEAD (ChaCha20-Poly1305 framing, 3 tests). Feature-flagged `iroh` on `mew` binary. CLI: `mew acp --listen iroh`, `mew chat --acp-agent iroh://<ticket>#<code>`.

### M9: Subagents
`mew-subagents`: `SubagentDef`, `Loader` (8-path discovery, YAML frontmatter, 10 tests), `SubagentRunner` trait. `subagent` built-in tool (agent intercepts). `MewSubagentRunner` in main.rs: resolves model (inherit/specific/router), filters tools, creates child Agent, forwards `SubagentEvent`s. `max_turns` enforcement. Config overrides via `[agents.<name>]`. System prompt `<available_subagents>` XML. TUI handles `SubagentStart`/`End`/`PermissionRequest`. @-mention subagent invocation: `@subagent-name` → `InsertSubagentMention` → rewrite on submit.

### MCP protocol version fix
`HttpTransport.set_protocol_version` as trait impl override (was inherent → vtable no-op). `std::sync::Mutex` for protocol_version/session_id (avoid `blocking_lock` panics). `MCP-Protocol-Version` header extraction. `version_acceptable()` checks `server_version <= MAX_PROTOCOL_VERSION`. grep/director/stage servers now connect.

### Cleanup (2026-05-05)
- **Hard blocker fixes**:
  - bash `pid.expect()` → `ok_or_else` error return; cancel polling in output loop
  - `assistant_msg.as_ref().unwrap()` in 8 locations → captured `assistant_id` with early-return guards
  - `subagent_runner.unwrap()` → `if let Some` guard
  - glob: cancel polling in walker loop
  - `read` tool: 10MB file size limit, null-byte binary detection
- **Workspace path sandboxing**:
  - Config: `[workspace] roots = [...]`, defaults to cwd
  - `AgentEvent::WorkspacePermissionRequest` with oneshot response channel
  - `ensure_workspace_path()` checks containment, prompts TUI modal if outside workspace
  - Applies to read/write/edit (file path) + glob/grep (directory path); bash/echo skip
  - Non-interactive CLI auto-allows workspace requests
- **Sidebar compaction**: removed blank headers-and-content gaps, trimmed divider padding (saves 3-4 lines/section).
- **UI module split**: `ui.rs` → `ui/` with `mod.rs` + `chat.rs` + `sidebar.rs` + `input.rs` + `status.rs` + `overlays.rs` + `welcome.rs`.
- MCP sidebar status, context file precedence, copy fix, sidebar collapse, welcome screen.

### 2026-06-13: config system overhaul

- **dotenvy**: `.env` loaded by binary at startup (before tracing, so `RUST_LOG` works).
- **Credential errors**: actionable message showing env var name, keyring command, credentials.json path.
- **config-rs migration** (`config` crate 0.15, TOML feature only):
  - `Config::default()` now contains built-in providers (opencode-zen, opencode-go, z-ai, deepseek) instead of hardcoded fallbacks in `build_provider`.
  - `mew_config::load()` uses config-rs builder: defaults → config.toml → `MEW_` env vars (`__` separator for nesting).
  - `build_provider` simplified: provider lookup is a plain HashMap get, no inline provider definitions.
- **clap state fallback fix**: removed `default_value = "opencode-zen"` from `--provider`; state.toml `last_provider`/`last_model` now actually works. Extracted `resolve_provider`/`resolve_model_opt` helpers.
- **Dead code removal**: `Config.agents` + `AgentConfig` (never read), `PermissionEngine::check_skill` + `SkillPermissionRule`/`SkillMatchConditions` + `PermissionsConfig.skills` (loaded but never enforced), `MEW_RECORD` doc (unimplemented).
- **Clippy/fmt fixes**: pre-existing warnings in mew-tui, mew, mew-acp-iroh cleaned up. Full CI gate passes (`just ci` equivalent: fmt + clippy -D warnings + test --all).

### 2026-06-14: subagent polish (M9.1)

Re-framing: `subagent_start`/`subagent_wait` are model-visible tools, but each invocation creates an isolated **subsession** — its own session file, separate history. The parent never sees the subagent's tool calls in its own message history (already true). Goal: make this concrete, persistent, and observable.

**Phase plan:**
- **0. session folder migration** (foundation): `Writer::open` → `<dir>/<id>/session.jsonl` + `meta.json`. Flat `<id>.jsonl` auto-migrated on first read/write.
- **1. subagent session persistence**: `SubagentRunner::run` gains `parent_session_id`. `SimpleRunner` writes subagent's own session to `<parent>/subagents/<child>/session.jsonl`. Parent's `meta.json` tracks `children_session_ids`.
- **2. nested depth cap**: `max_subagent_depth = 3`. `start_subagent` rejects beyond cap.
- **3. structured `subagent_wait` result**: `SubagentResult` = `Complete { text, turns_used } | Error { reason } | Cancelled`. Tool result encodes status.
- **4. sidebar error state**: `SubagentState.error: Option<String>`. ✗ for failed/cancelled.
- **5. auto-delivery (sync default)**: `subagent_start` gains `async: bool = false`. Default blocks until done, returns final result. `subagent_wait` becomes the opt-in async path.
- **6. cancel**: per-task `CancellationToken` (child of parent's). Sidebar keybinding (`x`). Cascading on parent cancel. `wait` returns `Cancelled` on mid-wait cancel.
- **7. per-subagent model override**: `def.model.unwrap_or(parent_model)`. Future session pop-in (`<parent>/subagents/<child>`) unlocks viewing subagent transcripts.
