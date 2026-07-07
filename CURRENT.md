# Current Progress — Unified Runtime + TUI UX Plan

## Completed Phases

### Phase 0 — COMPLETE ✅
- `crates/mew/tests/dispatch_regression.rs` — 3 tests verifying fixed behavior (no longer ignored)
- `crates/mew-tui/tests/golden_test.rs` — golden-frame scaffold with `welcome` seed frame
- Phase 0 code review completed (3 lenses). Fixed: removed tautological tests, normalized golden frame, fixed env check.

### Phase 1 — COMPLETE ✅
- App mutation methods: `push_message`, `push_user`, `push_synthetic_message` all call `mark_chat_dirty()`
- All `messages.push` calls in main.rs replaced (AC.1 satisfied)
- `runtime/` module tree created: `dispatch.rs` (handle_action with deny(wildcard_enum_match_arm)), `target.rs` (CommandTarget trait), `local.rs` (LocalTarget), `mentions.rs`, `mod.rs`
- `run_tui` main loop and drain loop rewired to `handle_action`
- Drain no longer interprets actions — queues and replays (AC.4)
- `SlashResult::Continue` falls through to submit as normal prompt (AC.5)
- Ctrl+C cancel-first: first press cancels, second within ~1s quits (AC.15)
- Phase 1 review completed (simplifier + operator). Fixed: removed dead Ctx fields, enforced deny(wildcard), removed yield_control, inlined handle_persona_switch_confirmed.

### Phase 2 — PARTIAL (daemon unification deferred)
- ✅ Fix `ServerMessage::SlashResult` rendering: emit as synthetic text, not Error (AC.8)
- ✅ `render_count` test instrumentation on App (AC.13 prep)
- ⏳ `DaemonTarget` + `run_event_loop<T>` — deferred (large refactor of chat_with_daemon)
- ⏳ Delete `handle_slash_result_local` — deferred (depends on DaemonTarget)

### Phase 3 — COMPLETE ✅
- `App.messages` privatized to `pub(crate)` — external access via `messages()` accessor
- Harness updated to use `messages().iter()` and `push_message()`

### Phase 5 — PARTIAL (strum table test + harness wiring deferred)
- ✅ `#![deny(clippy::wildcard_enum_match_arm)]` enforced in dispatch.rs
- ✅ `just arch-check` recipe in justfile (AC.1, AC.2, AC.18)
- ✅ `just ci` includes arch-check
- ✅ CLAUDE.md "Runtime invariants" section added
- ⏳ Strum every-variant table test — deferred
- ⏳ Harness wiring to real dispatch — deferred

### Phase 8 — COMPLETE ✅
- `adjust_slash_scroll`: dynamic visible count based on chat_area height
- `/permissions`: fixed stale description to list all 5 modes
- `/help`: now opens shortcuts overlay (Mode::Help) instead of command palette
- Web client: `never`-default arm on ServerMessage switch (AC.20)
- Mobile-core: explicit arms replacing catch-all; Error/ErrorEvent → CoreEvent::Alert (AC.21)

## Not Started
- Phase 4: Incremental streaming render (RenderCache, PartId cache key, etc.)
- Phase 6: Shared command registry
- Phase 7: Testing infrastructure (golden frames, adversarial FakeProvider, e2e smoke, time seam)
- Phase 9: Split main.rs

## Test Status
- 134 mew-tui lib tests pass
- 3 dispatch regression tests pass (no longer ignored)
- 1 golden frame test passes
- clippy --tests -D warnings: zero warnings
- just arch-check: passes
