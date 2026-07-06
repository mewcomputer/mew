# Finish-up parity plan

> Roll-up of remaining cross-frontend parity work after the 2026-07-05 audit plans. Companion to `tui-parity-plan.md`, `web-parity-plan.md`, and `ios-parity-plan.md`.
>
> The three plans are complete in detail; this doc exists to show what's left, in what order, and which pieces block multiple frontends.

## Already landed (no longer in the TODO column)

- **Tool sensitivity threading** (`742f4bdc`): `ToolCallPart.sensitivity` is stamped by the agent, flows through `mew-protocol`, and is read by web + iOS. Hard-coded tool-name → sensitivity maps are gone.
- **TUI scroll/render cache** (`7ff470a6`): `draw_chat` is now O(visible) via `App::ensure_chat_rendered` + `build_chat_lines`. Tool-call batching (TUI plan Item 6) and `SessionHistory` replay (Item 4) should be implemented against this cache.
- **Reviewer built-in renamed to `plan-reviewer`**: the previously broken `"reviewer"` subagent registration now reads `subagents/plan-reviewer.md` (the file that actually exists). Duplicate `reviewer.md` removed.
- **Clippy clean across the workspace**: `cargo clippy --all --tests -- -D warnings` passes.

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

TUI plan Items 1–8 all depend on Item 0, the daemon notification transport. The cache work above doesn't change the plan, but it changes where the rendering code lives.

- **Item 0 — Daemon notification transport** (foundation): add a persistent `notify_rx` channel to `DaemonClient`, forward the currently-dropped `ServerMessage` variants, and add `apply_daemon_notification` to `App`.
- **Item 6 — Tool-call batching**: implement inside `build_chat_lines`; toggle calls `mark_chat_dirty()`.
- **Item 4 — SessionHistory replay**: reducer calls `clear_messages()` + `mark_chat_dirty()`; scroll ceiling recomputed automatically.
- **Items 1/3/5/7/8/2**: straightforward once Item 0 exists; most are reducer + sidebar/status/picker wiring.

### Web remaining

- **Wave 1 quick wins**: retry toast, daemon version, Jobs tab, input history (no Rust changes).
- **Wave 2**: presence/yield, file service, group management (web-only UI on existing client methods).
- **Wave 3**: personas (blocked on daemon protocol above), attachments (blocked on daemon forwarding above), dynamic slash commands.

### iOS remaining

iOS is the implementation track with the most remaining work. Phases A–G are in `ios-parity-plan.md`; the key ordering is:

- **Phase A** (A1 rename, A2 retry): pure SwiftUI, no core regen — ship immediately.
- **Phase B batch** (B1 permission modes, B2 slash, B3 usage/context, B4 turn failure, B5 todos; optionally B6 thinking): do all Rust first, run `just ios-core` once, then Swift.
  - Note: `context_window` already exists in `mew-mobile-core` (`events.rs:192`, `state.rs:92`), so B3 Rust work is threading it through, not adding a new protocol field.
- **Phase C** (attachments): after daemon attachment forwarding lands.
- **Phase D** (subagents): largest item; one regen pass.
- **Phase E** (alerts banner): no regen.
- **Phase F** (auto-title/summary): rides any regen batch.
- **Phase G** (personas): blocked on web plan item 2 protocol.

## Suggested global execution order

1. **Daemon protocol batch**: persona + attachment forwarding + `ListSlashCommands`. This unblocks web Wave 3 and iOS Phases C/G.
2. **Web Wave 1 + Wave 2** and **iOS Phase A** in parallel — they don't touch Rust.
3. **TUI Item 0** (notification transport), then **Item 6** (batching) and **Item 4** (replay), then the rest of the TUI items.
4. **iOS Phase B** batch (Rust → regen → Swift) while web Wave 3 personas/attachments land.
5. **iOS Phase C** after daemon attachment forwarding.
6. **iOS Phase D** (subagents) — large, isolate it.
7. **iOS Phase E/F/G** as follow-ups; Phase G after persona protocol.

## Things to keep in mind

- **Binding regen is all-or-nothing.** Every iOS Rust change that touches the FFI surface needs `just ios-core`. Batch them aggressively.
- **The TUI now has a render cache.** Any new chat-affecting state mutation must call `mark_chat_dirty()`; any new code that builds chat lines belongs in `build_chat_lines`.
- **Clippy is now a hard gate.** New `mew-protocol` test patterns should avoid nested `match` arms on wire variants (collapsible_match lint).
- **Shared tree hazard.** Multiple agents are touching this repo concurrently. Keep commits focused and intentional; don't let `cargo fmt` runs reformat unrelated in-flight changes.
