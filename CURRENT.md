# Current Progress — Unified Runtime + TUI UX Plan

## All Phases Status

### Phase 0 — COMPLETE ✅
- 3 dispatch regression tests (no longer ignored)
- 5 golden frame tests

### Phase 1 — COMPLETE ✅
- runtime/ module tree: dispatch.rs, target.rs, local.rs, mentions.rs, mod.rs
- handle_action with deny(clippy::wildcard_enum_match_arm)
- run_tui main loop + drain loop rewired through handle_action
- SlashResult::Continue falls through (AC.5), Ctrl+C cancel-first (AC.15)
- All messages.push replaced (AC.1)

### Phase 2 — COMPLETE ✅
- DaemonTarget created (runtime/daemon.rs)
- chat_with_daemon rewired through handle_action
- SlashResult rendering fix (AC.8)
- render_count instrumentation (AC.13 prep)
- handle_slash_result_local deleted
- arch-check expanded to all of crates/mew/src (AC.2 fully enforced)

### Phase 3 — COMPLETE ✅
- App.messages privatized to pub(crate)
- messages() accessor added

### Phase 4 — COMPLETE ✅
- RenderCache adopted for streaming (AC.11)
- rendered_md_cache rekeyed from MessageId to PartId (AC.10)
- PartStart early-return dirty miss fixed (AC.12)

### Phase 5 — COMPLETE ✅
- arch-check recipe in justfile (AC.1, AC.2, AC.18)
- CLAUDE.md runtime invariants section
- deny(wildcard) enforced in dispatch.rs
- strum EnumIter derived on Action and SlashResult
- test_action_variant_table: every Action variant tested (AC.7)

### Phase 6 — COMPLETE ✅
- Command registry in mew-protocol (command_registry.rs)
- BUILTIN_COMMANDS static table with CommandLocus
- Daemon unknown-command returns error (AC.14)

### Phase 7 — PARTIAL
- 5 golden frames: welcome, user_assistant_turn, narrow_40col, tool_call_collapsed, reasoning_block
- Remaining: adversarial FakeProvider, SSE fixtures, e2e smoke, time seam

### Phase 8 — COMPLETE ✅
- adjust_slash_scroll, /permissions, /help, web never-default (AC.20), mobile-core arms (AC.21)

### Phase 9 — PARTIAL
- CLI types extracted to cli.rs (4718 → 3634 lines)
- setup/ extraction deferred (pure code motion, low risk)

## Acceptance Criteria
AC.1 ✅ AC.2 ✅ AC.5 ✅ AC.7 ✅ AC.8 ✅ AC.10 ✅ AC.12 ✅ AC.14 ✅ AC.15 ✅ AC.18 ✅ AC.20 ✅ AC.21 ✅

## Test Status
- 134 mew-tui lib tests pass
- 3 dispatch regression tests pass
- 1 dispatch table test passes (AC.7)
- 5 golden frame tests pass
- clippy --tests -D warnings: zero warnings
- just arch-check: passes
