//! Dispatch regression tests (Phase 0).
//!
//! These tests pin the **current broken behavior** of the dispatch layer so
//! that Phase 1 can prove it fixed them. They are `#[ignore]`d with
//! `FIXME(phase-1)` markers so CI stays green.
//!
//! In Phase 1 step 8 they are rewritten to call `handle_action` directly (via
//! `LocalTarget` + `FakeProvider`) instead of the harness stub, and un-ignored.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_tui::harness::Harness;

// ---------------------------------------------------------------------------
// Bug 1: `SlashResult::Continue` (unknown `/` command) is swallowed.
// ---------------------------------------------------------------------------

/// Today, an unknown slash command like `/xyz` returns `SlashResult::Continue`
/// which the drain match swallows with `{}` — nothing happens. After Phase 1,
/// `Continue` submits the original text as a normal prompt to the provider.
///
/// NOTE: This test currently exercises the Harness stub, not the production
/// drain loop. It pins the harness-stub behavior (which mirrors the drain's
/// swallow). In Phase 1 it must be rewritten to call `handle_action` directly.
#[test]
#[ignore = "FIXME(phase-1): will flip to asserting fall-through to the model"]
fn slash_continue_currently_swallowed() {
    let mut h = Harness::new(80, 24);
    h.type_str("/xyz");
    h.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    // Today: the harness's apply_action sees `Action::SlashCommand("/xyz")`
    // but handle_slash returns `SlashResult::Continue` which the harness stub
    // does not act on. No user message is pushed. After Phase 1, `/xyz` falls
    // through as a normal prompt → a user message appears.
    assert!(
        h.app.messages.iter().all(|m| m.role != mew_message::Role::User),
        "today no user message is pushed for unknown slash — after Phase 1 this fails"
    );
}
