use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mew_client_core::{ClientState, TurnOutcome};
use mew_protocol::{AlertKind, ServerMessage, SessionInfo};
use mew_tui::{events::handle_input_event, App, Mode};

fn session(id: &str) -> SessionInfo {
    serde_json::from_value(serde_json::json!({
        "session_id": id, "created_at": 0, "message_count": 0, "summary": id,
        "group_id": "default", "cwd": "/tmp/project", "state": "idle", "client_count": 0
    }))
    .unwrap()
}

fn update(app: &mut App, state: &mut ClientState, message: ServerMessage) {
    let events = state.apply_server_message(message);
    app.apply_client_update(&events, state, None);
}

fn ready(app: &mut App, state: &mut ClientState, id: &str) {
    update(
        app,
        state,
        ServerMessage::SessionReady {
            session_id: id.into(),
            cwd: Some("/tmp/project".into()),
            model: None,
            provider: None,
            permission_mode: None,
        },
    );
    update(
        app,
        state,
        ServerMessage::SessionHistory { messages: vec![] },
    );
}

fn outcome(app: &mut App, state: &mut ClientState, id: &str, kind: AlertKind) {
    update(
        app,
        state,
        ServerMessage::SessionAlert {
            session_id: id.into(),
            kind,
            title: id.into(),
            detail: Some("provider unavailable".into()),
        },
    );
}

#[test]
fn each_terminal_tracks_only_conversations_confirmed_in_that_window() {
    let mut a = App::new();
    let mut b = App::new();
    let mut sa = ClientState::default();
    let mut sb = ClientState::default();
    ready(&mut a, &mut sa, "a");
    ready(&mut b, &mut sb, "b");
    for (app, state) in [(&mut a, &mut sa), (&mut b, &mut sb)] {
        update(
            app,
            state,
            ServerMessage::SessionList {
                sessions: vec![session("a"), session("b")],
            },
        );
        update(
            app,
            state,
            ServerMessage::SessionAttentionChanged {
                session_id: "b".into(),
                pending_permissions: 1,
                pending_questions: 0,
            },
        );
    }
    assert_eq!(a.session_attention_count(), 0);
    assert_eq!(b.session_attention_count(), 1);
    a.open_session_picker();
    assert_eq!(a.picker.as_ref().unwrap().items.len(), 2);
    a.open_window_picker();
    assert_eq!(
        a.picker
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    ready(&mut a, &mut sa, "b");
    ready(&mut a, &mut sa, "b");
    assert_eq!(a.open_sessions, ["a", "b"]);
    assert_eq!(a.session_attention_count(), 1);
}

#[test]
fn unfocused_completion_survives_snapshots_and_is_acknowledged_when_visible() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    handle_input_event(&mut app, Event::FocusLost);
    outcome(&mut app, &mut state, "a", AlertKind::TurnComplete);
    update(
        &mut app,
        &mut state,
        ServerMessage::SessionList {
            sessions: vec![session("a")],
        },
    );
    assert!(app.unread_completions.contains("a"));
    assert_eq!(app.session_attention_count(), 1);
    handle_input_event(&mut app, Event::FocusGained);
    assert!(!app.unread_completions.contains("a"));
    assert_eq!(app.session_attention_count(), 0);
}

#[test]
fn a_new_conversation_without_history_acknowledges_its_first_visible_response() {
    use mew_message::{MessageId, Part, PartBase, PartId, ProviderEventWire, SessionId, TextPart};
    let mut app = App::new();
    let mut state = ClientState::default();
    let session_id = SessionId::new();
    let id = session_id.to_string();
    handle_input_event(&mut app, Event::FocusGained);
    update(
        &mut app,
        &mut state,
        ServerMessage::SessionReady {
            session_id: id.clone(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        },
    );
    let events = state.apply_server_message(ServerMessage::Provider {
        event: ProviderEventWire::PartStart {
            part: Part::Text(TextPart {
                base: PartBase {
                    id: PartId::new(),
                    message_id: MessageId::new(),
                    session_id,
                },
                text: "first response".into(),
                synthetic: false,
            }),
        },
    });
    let changed = state.session(&id).unwrap().messages.last().unwrap();
    app.apply_client_update(&events, &state, Some(changed));
    assert!(!app.messages().is_empty());
    outcome(&mut app, &mut state, &id, AlertKind::TurnComplete);
    assert!(!app.unread_completions.contains(&id));
}

#[test]
fn focus_does_not_acknowledge_a_result_hidden_by_an_overlay_or_scrollback() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    handle_input_event(&mut app, Event::FocusLost);
    outcome(&mut app, &mut state, "a", AlertKind::TurnComplete);
    app.open_session_picker();
    handle_input_event(&mut app, Event::FocusGained);
    assert!(app.unread_completions.contains("a"));
    app.mode = Mode::Normal;
    app.picker = None;
    app.auto_scroll = false;
    app.scroll = 2;
    app.max_scroll = 10;
    app.acknowledge_visible_result();
    assert!(app.unread_completions.contains("a"));
    app.scroll = 10;
    app.acknowledge_visible_result();
    assert!(!app.unread_completions.contains("a"));
}

#[test]
fn terminals_without_focus_reporting_keep_results_until_interaction() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    assert_eq!(app.terminal_focused, None);
    outcome(&mut app, &mut state, "a", AlertKind::TurnComplete);
    app.acknowledge_visible_result();
    assert!(app.unread_completions.contains("a"));
    handle_input_event(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
    );
    assert!(!app.unread_completions.contains("a"));
    assert_eq!(app.input, "z");
}

#[test]
fn failure_stays_actionable_after_reading_until_a_new_turn_starts() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    handle_input_event(&mut app, Event::FocusGained);
    outcome(&mut app, &mut state, "a", AlertKind::TurnFailed);
    app.acknowledge_visible_result();
    assert_eq!(app.session_attention_count(), 1);
    assert!(matches!(
        app.session_turn_outcomes.get("a"),
        Some(TurnOutcome::Failed { .. })
    ));
    update(
        &mut app,
        &mut state,
        ServerMessage::SessionActivityChanged {
            session_id: "a".into(),
            activity: mew_protocol::SessionState::Running,
        },
    );
    assert_eq!(app.session_attention_count(), 0);
}

#[test]
fn closing_a_background_view_keeps_its_draft_and_daemon_running_state() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    app.input = "unfinished draft".into();
    app.queued_messages.push("follow up after this turn".into());
    app.cursor = app.input.len();
    ready(&mut app, &mut state, "b");
    update(
        &mut app,
        &mut state,
        ServerMessage::SessionList {
            sessions: vec![session("a"), session("b")],
        },
    );
    update(
        &mut app,
        &mut state,
        ServerMessage::SessionActivityChanged {
            session_id: "a".into(),
            activity: mew_protocol::SessionState::Running,
        },
    );
    assert!(app.close_window_conversation("a").is_none());
    assert_eq!(app.open_sessions, ["b"]);
    assert!(!app.daemon_sessions[0].archived);
    assert_eq!(
        app.daemon_sessions[0].state,
        mew_protocol::SessionState::Running
    );
    ready(&mut app, &mut state, "a");
    assert_eq!(app.input, "unfinished draft");
    assert_eq!(app.queued_messages, ["follow up after this turn"]);
}

#[test]
fn closing_current_view_waits_for_replacement_and_failure_restores_membership() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    ready(&mut app, &mut state, "b");
    assert!(
        matches!(app.close_window_conversation("b"), Some(mew_tui::events::Action::AttachSession(id)) if id == "a")
    );
    assert_eq!(app.open_sessions, ["a", "b"]);
    app.attaching_session = true;
    update(
        &mut app,
        &mut state,
        ServerMessage::Error {
            message: "cannot attach".into(),
        },
    );
    assert!(!app.attaching_session);
    assert_eq!(app.open_sessions, ["a", "b"]);
    assert!(app.close_window_conversation("b").is_some());
    ready(&mut app, &mut state, "a");
    assert_eq!(app.open_sessions, ["a"]);
}

#[test]
fn closing_the_last_view_exits_the_frontend_without_cancelling_the_session() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    assert!(matches!(
        app.close_window_conversation("a"),
        Some(mew_tui::events::Action::Quit)
    ));
    assert_eq!(app.status.session_id, "a");
}

#[test]
fn raw_alerts_from_conversations_outside_this_window_do_not_interrupt_it() {
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    app.alert = None;
    app.apply_daemon_notification(&ServerMessage::SessionAlert {
        session_id: "b".into(),
        kind: AlertKind::PermissionNeeded,
        title: "shell".into(),
        detail: None,
    });
    assert!(app.alert.is_none());
}

#[test]
fn window_shortcuts_navigate_and_close_views_while_history_remains_available() {
    use mew_tui::events::{handle_key_event, Action};
    let mut app = App::new();
    let mut state = ClientState::default();
    ready(&mut app, &mut state, "a");
    app.input = "draft a".into();
    app.cursor = app.input.len();
    ready(&mut app, &mut state, "b");
    update(
        &mut app,
        &mut state,
        ServerMessage::SessionList {
            sessions: vec![session("a"), session("b"), session("elsewhere")],
        },
    );
    let modifiers = KeyModifiers::CONTROL | KeyModifiers::ALT;
    for code in [KeyCode::Left, KeyCode::Right] {
        assert!(
            matches!(handle_key_event(&mut app, KeyEvent::new(code, modifiers)),
            Some(Action::AttachSession(id)) if id == "a")
        );
    }
    assert!(
        matches!(handle_key_event(&mut app, KeyEvent::new(KeyCode::Enter, modifiers)),
        Some(Action::NewSessionInProject(path)) if path == "/tmp/project")
    );
    handle_key_event(&mut app, KeyEvent::new(KeyCode::Char('o'), modifiers));
    assert_eq!(app.picker.as_ref().unwrap().kind, "session_open");
    assert_eq!(app.picker.as_ref().unwrap().items.len(), 2);
    app.picker.as_mut().unwrap().selected = 0;
    handle_key_event(&mut app, KeyEvent::new(KeyCode::Char('w'), modifiers));
    assert_eq!(app.open_sessions, ["b"]);
    assert_eq!(app.picker.as_ref().unwrap().items.len(), 1);
    handle_key_event(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.picker.as_ref().unwrap().kind, "session");
    assert_eq!(app.picker.as_ref().unwrap().items.len(), 3);
    ready(&mut app, &mut state, "a");
    assert_eq!(app.input, "draft a");
    app.attaching_session = true;
    assert!(app.close_window_conversation("a").is_none());
    assert!(app.adjacent_window_conversation(1).is_none());
    assert_eq!(app.open_sessions, ["b", "a"]);
}

#[test]
fn a_single_view_stays_quiet_and_multiple_views_only_render_this_windows_work() {
    use mew_tui::{app::SidebarItem, harness::Harness};
    let mut harness = Harness::new(120, 40);
    let mut state = ClientState::default();
    harness.app.daemon_mode = true;
    ready(&mut harness.app, &mut state, "a");
    update(
        &mut harness.app,
        &mut state,
        ServerMessage::SessionList {
            sessions: ["a", "b", "c", "d", "elsewhere"].map(session).into(),
        },
    );
    harness
        .app
        .session_attention
        .insert("elsewhere".into(), (1, 0));
    let frame = harness.render();
    assert!(!frame.contains("Open here"), "{frame}");
    assert!(harness.app.bell_area.is_empty());
    for id in ["b", "c", "d"] {
        ready(&mut harness.app, &mut state, id);
    }
    let frame = harness.render();
    assert!(frame.contains("Open here"), "{frame}");
    for id in ["a", "b", "c", "d"] {
        assert!(
            harness
                .app
                .sidebar_item_rows
                .iter()
                .any(|(_, item)| matches!(item, SidebarItem::Session(owner) if owner == id)),
            "{id}: {frame}"
        );
    }
    assert!(!harness
        .app
        .sidebar_item_rows
        .iter()
        .any(|(_, item)| matches!(item, SidebarItem::Session(owner) if owner == "elsewhere")));
    assert!(harness.app.bell_area.is_empty());
}
