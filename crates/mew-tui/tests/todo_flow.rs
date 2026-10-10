use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mew_agent::{Todo, TodoStatus};
use mew_tui::{
    app::Mode,
    events::{handle_input_event, Action},
    harness::Harness,
};

fn fixture(width: u16, height: u16) -> Harness {
    let mut h = Harness::new(width, height);
    h.app.status.session_id = "session-a".into();
    h.app.input = "chat draft".into();
    h.app.cursor = h.app.input.len();
    h.app.todos = vec![Todo {
        id: 7,
        content: "first task".into(),
        status: TodoStatus::Pending,
        depends_on: vec![2],
    }];
    h.app.open_todos(Some(7));
    h
}

fn key(h: &mut Harness, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    handle_input_event(&mut h.app, Event::Key(KeyEvent::new(code, modifiers)))
}

#[test]
fn editor_owns_paste_and_save_waits_for_confirmation() {
    let mut h = fixture(80, 24);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    handle_input_event(&mut h.app, Event::Paste(" 日本語".into()));
    key(&mut h, KeyCode::Tab, KeyModifiers::NONE);
    let action = key(&mut h, KeyCode::Char('s'), KeyModifiers::CONTROL).unwrap();
    assert!(
        matches!(action, Action::UpdateTodo { session_id, edit } if session_id == "session-a" && edit.todo_id == 7 && edit.expected_content == "first task" && edit.content == "first task 日本語" && edit.status == "in_progress")
    );
    assert_eq!(h.app.todos[0].content, "first task");
    assert_eq!(h.app.input, "chat draft");
    h.app
        .finish_todo_save(Err("cannot complete dependency".into()));
    assert!(h.render().contains("cannot complete dependency"));
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .content,
        "first task 日本語"
    );
    let mut updated = h.app.todos.clone();
    updated[0].content = "first task 日本語".into();
    updated[0].status = TodoStatus::InProgress;
    h.app.finish_todo_save(Ok(updated));
    assert!(h.app.todo_editor.as_ref().unwrap().draft.is_none());
    assert_eq!(h.app.todos[0].status, TodoStatus::InProgress);
    assert_eq!(h.app.todos[0].depends_on, vec![2]);
}

#[test]
fn cancellation_and_changed_session_preserve_chat_and_do_not_save() {
    let mut h = fixture(40, 16);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    handle_input_event(&mut h.app, Event::Paste("changed".into()));
    h.app.status.session_id = "session-b".into();
    assert!(key(&mut h, KeyCode::Char('s'), KeyModifiers::CONTROL).is_none());
    assert!(h.render().contains("session changed"));
    key(&mut h, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut h, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(h.app.mode, Mode::Normal);
    assert_eq!(h.app.input, "chat draft");
    assert_eq!(h.app.todos[0].content, "first task");
}

#[test]
fn long_unicode_draft_keeps_caret_status_and_save_visible() {
    let mut h = fixture(30, 12);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    handle_input_event(
        &mut h.app,
        Event::Paste(format!("{}tail", "界e\u{301} ".repeat(200))),
    );
    let frame = h.render();
    assert!(frame.contains("tail"), "{frame}");
    assert!(frame.contains("save"), "{frame}");
    assert!(frame.contains("pending"), "{frame}");
}

#[test]
fn live_todo_updates_do_not_replace_unsaved_draft_or_expected_value() {
    let mut h = fixture(80, 24);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    h.app.todos[0].content = "agent changed task".into();
    let action = key(&mut h, KeyCode::Char('s'), KeyModifiers::CONTROL).unwrap();
    assert!(
        matches!(action, Action::UpdateTodo { edit, .. } if edit.expected_content == "first task" && edit.content == "first task")
    );
}

#[test]
fn empty_and_removed_tasks_have_truthful_states() {
    let mut h = fixture(40, 16);
    h.app.todos.clear();
    assert!(h.render().contains("no todos"));
    assert!(key(&mut h, KeyCode::Enter, KeyModifiers::NONE).is_none());
    assert!(h.app.todo_editor.as_ref().unwrap().draft.is_none());
}

#[test]
fn empty_edit_and_read_only_paste_do_not_change_tasks() {
    let mut h = fixture(80, 24);
    handle_input_event(&mut h.app, Event::Paste("ignored".into()));
    assert!(h.app.todo_editor.as_ref().unwrap().draft.is_none());
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut h, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert!(key(&mut h, KeyCode::Char('s'), KeyModifiers::CONTROL).is_none());
    assert!(h.render().contains("cannot be empty"));
    assert_eq!(h.app.todos[0].content, "first task");
}

#[test]
fn scrolling_and_mouse_editing_use_visible_rows() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut h = fixture(40, 16);
    h.app.todos[0].content = "long todo line ".repeat(50);
    let _ = h.render();
    key(&mut h, KeyCode::PageDown, KeyModifiers::NONE);
    let _ = h.render();
    let state = h.app.todo_editor.as_ref().unwrap();
    assert!(state.scroll > 0);
    let (row, id) = state.row_hits[0];
    assert_eq!(id, 7);
    let column = state.body.x;
    handle_input_event(
        &mut h.app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .original
            .id,
        7
    );
}

#[test]
fn multiline_editor_moves_vertically_and_can_reload_a_conflict() {
    let mut h = fixture(40, 16);
    h.app.todos[0].content = "first\nsecond".into();
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    let _ = h.render();
    key(&mut h, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .cursor,
        5
    );
    key(&mut h, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .cursor,
        11
    );
    h.app.todos[0].content = "updated elsewhere".into();
    h.app
        .finish_todo_save(Err("task changed; reopen editor".into()));
    key(&mut h, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .original
            .content,
        "updated elsewhere"
    );
}

#[test]
fn todo_browser_cannot_edit_a_different_sessions_snapshot() {
    let mut h = fixture(40, 16);
    h.app.status.session_id = "session-b".into();
    h.app.todos[0].content = "another sessions task".into();
    let frame = h.render();
    assert!(!frame.contains("another sessions task"));
    assert!(frame.contains("session changed"));
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    assert!(h.app.todo_editor.as_ref().unwrap().draft.is_none());
}

#[test]
fn todo_popover_padding_clears_underlying_transcript() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut h = fixture(40, 16);
    h.say(&"Q".repeat(2000));
    let mut terminal = Terminal::new(TestBackend::new(40, 16)).unwrap();
    terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut h.app))
        .unwrap();
    let body = h.app.todo_editor.as_ref().unwrap().body;
    for y in body.y..body.bottom() {
        assert_eq!(terminal.backend().buffer()[(body.x - 1, y)].symbol(), " ");
    }
}

#[test]
fn returning_from_edit_reveals_the_selected_task() {
    let mut h = fixture(40, 16);
    h.app.todos = (1..20)
        .map(|id| Todo {
            id,
            content: format!("task-{id}"),
            status: TodoStatus::Pending,
            depends_on: vec![],
        })
        .collect();
    h.app.open_todos(Some(19));
    assert!(h.render().contains("task-19"));
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    let _ = h.render();
    key(&mut h, KeyCode::Esc, KeyModifiers::NONE);
    assert!(h.render().contains("task-19"));
}

#[test]
fn idle_notifications_refresh_todos_and_preserve_an_edit_in_progress() {
    let mut h = fixture(80, 24);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    h.app
        .apply_daemon_notification(&mew_protocol::ServerMessage::TodosUpdated {
            todos: vec![mew_protocol::Todo {
                id: 7,
                content: "edited by companion".into(),
                status: "blocked".into(),
                depends_on: vec![2],
            }],
        });
    assert_eq!(h.app.todos[0].content, "edited by companion");
    assert_eq!(h.app.todos[0].status, TodoStatus::Blocked);
    assert_eq!(h.app.todos[0].depends_on, vec![2]);
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .content,
        "first task"
    );
}

#[test]
fn narrow_todo_views_keep_exit_and_save_hints_visible() {
    for width in [20, 30, 40] {
        let mut h = fixture(width, 14);
        assert!(h.render().contains("Esc"));
        key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
        let frame = h.render();
        assert!(frame.contains("Esc") && frame.contains("^s"), "{frame}");
    }
}

#[test]
fn reopening_todos_after_an_interruption_preserves_unsaved_edits() {
    let mut h = fixture(80, 24);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    handle_input_event(&mut h.app, Event::Paste(" unsaved".into()));
    h.app.mode = Mode::Normal;
    h.app.open_todos(None);
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .content,
        "first task unsaved"
    );
}

#[test]
fn confirmed_session_change_clears_old_sidebar_work() {
    let mut h = fixture(80, 24);
    h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
        job_id: "old-job".into(),
        command: "old command".into(),
        state: "running".into(),
    });
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "old-agent".into(),
            name: "researcher".into(),
            child_session_id: "child".into(),
            display_name: None,
        });
    h.app
        .apply_daemon_notification(&mew_protocol::ServerMessage::SessionReady {
            session_id: "session-b".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
    assert!(h.app.todos.is_empty());
    assert!(h.app.subagents.is_empty());
    assert!(h.app.background_jobs.is_empty());
    assert!(h.app.input.is_empty());
    h.app
        .apply_daemon_notification(&mew_protocol::ServerMessage::SessionReady {
            session_id: "session-a".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
    assert_eq!(h.app.input, "chat draft");
}

#[test]
fn resolving_a_permission_returns_to_the_unsaved_todo_editor() {
    let mut h = fixture(80, 24);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    handle_input_event(&mut h.app, Event::Paste(" unsaved".into()));
    let (tx, _rx) = tokio::sync::oneshot::channel();
    h.app.handle_daemon_required_action(
        "permission".into(),
        mew_agent::AgentEvent::PermissionRequest {
            call: mew_hooks::ToolCall {
                tool_name: "bash".into(),
                call_id: "call".into(),
                input: serde_json::json!({}),
            },
            tx,
        },
    );
    assert_eq!(h.app.mode, Mode::PermissionPrompt);
    h.app
        .apply_daemon_notification(&mew_protocol::ServerMessage::RequestResolved {
            request_id: "permission".into(),
        });
    assert_eq!(h.app.mode, Mode::TodoEditor);
    assert_eq!(
        h.app
            .todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .content,
        "first task unsaved"
    );
}

#[test]
fn removing_the_selected_todo_keeps_its_replacement_visible() {
    let mut h = fixture(40, 16);
    h.app.todos = (1..20)
        .map(|id| Todo {
            id,
            content: format!("task-{id}"),
            status: TodoStatus::Pending,
            depends_on: vec![],
        })
        .collect();
    h.app.open_todos(Some(19));
    let _ = h.render();
    h.app.todos.retain(|todo| todo.id != 19);
    let _ = h.render();
    let state = h.app.todo_editor.as_ref().unwrap();
    assert_eq!(state.selected, Some(1));
    assert!(state.row_hits.iter().any(|(_, id)| *id == 1));
}
