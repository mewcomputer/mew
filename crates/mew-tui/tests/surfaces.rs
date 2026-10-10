use mew_tui::{app::*, harness::Harness, theme::Theme, ui};
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn surface_theme() -> Theme {
    Theme::from_json_str(
        r##"{"tokens": {
        "chat.surface.background": "#112233",
        "panel.overlay": "#445566"
    }}"##,
    )
    .unwrap()
}

#[test]
fn conversation_and_tool_edges_keep_an_opaque_canvas() {
    for (width, height) in [(40, 24), (80, 30), (160, 36)] {
        let mut harness = Harness::new(width, height);
        harness.app.theme = surface_theme();
        harness.say("text before a tool");
        harness.say_tool_call("bash", "first line\nsecond line", None);
        let buffer = render(&mut harness.app, width, height);
        let chat = harness.app.chat_area;
        let mut edges = 0;
        for y in chat.y..chat.bottom() {
            for x in chat.x..chat.right() {
                let cell = &buffer[(x, y)];
                assert_ne!(cell.bg, Color::Reset, "unpainted cell at {x},{y}");
                if matches!(cell.symbol(), "▄" | "▀") {
                    edges += 1;
                    assert_eq!(cell.bg, Color::Rgb(17, 34, 51));
                }
            }
        }
        assert!(edges > 0, "fixture must exercise tool edges");
    }
}

#[test]
fn modal_padding_uses_the_raised_surface() {
    for mode in [
        Mode::PermissionPrompt,
        Mode::CommandPalette,
        Mode::PersonaSwitchConfirm,
        Mode::GoalProposal,
        Mode::GoalCompose,
        Mode::Help,
    ] {
        let mut app = App::new();
        app.theme = surface_theme();
        app.push_synthetic_message("conversation behind the dialog".into());
        app.permission = Some(PermissionState {
            tool_name: "bash".into(),
            call_id: "call".into(),
            input: serde_json::json!({"command": "pwd"}),
            tx: None,
            selected: 0,
            scroll: 0,
            view: Default::default(),
        });
        app.open_command_palette();
        app.persona_switch_confirm = Some(PersonaSwitchConfirmState {
            target: PersonaSummary {
                name: "builder".into(),
                ..Default::default()
            },
            current: None,
            selected: 0,
            scroll: 0,
            view: Default::default(),
        });
        app.goal_proposal = Some(GoalProposalState {
            call_id: "goal".into(),
            objective: "finish the task".into(),
            selected: 0,
            tx: None,
            scroll: 0,
            view: Default::default(),
        });
        app.goal_compose = Some(GoalComposeState {
            title: "finish the task".into(),
            cursor: 0,
            undo: Default::default(),
            original: "finish the task".into(),
            replacing: None,
        });
        app.mode = mode;
        let buffer = render(&mut app, 80, 30);
        // Left padding beside the dialog's content is part of its surface,
        // including borderless permission and picker dialogs.
        let x = match mode {
            Mode::PermissionPrompt | Mode::CommandPalette => 10,
            Mode::PersonaSwitchConfirm => 8,
            Mode::GoalProposal | Mode::GoalCompose => 5,
            Mode::Help => 8,
            _ => unreachable!(),
        };
        assert_eq!(buffer[(x, 15)].bg, Color::Rgb(68, 85, 102), "{mode:?}");
        assert_eq!(
            buffer[(0, 0)].bg,
            Color::Rgb(17, 34, 51),
            "backdrop for {mode:?}"
        );
    }
}

#[test]
fn inline_autocomplete_has_a_distinct_opaque_surface() {
    let mut app = App::new();
    app.theme = surface_theme();
    app.input = "/".into();
    app.cursor = 1;
    app.mode = Mode::SlashCommand;
    let buffer = render(&mut app, 80, 30);
    let bottom = app.input_area.y - 1;
    assert_eq!(
        buffer[(app.input_area.x, bottom)].bg,
        Color::Rgb(68, 85, 102)
    );
}

#[test]
fn builtin_popovers_are_distinct_from_the_canvas() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../resources/theme_manifest.json")).unwrap();
    for name in manifest["themes"].as_object().unwrap().keys() {
        let theme = Theme::load(name);
        assert_ne!(
            theme.resolve("popover"),
            theme.resolve("background"),
            "{name}"
        );
    }
}

#[test]
fn question_slot_and_toast_padding_are_opaque() {
    let mut app = App::new();
    app.theme = surface_theme();
    app.push_synthetic_message("conversation".into());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.handle_agent_event(mew_agent::AgentEvent::AskUser {
        call_id: "question".into(),
        questions: vec![mew_agent::AskUserQuestion {
            prompt: "which branch?".into(),
            options: vec![],
        }],
        tx,
    });
    let buffer = render(&mut app, 80, 30);
    assert_eq!(
        buffer[(app.input_area.x, app.input_area.y)].bg,
        Color::Rgb(68, 85, 102)
    );

    app.mode = Mode::Normal;
    app.toasts.push(("done".into(), std::time::Instant::now()));
    app.alert = Some(("done".into(), std::time::Instant::now()));
    let buffer = render(&mut app, 80, 30);
    for cell in &buffer.content {
        assert_ne!(
            cell.bg,
            Color::Reset,
            "toast and alert padding must be painted"
        );
    }
}
