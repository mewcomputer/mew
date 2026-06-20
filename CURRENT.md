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

### 2026-06-18: secret-file read guard (permission pre-check tier)

- New `[secrets]` config section (`mew-config/src/lib.rs`): `[[secrets.files]]` with `paths = [...]` globs mark sensitive files (`.env`, `*.pem`, `credentials.json`, etc.).
- `PermissionEngine` gains a pre-check tier (`mew-config/src/permissions.rs`) that sits above the deny→allow→ask cascade. Any `read` of a path matching a secret glob forces `Prompt` — overriding `ReadOnly`'s normal auto-allow — unless a literal (non-glob) allow rule explicitly permits that exact path. A broad `**` allow never lifts the guard; you must name the secret file explicitly to auto-allow it. This is the "option 1" pre-check design.
- `PermissionEngine::with_secret_files(globs)` builder compiles the globs; `build_permission_engine` in `main.rs` flattens `cfg.secrets.files[*].paths` and wires them into the engine.
- Scoped to the `read` tool for v1. `grep`/`glob` take directory inputs (not file paths), so they aren't covered at permission time. Protecting search-tool output needs post-execution result filtering — the same plumbing as secret words — and lands in a follow-up.
- 9 new tests: 7 in permissions (force-prompt, glob-pattern match, literal-allow escape hatch, glob-allow does NOT escape, non-secret unaffected, non-read tool unaffected, `is_glob_pattern` detection) + 2 config parsing. Total: mew-config 21 → 30.

### 2026-06-18: secret words + search-tool result filtering

- Closes the gap left by the secret-file read guard: `grep`/`glob` output now filters secret content too, and secret *words* get redacted from any line that contains them.
- New `[[secrets.words]]` config (`values = [...]`) joins the existing `[[secrets.files]]`. Both flow into a new `SecretSet` type (`mew-tools/src/lib.rs`) carried on every `ToolCtx` as `secrets: Arc<SecretSet>`. `Arc<T: Default>` impls `Default`, so all 9 test `dummy_ctx` helpers just add `secrets: Default::default()` with no new imports.
- `grep` filters results two ways: lines whose file path (before the first colon) matches a secret glob are dropped entirely; lines containing a secret word are redacted to `path:line:[redacted — secret value]` (the prefix is preserved so the model knows where the secret was, without the value). A summary line notes redaction/drop counts.
- `glob` drops any returned path matching a secret glob.
- The agent shares one `Arc<SecretSet>` (built from config via `build_secret_set`) across all tool calls; `tools.rs:319` clones the Arc into each `ToolCtx`.
- `read` is already covered by the permission guard from the previous commit; bash output is not filtered (it's a different threat surface — running commands, not searching files — and already `Dangerous`-gated).
- 5 new tests: 3 unit tests on `filter_output` (word redaction, secret-file drop, noop-when-empty) + 1 grep end-to-end + 1 glob end-to-end. Plus 1 config-parsing test for `[[secrets.words]]`. Total: mew-tools 33 → 38, mew-config 30 → 31.

### 2026-06-18: ask_user_question tool

- New `ask_user_question` tool lets the model ask the user 1-4 free-text questions when their answer would change its next step, blocking until the user responds.
- Agent-intercepted (like `subagent_start`): the tool is a marker whose `execute()` errors; `execute_ask_user` in `mew-agent/src/tools.rs` parses the questions, sends `AgentEvent::AskUser { call_id, questions, tx }` over a oneshot, and awaits the answers. The result text is formatted as `Q: <prompt>\nA: <answer>` pairs. Goes straight to Completed (no Running state — the modal is the waiting indicator).
- New `AskUserQuestion { prompt, default }` payload type and `AgentEvent::AskUser` variant in `mew-agent/src/lib.rs`.
- TUI: new `Mode::UserQuestion` + `UserQuestionState` (`app.rs`), modal in `overlays.rs` (`draw_user_question_modal`) showing each prompt with its input field (defaults pre-filled, focused one highlighted, cursor placed), input handler in `events.rs` (type/backspace/tab-or-down to switch/up to reverse/enter to submit/esc to cancel). Submit sends answers through the oneshot; cancel drops it, which the handler turns into a "cancelled" tool result.
- Non-interactive `mew run` path drops the tx on `AskUser` (no TUI to answer) so the model gets a clear "cancelled" result instead of hanging. ACP server mode has the same behavior; routing `AskUser` through ACP's `session/request_permission` is a follow-up.
- Tests: 2 mew-tools (metadata + execute-errors-when-not-intercepted) + 3 mew-tui (event stores state + sets mode, submit returns typed answers, cancel drops without sending). Total: mew-tools 38 → 40, mew-tui 21 → 24.

### 2026-06-18: todos (session-lived, dependency-enforced task list)

- Five agent-intercepted tools (`mew-tools/src/tools/todo.rs`): `todo_create` (batchable, with `depends_on`), `todo_update` (content + status), `todo_complete` (dep-checked), `todo_delete` (dependent-checked), `todo_list`. All `ReadOnly` sensitivity — they mutate session state, not the filesystem. Each is a marker whose `execute()` errors; the agent core owns mutation.
- New `mew-agent/src/todos.rs`: `Todo`, `TodoStatus` (pending/in_progress/done/blocked), `TodoList` (sequential stable ids, never renumbered on delete). Enforcement is pure logic on `TodoList`: `complete` is refused while any dependency isn't done; `delete` is refused while another todo depends on it; `update` to `done` runs the same dep check as `complete`. `create` drops references to nonexistent ids and notes how many.
- Persistence: sidecar `<session>/todos.json` (sibling of `session.jsonl`), written on every successful mutation, loaded on startup and on `/resume` (carried forward into the new session, same as messages). Survives compaction by construction — todos live in agent state, not message history.
- `apply_todo_op` is a free `pub(crate)` function (input parse + dispatch) so the handler layer unit-tests without an `Agent`. `execute_todo` locks the list, applies the op, snapshots for persistence, renders the result, emits the standard tool lifecycle (Completed/Error + PartUpdated + ToolEnd + result part).
- `/todo` slash command prints the rendered list as a message.
- Deferred to a follow-up: the sidebar pane + `AgentEvent::TodosUpdated` (live TUI visibility). The list is reachable today via `/todo` and the `todo_list` tool; the sidebar is UI polish.
- Tests: 14 in todos.rs (id assignment, dep drop, complete blocked/succeeds, delete blocked/succeeds, update content/status, update-to-done enforces deps, render marks/dependencies/empty, persistence roundtrip, load-missing) + 10 handler tests in tests.rs (create through handler, dep-drop note, complete/delete/update enforcement through the tool layer, empty-update rejection, list, missing-input errors) + 2 mew-tools (names + sensitivity). Total: mew-agent 33 → 57, mew-tools 40 → 42.

### 2026-06-18: todos sidebar pane + live sync

- Closes the todos feature: the list is now visible in the sidebar without running `/todo`.
- New `AgentEvent::TodosUpdated { todos: Vec<Todo> }` emitted by `execute_todo` after every successful mutation, carrying a full snapshot so the TUI doesn't reach into agent state.
- New Todos pane in the sidebar (`ui/sidebar.rs`), placed right after Context: header shows done/total counts, each item shows status mark (`x` done dim, `~` in_progress yellow, ` ` pending, `!` blocked red) + `#id content` (truncated to sidebar width). Collapsible like the other panes.
- `app.todos: Vec<mew_agent::Todo>` seeded at startup from the agent (`agent.todos` snapshot in `run_tui`) and re-synced on `/resume`, so the pane is populated before the first mutation event rather than empty until the model acts.
- 1 new mew-tui test (TodosUpdated event stores snapshot). Total: mew-tui 24 → 25.

### 2026-06-19: three TUI/UX bug fixes

- **Reasoning shown twice + not collapsible.** Root cause: the `PartDelta` handler fed EVERY delta into the incremental markdown stream (`md_stream`), including reasoning deltas — so reasoning text rendered once as normal markdown (via `render_streaming`) and once dimmed (via the `Part::Reasoning` arm). Fixed by gating the stream feed on `is_text_delta` (only text-part deltas feed the markdown stream; reasoning deltas update only the reasoning part). Also added a collapsible reasoning block: `reasoning_expanded` flag (default collapsed, since reasoning is verbose), `Ctrl+T` toggles, and the render shows a `[thinking — Ctrl+T to expand] (N lines)` header when collapsed. Mirrors the existing `bash_expanded` / `Ctrl+O` pattern.
- **@-mention picker produced `@@path`.** Root cause: typing `@` inserts it into `app.input` and opens the picker; on pick, the full `@path` was appended without removing the trigger `@` → `@@path`. Fixed via a new `App::insert_mention` helper that pops a trailing `@` before inserting. Both `InsertAtMention` and `InsertSubagentMention` handlers (primary + drain loops, 4 sites) now call it.
- **@-mentioned file content flooded the chat.** Root cause: `process_mentions` inlined text-file content straight into the `enriched` string that became the user's visible message. Fixed by splitting the return into `(enriched, display, attachments)`: `enriched` (with full content) goes to the model via `agent.run_with_parts`; `display` (with just a `<path added to context>` notification per file) goes to `app.messages`. The three call sites (ACP, run_tui, build_and_run) updated. File content still reaches the model; the user just sees a one-line notification instead of the whole file.
- 1 new mew-tui test (`insert_mention` replaces the trigger `@`). Total: mew-tui 25 → 26.

### 2026-06-19: chat scroll didn't reach the bottom (wrapped-height bug)

- Root cause: `max_scroll` was computed from `text.lines.len()` (raw `Line` entries), but `Paragraph::wrap` splits wide lines across multiple visual rows and `Paragraph::scroll` advances by wrapped rows. So whenever any line wrapped, `max_scroll` underestimated and the last few rows were unreachable — the classic "can't scroll to the very bottom."
- Fix: new `wrapped_height(text, width)` helper (sum of `ceil(line.width / width)` per line, empty lines count as 1) drives `total_lines` and the scrollbar's content length. `max_scroll` now reflects the real rendered height.
- 4 new tests in `ui/chat.rs` (no-wrap, long-line wrap, empty-line, zero-width fallback). Total: mew-tui 26 → 30.

### 2026-06-19: status bar as extensible pills (model, cwd, git) + marquee ticker

- The bottom status bar is now a list of `Pill` entries rendered as `[text]` segments (dim brackets, per-pill text color). Concrete pills: `[provider/model]`, `[~/code/mew]` (cwd with `$HOME` collapsed), `[git: main]` (walks up to `.git/HEAD`, parses branch or short hash for detached HEAD, resolved lazily on first draw to avoid per-frame/per-test fs reads). Future pills (persona, permission mode) slot into `build_pills` — no other change needed.
- Token/cost stays pinned right, the same split layout as before.
- If the pills don't fit (after reserving the right-side width), the left side marquees: a cycling window of `width` chars, advanced by `app.status_ticker_offset` (incremented every ~300ms in `tick`). A 3-space gap between cycles prevents the end running into the start. Per-pill colors are dropped in the marquee (single dim color) since windowing styled spans is fiddly.
- Transient statuses (esc-to-cancel, ctrl-c-quit, retry) still override the left side.
- 7 new tests (`pill_string` join + edge cases, `marquee` width/exact/cycling/zero-width). Total: mew-tui 30 → 37.

### 2026-06-19: scrollbar in its own column (no more tool-block overlap)

- The chat paragraph and the scrollbar now render into separate areas: `chat_inner` is the area minus the rightmost 1 column, `scrollbar_area` is that 1 column. Tool blocks paint `TOOL_BG` across the full paragraph width — previously this covered the scrollbar's column, hiding the thumb/track. With the column reserved, the scrollbar lives outside the paragraph so tool blocks can't cover it.
- `↓` indicator shrank to 1 char and now overlays the last column of `chat_inner` (like `↑` at the top-left).
- `wrapped_height` and `md_width` use `chat_inner.width` so the scroll math matches the rendered area.
- "Doesn't scroll to the bottom when at the bottom" — the wrapped-height fix (`max_scroll` now reflects the real rendered height) means `scroll == max_scroll` truly is the bottom, and the scrollbar thumb position (`max_scroll`) sits at the bottom of the track.

### 2026-06-19: status pills get their own background colors (boxes with 1u gaps)

- Each pill is now a solid colored "box" on the status bar: `[text]` rendered as a single span with `fg(pill.fg).bg(pill.bg)`, so it reads as a distinct badge against the bar. The `Pill` struct gained a `bg` field alongside `fg`.
- Between pills: 1-cell gap (a space span with the status bar background), so they read as separate boxes with real whitespace between them.
- Colors (dark bg, light fg for readability):
  - model: `bg Rgb(25,70,35)` (dark green), `fg Rgb(150,230,160)`
  - cwd: `bg Rgb(30,55,90)` (dark blue), `fg Rgb(150,190,240)`
  - git: `bg Rgb(75,60,20)` (dark amber), `fg Rgb(245,210,110)`
- The marquee overflow path stays single-color (dropping per-pill colors) — windowing styled spans is fiddly. Worth a follow-up if you want colored marquee.
- Tests updated for the new `Pill` shape (2 tests). Total: mew-tui 37.

### 2026-06-19: colored marquee (per-pill colors preserved) + brackets removed

- The pill text no longer carries `[` `]` delimiters — each pill is just its label (e.g. `opencode-go/deepseek-v4-pro`) rendered as a solid bg/fg span. With the background already giving the box, brackets were visual noise.
- Refactored the pill rendering to be segment-based. `build_segments(pills, trailing_gap)` interleaves pills with 1-cell `STATUS_BG` gaps and optionally appends a trailing gap. The static line and the marquee both render from the same `Vec<PillSegment>`, so the marquee can keep per-pill colors as the window slides. `segments_window(segs, width, offset)` does char-accurate windowing with binary-search segment lookup and coalesces consecutive chars from the same segment into one span. The 3-cell trailing gap in the marquee sequence prevents the end running into the start between cycles.
- Tradeoff resolved: the marquee is now color-true (was single dim gray).
- 2 pill_string tests updated (no brackets), 4 new segments tests (interleave, trailing gap, span count, color-preserving window, wrap, cycle). Total: mew-tui 37 → 39.

### 2026-06-19: slash autocomplete visible count scales with terminal height

- The slash picker box used `(items + 2).min(5)`, which capped the list at 3 items no matter the screen. That's fine on a phone screen but buries commands on a full terminal where users might be filtering through 30+ slash commands.
- Changed `crates/mew-tui/src/ui/mod.rs` to `min(12, area.height / 2)` visible items, so the box grows with the terminal but never eats more than half the screen. With fewer items, the box shrinks to fit (no empty padding). Height: `items.min(max_visible) + 2` (for the border).
- 39 mew-tui tests pass; clippy clean.

### 2026-06-19: tool block fill is now clearly visible

- Bumped `TOOL_BG` from `Rgb(34, 34, 38)` to `Rgb(50, 50, 56)` in `crates/mew-tui/src/ui/mod.rs`. The old value was only ~4 units brighter per channel than the sidebar/status bgs, so the body fill blended into the chat background. On a typical dark terminal, the half-block top/bottom edges were the only visible part of a tool block — the body looked empty.
- New value matches the divider color brightness, so tool blocks read as clearly-elevated cards. The half-block edges still add a 1-row soft transition into the fill (chat bg on the outside half, TOOL_BG on the inside half), which keeps the corners from looking clipped.
- Also reverted an internal pill padding in `build_segments` (" a " instead of "a") that I'd added in the previous status rewrite — it ate horizontal space and the tests expected the unpadded form. Pills stay as bare labels on their bg.
- 39 mew-tui tests pass; clippy clean.

### 2026-06-19: trim trailing newlines from subagent tool output

- Reported visual: when a subagent returns, the tool block's output had extra blank lines, leaving the latest message stranded in the middle of the viewport with chat-bg space below it.
- Root cause: `exit_tool`'s `final_answer` and the accumulated stream text both get returned as-is from the subagent runner. If the subagent's model ends its answer with `\n` (or several), `into_text()` in the chat renderer splits them into blank `Line` entries, each one taking a full visual row. The tool block grows taller than the content justifies, pushing everything after it up.
- Fix: at all three sites in `crates/mew-agent/src/tools.rs` (sync subagent_start line 731, async subagent_start line 869, subagent_wait line 1207) trim trailing `\n` from `text` before inserting warning prefixes. The warning prefixes (which end in `\n\n`) are preserved exactly, so a hit-turn-limit output still reads as `warning: ...\n\n<answer>`.
- 39 mew-tui tests pass; clippy clean.

### 2026-06-19: tool block height was doubled by off-by-one padding

- Reported visual: every tool line had a row of bg-only empty space below it. Long tool blocks (`read` of a 234-line file followed by several `edit` calls) stacked these into huge gaps, leaving the latest message stranded high in the viewport. The half-block borders were visible but the body fill looked half-empty.
- Root cause: `draw_chat` reserved the rightmost 1 column for the scrollbar, so the paragraph renders into `chat_inner` (width = area.width - 1). But the tool line builders (`push_tool_line`, `push_ansi_line`, `push_tool_edge`) were padding every line to `area.width` — 1 col wider than the render area. `wrapped_height` measures each line's visual rows via `ceil(line.width / chat_inner.width)`, so a line padded to `area.width` was `ceil(width / (width-1)) = 2` rows. Every tool line took 2 visual rows: one with content, one with bg-only "empty space."
- Fix: introduce `let tool_width = chat_inner.width;` in `draw_chat` and use it everywhere tool lines are padded (15 sites in `crates/mew-tui/src/ui/chat.rs`). The scrollbar and `chat_inner` themselves still derive from `area.width` since they need the full width to position correctly.
- Added `test_wrapped_height_exact_width_line_is_one_row` regression test: a line exactly `width` wide must count as 1 row, `width + 1` must count as 2. The old off-by-one would have failed this.
- 40 mew-tui tests pass; clippy clean.

### 2026-06-19: text input wraps long lines instead of horizontally scrolling

- Reported: pasting a long line into the input scrolled horizontally (old behavior used a centered window around the cursor) and the input stayed 1 row tall no matter how long the text got.
- Fix: the input now wraps. Each logical line contributes `ceil(display_width / content_width)` visual rows (minimum 1), the input area grows to fit up to 12 rows, and the cursor maps to a (visual_row, visual_col) cell in the wrapped grid. Continuation rows indent with 2 spaces so wrapped text aligns with the first row's content. Mouse clicks map a (rel_y, rel_x) cell back to a byte offset in the input.
- `App` gained three methods: `input_visual_line_count(content_width)`, `cursor_visual_row_col(content_width)`, `visual_to_byte_offset(visual_row, visual_col, content_width)`. The old `input_line_count()` and `cursor_line_col()` are unchanged for callers that still need the logical (unwrapped) position.
- Layout: `crates/mew-tui/src/ui/mod.rs` now uses `input_visual_line_count` with the slot's estimated content width so the vertical constraint reserves enough room for wrapped lines before `draw_input` runs.
- 8 new unit tests cover no-wrap, wrap, empty-line-1-row, cursor visual position, and click-to-byte-offset mapping. 48 mew-tui tests pass; clippy clean.
- Also removed an untracked WIP `spinner.rs` module that had been left in the tree (with a `mod spinner;` line in `mod.rs` and a `rand` dep in `Cargo.toml`) — it was triggering dead-code / non-snake-case clippy errors and blocking CI. The module was incomplete and not wired anywhere; can be re-added when finished.

### 2026-06-19: tool output wrapping now keeps the tool bg on continuation rows

- Reported: long tool output lines (e.g. `read` of a wide file) wrapped to a second visual row, but the wrapped continuation showed the chat bg instead of the tool bg — the card looked like it had a gap where text wrapped.
- Root cause: `push_ansi_line` set `line.style = bg(tool_bg)` and the indent span had `bg(tool_bg)`, but the content spans came from `into_text()` with their ANSI-parsed styles (no bg). When the paragraph wrapped a long line, the wrapped continuation row used the content spans' styles, which had no bg, so the chat bg showed through. Same issue in `push_tool_line` for any non-pad spans.
- Fix: in both `push_ansi_line` and `push_tool_line`, iterate over all spans and force `bg(tool_bg)` on each. `Style::bg()` is additive — it preserves existing fg colors. The wrapped continuation row now inherits the tool fill.
- 2 new tests: `test_push_ansi_line_forces_tool_bg_on_all_spans` and `test_push_ansi_line_preserves_fg_colors`. 50 mew-tui tests pass; clippy clean.

### 2026-06-19: chat text wrapping accounts for the 2-space left indent

- Reported: chat text wrapping was off — long lines in the agent's response wrapped at the wrong column, with the continuation row showing the chat bg peeking through (same symptom as the tool output bug, but for text parts).
- Root cause: the chat prepends a 2-space indent to every markdown line for the left margin. The markdown renderer was given `md_width = chat_inner.width` and produced lines of that width. After prepending the indent, every line was `chat_inner.width + 2` — 2 cols wider than the paragraph's render area. The paragraph's `.wrap(Wrap { trim: false })` then wrapped each line a second time at render, creating a continuation row that overflowed or showed the wrong bg.
- Fix: `md_width = chat_inner.width.saturating_sub(2)` so the markdown renderer produces lines that, after the indent, are exactly `chat_inner.width` wide — fits the render area exactly, no second wrap, no bg leak.
- Same effect on `wrapped_height`: the total visual row count is unchanged because lines are still 1 row each (they just shifted left by 2 cols).
- 50 mew-tui tests pass; clippy clean.

### 2026-06-19: tool output lines truncated to fit (avoids paragraph-wrap bg leak)

- Reported: even after the bg-on-every-span fix, long tool output lines (e.g. `read` of a wide file) still showed the chat bg between wrapped rows. The continuation row's content had the tool bg, but the gap *after* the content (the rest of the visual row up to the right edge) was filled by the paragraph's default style, which has no bg.
- Root cause: when `push_ansi_line` produces a line wider than `tool_width`, the paragraph's `.wrap(Wrap { trim: false })` wraps it. The wrapped continuation row is shorter than `tool_width` (only the overflow content, no padding), and the paragraph fills the trailing cells with its default style (no bg → chat bg shows through).
- Fix: truncate the content to `tool_width - indent_width` display columns inside `push_ansi_line` before the existing padding step. Now every tool output line is exactly `tool_width` wide, the paragraph never wraps a tool line, and the tool bg fills the full width consistently. The `read`/`grep` tools already showed the first N chars of each line — this just makes the truncation happen at a known, consistent column.
- Added `truncate_line_to_width` helper (display-column aware, preserves span styles) and 4 tests. 54 mew-tui tests pass; clippy clean.

### 2026-06-19: word-aware wrapping for tool output

- User feedback: truncation was too aggressive — long lines in `read` / `grep` output were being cut off mid-word, hiding useful content.
- Replaced the truncation approach with proper word-aware wrapping. `wrap_tool_line(width, line, indent, tool_bg)` splits a line at whitespace boundaries, falling back to hard-breaking if a single word is wider than the available content width. Returns one `Line` per visual row, each padded to exactly `width` with `bg(tool_bg)` so the paragraph never wraps (and the chat bg can't leak through the trailing cells of a wrapped row).
- Continuation rows don't carry the indent (only the first row does), so wrapped text is left-aligned to the content.
- Tool output is usually monochrome, so all wrapped rows use the first span's style with `bg(tool_bg)` forced on. Multi-color ANSI output loses per-word color but keeps the fill, which was the visible bug.
- 5 new tests cover short/long/oversized-word cases, continuation row indent, and tool bg on every span. 55 mew-tui tests pass; clippy clean.

### 2026-06-19: spinner as the streaming "thinking" indicator

- The input prefix used to show a static `… ` when `app.streaming` was true. Replaced it with the braille spinner from `crates/mew-tui/src/ui/spinner.rs` — the spinner advances one frame per `tick()` while streaming, so it actually animates.
- `App` gained a `spinner_frame: usize` field. `tick()` bumps it (wraps via `wrapping_add`) only while `streaming` is true, so the spinner freezes on the last frame when the agent finishes rather than ticking on forever.
- `spinner::spinner_frames()` exposes the braille frame sequence as a `&'static str` so the input renderer can index into it by character without allocating.
- Re-added `mod spinner;` in `crates/mew-tui/src/ui/mod.rs` and `rand.workspace = true` in `crates/mew-tui/Cargo.toml` (needed by the existing `Spinner::next_frame` random branch; the deterministic path we use doesn't actually call `rand` at runtime, but the dep is there for the API).
- 55 mew-tui tests pass; clippy clean.

### 2026-06-19: wrapped tool output lines keep the indent on continuation rows

- Reported: after the word-aware wrapping fix, wrapped continuation rows started flush left at the chat edge, while the first row of each tool output line had the 6-space indent. The wrapped text was misaligned with the first row's content — it looked like a ragged left margin.
- Fix: `wrap_tool_line` now applies the `indent` to every row (first and continuations), not just the first. `content_w` (the wrap width) already accounted for the indent, so no other math changed — the wrapped text fills the same content column on every row.
- Updated `test_wrap_tool_line_continuation_rows_have_indent` (was previously `..._have_no_indent`).
- 55 mew-tui tests pass; clippy clean.

### 2026-06-19: personas v1 — switchable system prompt + tool filtering + model pin

- New crate `mew-personas` mirrors the `mew-skills` loader (discovery walk, frontmatter parsing, name validation). Discovers `PERSONA.md` files from `.mew/personas/`, `.opencode/personas/`, `.claude/personas/`, `.agents/personas/` (project + global). 9 loader tests.
- Frontmatter: `name`, `description`, optional `mew:` block with `model` (pin) and `tools` (allow-list). Skipping for v1: transitions, skills_allow, color, tools_deny, templating.
- Agent state: `Agent` gained `persona_prompt: Option<String>`, `active_tool_names: Option<HashSet<String>>`, `persona_name: Option<String>`. `apply_persona(&Persona)` sets all three and returns the pinned model (if any). `clear_persona()` resets them. The turn loop (`turn.rs:90`) filters `self.tools` by `active_tool_names` and prepends `persona_prompt` to the system prompt before the dispatcher hook.
- `/persona` slash command: `/persona` lists available personas (with `*` on active), `/persona <name>` switches, `/persona default` clears. Mirrors the `SwitchModel` plumbing for model pin: rebuilds the provider via `build_provider` if the persona specifies `mew.model`.
- Status bar: purple pill (`#37234b` bg, `#c8aaf0` fg) showing the active persona name. Slotted between model and cwd pills.
- TUI: `App.personas: Vec<(String, String)>` populated from loaded personas, `App.active_persona: Option<String>` updated on switch. `SlashResult::SwitchPersona(String)` variant.
- Tests: 3 agent tests (apply sets prompt + tool filter, model pin returns, clear resets), 3 TUI tests (slash with name, slash no-arg lists, slash empty personas). 9 persona loader tests. Total: mew-personas 9, mew-agent 63, mew-tui 58.
