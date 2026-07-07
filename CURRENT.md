# Current Progress — Unified Runtime + TUI UX Plan

## Phase 0 — COMPLETE ✅
- `crates/mew/tests/dispatch_regression.rs` — 1 `#[ignore]`d test pinning `SlashResult::Continue` swallow
- `crates/mew-tui/tests/golden_test.rs` — golden-frame scaffold with `welcome` seed frame
- Phase 0 code review completed (3 lenses). Fixed: removed tautological tests, normalized golden frame, fixed `MEW_UPDATE_GOLDEN` env check.

## Phase 1 — IN PROGRESS (steps 1-5 complete, 6-8 remaining)

### Completed:
1. **App mutation methods** — `push_message`, `push_user`, `push_synthetic_message` all call `mark_chat_dirty()`. `push_synthetic_message` was missing the dirty mark — now fixed.
2. **All `messages.push` replaced** — Zero `.messages.push(` calls remain in `crates/mew/src/` outside `mew-tui/src/app.rs` (AC.1 satisfied). Removed dead `user_message` and `synthetic_message` helper functions.
3. **`runtime/dispatch.rs`** created — `handle_action<T: CommandTarget>` function with `#![deny(clippy::wildcard_enum_match_arm)]`. All `Action` and `SlashResult` variants explicitly handled. `SlashResult::Continue` now falls through to the model (AC.5).
4. **`runtime/target.rs`** created — `CommandTarget` trait + `Unsupported` type + `SwitchedModel`/`PersonaApplied` structs.
5. **`runtime/local.rs`** created — `LocalTarget<'a>` borrowing `&mut Agent`, implements all `CommandTarget` methods.
6. **`runtime/mentions.rs`** created — `process_mentions` and `image_mime` moved from main.rs.
7. **`runtime/mod.rs`** created — module declarations + re-exports.

### Remaining:
- **Step 6**: Rewire `run_tui` and `chat_with_daemon` event loops to call `handle_action` instead of inline dispatch. This is ~700 lines of surgical replacement. The `handle_action` function and `LocalTarget` are ready but not yet wired in.
- **Step 7**: Ctrl+C cancel-first behavior (first press cancels turn, second within ~1s quits).
- **Step 8**: Un-ignore Phase 0 regression tests and rewrite to use `handle_action` directly.

## Key files modified:
- `crates/mew/src/main.rs` — `mod runtime` added, `messages.push` calls replaced, `user_message`/`synthetic_message` removed, `persona_summary`/`toggle_mouse_capture`/`copy_to_clipboard` made `pub(crate)`
- `crates/mew/src/runtime/` — new module tree (mod.rs, dispatch.rs, target.rs, local.rs, mentions.rs)
- `crates/mew-tui/src/app.rs` — `push_message`, `push_user`, `push_synthetic_message` methods added with dirty marking
- `crates/mew/tests/dispatch_regression.rs` — Phase 0 regression test
- `crates/mew-tui/tests/golden_test.rs` — golden frame scaffold
- `crates/mew-tui/tests/golden/welcome.frame` — seed golden frame

## Test status:
- All 134 mew-tui lib tests pass
- dispatch_regression test compiles and is properly ignored
- golden_test passes
- `cargo check -p mew` succeeds with warnings (unused code — expected since dispatch isn't wired in yet)
