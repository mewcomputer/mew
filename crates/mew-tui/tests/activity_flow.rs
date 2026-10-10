use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use mew_tui::{app::Mode, events::handle_input_event, harness::Harness};

#[test]
fn activity_details_do_not_follow_an_id_into_another_session() {
    let mut h = Harness::new(80, 24);
    h.app.status.session_id = "first-session".into();
    h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
        job_id: "same-id".into(),
        command: "first-session-command".into(),
        state: "running".into(),
    });
    h.app.open_job("same-id".into());
    assert!(h.render().contains("first-session-command"));
    h.app.status.session_id = "second-session".into();
    h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
        job_id: "same-id".into(),
        command: "second-session-command".into(),
        state: "running".into(),
    });
    let frame = h.render();
    assert!(frame.contains("Session changed"), "{frame}");
    assert!(!frame.contains("first-session-command"), "{frame}");
    assert!(!frame.contains("second-session-command"), "{frame}");
}

#[test]
fn changing_session_releases_the_viewers_completed_agent_record() {
    let mut h = Harness::new(80, 24);
    h.app.status.session_id = "first-session".into();
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "agent".into(),
            name: "researcher".into(),
            child_session_id: "child".into(),
            display_name: None,
        });
    h.app.open_subagent("agent".into());
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentEnd {
            parent_call_id: "agent".into(),
            child_session_id: "child".into(),
            outcome: mew_agent::SubagentOutcome::Completed,
            manifests: vec![],
        });
    h.render();
    assert_eq!(h.app.subagents.len(), 1);
    h.app.status.session_id = "second-session".into();
    let frame = h.render();
    assert!(frame.contains("Session changed"), "{frame}");
    assert!(h.app.subagents.is_empty());
}

#[test]
fn sidebar_job_click_opens_details_without_touching_the_chat_draft() {
    let mut h = Harness::new(120, 30);
    h.app.input = "saved draft".into();
    h.app.cursor = h.app.input.len();
    h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
        job_id: "build".into(),
        command: "cargo test --workspace --all-features -- long-command-tail".into(),
        state: "running".into(),
    });
    let frame = h.render();
    let row = frame
        .lines()
        .position(|line| line.contains("cargo test"))
        .unwrap() as u16;
    let column = h.app.sidebar_rect.x + 3;
    handle_input_event(
        &mut h.app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_ne!(
        h.app.mode,
        Mode::Normal,
        "clicking visible background work must open its details"
    );
    let frame = h.render();
    assert!(
        frame.contains("long-command-tail"),
        "full command must be readable: {frame}"
    );
    assert_eq!(h.app.input, "saved draft");
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert_eq!(h.app.mode, Mode::Normal);
    assert_eq!(h.app.input, "saved draft");
}

#[test]
fn subagent_details_follow_live_state_and_survive_completion_until_closed() {
    let mut h = Harness::new(80, 24);
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "research".into(),
            name: "researcher".into(),
            child_session_id: "child".into(),
            display_name: Some("Curie".into()),
        });
    h.app.open_subagent("research".into());
    assert!(h.render().contains("researcher"));
    let progress = format!("{} final-progress-tail", "progress detail ".repeat(100));
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStatus {
            parent_call_id: "research".into(),
            tool_name: "read".into(),
            message: progress,
        });
    for (width, height) in [(80, 24), (30, 14)] {
        h.resize(width, height);
        h.render();
        handle_input_event(
            &mut h.app,
            Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        );
        let frame = h.render();
        assert!(frame.contains("final-progress-tail"), "{frame}");
        assert!(frame.contains("Esc close"), "{frame}");
    }
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentEnd {
            parent_call_id: "research".into(),
            child_session_id: "child".into(),
            outcome: mew_agent::SubagentOutcome::Completed,
            manifests: vec![],
        });
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
    );
    let frame = h.render();
    assert!(frame.contains("Completed"), "{frame}");
    assert_eq!(
        h.app.subagents.len(),
        1,
        "open detail record must remain available"
    );
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    h.render();
    assert!(
        h.app.subagents.is_empty(),
        "closing completed details releases retention"
    );
}

#[test]
fn job_details_use_stable_ids_and_show_current_status_or_missing_source() {
    let mut h = Harness::new(80, 24);
    for id in ["first", "second"] {
        h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
            job_id: id.into(),
            command: format!("command {id}"),
            state: "running".into(),
        });
    }
    h.app.open_job("second".into());
    h.app.background_jobs.reverse();
    h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
        job_id: "second".into(),
        command: "updated command second".into(),
        state: "failed".into(),
    });
    let frame = h.render();
    assert!(
        frame.contains("updated command second") && frame.contains("Failed"),
        "{frame}"
    );
    assert!(
        !frame.contains("command first"),
        "details switched source: {frame}"
    );
    h.app.background_jobs.clear();
    let frame = h.render();
    assert!(
        frame.contains("no longer available") && frame.contains("Esc close"),
        "{frame}"
    );
}

#[test]
fn keyboard_activity_picker_opens_the_selected_record() {
    let mut h = Harness::new(80, 24);
    for id in ["first", "second"] {
        h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
            job_id: id.into(),
            command: format!("command {id}"),
            state: "running".into(),
        });
    }
    h.app.open_job_picker();
    assert!(h.render().contains("Background jobs"));
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
    );
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    let frame = h.render();
    assert!(frame.contains("command second"), "{frame}");
    assert_eq!(h.app.mode, Mode::Activity);
}

#[test]
fn sidebar_rows_route_todos_and_agent_progress_by_id_after_scrolling() {
    use mew_tui::app::{ActivityTarget, SidebarItem};
    let mut h = Harness::new(120, 18);
    h.app.todos = (0..20)
        .map(|id| mew_agent::Todo {
            id,
            content: format!("todo {id}"),
            status: mew_agent::TodoStatus::Pending,
            depends_on: vec![],
        })
        .collect();
    h.render();
    let (row, _) = h
        .app
        .sidebar_item_rows
        .iter()
        .find(|(_, item)| matches!(item, SidebarItem::Todo(0)))
        .unwrap()
        .clone();
    let column = h.app.sidebar_rect.x + 3;
    handle_input_event(
        &mut h.app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(h.app.mode, Mode::TodoEditor);
    assert_eq!(h.app.todo_editor.as_ref().unwrap().selected, Some(0));
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "agent".into(),
            name: "researcher".into(),
            child_session_id: "child".into(),
            display_name: None,
        });
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStatus {
            parent_call_id: "agent".into(),
            tool_name: "read".into(),
            message: "progress with a very long label that is clipped".into(),
        });
    h.render();
    h.app.sidebar_scroll = h.app.sidebar_max_scroll;
    let frame = h.render();
    let hits: Vec<_> = h
        .app
        .sidebar_item_rows
        .iter()
        .filter(|(_, item)| matches!(item, SidebarItem::Subagent(id) if id == "agent"))
        .cloned()
        .collect();
    assert_eq!(
        hits.len(),
        2,
        "agent identity and progress should both be clickable"
    );
    let (row, _) = hits.last().unwrap();
    assert!(frame
        .lines()
        .nth(*row as usize)
        .unwrap()
        .contains("progress"));
    handle_input_event(
        &mut h.app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row: *row,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(
        h.app.activity_view.as_ref().unwrap().target,
        ActivityTarget::Subagent("agent".into())
    );
    handle_input_event(&mut h.app, Event::Paste("unexpected paste".into()));
    assert!(h.app.input.is_empty());
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    h.app.todos.clear();
    h.resize(80, 24);
    h.render();
    assert!(
        h.app.sidebar_item_rows.is_empty(),
        "hidden sidebar must clear its hit targets"
    );
}

#[test]
fn failed_agent_details_remain_readable_after_sidebar_expiry() {
    let mut h = Harness::new(120, 24);
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "agent".into(),
            name: "researcher".into(),
            child_session_id: "child".into(),
            display_name: None,
        });
    h.app.open_subagent("agent".into());
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentEnd {
            parent_call_id: "agent".into(),
            child_session_id: "child".into(),
            outcome: mew_agent::SubagentOutcome::Failed {
                reason: "source unavailable\nretry after credentials recover".into(),
            },
            manifests: vec![],
        });
    h.app.subagents[0].finished_at =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(181));
    let frame = h.render();
    assert!(
        frame.contains("Failed")
            && !h
                .app
                .sidebar_item_rows
                .iter()
                .any(|(_, item)| matches!(item, mew_tui::app::SidebarItem::Subagent(_)))
    );
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
    );
    let frame = h.render();
    assert!(frame.contains("retry after credentials recover"), "{frame}");
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(h.app.subagents.is_empty());
}
