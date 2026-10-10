use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{app::*, events::handle_input_event, ui};
use ratatui::{
    backend::{Backend, TestBackend},
    Terminal,
};

fn render(app: &mut App, width: u16, height: u16) -> (String, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let visible = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let cursor = terminal.get_cursor_position().unwrap();
    (visible, (cursor.x, cursor.y))
}

fn paste(app: &mut App, text: &str) {
    handle_input_event(app, Event::Paste(text.into()));
}
fn key(app: &mut App, code: KeyCode) {
    handle_input_event(app, Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

#[test]
fn long_landing_drafts_keep_the_edited_tail_visible_on_short_screens() {
    let mut app = App::new();
    paste(&mut app, &format!("{} tail", "目標 ".repeat(150)));
    for (width, height) in [(80, 24), (40, 10), (20, 6), (8, 4)] {
        let (visible, cursor) = render(&mut app, width, height);
        assert!(
            visible
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
                .contains("tail"),
            "landing prompt invisible at {width}x{height}\n{visible}"
        );
        assert!(cursor.0 < width && cursor.1 < height);
    }
}

#[test]
fn large_paste_confirmation_and_accepted_text_stay_visible_on_short_screens() {
    for (width, height) in [(80, 8), (20, 6), (8, 4)] {
        let mut app = App::new();
        app.input = "draft ".into();
        app.cursor = app.input.len();
        paste(&mut app, &format!("{} tail", "a".repeat(2500)));
        assert_eq!(app.mode, Mode::PasteConfirm);
        let (visible, _) = render(&mut app, width, height);
        assert!(
            visible.contains("y/N"),
            "paste choice invisible at {width}x{height}\n{visible}"
        );
        key(&mut app, KeyCode::Char('y'));
        let (visible, cursor) = render(&mut app, width, height);
        assert!(
            visible
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
                .contains("tail"),
            "accepted paste invisible at {width}x{height}\n{visible}"
        );
        assert!(cursor.0 < width && cursor.1 < height);
        app.undo();
        assert_eq!(app.input, "draft ");
    }
}

#[test]
fn unicode_paste_threshold_and_label_count_characters() {
    let mut app = App::new();
    paste(&mut app, &"目".repeat(2000));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.input.chars().count(), 2000);
    paste(&mut app, &"目".repeat(2001));
    assert_eq!(app.mode, Mode::PasteConfirm);
    assert!(render(&mut app, 80, 24).0.contains("2001 chars"));
}

#[test]
fn paste_normalizes_terminal_line_endings_as_one_undo_step() {
    let mut app = App::new();
    paste(&mut app, "first\r\nsecond\rthird");
    assert_eq!(app.input, "first\nsecond\nthird");
    app.undo();
    assert_eq!(app.input, "");
    app.open_goal_compose("goal ".into());
    paste(&mut app, "first\r\nsecond\rthird");
    assert_eq!(
        app.goal_compose.as_ref().unwrap().title,
        "goal first\nsecond\nthird"
    );
}

#[test]
fn accepting_a_large_slash_paste_restores_slash_mode() {
    let mut app = App::new();
    paste(&mut app, &format!("/goal {}", "a".repeat(2100)));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::SlashCommand);
    assert!(app.input.starts_with("/goal "));
}

#[test]
fn permission_uses_a_raised_panel_without_an_outline() {
    let mut app = App::new();
    app.permission = Some(PermissionState {
        tool_name: "bash".into(),
        call_id: "permission".into(),
        input: serde_json::json!({"command":"pwd"}),
        tx: None,
        selected: 0,
        scroll: 0,
        view: Default::default(),
    });
    app.mode = Mode::PermissionPrompt;
    let (visible, _) = render(&mut app, 80, 24);
    for character in ['┌', '┐', '└', '┘', '─', '│'] {
        assert!(
            !visible.contains(character),
            "permission outline still drawn"
        );
    }
    for label in ["Allow tool?", "allow once", "session", "deny"] {
        assert!(visible.contains(label));
    }
}

#[test]
fn pasting_in_search_and_picker_edits_the_visible_field_only() {
    let mut app = App::new();
    app.input = "saved draft".into();
    app.cursor = app.input.len();
    app.mode = Mode::HistorySearch;
    paste(&mut app, "query");
    assert_eq!(app.history_search_query, "query");
    assert_eq!(app.input, "saved draft");
    app.open_command_palette();
    paste(&mut app, "review");
    assert_eq!(app.picker.as_ref().unwrap().filter, "review");
    assert_eq!(app.input, "saved draft");
    app.mode = Mode::Help;
    paste(&mut app, "hidden");
    assert_eq!(app.input, "saved draft");
}

#[test]
fn pasting_a_freeform_answer_updates_the_answer_only() {
    let mut app = App::new();
    app.input = "saved draft".into();
    app.cursor = app.input.len();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::AskUser {
        call_id: "question".into(),
        questions: vec![mew_agent::AskUserQuestion {
            prompt: "which branch?".into(),
            options: vec![],
        }],
        tx,
    });
    paste(&mut app, "feature/目標");
    assert_eq!(
        app.user_question.as_ref().unwrap().freeform_text,
        "feature/目標"
    );
    assert_eq!(app.input, "saved draft");
}

#[test]
fn non_editor_modals_hide_the_underlying_chat_caret() {
    let mut app = App::new();
    app.input = "saved draft".into();
    app.cursor = app.input.len();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let mut visible = terminal.backend().clone();
    visible.show_cursor().unwrap();
    assert!(terminal.backend() == &visible);
    for mode in [
        Mode::Help,
        Mode::PermissionPrompt,
        Mode::PersonaSwitchConfirm,
        Mode::GoalProposal,
    ] {
        app.mode = mode;
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let mut hidden = terminal.backend().clone();
        hidden.hide_cursor().unwrap();
        assert!(
            terminal.backend() == &hidden,
            "hidden composer caret visible in {mode:?}"
        );
    }
}

#[test]
fn question_freeform_viewport_follows_unicode_caret_after_paste_and_resize() {
    let mut app = App::new();
    app.push_synthetic_message("waiting for the branch selection".into());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::AskUser {
        call_id: "long-answer".into(),
        questions: vec![mew_agent::AskUserQuestion {
            prompt: "which branch?".into(),
            options: vec![],
        }],
        tx,
    });
    let answer = format!("{}finaltail", "目標 ".repeat(180));
    paste(&mut app, &answer);
    for width in [80, 30, 20] {
        let (visible, cursor) = render(&mut app, width, 16);
        assert!(
            visible.contains("tail"),
            "answer tail hidden at width{width}\n{visible}"
        );
        assert!(cursor.0 < width && cursor.1 < 16);
    }
    key(&mut app, KeyCode::Home);
    let (visible, _) = render(&mut app, 30, 16);
    assert!(visible.contains("目"), "answer beginning hidden after Home");
    key(&mut app, KeyCode::Char('!'));
    assert!(render(&mut app, 30, 16).0.contains('!'));
    assert_eq!(
        app.user_question.as_ref().unwrap().freeform_text,
        format!("!{answer}")
    );
}

#[test]
fn question_multiline_paste_keeps_text_and_caret_after_the_last_logical_line() {
    let mut app = App::new();
    app.push_synthetic_message("waiting for an answer".into());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::AskUser {
        call_id: "multiline-answer".into(),
        questions: vec![mew_agent::AskUserQuestion {
            prompt: "what changed?".into(),
            options: vec![],
        }],
        tx,
    });
    let answer = format!("{}\nfinaltail", "joined 👨‍👩‍👧‍👦 e\u{301} ".repeat(100));
    paste(&mut app, &answer);
    let mut terminal = Terminal::new(TestBackend::new(30, 16)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    let buffer = terminal.backend().buffer();
    let row = (0..30)
        .map(|x| buffer[(x, cursor.y)].symbol())
        .collect::<String>();
    assert!(row.contains("tail"), "multiline answer tail hidden\n{row}");
    assert_eq!(buffer[(cursor.x, cursor.y)].symbol(), " ");
    assert_eq!(app.user_question.as_ref().unwrap().freeform_text, answer);
}

#[test]
fn question_without_a_transcript_reserves_space_for_its_answer() {
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::AskUser {
        call_id: "empty-question".into(),
        questions: vec![mew_agent::AskUserQuestion {
            prompt: "what next?".into(),
            options: vec![],
        }],
        tx,
    });
    paste(&mut app, "answer-tail");
    let (visible, cursor) = render(&mut app, 80, 24);
    assert!(
        visible.contains("answer-tail"),
        "active answer missing without transcript\n{visible}"
    );
    assert!(cursor.0 < 80 && cursor.1 < 24);
}

#[test]
fn question_freeform_stays_visible_below_long_choice_descriptions() {
    let mut app = App::new();
    app.push_synthetic_message("waiting for an answer".into());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::AskUser {
        call_id: "described-choice".into(),
        questions: vec![mew_agent::AskUserQuestion {
            prompt: "what next?".repeat(30),
            options: vec![mew_agent::QuestionOption {
                label: "native browser".into(),
                description: "restore the retained page and session reliably ".repeat(100),
            }],
        }],
        tx,
    });
    key(&mut app, KeyCode::Down);
    paste(&mut app, "a long answer with a visible tail");
    for width in [80, 30, 20] {
        let (visible, cursor) = render(&mut app, width, 16);
        assert!(
            visible.contains("tail"),
            "answer hidden at {width}\n{visible}"
        );
        assert!(
            visible.contains("Esc"),
            "cancel hidden at {width}\n{visible}"
        );
        assert!(cursor.0 < width && cursor.1 < 16);
    }
}

#[test]
fn question_review_keeps_actions_visible_and_submits_the_full_pasted_answer() {
    for transcript in [false, true] {
        let mut app = App::new();
        app.input = "saved draft".into();
        app.cursor = app.input.len();
        if transcript {
            app.push_synthetic_message("waiting for an answer".into());
        }
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.handle_agent_event(mew_agent::AgentEvent::AskUser {
            call_id: "review-long-answer".into(),
            questions: vec![
                mew_agent::AskUserQuestion {
                    prompt: "which surface?".into(),
                    options: vec![],
                },
                mew_agent::AskUserQuestion {
                    prompt: "what next?".into(),
                    options: vec![],
                },
            ],
            tx,
        });
        paste(&mut app, "native browser");
        key(&mut app, KeyCode::Enter);
        let answer = "keep the browser and session together 目標 ".repeat(180);
        paste(&mut app, &answer);
        key(&mut app, KeyCode::Enter);
        assert!(app.user_question.as_ref().unwrap().review);
        assert_eq!(
            app.user_question.as_ref().unwrap().call_id,
            "review-long-answer"
        );
        for (width, height) in [(80, 24), (30, 16), (20, 16)] {
            let (visible, _) = render(&mut app, width, height);
            for label in ["Submit", "Cancel", "Esc", "…"] {
                assert!(
                    visible.contains(label),
                    "{label} hidden at {width}x{height}\n{visible}"
                );
            }
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            rx.try_recv().unwrap(),
            vec!["native browser".to_string(), answer]
        );
        assert_eq!(app.input, "saved draft");
        assert!(app.user_question.is_none());
    }
}
