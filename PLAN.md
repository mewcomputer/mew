# Plan: daemon-only TUI, immediate two-press cancel, and guided queued messages

## Context

The local-mode TUI (`run_tui`) embeds the `Agent` directly and has no server-side
serialization, so a cancelled turn can still be winding down while a new submit
starts a second concurrent turn — the reported race. The daemon already
serializes turns (`turn_lock` + `current_turn_cancel` in `mew-daemon/src/lib.rs`),
so making the daemon the only runtime path removes the race at the source.

Two UX problems compound it:
- Esc only *arms* a 2-second hint on the first press; the user doesn't realize the
  turn is still running and submits again, which **queues** messages that all fire
  in a burst when the turn finally ends ("multiple requests at the same time").
- Queued messages have no way to steer the *running* turn; they only become a new
  turn after the current one finishes.

Decisions (confirmed):
1. **Sunset local mode** — the `mew` TUI always talks to a daemon (spawn one if
   none is running). The deterministic harness (`mew-tui/src/harness.rs`) and
   `tui-capture` harness mode stay — they are test tooling, not the user runtime.
2. **Guide command** — a queued message can be injected into the running turn's
   *next* provider request (even a tool-call continuation) to steer the LLM now.
3. **Two-press Esc cancels immediately** — the second press cancels and the TUI
   keeps the turn "active" until the turn actually ends, so later submits are
   queued rather than sent into the daemon's "turn in progress" window.

## Phase 1 — Agent runtime: guidance injection

Goal: let the daemon inject a short user message into the running turn's next
provider request (mid-tool-loop), without starting a new turn.

- `crates/mew-agent/src/agent.rs`: add a shared pending-guidance queue on `Agent`
  (e.g. `pending_guidance: Arc<tokio::sync::Mutex<VecDeque<String>>>`, or a
  `tokio::sync::mpsc` channel). Because `run_with_parts` clones the agent but
  shares `Arc` fields, the running turn sees the guidance.
- `crates/mew-agent/src/turn.rs`: at the top of each `turn_loop` iteration (before
  `prepare_and_compact_messages` builds the request), drain pending guidance and
  append each as a user `Message` to `self.messages` so it is included in the next
  request. If the turn already ended, the guidance stays queued and is picked up
  by the next turn (matches the "else it gets picked up on the next turn" fallback).
- Add a public method `Agent::enqueue_guidance(text)` (or `inject_guidance`).
- Emit an `AgentEvent` (e.g. a `Message`/`ProviderEvent`-style marker) so the TUI
  can show the injected guidance in the transcript.

## Phase 2 — Daemon + protocol: Guide message

- `crates/mew-protocol`: add `ClientMessage::Guide { text }` (wire roundtrip
  coverage). Keep it separate from `Prompt` (session-management vs streaming).
- `crates/mew-daemon/src/lib.rs`: handle `ClientMessage::Guide`, resolve the
  attached session, and call `agent.enqueue_guidance(text)`. Also broadcast a
  user-message event so attached clients see it. Update the client/TS client if
  the wire enum is shared there.
- `crates/mew-daemon/src/client.rs`: add `DaemonClient::guide(text)`.

## Phase 3 — TUI: guided queued messages + reminder

- `crates/mew-tui/src/events.rs`: add `Action::GuideQueued(String)` (or reuse the
  queued-send path with a mode). Wire a keybinding (Ctrl+Up / Shift+Up, or the
  existing Up-Up) that, when a queued message exists, pops it and sends it as
  guidance instead of cancelling-and-resubmitting.
- `crates/mew-tui/src/app/mod.rs`: track queued messages with a "guide vs send"
  intent; add a visible reminder (status pill + the existing input preview strip)
  explaining the guide key and that queued messages will fire on the next turn.
- `crates/mew/src/runtime/dispatch.rs`: add the dispatch arm for `Action::GuideQueued`
  → `target.guide(text)`. Add a `CommandTarget::guide` method to both `LocalTarget`
  (dropped in Phase 5) and `DaemonTarget`.
- Update the help overlay (`ui/overlays.rs`) to document the guide key.

## Phase 4 — TUI: two-press Esc that stops immediately + turn-alive guard

- Keep the two-press Esc arm/cancel in `handle_normal_key` (`events.rs`).
- On the second press (`Action::Cancel`), do **not** set `streaming=false`
  immediately. Instead set a `cancelling` flag and keep streaming truthy until the
  turn-ending `MessageEnd`/`Error` event actually arrives. This way submits during
  the cancel-window are queued, not sent into the daemon's "turn in progress"
  window.
- `crates/mew/src/runtime/dispatch.rs`: `Action::Cancel` sends daemon cancel and
  marks `cancelling`; does not clear `streaming`. `app/mod.rs` clears `cancelling`
  + `streaming` on `MessageEnd`/`Error`.
- If the daemon still replies with "turn in progress" (edge race), queue the
  message instead of dropping it.

## Phase 5 — Sunset local mode (daemon-only)

- `crates/mew/src/commands/tui.rs`: make `chat_cmd` connect to a daemon, spawning
  one if none is healthy (reuse the supervisor pattern from
  `mew-desktop-supervisor`, or a CLI-side spawn of `mew daemon --socket ...`).
  Remove the `run_tui` local path from the default UX.
- Keep `mew-tui/src/harness.rs` and `tui-capture` harness mode (deterministic tests).
- Keep `LocalTarget` only if the harness/tests still need it; otherwise remove it.
- Update `mew-tui/src/harness.rs` `Backend`/`LocalBackend` if the guide/cancel
  actions change the harness's action expectations.

## Phase 6 — Plan-presentation race

- Investigate the plan-approval modal path (`PlanApprovalRequest`,
  `plan_approval_confirm`). The turn pauses at plan approval while `streaming` is
  still true; confirm the new turn-alive guard + daemon-only serialization covers
  the case where the user submits during plan review. Add a queued/guard behavior
  if needed.

## Phase 7 — Verification

- Tests: protocol roundtrip for `Guide`; agent guidance injection (guidance is
  included in the next request even when `Finish::ToolUse`); TUI queued-message
  guide key; two-press Esc cancel keeps `streaming` until the turn ends.
- Run `cargo test --all`, `cargo clippy --all -- -D warnings`, `cargo fmt`,
  `just arch-check`, `just theme-codegen-check`, and the JS checks if the wire
  protocol changed.
- Update `CURRENT.md` with an append-only dated entry.

## Out of scope (for now)

- Full removal of the harness / `tui-capture` local mode (kept as test tooling).
- Remote/iroh daemon changes beyond what the shared `Guide` wire message needs.