# Finish-up parity plan

> Roll-up of remaining cross-frontend parity work after the 2026-07-05 audit plans. Companion to `tui-parity-plan.md`, `web-parity-plan.md`, and `ios-parity-plan.md`.
>
> The three plans are complete in detail; this doc exists to show what's left, in what order, and which pieces block multiple frontends.

## Already landed (no longer in the TODO column)

- **Tool sensitivity threading** (`742f4bdc`): `ToolCallPart.sensitivity` is stamped by the agent, flows through `mew-protocol`, and is read by web + iOS. Hard-coded tool-name → sensitivity maps are gone.
- **TUI scroll/render cache** (`7ff470a6`): `draw_chat` is now O(visible) via `App::ensure_chat_rendered` + `build_chat_lines`. Tool-call batching (TUI plan Item 6) and `SessionHistory` replay (Item 4) should be implemented against this cache.
- **Reviewer built-in renamed to `plan-reviewer`**: the previously broken `"reviewer"` subagent registration now reads `subagents/plan-reviewer.md` (the file that actually exists). Duplicate `reviewer.md` removed.
- **Clippy clean across the workspace**: `cargo clippy --all --tests -- -D warnings` passes.
- **TUI Item 0 — Daemon notification transport** (`ee62b5a8`): Persistent `notify_tx` channel in `DaemonClient` (always alive, unlike prompt-scoped `event_tx`). Session-management `ServerMessage`s (SessionList, SessionHistory, SessionTitleChanged, SessionAlert, etc.) forwarded to notify channel instead of dropped. Typed request methods (`list_sessions`, `new_session_in`, `archive_session`, `pin_session`, `set_auto_title/summary`, `rename_session`, `unflag_file`). `App::apply_daemon_notification()` reducer. `tokio::select!` in `chat_with_daemon` event loop.
- **TUI Item 6 — Tool-call batching** (`ee62b5a8`): Consecutive `Part::ToolCall` entries collapsed into summary rows ("✓ 3 tool calls: read, glob, grep"). Active/streaming batches auto-expand. `tool_batch_expanded: HashSet<PartId>` for toggle state. Implemented inline in `draw_chat` (not `build_chat_lines` — the render cache was removed by another agent's refactor; batching is in the main render loop).
- **Web Wave 1** (`ee62b5a8`): Retry toast (mount `<Toaster>`, toast on `retry_wait`). Daemon version display (`ping`/`pong` client types, render in status footer). Background jobs tab (jobs Map in store, Jobs tab in right rail). Input history recall (ArrowUp/Down in input area). Dead code deleted (`subagent-panel.tsx`, `settings-modal.tsx`).
- **iOS Phase A** (`ee62b5a8`): Rename session UI (swipe action + alert in `SessionRailView`). Retry affordance (`lastPrompt` tracking, `retryLastTurn()`, retry banner in `ChatView` with disabled state when no prompt to retry).

## Remaining cross-frontend work

### Daemon-side blockers (do these first — they unblock multiple frontends)

1. **Persona protocol** (web plan item 2, blocks iOS Phase G)
   - Add `ListPersonas` / `SwitchPersona` `ClientMessage`s, `PersonaList` / `PersonaSwitched` `ServerMessage`s in `mew-protocol`.
   - Daemon handlers in `mew-daemon/src/lib.rs` read `agent.personas` and call `agent.apply_persona()`.
   - Once landed, web can wire `PersonaPill`/`InputArea` and iOS can implement Phase G.

2. **Daemon attachment forwarding** (web plan item 1, blocks iOS Phase C)
   - `ClientMessage::Prompt.attachments` is currently ignored; `run_turn` passes `vec![]` to `agent.run_with_parts`.
   - Decide bridge upload endpoint (option A) vs inline bytes (option B).
   - After the daemon forwards `Attachment` as `Part::file`, both web paste/drop and iOS file picker become purely UI work.

3. **ListSlashCommands / SlashCommandList** (web plan item 10 proper, shared by iOS B2 autocomplete)
   - Optional but recommended alongside the persona protocol edit — same `mew-protocol` + `mew-daemon` touch point.

### TUI remaining

Item 0 (daemon notification transport) and Item 6 (tool-call batching) have landed. The render cache was removed by another agent's refactor, so batching is implemented inline in `draw_chat` (not in `build_chat_lines`). Note: `mark_chat_dirty()` no longer exists — the cache is gone.

- **Item 4 — SessionHistory replay**: the reducer already handles `SessionHistory` (clears messages, pushes history, sets `auto_scroll`). The `/resume` path works. Remaining: verify end-to-end and potentially request a fresh `SessionList` after attach.
- **Items 1/3/5/7/8/2**: straightforward now that Item 0 exists; most are reducer + sidebar/status/picker wiring. Item 1 (session rail) is highest value.

### Web remaining

Wave 1 is done. Remaining:

- **Wave 2**: presence/yield, file service, group management (web-only UI on existing client methods).
- **Wave 3**: personas (blocked on daemon protocol above), attachments (blocked on daemon forwarding above), dynamic slash commands.
- **Ping/pong note**: the web client added `ping`/`pong` types, but the Rust daemon doesn't have a handler yet. The `client.ping()` call in `hooks.ts` will get an error response until a Rust `Pong` handler is added (Wave 3 or alongside the daemon protocol batch).

### iOS remaining

Phase A is done. Remaining:

- **Phase B batch** (B1 permission modes, B2 slash, B3 usage/context, B4 turn failure, B5 todos; optionally B6 thinking): do all Rust first, run `just ios-core` once, then Swift.
  - Note: `context_window` already exists in `mew-mobile-core` (`events.rs:192`, `state.rs:92`), so B3 Rust work is threading it through, not adding a new protocol field.
- **Phase C** (attachments): after daemon attachment forwarding lands.
- **Phase D** (subagents): largest item; one regen pass.
- **Phase E** (alerts banner): no regen.
- **Phase F** (auto-title/summary): rides any regen batch.
- **Phase G** (personas): blocked on web plan item 2 protocol.

## Suggested global execution order

1. **Daemon protocol batch**: persona + attachment forwarding + `ListSlashCommands`. This unblocks web Wave 3 and iOS Phases C/G.
2. **Web Wave 2** and **iOS Phase B** batch in parallel — Wave 2 is web-only UI, Phase B is Rust → regen → Swift.
3. **TUI Items 1–5, 7–8**: session rail (Item 1, highest value), alerts (Item 3), auto-title (Item 7), project picker (Item 5), change stats (Item 8), archive/pin/groups (Item 2).
4. **iOS Phase C** after daemon attachment forwarding.
5. **iOS Phase D** (subagents) — large, isolate it.
6. **iOS Phase E/F/G** as follow-ups; Phase G after persona protocol.

## Things to keep in mind

- **Binding regen is all-or-nothing.** Every iOS Rust change that touches the FFI surface needs `just ios-core`. Batch them aggressively.
- **The TUI render cache was removed** by another agent's refactor. `mark_chat_dirty()` and `RenderedChat` no longer exist. Chat rendering is inline in `draw_chat`. The `ios-core` justfile recipe was also removed (build moved to Xcode project).
- **Clippy is now a hard gate.** New `mew-protocol` test patterns should avoid nested `match` arms on wire variants (collapsible_match lint).
- **Shared tree hazard.** Multiple agents are touching this repo concurrently. Keep commits focused and intentional; don't let `cargo fmt` runs reformat unrelated in-flight changes.
