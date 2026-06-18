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

### 2026-06-14: bug fixes + subagent time limit

- **plugin settings persistence** (3 layered bugs): `ConfigEditor::save()` only wrote `config.toml`, not `state.toml` (config_editor.rs:525). Model-switch paths in main.rs:1271 and main.rs:1364 constructed a fresh `State` with `disabled_plugins: Vec::new()` and `save_state`'d it, wiping any prior state. Now: `save()` also writes `disabled_plugins` from `self.plugins` (load + mutate + save, preserving other state fields); model switches load existing state and mutate in place. Added `mew_config::save_state_to` for testable roundtrips; 2 mew-config tests.
- **config editor toast height** (config_editor.rs:975): `centered_rect` used `Constraint::Percentage` for height — `3%` of a 24-row terminal rounds to 0/1 rows, so the toast's `Borders::ALL` consumed the only available row leaving just the top border visible. Rewrote to `Constraint::Length` for both axes. Also fixed the "New Provider" naming popup, same bug.
- **streaming tool-call input not persisted** (mew-agent): both providers' `acc.finalize()` parsed `arguments` from streamed deltas into a `Value`, but only into a local copy that was never re-emitted to the agent. `state.input` stayed at `Value::Null` for the entire stream. Effect: every tool call on disk had `"input": null`; `subagent_start` was being called with `prompt = ""` (subagent did nothing). Fix: added `ToolState::set_input()` (mew-message) and `Agent::reconcile_tool_call_input()` hooked into the `PartEnd` event in `mew-agent/src/events.rs`. 4 mew-agent regression tests (3 unit + 1 streaming integration).
- **subagent wall-clock cap**: existing `max_turns` (15-30 on built-ins) is the wrong primary safeguard — a stuck subagent in a tight tool loop burns tokens forever. Added `max_duration_secs` to `SubagentDef` (frontmatter + `AgentConfigOverride`), `hit_time_limit` to `SubagentResult::Complete`, time check in `SimpleRunner` at each `MessageEnd` (Duration comparison, ms resolution). Built-in defaults: 500 turns / 5 minutes (per request). All three tools.rs call sites destructure the new field and emit a parallel warning. 4 mew-agent runner tests (turn limit trips, time limit trips, neither trips, defaults apply).

### 2026-06-15: subagent persistence surfaced

- **Subagent transcript-loss bug**: when `Writer::open_subagent` failed, the runner logged a `warn!` and proceeded with no `Writer`, leaving the user with a subagent that ran but left no trace. Surfaced via new `session_unavailable: bool` field on `SubagentResult::Complete`; tools.rs dispatches now prefix the result with `warning: subagent transcript could not be written; result is unrecorded` (parallel to the existing time/turn limit warnings).
- **Testability of the runner**: added `SimpleRunner::with_session_root(root)` so tests can target a tmp dir without env-var mutation (which races across tokio's parallel test threads). Two new tests: `test_subagent_writes_session_and_updates_parent_meta` (happy path) and `test_session_unavailable_when_cannot_open` (failure path).
- **Bonus**: `MEW_SESSION_DIR` env var now overrides the global session dir (consistent with the project's `MEW_*` env-var pattern, useful for CI / sandboxed runs / multiple instances).
- Also fixed a clippy `manual_range_contains` warning in the previous turn's time-limit test.

### 2026-06-15: subagent exit_tool + progress_update

- **`exit_tool`** (mew-tools/src/tools/exit_tool.rs): graceful-exit tool for subagents. Takes `final_answer: String`, echoes it as the tool's output, ReadOnly sensitivity. The runner detects the call by name (via `AgentEvent::PartUpdated`) and uses the part's `ToolState::Completed::output` as the subagent's `result_text` before breaking the loop. This lets a subagent say "I'm done, here's my final answer" without burning remaining turn/time budget.
- **`progress_update`** (mew-tools/src/tools/progress_update.rs): informational status tool for subagents. Takes `message: String`, returns a confirmation. Calling it does not terminate the run. The tool-call event is forwarded as `SubagentEvent::ToolStart`/`ToolEnd` to the parent, so a UI can show what the subagent is doing.
- **Runner changes** (mew-agent/src/runner.rs): tracks `call_id → tool_name` from `PartStart` events so it can emit `SubagentEvent::ToolStart { call_id, tool_name }` (the previous code only emitted `TextDelta`; `ToolStart`/`ToolEnd` were never sent, leaving a gap in the parent's visibility). Also watches `AgentEvent::PartUpdated` for the `exit_tool` Completed state to short-circuit the loop.
- **Tests**: 3 new runner tests (`test_exit_tool_short_circuits_with_final_answer`, `test_progress_update_does_not_terminate_run`, `test_subagent_tool_start_end_events_emitted`) + 6 tool unit tests. Total: mew-agent 26 → 29, mew-tools 19 → 25.
- **Note on UX**: the TUI doesn't yet render the `progress_update` message in a special way — the parent's view will show "subagent called: progress_update" via the existing `SubagentProgress` plumbing, but the message text isn't surfaced separately. That's a UI-side follow-up, not a runtime issue.

### 2026-06-15: progress_update visible in sidebar

- New `SubagentEvent::Progress { call_id, tool_name, message }` and `AgentEvent::SubagentStatus { parent_call_id, tool_name, message }` variants carry the subagent's `progress_update` message up through the runner → parent pump → TUI chain.
- Runner mirrors the agent's `apply_delta` for `PartDelta { field: "arguments" }` and reconciles the streamed raw input into `state.input` at `PartEnd` time, so it can extract `message` for `progress_update` parts. (The agent's own reconcile also runs at PartEnd, but the runner doesn't share state with the agent's in-memory parts, so the runner has its own minimal copy.)
- `SubagentState` got a `last_progress: Option<String>` field. `handle_agent_event` stores the latest message; the sidebar renders it under the subagent's row (truncated to fit, with `…`).
- Fixed a related test-infra issue: `ScriptedProvider` was replaying the entire script on every `stream()` call, which made the runner loop forever for single-turn tests. Now it yields the script once and returns an empty stream for subsequent calls.
- 1 new runner test (`test_progress_update_emits_subagent_progress_event`) + 1 new TUI test (`test_subagent_status_event_stores_progress`). 10x flake check clean. Total: mew-agent 28 → 29, mew-tui 19 → 20.

### 2026-06-15: subagent personality names

- Just for fun: every subagent run gets a human-friendly name ("Curie", "Turing", "Lovelace", etc.) picked from a 25-name pool. Two `researcher` runs at the same time can now be told apart in the sidebar.
- New `mew_subagents::DISPLAY_NAMES: &[&str]` (25 names) and `pick_display_name(seed: u128) -> &'static str` (splitmix64-based, deterministic, no `rand` dep).
- `SubagentEvent::Started` and `AgentEvent::SubagentStart` got `display_name: Option<String>`. Runner picks the name from the subagent's fresh `SessionId`, so different runs naturally get different names.
- `SubagentState` got `display_name: Option<String>`. Sidebar renders the display name as the primary label, with the def name in parens: `▸ Curie (researcher)  3s`. Falls back to just the def name if no display name was set.
- 4 new picker tests (deterministic, returns known name, covers full pool of 25 across 1000 ulids, distribution not too skewed) + 1 new runner test (Started event includes a valid display_name) + 1 new TUI test (state stores display_name).
- Total: mew-agent 29 → 30, mew-tui 20 → 21, mew-subagents 10 → 14.

### 2026-06-18: /clear clears agent context, not just display

- Previously `/clear` (and the clear keybinding) only wiped the TUI display store via `app.clear_messages()`; the agent's in-memory `messages` (API history) was untouched, so the model still saw the full prior conversation after a "clear."
- New `Agent::clear_context()` (`crates/mew-agent/src/agent.rs`): empties `self.messages` and appends a synthetic clear-marker `Message` to the session JSONL. The session file keeps everything (immutable event log); only what the model sees this turn is reset. Mirrors the compaction-marker pattern in `turn.rs`. Resume reconstructs forward from the marker.
- Wired into all four clear entry points in `run_tui` (`main.rs`): primary-loop `Action::Clear`, drain-loop `Action::Clear`, primary-loop `SlashResult::Clear`, drain-loop `SlashResult::Clear`. ACP path (`chat_with_acp`) intentionally unchanged — no local agent, the remote owns its context. `ResumeSession` path unchanged — it clears display before loading resumed messages, so clearing agent context there would wipe what we just loaded.
- Each clear now pushes a "context cleared" synthetic message so the chat doesn't vanish silently.
- This lands the first piece of the session/context vocabulary from `POLYTOKEN_PARITY.md`: the session is the immutable event log, context is what the model sees this turn.
- 2 new mew-agent tests (empty-messages, marker-written-to-session via tempdir). Total: mew-agent 30 → 32.

### 2026-06-18: flag_important tool (files survive compaction)

- New `flag_important` tool (`crates/mew-tools/src/tools/flag_important.rs`): model marks a file as important for the session so it survives context compaction. `included` mode re-injects the file's content into the request after compaction; `referenced` mode records only a pointer. Re-flagging the same path updates the mode rather than duplicating. `ReadOnly` sensitivity (it mutates session state, not the filesystem).
- `FlaggedFile`/`FlagMode` types live with the tool; shared between tool and agent via `Arc<Mutex<Vec<FlaggedFile>>>`.
- Agent gains a `flagged_files` field (`crates/mew-agent/src/agent.rs`), the Arc created in `main.rs` and handed to both the tool and the agent so they share one list.
- Compaction (`crates/mew-agent/src/turn.rs`) re-injects flagged files into the per-request context after compacting: `Included` files inlined as synthetic user messages, `Referenced` files get a pointer note. Iterated in reverse so flag order is preserved after `insert(0, ...)`. Read failures are logged and skipped. Compaction still doesn't mutate `self.messages` (the session log keeps everything); only the per-request clone is affected, consistent with the session/context model `/clear` introduced.
- Registered in all three agent-construction paths (`run_tui`, `run_acp_server`, `build_and_run`).
- 8 tool unit tests (modes, defaults, errors, re-flag dedup, metadata) + 1 compaction integration test using a new `CapturingProvider` test fixture that records the request messages sent to the provider. Total: mew-tools 25 → 33, mew-agent 32 → 33.
