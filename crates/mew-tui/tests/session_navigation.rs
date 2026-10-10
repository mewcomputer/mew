use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use mew_tui::{
    app::Mode,
    events::{handle_input_event, handle_key_event, Action},
    ui, App,
};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

fn session(id: &str, title: &str, archived: bool) -> mew_protocol::SessionInfo {
    serde_json::from_value(serde_json::json!({"session_id":id,"state":"idle","created_at":10,"client_count":0,"summary":title,"archived":archived})).unwrap()
}
fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}
fn row(buffer: &Buffer, needle: &str) -> u16 {
    (0..buffer.area.height)
        .find(|&y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
        .unwrap()
}
fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    handle_key_event(app, KeyEvent::new(code, modifiers))
}

#[test]
fn bell_counts_conversations_once_and_opens_archived_attention_without_losing_the_draft() {
    let mut app = App::new();
    app.daemon_mode = true;
    app.status.session_id = "active".into();
    app.input = "saved draft".into();
    app.cursor = app.input.len();
    app.daemon_sessions = vec![
        session("active", "current", false),
        session("approval", "approval work", false),
        session("done", "finished work", false),
        session("failed", "failed work", true),
        session("quiet", "quiet work", false),
    ];
    app.session_attention.insert("active".into(), (1, 0));
    app.session_attention.insert("approval".into(), (2, 1));
    app.unread_completions
        .extend(["done".into(), "failed".into()]);
    app.session_turn_outcomes
        .insert("done".into(), mew_client_core::TurnOutcome::Completed);
    app.session_turn_outcomes.insert(
        "failed".into(),
        mew_client_core::TurnOutcome::Failed {
            detail: Some("provider unavailable".into()),
        },
    );
    assert_eq!(app.session_attention_count(), 3);
    let buffer = draw(&mut app, 120, 30);
    let y = row(&buffer, "3 updates");
    let x = app.sidebar_rect.x + 2;
    handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(app.mode, Mode::CommandPalette);
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "session_attention");
    assert_eq!(
        picker
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["approval", "done", "failed"]
    );
    assert!(
        picker.items[0]
            .context
            .as_ref()
            .unwrap()
            .metadata
            .contains("2 approvals")
            && picker.items[0]
                .context
                .as_ref()
                .unwrap()
                .metadata
                .contains("1 question")
    );
    assert_eq!(picker.items[0].label, "! approval work");
    assert_eq!(picker.items[1].label, "● finished work");
    assert!(
        picker.items[2].description.contains("provider unavailable")
            && picker.items[2].description.contains("Archived")
    );
    app.picker.as_mut().unwrap().selected = 2;
    assert!(
        matches!(key(&mut app,KeyCode::Enter,KeyModifiers::NONE),Some(Action::AttachSession(id)) if id=="failed")
    );
    assert_eq!(app.input, "saved draft");
}
#[test]
fn session_picker_toggle_recovers_archived_sessions_and_preserves_query_when_lists_refresh() {
    let mut app = App::new();
    app.daemon_sessions = vec![
        session("new", "needle recent", false),
        session("old", "needle archived", true),
    ];
    app.open_session_picker();
    assert_eq!(app.picker.as_ref().unwrap().items.len(), 1);
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "session_archived");
    assert_eq!(picker.items[0].id, "old");
    key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
    app.daemon_sessions
        .insert(0, session("oldest", "not a match", true));
    app.refresh_session_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.filter, "n");
    assert_eq!(picker.selected_item().unwrap().id, "old");
    assert!(
        matches!(key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL),Some(Action::ToggleSessionArchived(id)) if id=="old")
    );
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().kind, "session");
}
#[test]
fn attention_is_keyboard_reachable_without_a_sidebar_and_empty_is_truthful() {
    let mut app = App::new();
    app.daemon_mode = true;
    app.input = "draft".into();
    app.cursor = 5;
    key(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert_eq!(app.mode, Mode::CommandPalette);
    assert_eq!(app.picker.as_ref().unwrap().kind, "session_attention");
    let buffer = draw(&mut app, 40, 16);
    row(&buffer, "No unread updates");
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().kind, "session");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.input, "draft");
}

#[test]
fn global_help_exposes_the_attention_shortcut() {
    let mut app = App::new();
    key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    assert_eq!(app.mode, Mode::Help);
    let buffer = draw(&mut app, 100, 40);
    let y = row(&buffer, "conversation notifications");
    assert!((0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .contains("Ctrl+N"));
}

#[test]
fn connection_state_stays_visible_after_the_toast_and_precedes_turn_status() {
    use mew_client_core::ConnectionStatus;

    let mut app = App::new();
    app.daemon_mode = true;
    app.cancelling = true;
    app.retry_status = Some("provider retry".into());
    for (connection, expected) in [
        (ConnectionStatus::Connecting, "connecting"),
        (ConnectionStatus::Disconnected, "disconnected"),
        (
            ConnectionStatus::Backoff {
                attempt: 3,
                error: "connection lost".into(),
            },
            "reconnecting (3)",
        ),
    ] {
        app.connection = Some(connection);
        app.alert = None;
        let buffer = draw(&mut app, 80, 24);
        let status = (0..buffer.area.width)
            .map(|x| buffer[(x, 23)].symbol())
            .collect::<String>();
        assert!(status.contains(expected), "{status}");
        assert!(!status.contains("provider retry") && !status.contains("cancelling"));
    }
    app.connection = Some(ConnectionStatus::Connected);
    app.attaching_session = true;
    app.alert = None;
    let buffer = draw(&mut app, 80, 24);
    assert_eq!(row(&buffer, "opening conversation"), 23);

    let mut state = mew_client_core::ClientState::default();
    let events = state.apply_server_message(mew_protocol::ServerMessage::SessionReady {
        session_id: "ready".into(),
        cwd: None,
        model: Some("ready-model".into()),
        provider: Some("fake".into()),
        permission_mode: None,
    });
    app.apply_client_update(&events, &state, None);
    app.alert = None;
    app.cancelling = false;
    let buffer = draw(&mut app, 80, 24);
    assert_eq!(row(&buffer, "fake/ready-model"), 23);
    assert!(!app.attaching_session);
}
#[test]
fn attention_refresh_preserves_selected_identity_and_removes_acknowledged_rows() {
    let mut app = App::new();
    app.daemon_sessions = vec![session("a", "first", false), session("b", "second", false)];
    app.unread_completions.extend(["a".into(), "b".into()]);
    app.open_attention_picker();
    app.picker.as_mut().unwrap().selected = 1;
    app.daemon_sessions.reverse();
    app.refresh_session_picker();
    assert_eq!(
        app.picker.as_ref().unwrap().selected_item().unwrap().id,
        "b"
    );
    app.unread_completions.remove("b");
    app.refresh_session_picker();
    assert_eq!(
        app.picker.as_ref().unwrap().selected_item().unwrap().id,
        "a"
    );
    app.unread_completions.remove("a");
    app.refresh_session_picker();
    assert!(app.picker.as_ref().unwrap().items.is_empty());
    draw(&mut app, 40, 16);
    assert!(app.picker_mouse.rows.is_empty());
}

#[test]
fn attention_arriving_before_session_metadata_remains_reachable() {
    let mut app = App::new();
    app.session_attention
        .insert("not-yet-listed".into(), (1, 0));
    assert_eq!(app.session_attention_count(), 1);
    app.open_attention_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.items.len(), 1);
    assert_eq!(picker.items[0].id, "not-yet-listed");
    assert!(picker.items[0].description.is_empty());
    let context = picker.items[0].context.as_ref().unwrap();
    assert_eq!(context.path, "Project unavailable");
    assert_eq!(context.metadata, "1 approval");
}
#[test]
fn canonical_completion_and_ready_ack_drive_the_bell() {
    let mut app = App::new();
    app.status.session_id = "active".into();
    let mut state = mew_client_core::ClientState::default();
    let events = state.apply_server_message(mew_protocol::ServerMessage::SessionList {
        sessions: vec![session("done", "finished work", false)],
    });
    app.apply_client_update(&events, &state, None);
    let events = state.apply_server_message(mew_protocol::ServerMessage::SessionAlert {
        session_id: "done".into(),
        kind: mew_protocol::AlertKind::TurnComplete,
        title: "finished".into(),
        detail: None,
    });
    app.apply_client_update(&events, &state, None);
    assert_eq!(app.session_attention_count(), 1);
    app.open_attention_picker();
    assert_eq!(app.picker.as_ref().unwrap().items.len(), 1);
    assert!(
        matches!(key(&mut app,KeyCode::Enter,KeyModifiers::NONE),Some(Action::AttachSession(id)) if id=="done")
    );
    assert_eq!(app.session_attention_count(), 1);
    let events = state.apply_server_message(mew_protocol::ServerMessage::SessionReady {
        session_id: "done".into(),
        cwd: None,
        model: None,
        provider: None,
        permission_mode: None,
    });
    app.apply_client_update(&events, &state, None);
    assert_eq!(app.session_attention_count(), 0);
    assert!(app.picker.is_none());
}

#[test]
fn canonical_failed_alert_is_unread_until_ready_and_keeps_its_failure_detail() {
    let mut app = App::new();
    app.status.session_id = "active".into();
    let mut state = mew_client_core::ClientState::default();
    for _ in 0..2 {
        let events = state.apply_server_message(mew_protocol::ServerMessage::SessionAlert {
            session_id: "failed".into(),
            kind: mew_protocol::AlertKind::TurnFailed,
            title: "turn failed".into(),
            detail: Some("provider unavailable".into()),
        });
        app.apply_client_update(&events, &state, None);
    }
    assert_eq!(app.session_attention_count(), 1);
    let events = state.apply_server_message(mew_protocol::ServerMessage::SessionList {
        sessions: vec![session("failed", "failed work", true)],
    });
    app.apply_client_update(&events, &state, None);
    app.open_attention_picker();
    let item = &app.picker.as_ref().unwrap().items[0];
    assert_eq!(item.label, "● failed work");
    assert!(item.description.contains("Archived"));
    assert!(item.description.contains("provider unavailable"));
    let events = state.apply_server_message(mew_protocol::ServerMessage::SessionReady {
        session_id: "failed".into(),
        cwd: None,
        model: None,
        provider: None,
        permission_mode: None,
    });
    app.apply_client_update(&events, &state, None);
    assert_eq!(app.session_attention_count(), 0);
    app.open_session_picker();
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    let item = &app.picker.as_ref().unwrap().items[0];
    assert_eq!(item.label, "○ failed work");
    assert!(item.description.contains("provider unavailable"));
}

#[test]
fn narrow_status_keeps_a_clickable_bell_visible_and_clears_its_hit_bounds_on_resize() {
    let mut app = App::new();
    app.daemon_mode = true;
    app.input = "saved draft".into();
    app.cursor = app.input.len();
    app.daemon_sessions = vec![session("done", "finished work", false)];
    app.unread_completions.insert("done".into());
    let buffer = draw(&mut app, 80, 24);
    let y = 23;
    let x = (0..80)
        .find(|&x| buffer[(x, y)].symbol() == "🔔")
        .expect("pinned bell on narrow status");
    handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(app.picker.as_ref().unwrap().kind, "session_attention");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut app, 1, 1);
    handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.input, "saved draft");
}

#[test]
fn unread_failure_marker_survives_a_long_sidebar_title_and_routes_by_id() {
    let mut app = App::new();
    app.daemon_mode = true;
    app.daemon_sessions = vec![session(
        "failed",
        &"long conversation title ".repeat(5),
        false,
    )];
    app.unread_completions.insert("failed".into());
    app.session_turn_outcomes.insert(
        "failed".into(),
        mew_client_core::TurnOutcome::Failed { detail: None },
    );
    let buffer = draw(&mut app, 120, 30);
    let y = row(&buffer, "● long conversation");
    let x = app.sidebar_rect.x + 2;
    let action = handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(matches!(action,Some(Action::AttachSession(id)) if id=="failed"));
}
#[test]
fn core_recency_order_reaches_both_the_sidebar_and_picker_without_grouping() {
    let mut state = mew_client_core::ClientState::default();
    let mut app = App::new();
    app.daemon_mode = true;
    let mut older = session("older", "older work", false);
    older.created_at = 10;
    older.state = mew_protocol::SessionState::Running;
    let mut newer = session("newer", "newer work", false);
    newer.created_at = 20;
    let events = state.apply_server_message(mew_protocol::ServerMessage::SessionList {
        sessions: vec![older, newer],
    });
    app.apply_client_update(&events, &state, None);
    app.open_session_picker();
    assert_eq!(app.picker.as_ref().unwrap().items[0].id, "newer");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let buffer = draw(&mut app, 120, 30);
    assert!(row(&buffer, "newer work") < row(&buffer, "older work"));
}

#[test]
fn conversation_details_format_the_daemons_millisecond_timestamp() {
    let mut app = App::new();
    let mut item = session("conversation", "recent work", false);
    item.last_message_at = Some(1_728_556_200_000);
    app.daemon_sessions = vec![item];
    app.open_session_picker();
    assert!(app.picker.as_ref().unwrap().items[0]
        .context
        .as_ref()
        .unwrap()
        .metadata
        .contains("10-10 10:30"));
}

#[test]
fn conversation_icons_distinguish_decisions_unread_updates_and_running_work() {
    for (permissions, questions, unread, running, expected) in [
        (0, 0, false, false, "○"),
        (0, 0, true, false, "●"),
        (0, 1, false, false, "?"),
        (1, 0, false, false, "!"),
        (1, 1, true, true, "!"),
        (0, 1, true, true, "?"),
        (0, 0, false, true, "▶"),
    ] {
        let mut app = App::new();
        let mut conversation = session("work", "chosen work", false);
        conversation.pending_permissions = permissions;
        conversation.pending_questions = questions;
        if running {
            conversation.state = mew_protocol::SessionState::Running;
        }
        app.daemon_sessions = vec![conversation];
        app.session_turn_outcomes
            .insert("work".into(), mew_client_core::TurnOutcome::Completed);
        if unread {
            app.unread_completions.insert("work".into());
        }
        app.open_session_picker();
        assert_eq!(
            app.picker.as_ref().unwrap().items[0].label,
            format!("{expected} chosen work")
        );
    }
}

#[test]
fn conversation_details_keep_context_without_repeating_quiet_state_or_cost() {
    let mut app = App::new();
    let mut conversation = session("work", "chosen work", false);
    conversation.cwd = Some("/tmp/project".into());
    conversation.last_message_at = Some(1_728_556_200_000);
    conversation.usage = Some(
        serde_json::from_value(serde_json::json!({
            "input_tokens": 100, "output_tokens": 200, "cache_read_tokens": 0,
            "cache_write_tokens": 0, "cost": 2.5, "turns": 1
        }))
        .unwrap(),
    );
    app.daemon_sessions = vec![conversation];
    app.open_session_picker();
    let picker = app.picker.as_ref().unwrap();
    assert!(picker.items[0].description.is_empty());
    let context = picker.items[0].context.as_ref().unwrap();
    assert_eq!(context.path, "/tmp/project");
    assert_eq!(context.metadata, "10-10 10:30");
    let buffer = draw(&mut app, 80, 24);
    let text = (0..buffer.area.height)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .map(|(x, y)| buffer[(x, y)].symbol())
        .collect::<String>();
    assert!(!text.contains("choices") && !text.contains("choice"));
    assert!(!text.contains("move") && !text.contains("choose") && !text.contains("close"));
    assert!(
        text.contains("Tab archived") && text.contains("^A archive") && text.contains("^P pin")
    );
}

#[test]
fn unlisted_conversations_use_the_same_decision_icons() {
    let mut app = App::new();
    app.session_attention.insert("approval".into(), (1, 0));
    app.session_attention.insert("question".into(), (0, 1));
    app.unread_completions.insert("update".into());
    app.open_attention_picker();
    let picker = app.picker.as_ref().unwrap();
    for (id, marker) in [("approval", "!"), ("question", "?"), ("update", "●")] {
        assert_eq!(
            picker
                .items
                .iter()
                .find(|item| item.id == id)
                .unwrap()
                .label,
            format!("{marker} {id}")
        );
    }
}

#[test]
fn read_failures_without_error_details_remain_visible_when_selected() {
    for persisted in [false, true] {
        let mut app = App::new();
        let mut conversation = session("failed", "previous work", false);
        conversation.last_turn_failed = persisted;
        app.daemon_sessions = vec![conversation];
        if !persisted {
            app.session_turn_outcomes.insert(
                "failed".into(),
                mew_client_core::TurnOutcome::Failed { detail: None },
            );
        }
        app.open_session_picker();
        let item = &app.picker.as_ref().unwrap().items[0];
        assert_eq!(item.label, "○ previous work");
        assert_eq!(item.description, "Last turn failed");
    }
}

#[test]
fn project_picker_does_not_repeat_its_title_in_the_footer() {
    let mut app = App::new();
    app.open_project_picker();
    assert!(app.picker.as_ref().unwrap().hint.is_none());
}

#[test]
fn project_details_format_the_daemons_millisecond_timestamp() {
    let mut app = App::new();
    app.projects = vec![mew_protocol::ProjectInfo {
        path: "/tmp/project".into(),
        display_name: "project".into(),
        session_count: 1,
        last_used_at: Some(1_728_556_200_000),
        pinned: false,
    }];
    app.open_project_picker();
    assert!(app.picker.as_ref().unwrap().items[0]
        .description
        .contains("last: 10-10 10:30"));
}

#[test]
fn conversation_context_moves_project_time_and_pending_counts_out_of_selected_details() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut app = App::new();
    app.status.session_id = "work".into();
    let mut conversation = session("work", "chosen work", true);
    conversation.pinned = true;
    conversation.cwd = Some(home.join("projects/日本語").display().to_string());
    conversation.last_message_at = Some(1_728_556_200_000);
    app.daemon_sessions = vec![conversation];
    app.session_attention.insert("work".into(), (2, 3));
    app.session_turn_outcomes.insert(
        "work".into(),
        mew_client_core::TurnOutcome::Failed {
            detail: Some("provider unavailable".into()),
        },
    );
    app.open_session_picker();
    app.toggle_session_picker_archive();
    let item = &app.picker.as_ref().unwrap().items[0];
    let context = item.context.as_ref().unwrap();
    assert_eq!(context.path, "~/projects/日本語");
    assert_eq!(context.metadata, "2 approvals · 3 questions · 10-10 10:30");
    assert_eq!(
        item.description,
        "Archived · Pinned · current\nprovider unavailable"
    );
    assert!(!item.description.contains("projects") && !item.description.contains("10:30"));
    assert!(!item.description.contains("approvals") && !item.description.contains("questions"));
}

#[test]
fn conversation_context_paths_and_metadata_remain_searchable_in_all_catalog_views() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let absolute = home.join("projects/日本語").display().to_string();
    let uppercase = absolute.to_uppercase();
    for kind in ["recent", "archived", "attention"] {
        let mut app = App::new();
        let mut conversation = session("work", "chosen work", kind == "archived");
        conversation.cwd = Some(absolute.clone());
        conversation.last_message_at = Some(1_728_556_200_000);
        app.daemon_sessions = vec![conversation, session("other", "other work", false)];
        app.session_attention.insert("work".into(), (2, 1));
        app.session_turn_outcomes.insert(
            "work".into(),
            mew_client_core::TurnOutcome::Failed {
                detail: Some("provider unavailable".into()),
            },
        );
        if kind == "attention" {
            app.open_attention_picker();
        } else {
            app.open_session_picker();
            if kind == "archived" {
                app.toggle_session_picker_archive();
            }
        }
        let picker = app.picker.as_mut().unwrap();
        for query in [
            absolute.as_str(),
            uppercase.as_str(),
            "~/projects/日本語",
            "日本語",
            "2 approvals",
            "1 question",
            "10-10 10:30",
            "provider unavailable",
            "chosen work",
        ] {
            picker.filter = query.into();
            assert_eq!(
                picker
                    .filtered()
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["work"],
                "{kind}: {query}"
            );
        }
        picker.filter = format!("{}-other/projects/日本語", home.display());
        assert!(
            picker.filtered().is_empty(),
            "home sibling must retain its own identity"
        );
    }
}

#[test]
fn unknown_and_external_project_contexts_remain_explicit_and_searchable() {
    let mut app = App::new();
    let mut external = session("external", "external work", false);
    external.cwd = Some("/tmp/別-project".into());
    app.daemon_sessions = vec![external, session("unknown", "unknown work", false)];
    app.open_session_picker();
    let picker = app.picker.as_mut().unwrap();
    assert_eq!(
        picker.items[0].context.as_ref().unwrap().path,
        "/tmp/別-project"
    );
    assert_eq!(
        picker.items[1].context.as_ref().unwrap().path,
        "Project unavailable"
    );
    for (query, id) in [
        ("/tmp/別-project", "external"),
        ("Project unavailable", "unknown"),
    ] {
        picker.filter = query.into();
        assert_eq!(picker.filtered()[0].id, id);
        assert_eq!(picker.filtered().len(), 1);
    }
    app.session_attention.insert("unlisted".into(), (0, 2));
    app.open_attention_picker();
    let picker = app.picker.as_mut().unwrap();
    assert!(picker.items[0].description.is_empty());
    assert_eq!(
        picker.items[0].context.as_ref().unwrap().metadata,
        "2 questions"
    );
    picker.filter = "2 questions".into();
    assert_eq!(picker.filtered()[0].id, "unlisted");
}
