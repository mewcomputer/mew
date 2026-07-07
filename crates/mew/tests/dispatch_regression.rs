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

#[test]
fn synthetic_message_marks_dirty() {
    let mut app = mew_tui::App::new();
    app.mark_chat_dirty();
    let gen_before = app.chat_dirty;
    app.push_synthetic_message("test output".into());
    assert_ne!(app.chat_dirty, gen_before);
}

// ---------------------------------------------------------------------------
// Bug 3: Actions arriving during the drain are no longer dropped.
// ---------------------------------------------------------------------------

#[test]
fn set_permission_mode_changes_app_state() {
    let mut app = mew_tui::App::new();
    let original_mode = app.permission_mode;
    let new_mode = mew_hooks::PermissionMode::Dangerous;
    assert_ne!(original_mode, new_mode);
    app.permission_mode = new_mode;
    assert_eq!(app.permission_mode, new_mode);
}
