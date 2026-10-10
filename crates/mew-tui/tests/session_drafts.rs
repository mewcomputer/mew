use mew_protocol::ServerMessage;
use mew_tui::App;

fn attach(app: &mut App, id: &str) {
    app.apply_daemon_notification(&ServerMessage::SessionReady {
        session_id: id.into(),
        cwd: None,
        model: None,
        provider: None,
        permission_mode: None,
    });
}

#[test]
fn switching_conversations_restores_each_draft_queue_and_undo_history() {
    let mut app = App::new();
    attach(&mut app, "a");
    app.insert_char('a');
    app.queued_messages.push("a follow-up".into());
    app.context_files.push("a.txt".into());
    attach(&mut app, "b");
    assert!(app.input.is_empty());
    assert!(app.queued_messages.is_empty());
    assert!(app.context_files.is_empty());
    app.insert_char('b');
    app.queued_messages.push("b follow-up".into());
    attach(&mut app, "a");
    assert_eq!(app.input, "a");
    assert_eq!(app.cursor, 1);
    assert_eq!(app.queued_messages, ["a follow-up"]);
    assert_eq!(app.context_files, ["a.txt"]);
    app.undo();
    assert!(app.input.is_empty());
    attach(&mut app, "b");
    assert_eq!(app.input, "b");
    assert_eq!(app.queued_messages, ["b follow-up"]);
    app.undo();
    assert!(app.input.is_empty());
}

#[test]
fn confirming_the_same_conversation_preserves_the_live_editor() {
    let mut app = App::new();
    attach(&mut app, "a");
    app.input = "draft".into();
    app.cursor = 2;
    app.queued_messages.push("next".into());
    attach(&mut app, "a");
    assert_eq!(app.input, "draft");
    assert_eq!(app.cursor, 2);
    assert_eq!(app.queued_messages, ["next"]);
}

#[test]
fn daemon_submission_keeps_the_exact_editor_until_acceptance() {
    let mut app = App::new();
    app.daemon_mode = true;
    app.input = "  a draft\nwith whitespace  ".into();
    app.cursor = 4;
    assert_eq!(
        app.submit_input().as_deref(),
        Some("a draft\nwith whitespace")
    );
    assert_eq!(app.input, "  a draft\nwith whitespace  ");
    assert_eq!(app.cursor, 4);
    assert!(app.history.is_empty());
}

#[test]
fn delayed_acceptance_clears_only_the_submitted_conversations_matching_draft() {
    let mut app = App::new();
    app.daemon_mode = true;
    attach(&mut app, "a");
    app.input = "  original  ".into();
    app.begin_submission("receipt".into(), "original".into(), vec![], false);
    attach(&mut app, "b");
    app.input = "b draft".into();
    app.resolve_submission("a", "receipt", mew_protocol::PromptStatus::Accepted, None);
    assert_eq!(app.input, "b draft");
    attach(&mut app, "a");
    assert!(app.input.is_empty());
    assert_eq!(app.history, ["original"]);
}

#[test]
fn failed_queued_submission_returns_to_its_owners_queue() {
    let mut app = App::new();
    attach(&mut app, "a");
    app.begin_submission("receipt".into(), "queued".into(), vec![], true);
    attach(&mut app, "b");
    app.queued_messages.push("b queue".into());
    app.resolve_submission(
        "a",
        "receipt",
        mew_protocol::PromptStatus::Rejected,
        Some("busy"),
    );
    assert_eq!(app.queued_messages, ["b queue"]);
    attach(&mut app, "a");
    assert_eq!(app.queued_messages, ["queued"]);
    assert!(!app.pending_queued_send);
}

#[test]
fn old_completion_releases_only_its_conversations_saved_queue() {
    let mut app = App::new();
    attach(&mut app, "a");
    app.streaming = true;
    app.queued_messages.push("follow a".into());
    attach(&mut app, "b");
    app.streaming = true;
    app.queued_messages.push("follow b".into());
    app.finish_session_turn("a");
    assert!(app.streaming);
    assert!(!app.pending_queued_send);
    assert_eq!(app.queued_messages, ["follow b"]);
    attach(&mut app, "a");
    assert!(app.pending_queued_send);
    assert_eq!(app.pop_queued_message().as_deref(), Some("follow a"));
}
