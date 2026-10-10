use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_agent::{AgentEvent, AskUserQuestion, Todo, TodoStatus};
use mew_tui::{
    app::{Mode, PickerItem},
    events::{handle_key_event, Action},
    App,
};

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    handle_key_event(app, KeyEvent::new(code, modifiers))
}

fn ctrl(app: &mut App, c: char) -> Option<Action> {
    key(app, KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn composer(text: &str) -> App {
    let mut app = App::new();
    app.input = text.into();
    app.cursor = text.len();
    app
}

fn inline_reference(text: &str) -> App {
    let mut app = composer(text);
    app.open_at_picker();
    app.picker.as_mut().unwrap().items = vec![PickerItem {
        id: "file.rs".into(),
        label: "@file.rs".into(),
        ..Default::default()
    }];
    app
}

fn session_picker() -> App {
    let mut app = App::new();
    app.daemon_mode = true;
    app.daemon_sessions = ["first", "second", "third"]
        .into_iter()
        .map(|id| {
            serde_json::from_value(serde_json::json!({
                "session_id": id, "state": "idle", "created_at": 10,
                "client_count": 0, "summary": format!("work {id}")
            }))
            .unwrap()
        })
        .collect();
    app.open_session_picker();
    app
}

#[test]
fn ctrl_w_handles_multibyte_whitespace_in_composer_and_reference_input() {
    for inline in [false, true] {
        let mut app = if inline {
            inline_reference("@one\u{3000}two")
        } else {
            composer("one\u{3000}two")
        };
        assert!(ctrl(&mut app, 'w').is_none());
        assert_eq!(
            app.input,
            if inline {
                "@one\u{3000}"
            } else {
                "one\u{3000}"
            }
        );
        assert_eq!(app.cursor, app.input.len());
        assert!(ctrl(&mut app, 'y').is_none());
        assert_eq!(
            app.input,
            if inline {
                "@one\u{3000}two"
            } else {
                "one\u{3000}two"
            }
        );
    }
}

#[test]
fn shifted_at_opens_the_reference_picker_after_inserting_the_character() {
    let mut app = composer("");
    key(&mut app, KeyCode::Char('@'), KeyModifiers::SHIFT);
    assert_eq!(app.input, "@");
    assert_eq!(app.picker.as_ref().unwrap().kind, "at");
}

#[test]
fn slash_completion_mode_changes_do_not_split_consecutive_composer_kills() {
    let mut app = composer("/foo\nbar");
    app.mode = Mode::SlashCommand;
    app.cursor = 4;
    ctrl(&mut app, 'u');
    ctrl(&mut app, 'k');
    ctrl(&mut app, 'y');
    assert_eq!(app.input, "/foo\nbar");
}

#[test]
fn composer_line_kills_preserve_other_lines_and_yank_exact_unicode_text() {
    let original = "first\nkeep 界 tail\nlast";
    let mut app = composer(original);
    app.cursor = original.find(" 界").unwrap();
    ctrl(&mut app, 'k');
    assert_eq!(app.input, "first\nkeep\nlast");
    ctrl(&mut app, 'y');
    assert_eq!(app.input, original);

    app.cursor = original.find(" 界").unwrap();
    ctrl(&mut app, 'u');
    assert_eq!(app.input, "first\n 界 tail\nlast");
    ctrl(&mut app, 'y');
    assert_eq!(app.input, original);
}

#[test]
fn kill_at_line_end_removes_the_newline_and_yank_restores_it() {
    let mut app = composer("first\nsecond");
    app.cursor = "first".len();
    ctrl(&mut app, 'k');
    assert_eq!(app.input, "firstsecond");
    ctrl(&mut app, 'y');
    assert_eq!(app.input, "first\nsecond");
}

#[test]
fn alt_y_replaces_the_last_yank_with_the_previous_kill() {
    let mut app = composer("alpha");
    ctrl(&mut app, 'u');
    for c in "beta".chars() {
        key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    ctrl(&mut app, 'u');
    ctrl(&mut app, 'y');
    assert_eq!(app.input, "beta");
    key(&mut app, KeyCode::Char('y'), KeyModifiers::ALT);
    assert_eq!(app.input, "alpha");
    assert_eq!(app.cursor, "alpha".len());
}

#[test]
fn inline_ctrl_d_edits_the_reference_token_and_resynchronizes_the_filter() {
    let mut app = inline_reference("prefix @file");
    app.cursor = "prefix @fi".len();
    assert!(ctrl(&mut app, 'd').is_none());
    assert_eq!(app.input, "prefix @fie");
    assert_eq!(app.cursor, "prefix @fi".len());
    assert_eq!(app.picker.as_ref().unwrap().filter, "fi");
    key(&mut app, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().filter, "fie");
}

#[test]
fn unknown_control_and_alt_chords_never_insert_literal_characters() {
    for mode in [Mode::Normal, Mode::SlashCommand, Mode::CommandPalette] {
        for (c, modifiers) in [
            ('q', KeyModifiers::CONTROL),
            ('q', KeyModifiers::ALT),
            ('q', KeyModifiers::CONTROL | KeyModifiers::ALT),
            ('q', KeyModifiers::META),
            ('q', KeyModifiers::SUPER),
        ] {
            let mut app = match mode {
                Mode::CommandPalette => inline_reference("@file"),
                Mode::SlashCommand => composer("/model"),
                _ => composer("saved draft"),
            };
            app.mode = mode;
            let original = app.input.clone();
            assert!(key(&mut app, KeyCode::Char(c), modifiers).is_none());
            assert_eq!(app.input, original, "{mode:?}: {modifiers:?}");
            assert_eq!(app.mode, mode);
        }
    }
}

#[test]
fn shift_and_alt_enter_insert_composer_newlines_without_submitting() {
    for modifiers in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
        for slash in [false, true] {
            let mut app = composer(if slash { "/model" } else { "hello界" });
            app.mode = if slash {
                Mode::SlashCommand
            } else {
                Mode::Normal
            };
            app.cursor = 1;
            let original = app.input.clone();
            assert!(key(&mut app, KeyCode::Enter, modifiers).is_none());
            assert_eq!(app.input, format!("{}\n{}", &original[..1], &original[1..]));
            assert_eq!(app.cursor, 2);
            assert!(app.history.is_empty());
        }
    }
}

#[test]
fn modified_enter_in_reference_picker_never_chooses_the_highlighted_mention() {
    for modifiers in [
        KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL,
        KeyModifiers::META,
    ] {
        let mut app = inline_reference("@file");
        assert!(key(&mut app, KeyCode::Enter, modifiers).is_none());
        if matches!(modifiers, KeyModifiers::SHIFT | KeyModifiers::ALT) {
            assert_eq!(app.input, "@file\n");
        } else {
            assert_eq!(app.input, "@file");
        }
    }
}

#[test]
fn composer_ctrl_p_n_restore_history_and_the_unsent_draft() {
    let mut app = composer("draft");
    app.history = vec!["old".into(), "recent".into()];
    ctrl(&mut app, 'p');
    assert_eq!(app.input, "recent");
    ctrl(&mut app, 'p');
    assert_eq!(app.input, "old");
    ctrl(&mut app, 'n');
    assert_eq!(app.input, "recent");
    ctrl(&mut app, 'n');
    assert_eq!(app.input, "draft");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.picker.is_none());
}

#[test]
fn ctrl_t_transposes_graphemes_instead_of_opening_an_application_action() {
    let mut app = composer("a界");
    assert!(ctrl(&mut app, 't').is_none());
    assert_eq!(app.input, "界a");
    assert_eq!(app.cursor, app.input.len());
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn palette_and_attention_have_explicit_control_shortcuts() {
    for code in [KeyCode::Char(' '), KeyCode::Null] {
        let mut app = composer("draft");
        assert!(key(&mut app, code, KeyModifiers::CONTROL).is_none());
        assert_eq!(app.mode, Mode::CommandPalette);
        assert_eq!(app.picker.as_ref().unwrap().kind, "command");
        assert_eq!(app.input, "draft");
    }
    let mut app = composer("draft");
    assert!(key(
        &mut app,
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::ALT
    )
    .is_none());
    assert_eq!(app.picker.as_ref().unwrap().kind, "session_attention");
    assert_eq!(app.input, "draft");
}

#[test]
fn modified_digits_do_not_replace_the_thinking_budget() {
    let mut app = composer("saved draft");
    app.status.model = "budget-model".into();
    app.thinking_budget.insert(
        "budget-model".into(),
        mew_protocol::ThinkingBudgetInfo {
            min: 0,
            max: 10000,
            step: 100,
            default: 5000,
            by_effort: vec![],
        },
    );
    app.open_thinking_variant_picker();
    app.picker_down();
    for modifiers in [KeyModifiers::ALT, KeyModifiers::CONTROL, KeyModifiers::META] {
        key(&mut app, KeyCode::Char('3'), modifiers);
        assert_eq!(
            app.picker.as_ref().unwrap().budget.as_ref().unwrap().draft,
            "5000"
        );
    }
    assert_eq!(app.input, "saved draft");
}

#[test]
fn session_filter_ctrl_a_moves_the_caret_and_ctrl_alt_a_archives() {
    let mut app = session_picker();
    for c in "work".chars() {
        key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    assert!(ctrl(&mut app, 'a').is_none());
    assert_eq!(app.picker.as_ref().unwrap().cursor, 0);
    assert_eq!(app.picker.as_ref().unwrap().filter, "work");
    let id = app
        .picker
        .as_ref()
        .unwrap()
        .selected_item()
        .unwrap()
        .id
        .clone();
    assert!(matches!(
        key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::ALT),
        Some(Action::ToggleSessionArchived(selected)) if selected == id
    ));
}

#[test]
fn session_ctrl_p_n_navigate_rows_and_ctrl_alt_p_pins() {
    let mut app = session_picker();
    assert!(ctrl(&mut app, 'n').is_none());
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    assert!(ctrl(&mut app, 'p').is_none());
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    let id = app
        .picker
        .as_ref()
        .unwrap()
        .selected_item()
        .unwrap()
        .id
        .clone();
    assert!(matches!(
        key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL | KeyModifiers::ALT),
        Some(Action::ToggleSessionPinned(selected)) if selected == id
    ));
}

#[test]
fn goal_kill_and_yank_share_the_composer_kill_ring_without_changing_chat() {
    let mut app = composer("carry界");
    ctrl(&mut app, 'u');
    app.open_goal_compose("goal ".into());
    ctrl(&mut app, 'y');
    assert_eq!(app.goal_compose.as_ref().unwrap().title, "goal carry界");
    ctrl(&mut app, 'w');
    assert_eq!(app.goal_compose.as_ref().unwrap().title, "goal ");
    ctrl(&mut app, 'y');
    assert_eq!(app.goal_compose.as_ref().unwrap().title, "goal carry界");
    assert_eq!(app.input, "");
}

#[test]
fn changing_fields_starts_a_new_kill_entry() {
    let mut app = composer("composer");
    ctrl(&mut app, 'u');
    app.open_goal_compose("goal".into());
    ctrl(&mut app, 'u');
    ctrl(&mut app, 'y');
    assert_eq!(app.goal_compose.as_ref().unwrap().title, "goal");
    key(&mut app, KeyCode::Char('y'), KeyModifiers::ALT);
    assert_eq!(app.goal_compose.as_ref().unwrap().title, "composer");
}

#[test]
fn question_freeform_kill_and_yank_preserve_the_question_and_hidden_chat() {
    let mut app = composer("saved draft");
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "question".into(),
        questions: vec![AskUserQuestion {
            prompt: "answer?".into(),
            options: vec![],
        }],
        tx,
    });
    let question = app.user_question.as_mut().unwrap();
    question.freeform_text = "answer 界".into();
    question.freeform_cursor = question.freeform_text.len();
    ctrl(&mut app, 'w');
    assert_eq!(app.user_question.as_ref().unwrap().freeform_text, "answer ");
    ctrl(&mut app, 'y');
    assert_eq!(
        app.user_question.as_ref().unwrap().freeform_text,
        "answer 界"
    );
    assert_eq!(app.mode, Mode::UserQuestion);
    assert_eq!(app.input, "saved draft");
    assert_eq!(
        rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    );
}

#[test]
fn todo_kill_and_yank_only_edit_the_unsaved_draft() {
    let mut app = composer("saved draft");
    app.status.session_id = "session".into();
    app.todos = vec![Todo {
        id: 1,
        content: "task 界".into(),
        status: TodoStatus::Pending,
        depends_on: vec![],
    }];
    app.open_todos(Some(1));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ctrl(&mut app, 'w');
    assert_eq!(
        app.todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .content,
        "task "
    );
    ctrl(&mut app, 'y');
    assert_eq!(
        app.todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .content,
        "task 界"
    );
    assert_eq!(app.todos[0].content, "task 界");
    assert_eq!(app.input, "saved draft");
}

#[test]
fn history_search_kill_and_yank_only_edit_the_search_query() {
    let mut app = composer("saved draft");
    app.history = vec!["query 界".into()];
    app.start_history_search();
    app.history_search_query = "query 界".into();
    app.history_search_cursor = app.history_search_query.len();
    ctrl(&mut app, 'w');
    assert_eq!(app.history_search_query, "query ");
    ctrl(&mut app, 'y');
    assert_eq!(app.history_search_query, "query 界");
    assert_eq!(app.mode, Mode::HistorySearch);
    assert_eq!(app.input, "saved draft");
}

#[test]
fn history_search_caret_movement_keeps_the_selected_match() {
    let mut app = composer("saved draft");
    app.history = vec!["query old".into(), "query recent".into()];
    app.start_history_search();
    app.history_search_query = "query".into();
    app.history_search_cursor = app.history_search_query.len();
    app.history_search_index = Some(1);
    ctrl(&mut app, 'b');
    assert_eq!(app.history_search_cursor, "quer".len());
    assert_eq!(app.history_search_query, "query");
    assert_eq!(
        app.history_search_current_match().as_deref(),
        Some("query old")
    );
}

#[test]
fn reopening_history_search_resets_its_caret_and_query_undo() {
    let mut app = composer("saved draft");
    ctrl(&mut app, 'r');
    for c in "界query".chars() {
        key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    ctrl(&mut app, 'r');
    assert_eq!(app.history_search_cursor, 0);
    ctrl(&mut app, 'z');
    assert_eq!(app.history_search_query, "");
    key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(app.history_search_query, "x");
}

#[test]
fn todo_control_j_inserts_a_newline_in_the_unsaved_draft() {
    let mut app = composer("saved draft");
    app.todos = vec![Todo {
        id: 1,
        content: "task".into(),
        status: TodoStatus::Pending,
        depends_on: vec![],
    }];
    app.open_todos(Some(1));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(ctrl(&mut app, 'j').is_none());
    assert_eq!(
        app.todo_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .content,
        "task\n"
    );
    assert_eq!(app.todos[0].content, "task");
}

#[test]
fn modified_enter_does_not_accept_a_goal_or_choose_a_centered_picker_item() {
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::META] {
        let mut app = composer("hidden draft");
        app.open_goal_compose("goal".into());
        assert!(key(&mut app, KeyCode::Enter, modifiers).is_none());
        assert_eq!(app.mode, Mode::GoalCompose);
        assert_eq!(app.goal_compose.as_ref().unwrap().title, "goal");
        assert_eq!(app.input, "hidden draft");
    }
    for modifiers in [
        KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL,
        KeyModifiers::META,
    ] {
        let mut app = session_picker();
        assert!(key(&mut app, KeyCode::Enter, modifiers).is_none());
        assert_eq!(app.mode, Mode::CommandPalette);
        assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    }
}

#[test]
fn modified_enter_does_not_accept_the_history_search_match() {
    for modifiers in [
        KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL,
        KeyModifiers::META,
    ] {
        let mut app = composer("hidden draft");
        app.history = vec!["match".into()];
        app.start_history_search();
        app.history_search_query = "match".into();
        app.history_search_cursor = "match".len();
        app.history_search_index = Some(0);
        assert!(key(&mut app, KeyCode::Enter, modifiers).is_none());
        assert_eq!(app.mode, Mode::HistorySearch);
        assert_eq!(app.input, "hidden draft");
        assert_eq!(app.history_search_current_match().as_deref(), Some("match"));
    }
}

#[test]
fn goal_shift_and_alt_enter_insert_a_newline_and_keep_the_hidden_draft() {
    for modifiers in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
        let mut app = composer("hidden draft");
        app.open_goal_compose("goal界".into());
        app.goal_compose.as_mut().unwrap().cursor = "goal".len();
        assert!(key(&mut app, KeyCode::Enter, modifiers).is_none());
        assert_eq!(app.mode, Mode::GoalCompose);
        assert_eq!(app.goal_compose.as_ref().unwrap().title, "goal\n界");
        assert_eq!(app.goal_compose.as_ref().unwrap().cursor, "goal\n".len());
        assert_eq!(app.input, "hidden draft");
    }
}

#[test]
fn question_modified_enter_keeps_the_answer_editable_without_advancing_review() {
    for modifiers in [
        KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL,
        KeyModifiers::META,
    ] {
        let mut app = composer("hidden draft");
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "question".into(),
            questions: vec![AskUserQuestion {
                prompt: "answer?".into(),
                options: vec![],
            }],
            tx,
        });
        let question = app.user_question.as_mut().unwrap();
        question.freeform_text = "answer界".into();
        question.freeform_cursor = "answer".len();
        assert!(key(&mut app, KeyCode::Enter, modifiers).is_none());
        let question = app.user_question.as_ref().unwrap();
        assert!(!question.review);
        assert_eq!(
            question.freeform_text,
            if matches!(modifiers, KeyModifiers::SHIFT | KeyModifiers::ALT) {
                "answer\n界"
            } else {
                "answer界"
            }
        );
        assert_eq!(app.input, "hidden draft");
        assert_eq!(
            rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        );
    }
}

#[test]
fn canceling_a_goal_keeps_its_killed_text_available_in_the_composer() {
    let mut app = composer("draft ");
    app.open_goal_compose("discard 界".into());
    ctrl(&mut app, 'w');
    assert_eq!(app.goal_compose.as_ref().unwrap().title, "discard ");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.input, "draft ");
    ctrl(&mut app, 'y');
    assert_eq!(app.input, "draft 界");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn empty_streaming_ctrl_d_does_not_quit_or_cancel() {
    let mut app = composer("");
    app.streaming = true;
    assert!(ctrl(&mut app, 'd').is_none());
    assert!(!app.should_quit);
    assert!(app.streaming);
}

#[test]
fn empty_ctrl_x_does_not_cancel_the_active_child() {
    let mut app = composer("");
    app.handle_agent_event(AgentEvent::SubagentStart {
        parent_call_id: "agent".into(),
        name: "researcher".into(),
        child_session_id: "child".into(),
        display_name: None,
    });
    assert!(ctrl(&mut app, 'x').is_none());
    assert_eq!(app.most_recent_running_subagent().as_deref(), Some("agent"));
    assert_eq!(app.input, "");
}
