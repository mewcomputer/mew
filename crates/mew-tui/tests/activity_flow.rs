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

fn trace_messages() -> Vec<mew_message::Message> {
    serde_json::from_value(serde_json::json!([
        {
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FA1",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "role": "user", "time": {"created": 0},
            "parts": [{"type": "text", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA2", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA1", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "text": "check the attachment lifecycle"}]
        },
        {
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FA3",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "role": "assistant", "time": {"created": 1},
            "parts": [
                {"type": "text", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA4", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA3", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "text": "checking the session owner"},
                {"type": "tool_call", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA5", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA3", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "tool_name": "read", "call_id": "read-owner", "state": {"status": "completed", "input": {"path": "session.rs"}, "output": "owner is retained", "time": {"start": 1, "end": 2}}},
                {"type": "tool_call", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA6", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA3", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "tool_name": "bash", "call_id": "test-owner", "state": {"status": "error", "input": {"command": "cargo test"}, "error": "fixture unavailable", "time": {"start": 3, "end": 4}}}
            ]
        }
    ])).unwrap()
}

fn open_trace(h: &mut Harness) {
    h.app.status.session_id = "parent".into();
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "task".into(),
            name: "researcher".into(),
            child_session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            display_name: Some("Curie".into()),
        });
    h.app.open_subagent("task".into());
}

fn trace_reply(
    action: mew_tui::events::Action,
    messages: Vec<mew_message::Message>,
    error: Option<String>,
) -> mew_protocol::ServerMessage {
    let mew_tui::events::Action::ReadSubagentHistory {
        session_id,
        child_session_id,
        request_id,
    } = action
    else {
        panic!("expected history request")
    };
    mew_protocol::ServerMessage::SubagentHistory {
        session_id,
        child_session_id,
        request_id,
        messages,
        error,
    }
}

#[test]
fn subagent_surface_has_no_outline_and_shows_recorded_calls_and_assignment() {
    let mut h = Harness::new(128, 40);
    open_trace(&mut h);
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(request, trace_messages(), None));
    let frame = h.render();
    for text in [
        "Subagent",
        "Parent assignment",
        "check the attachment lifecycle",
        "checking the session owner",
        "read · completed",
        "session.rs",
        "owner is retained",
        "bash · failed",
        "fixture unavailable",
        "r refresh",
        "Esc close",
    ] {
        assert!(frame.contains(text), "missing {text}: {frame}");
    }
    let title = frame
        .lines()
        .find(|line| line.contains("Subagent · Running"))
        .unwrap();
    let popup_title: String = title.chars().take(h.app.sidebar_rect.x as usize).collect();
    assert!(
        !popup_title.contains('─') && !popup_title.contains('╭') && !popup_title.contains('┌'),
        "{title}"
    );
    assert!(
        h.app.messages().is_empty(),
        "child history must not replace parent chat"
    );
    h.resize(30, 14);
    h.render();
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
    );
    let frame = h.render();
    assert!(
        frame.contains("fixture unavailable") && frame.contains("Esc close"),
        "{frame}"
    );
}

#[test]
fn trace_replies_require_session_child_and_request_and_do_not_reopen_a_view() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    let reply = trace_reply(request, trace_messages(), None);
    for field in ["session_id", "child_session_id", "request_id"] {
        let mut wrong = serde_json::to_value(&reply).unwrap();
        wrong[field] = serde_json::json!("wrong");
        h.app
            .apply_daemon_notification(&serde_json::from_value(wrong).unwrap());
        assert!(!h.render().contains("checking the session owner"));
    }
    h.app.close_activity();
    h.app.apply_daemon_notification(&reply);
    assert!(h.app.activity_view.is_none());
    h.app.open_subagent("task".into());
    h.app.apply_daemon_notification(&reply);
    assert!(!h.render().contains("checking the session owner"));
    h.app.status.session_id = "other-parent".into();
    assert!(h
        .app
        .take_activity_request(std::time::Instant::now())
        .is_none());
}

#[test]
fn trace_refresh_is_bounded_preserves_scroll_and_stops_on_timeout_or_completion() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    let now = std::time::Instant::now();
    let first = h.app.take_activity_request(now).unwrap();
    assert!(h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(1))
        .is_none());
    h.app
        .apply_daemon_notification(&trace_reply(first, trace_messages(), None));
    h.render();
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
    );
    let scroll = h.app.activity_view.as_ref().unwrap().scroll;
    let request = h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(3))
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(request, trace_messages(), None));
    assert_eq!(h.app.activity_view.as_ref().unwrap().scroll, scroll);
    let _request = h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(6))
        .unwrap();
    assert!(h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(10))
        .is_none());
    let frame = h.render();
    assert!(frame.contains("not confirmed"), "{frame}");
    assert!(h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(20))
        .is_none());
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
    );
    let retry = h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(21))
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(retry, trace_messages(), None));
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentEnd {
            parent_call_id: "task".into(),
            child_session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            outcome: mew_agent::SubagentOutcome::Completed,
            manifests: vec![],
        });
    let final_request = h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(22))
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(final_request, trace_messages(), None));
    assert!(h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(30))
        .is_none());
}

#[test]
fn empty_recorded_trace_and_failed_refresh_are_truthful_and_recoverable() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app.apply_daemon_notification(&trace_reply(
        request,
        vec![],
        Some("first read failed".into()),
    ));
    assert!(!h.render().contains("Loading recorded turns"));
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
    );
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(request, vec![], None));
    let frame = h.render();
    assert!(frame.contains("No messages have been recorded"), "{frame}");
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
    );
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(request, trace_messages(), None));
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
    );
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app.apply_daemon_notification(&trace_reply(
        request,
        vec![],
        Some("recording unavailable".into()),
    ));
    let frame = h.render();
    assert!(frame.contains("recording unavailable"), "{frame}");
    assert!(!frame.contains("Loading recorded turns"), "{frame}");
    assert_eq!(
        h.app
            .activity_view
            .as_ref()
            .unwrap()
            .transcript
            .as_ref()
            .unwrap()
            .messages,
        trace_messages()
    );
}

#[test]
fn recorded_tool_revisions_and_resize_keep_full_output_reachable() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    let mut messages = trace_messages();
    let mew_message::Part::ToolCall(call) = &mut messages[1].parts[2] else {
        unreachable!()
    };
    call.state = mew_message::ToolState::Running(mew_message::ToolStateRunning {
        input: serde_json::json!({"command": "cargo test"}),
        output: "still running".into(),
        time: mew_message::ToolTime {
            start: 3,
            end: None,
        },
    });
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(request, messages.clone(), None));
    h.render();
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
    );
    assert!(h.render().contains("still running"));
    let mew_message::Part::ToolCall(call) = &mut messages[1].parts[2] else {
        unreachable!()
    };
    call.state = mew_message::ToolState::Completed(mew_message::ToolStateCompleted {
        input: call.state.input().clone(),
        output: format!(
            "{}\ntrace-result-tail",
            "Unicode 界e\u{301} result\n".repeat(50)
        ),
        metadata: None,
        diff: None,
        images: vec![],
        time: mew_message::ToolTime {
            start: 3,
            end: Some(4),
        },
    });
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
    );
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(request, messages, None));
    for (width, height) in [(80, 24), (30, 14), (120, 30)] {
        h.resize(width, height);
        h.render();
        handle_input_event(
            &mut h.app,
            Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        );
        let frame = h.render();
        assert!(
            frame.contains("trace-result-tail") && frame.contains("Esc close"),
            "{frame}"
        );
        assert!(!frame.contains("still running"), "{frame}");
    }
}

#[test]
fn repeated_refresh_and_completion_coalesce_while_a_trace_request_is_pending() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    let now = std::time::Instant::now();
    let first = h.app.take_activity_request(now).unwrap();
    for _ in 0..20 {
        handle_input_event(
            &mut h.app,
            Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
        );
        assert!(
            h.app
                .take_activity_request(std::time::Instant::now())
                .is_none(),
            "refresh must not bypass a pending read"
        );
    }
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentEnd {
            parent_call_id: "task".into(),
            child_session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            outcome: mew_agent::SubagentOutcome::Completed,
            manifests: vec![],
        });
    h.app
        .apply_daemon_notification(&trace_reply(first, trace_messages(), None));
    let final_request = h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(1))
        .unwrap();
    assert!(h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(1))
        .is_none());
    h.app
        .apply_daemon_notification(&trace_reply(final_request, trace_messages(), None));
    assert!(h
        .app
        .take_activity_request(now + std::time::Duration::from_secs(20))
        .is_none());
}

#[test]
fn a_restarted_child_cannot_accept_the_old_transcript_under_the_new_identity() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "task".into(),
            child_session_id: "replacement-child".into(),
            name: "coder".into(),
            display_name: Some("Turing".into()),
        });
    h.app
        .apply_daemon_notification(&trace_reply(request, trace_messages(), None));
    assert!(h
        .app
        .take_activity_request(std::time::Instant::now() + std::time::Duration::from_secs(4))
        .is_none());
    let frame = h.render();
    assert!(frame.contains("child changed"), "{frame}");
    assert!(!frame.contains("checking the session owner"), "{frame}");
    assert!(
        !frame.contains("Turing"),
        "old viewer must not display the new identity: {frame}"
    );
}

#[test]
fn a_delayed_completion_cannot_finish_the_replacement_child() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "task".into(),
            child_session_id: "replacement".into(),
            name: "coder".into(),
            display_name: None,
        });
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentEnd {
            parent_call_id: "task".into(),
            child_session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            outcome: mew_agent::SubagentOutcome::Cancelled,
            manifests: vec![],
        });
    assert!(matches!(
        h.app.subagents[0].status,
        mew_tui::app::SubagentStatus::Running
    ));
}

#[test]
fn recorded_trace_labels_instructions_and_keeps_available_reasoning_and_files_reachable() {
    let mut h = Harness::new(80, 24);
    open_trace(&mut h);
    let mut messages = trace_messages();
    let mew_message::Part::ToolCall(call) = &mut messages[1].parts[1] else {
        unreachable!()
    };
    call.state = mew_message::ToolState::Pending(mew_message::ToolStatePending {
        input: call.state.input().clone(),
        time: mew_message::ToolTime {
            start: 0,
            end: None,
        },
    });
    let base = serde_json::to_value(&messages[1].parts[0]).unwrap();
    for (kind, fields) in [
        (
            "reasoning",
            serde_json::json!({"text": "visible reasoning note", "encrypted_content": "opaque-provider-data"}),
        ),
        (
            "file",
            serde_json::json!({"mime": "text/plain", "url": "trace-attachment.txt"}),
        ),
        (
            "compaction",
            serde_json::json!({"auto": true, "summary": "saved context summary"}),
        ),
        ("compaction", serde_json::json!({"auto": true})),
    ] {
        let mut value = base.clone();
        value["type"] = serde_json::json!(kind);
        value["id"] = serde_json::json!(ulid::Ulid::new().to_string());
        value.as_object_mut().unwrap().remove("text");
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        messages[1]
            .parts
            .push(serde_json::from_value(value).unwrap());
    }
    let mut correction = messages[0].clone();
    correction.id = ulid::Ulid::new();
    let mew_message::Part::Text(text) = &mut correction.parts[0] else {
        unreachable!()
    };
    text.base.message_id = correction.id;
    text.base.id = ulid::Ulid::new();
    text.text = "schema correction".into();
    messages.push(correction);
    let request = h
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap();
    h.app
        .apply_daemon_notification(&trace_reply(request, messages, None));
    let mut frames = String::new();
    for _ in 0..8 {
        frames.push_str(&h.render());
        handle_input_event(
            &mut h.app,
            Event::Key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
        );
    }
    for text in [
        "read · pending",
        "visible reasoning note",
        "trace-attachment.txt",
        "saved context summary",
        "Context compacted",
        "Instructions",
        "schema correction",
    ] {
        assert!(frames.contains(text), "missing {text}: {frames}");
    }
    assert!(!frames.contains("opaque-provider-data"));
}
