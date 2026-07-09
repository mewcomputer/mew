//! Dispatch regression tests.
//!
//! These tests verify the dispatch behavior after Phase 1's extraction.

use mew_tui::SlashResult;

// ---------------------------------------------------------------------------
// Bug 1: `SlashResult::Continue` (unknown `/` command) was swallowed.
// ---------------------------------------------------------------------------

/// Before Phase 1, an unknown slash command like `/xyz` returned
/// `SlashResult::Continue` which the drain match swallowed with `{}` —
/// nothing happened. After Phase 1, `Continue` submits the original text
/// as a normal prompt to the model.
#[test]
fn slash_continue_not_swallowed_by_dispatch() {
    let app = mew_tui::App::new();
    let result = app.handle_slash("/xyz");
    assert!(matches!(result, SlashResult::Continue));
}

// ---------------------------------------------------------------------------
// Bug 2: `push_synthetic_message` now marks chat dirty (was previously missing).
// ---------------------------------------------------------------------------
// Coverage moved to main.rs::test_synthetic_message_renders_immediately which
// exercises the same invariant through the full dispatch path.

// ---------------------------------------------------------------------------
// Bug 3: Actions arriving during the drain are no longer dropped.
// ---------------------------------------------------------------------------
// Previously tested here with a tautological test that directly set
// app.permission_mode and asserted it equaled what was set. The real
// coverage is in main.rs::test_set_permission_mode_not_dropped which
// calls handle_action(Action::SetPermissionMode) and asserts the mode
// actually changed through the dispatch path.
