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
- **Daemon persona protocol** (`43b934ea`): `ListPersonas`/`SwitchPersona` `ClientMessage`s, `PersonaList`/`PersonaSwitched` `ServerMessage`s, `PersonaInfo` wire struct. Daemon handlers read `agent.personas` and call `agent.apply_persona()` with pinned model support. Roundtrip tests added.
- **Daemon attachment forwarding** (`43b934ea`): `Prompt.attachments` is no longer ignored — converted to `Part::File` and forwarded to `agent.run_with_parts()`. Unblocks web paste/drop and iOS file picker.
- **Web Wave 2** (`43b934ea`): Presence/yield control UI (chips + yield button in status footer). File service (file-tree.tsx with FileTreePanel + ChangesPanel, Files/Changes tabs in right rail, watchWorkspace). Session group management (create/rename/delete/reorder/color, move-to-group dropdown).
- **TUI Items 1+3+4+7** (`43b934ea`): Live session rail in sidebar (state glyph, title, attention badge, cost). Session switcher picker + `/sessions` command. `Action::AttachSession`. Cross-session attention pill. `/resume` verified end-to-end. Auto-title/summary toggle commands (`/autotitle`, `/autosummary`). Session title pill in status bar.

## Remaining cross-frontend work

### Daemon-side blockers

The persona protocol and attachment forwarding have landed. Remaining:

1. **ListSlashCommands / SlashCommandList** (web plan item 10 proper, shared by iOS B2 autocomplete)
   - Optional — same `mew-protocol` + `mew-daemon` touch point as the persona protocol (now landed).
   - Web has a minimal hardcoded list; iOS Phase B2 can use a hardcoded list for v1.

### TUI remaining

Items 0, 1, 3, 4, 6, 7 have landed. Remaining:

- **Item 5 — Project picker + new-session-in-cwd**: `/project` command, project picker, `Action::NewSessionInProject`. Needs `ListProjects` ClientMessage (doesn't exist yet — check if daemon tracks projects).
- **Item 8 — Change stats + flagged files**: `SessionStatsChanged`/`FlaggedFilesChanged` already arrive via notify channel. Need a "Changes" sidebar section + diff pill.
- **Item 2 — Archive / pin / groups**: Session picker actions for archive/pin, grouped rendering by `group_id`. Least valuable, do last.

### Web remaining

Wave 1 and Wave 2 are done. Remaining:

- **Wave 3**: personas (protocol landed — wire up `PersonaPill`/`InputArea` to call `listPersonas`/`switchPersona`), attachments (daemon forwarding landed — wire up paste/drop + bridge upload), dynamic slash commands (optional, needs `ListSlashCommands` protocol).
- **Ping/pong note**: the web client added `ping`/`pong` types, but the Rust daemon doesn't have a handler yet.

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

1. **Web Wave 3** (personas + attachments + paste/drop): protocol and daemon forwarding have landed — now pure UI work.
2. **TUI Items 5, 8, 2**: project picker, change stats, archive/pin/groups. All build on the Item 0 foundation.
3. **iOS Phase B** batch (Rust → regen → Swift). Note: `just ios-core` recipe was removed — need to investigate Xcode-based regen or restore the recipe first.
4. **iOS Phase C** (attachments — daemon forwarding has landed, just needs iOS UI).
5. **iOS Phase D** (subagents) — large, isolate it.
6. **iOS Phase E/F/G** as follow-ups; Phase G after persona protocol (now landed).

## Things to keep in mind

- **Binding regen is all-or-nothing.** Every iOS Rust change that touches the FFI surface needs `just ios-core`. Batch them aggressively.
- **The TUI render cache was removed** by another agent's refactor. `mark_chat_dirty()` and `RenderedChat` no longer exist. Chat rendering is inline in `draw_chat`. The `ios-core` justfile recipe was also removed (build moved to Xcode project).
- **Clippy is now a hard gate.** New `mew-protocol` test patterns should avoid nested `match` arms on wire variants (collapsible_match lint).
- **Shared tree hazard.** Multiple agents are touching this repo concurrently. Keep commits focused and intentional; don't let `cargo fmt` runs reformat unrelated in-flight changes.
