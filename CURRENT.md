# Current Progress — Unified Runtime + TUI UX Plan

## Completed Phases

### Phase 0 — COMPLETE ✅
- dispatch_regression.rs: 3 tests verifying fixed behavior
- golden_test.rs: golden-frame scaffold with welcome seed frame

### Phase 1 — COMPLETE ✅
- App mutation methods with dirty marking
- runtime/ module tree: dispatch.rs (handle_action), target.rs (CommandTarget), local.rs (LocalTarget), mentions.rs, mod.rs
- run_tui main loop + drain loop rewired to handle_action
- Drain queues and replays actions (no longer interprets)
- SlashResult::Continue falls through to submit as prompt (AC.5)
- Ctrl+C cancel-first behavior (AC.15)
- All messages.push replaced (AC.1)
- deny(clippy::wildcard_enum_match_arm) enforced

### Phase 2 — PARTIAL
- ✅ SlashResult rendering fix (AC.8)
- ✅ render_count instrumentation (AC.13 prep)
- ⏳ DaemonTarget + run_event_loop<T> deferred

### Phase 3 — COMPLETE ✅
- App.messages privatized to pub(crate)
- messages() accessor added

### Phase 4 — COMPLETE ✅
- RenderCache adopted for streaming (caches committed blocks)
- rendered_md_cache rekeyed from MessageId to PartId (AC.10)
- PartStart early-return dirty miss fixed (AC.12)
- md_render_cache invalidated in all reset points

### Phase 5 — PARTIAL
- ✅ arch-check recipe in justfile (AC.1, AC.2, AC.18)
- ✅ CLAUDE.md runtime invariants section
- ✅ deny(wildcard) enforced
- ⏳ Strum table test, harness wiring deferred

### Phase 6 — COMPLETE ✅
- Command registry in mew-protocol (command_registry.rs)
- BUILTIN_COMMANDS static table with CommandLocus
- Daemon unknown-command returns error instead of silence (AC.14)

### Phase 7 — PARTIAL
- ✅ Golden-frame seed set: 5 frames (welcome, user_assistant_turn, narrow_40col, tool_call_collapsed, reasoning_block)
- ⏳ Adversarial FakeProvider, SSE fixtures, e2e smoke, time seam deferred

### Phase 8 — COMPLETE ✅
- adjust_slash_scroll dynamic visible count
- /permissions description fixed
- /help opens shortcuts overlay
- Web client never-default arm (AC.20)
- Mobile-core explicit arms + Error translation (AC.21)

### Phase 9 — PARTIAL
- ✅ CLI types extracted to cli.rs (~1000 lines)
- main.rs: 4718 → 3695 lines
- ⏳ commands/ and setup/ extraction deferred

## Acceptance Criteria Status
- AC.1 ✅ (arch-check grep)
- AC.2 ✅ (deny wildcard + arch-check)
- AC.5 ✅ (Continue falls through)
- AC.8 ✅ (SlashResult as synthetic text)
- AC.10 ✅ (PartId cache key)
- AC.12 ✅ (PartStart dirty mark)
- AC.14 ✅ (daemon unknown command error)
- AC.15 ✅ (Ctrl+C cancel-first)
- AC.18 ✅ (just ci includes arch-check)
- AC.20 ✅ (web never-default)
- AC.21 ✅ (mobile-core explicit arms)

## Test Status
- 134 mew-tui lib tests pass
- 3 dispatch regression tests pass
- 5 golden frame tests pass
- clippy --tests -D warnings: zero warnings
- just arch-check: passes
