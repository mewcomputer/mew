use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{app::*, harness::Harness, ui};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

fn render(app: &mut App, width: u16, height: u16) -> (Buffer, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    (terminal.backend().buffer().clone(), (cursor.x, cursor.y))
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(h: &mut Harness, code: KeyCode) {
    h.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn short_permission_is_compact_and_keeps_all_choices() {
    let mut app = App::new();
    app.mode = Mode::PermissionPrompt;
    app.permission = Some(PermissionState {
        tool_name: "bash".into(),
        call_id: "tool-call".into(),
        input: serde_json::json!({"command": "pwd"}),
        tx: None,
        selected: 0,
        scroll: 0,
        view: Default::default(),
    });
    let (buffer, _) = render(&mut app, 80, 40);
    let surface = app.theme.resolve("panel.overlay");
    let rows = (0..40).filter(|y| buffer[(40, *y)].bg == surface).count();
    assert!(rows <= 14, "small request occupies {rows} popup rows");
    for choice in ["allow once", "session", "deny"] {
        assert!(text(&buffer).contains(choice), "missing {choice}");
    }
}

#[test]
fn proposed_goal_scrolls_without_hiding_or_changing_the_decision() {
    let mut h = Harness::new(40, 16);
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    h.app
        .handle_agent_event(mew_agent::AgentEvent::GoalProposed {
            call_id: "proposal".into(),
            objective: format!("{} finalobjective", "long objective ".repeat(60)),
            tx,
        });
    let first = text(&render(&mut h.app, 40, 16).0);
    assert!(first.contains("Accept") && first.contains("Reject"));
    for _ in 0..100 {
        key(&mut h, KeyCode::PageDown);
    }
    let last = text(&render(&mut h.app, 40, 16).0);
    assert!(
        last.contains("finalobjective"),
        "last objective row must be reachable\n{last}"
    );
    assert!(last.contains("Accept") && last.contains("Reject"));
    assert_eq!(h.app.goal_proposal.as_ref().unwrap().selected, 0);
    key(&mut h, KeyCode::Tab);
    key(&mut h, KeyCode::Enter);
    assert!(matches!(
        rx.try_recv(),
        Ok(mew_agent::GoalDecision::Rejected)
    ));
    assert!(h.app.goal_proposal.is_none());
}

#[test]
fn persona_details_scroll_instead_of_overwriting_actions() {
    let mut h = Harness::new(40, 16);
    h.app.request_persona_switch_confirm(
        PersonaSummary {
            name: "researcher".into(),
            description: "long description ".repeat(50),
            tools: Some(vec!["read".into(), "search".into()]),
            skills: Some(vec!["lastskill".into()]),
            ..Default::default()
        },
        None,
    );
    let first = text(&render(&mut h.app, 40, 16).0);
    assert!(first.contains("Confirm") && first.contains("Cancel"));
    for _ in 0..100 {
        key(&mut h, KeyCode::PageDown);
    }
    let last = text(&render(&mut h.app, 40, 16).0);
    assert!(
        last.contains("lastskill"),
        "persona restrictions must be reachable\n{last}"
    );
    assert!(last.contains("Confirm") && last.contains("Cancel"));
    assert_eq!(h.app.persona_switch_confirm.as_ref().unwrap().selected, 0);
    key(&mut h, KeyCode::Esc);
    assert!(h.app.persona_switch_confirm.is_none());
}

#[test]
fn goal_editor_keeps_unicode_caret_and_tail_visible_when_resized() {
    let mut app = App::new();
    app.open_goal_compose(format!("{} finaltail", "目標 ".repeat(70)));
    for (width, height) in [(80, 24), (40, 16), (20, 10), (8, 6), (2, 2)] {
        let (buffer, cursor) = render(&mut app, width, height);
        assert!(
            cursor.0 < width && cursor.1 < height,
            "caret outside {width}x{height}: {cursor:?}"
        );
        if width >= 20 {
            assert!(
                text(&buffer).contains("finaltail"),
                "edited tail not visible at {width}x{height}"
            );
            assert!(text(&buffer).contains("Set") && text(&buffer).contains("Cancel"));
        }
    }
}

#[test]
fn permission_scroll_matches_rendered_rows_after_resize_and_preserves_reply() {
    let mut h = Harness::new(80, 30);
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    h.app.permission = Some(PermissionState {
        tool_name: "bash".into(),
        call_id: "permission-request".into(),
        input: serde_json::json!({"data": "longinput ".repeat(100), "lastinput": true}),
        tx: Some(tx),
        selected: 1,
        scroll: 0,
        view: Default::default(),
    });
    h.app.mode = Mode::PermissionPrompt;
    for (width, height) in [(80, 30), (30, 14), (80, 40)] {
        render(&mut h.app, width, height);
        key(&mut h, KeyCode::End);
        let last = text(&render(&mut h.app, width, height).0);
        assert!(
            last.contains("lastinput"),
            "input end unavailable at {width}x{height}\n{last}"
        );
        for label in ["allow once", "session", "deny"] {
            assert!(last.contains(label));
        }
        let p = h.app.permission.as_ref().unwrap();
        assert_eq!(p.call_id, "permission-request");
        assert_eq!(p.scroll, p.view.max_scroll);
        assert_eq!(p.max_scroll(width, height).unwrap_or(0), p.scroll);
        assert_eq!(p.selected, 1);
    }
    key(&mut h, KeyCode::Enter);
    assert!(matches!(
        rx.try_recv(),
        Ok(mew_hooks::PermissionDecision::AllowSession)
    ));
}

#[test]
fn goal_compose_paste_updates_its_editor_and_undo_without_touching_chat() {
    let mut app = App::new();
    app.input = "saved draft".into();
    app.cursor = app.input.len();
    app.open_goal_compose("goal ".into());
    mew_tui::events::handle_input_event(
        &mut app,
        crossterm::event::Event::Paste("目標\nfinaltail".into()),
    );
    assert_eq!(
        app.goal_compose.as_ref().unwrap().title,
        "goal 目標\nfinaltail"
    );
    assert_eq!(app.input, "saved draft");
    mew_tui::events::handle_input_event(
        &mut app,
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
    );
    assert_eq!(app.goal_compose.as_ref().unwrap().title, "goal ");
}

#[test]
fn pending_decision_ignores_paste_into_hidden_chat_editor() {
    let mut app = App::new();
    app.input = "saved draft".into();
    app.cursor = app.input.len();
    for mode in [
        Mode::PermissionPrompt,
        Mode::PersonaSwitchConfirm,
        Mode::GoalProposal,
    ] {
        app.mode = mode;
        mew_tui::events::handle_input_event(
            &mut app,
            crossterm::event::Event::Paste("unexpected".into()),
        );
        assert_eq!(app.input, "saved draft", "hidden draft changed in {mode:?}");
        assert_eq!(app.mode, mode);
    }
}

#[test]
fn proposal_wheel_and_resize_keep_actions_pinned_and_focus_stable() {
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::GoalProposed {
        call_id: "wheel-proposal".into(),
        objective: "line of objective ".repeat(80),
        tx,
    });
    render(&mut app, 40, 16);
    mew_tui::events::handle_input_event(
        &mut app,
        crossterm::event::Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 20,
            row: 8,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(app.goal_proposal.as_ref().unwrap().scroll, 1);
    mew_tui::events::handle_input_event(
        &mut app,
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    mew_tui::events::handle_input_event(
        &mut app,
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
    );
    assert_eq!(app.goal_proposal.as_ref().unwrap().selected, 0);
    app.goal_proposal.as_mut().unwrap().scroll = u16::MAX;
    for (width, height) in [(20, 10), (80, 30), (8, 6), (2, 2)] {
        let previous_scroll = app.goal_proposal.as_ref().unwrap().scroll;
        let buffer = render(&mut app, width, height).0;
        let proposal = app.goal_proposal.as_ref().unwrap();
        assert_eq!(
            proposal.scroll,
            previous_scroll.min(proposal.view.max_scroll)
        );
        assert_eq!(proposal.call_id, "wheel-proposal");
        if width >= 20 {
            assert!(text(&buffer).contains("Accept") && text(&buffer).contains("Reject"));
        }
    }
}

#[test]
fn narrow_decision_dialogs_keep_the_cancel_key_visible() {
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::GoalProposed {
        call_id: "narrow".into(),
        objective: "long objective ".repeat(80),
        tx,
    });
    for width in [20, 30, 40] {
        let buffer = render(&mut app, width, 14).0;
        let visible = text(&buffer);
        assert!(
            visible.contains("Esc"),
            "cancel key clipped at width {width}\n{visible}"
        );
        assert!(
            visible.contains("Enter"),
            "confirmation key clipped at width {width}\n{visible}"
        );
    }
}

#[test]
fn goal_cursor_after_a_full_row_lands_on_a_blank_cell() {
    let mut app = App::new();
    for objective in ["a".repeat(66), "a".repeat(65), "目".repeat(33)] {
        app.open_goal_compose(objective);
        let (buffer, (x, y)) = render(&mut app, 80, 24);
        assert_eq!(
            buffer[(x, y)].symbol(),
            " ",
            "insertion caret covers final character"
        );
        assert!(
            [x.saturating_sub(1), x.saturating_sub(2)]
                .into_iter()
                .any(|column| buffer[(column, y)].symbol() != " "),
            "caret must follow visible final character"
        );
    }
}

#[test]
fn proposed_goal_preserves_logical_lines_and_can_scroll_to_the_last_one() {
    let mut h = Harness::new(40, 16);
    let (tx, _rx) = tokio::sync::oneshot::channel();
    h.app
        .handle_agent_event(mew_agent::AgentEvent::GoalProposed {
            call_id: "multiline".into(),
            objective: (0..40)
                .map(|n| format!("objective{n:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            tx,
        });
    let first = text(&render(&mut h.app, 40, 16).0);
    assert!(first.contains("objective00"));
    assert!(first.contains("objective01"));
    assert!(first.contains("objective02"));
    key(&mut h, KeyCode::End);
    let last = text(&render(&mut h.app, 40, 16).0);
    assert!(
        last.contains("objective39"),
        "last logical line missing\n{last}"
    );
}
