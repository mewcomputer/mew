use mew_tui::{app::*, harness::Harness, theme::Theme, ui};
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let frame = terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    frame.buffer.clone()
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
fn user_messages_have_an_opaque_surface_without_prompt_markers() {
    for width in [24, 80] {
        let mut app = App::new();
        app.theme = Theme::from_json_str(
            r##"{"tokens":{"muted":"#224455","chat.surface.background":"#112233"}}"##,
        )
        .unwrap();
        app.push_user(
            "first line\n\n日本語 and a longer line that wraps across the block".into(),
            vec![],
        );
        app.push_synthetic_message("assistant reply".into());
        let buffer = render(&mut app, width, 30);
        let chat = app.chat_content_area;
        let user_rows: Vec<_> = (chat.y..chat.bottom())
            .take_while(|&y| {
                !(chat.x..chat.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains("assistant")
            })
            .collect();
        let separator = *user_rows.last().unwrap();
        for &y in &user_rows[..user_rows.len() - 1] {
            assert_ne!(buffer[(chat.x, y)].symbol(), ">");
            for x in chat.x..chat.right() {
                assert_eq!(
                    buffer[(x, y)].bg,
                    Color::Rgb(34, 68, 85),
                    "user row {y}, col {x}, width {width}"
                );
            }
        }
        assert!(user_rows.len() >= 4);
        assert_eq!(buffer[(chat.x, separator)].bg, Color::Rgb(17, 34, 51));
        assert_eq!(buffer[(chat.x, separator + 1)].bg, Color::Rgb(17, 34, 51));
    }
}

#[test]
fn composer_uses_the_user_surface_without_a_prompt_marker() {
    let mut app = App::new();
    app.push_synthetic_message("assistant reply".into());
    app.input = "first line\nsecond line".into();
    app.cursor = app.input.len();
    let buffer = render(&mut app, 80, 24);
    for y in app.input_area.y..app.input_area.bottom() {
        for x in app.input_area.x..app.input_area.right() {
            assert_ne!(buffer[(x, y)].symbol(), ">");
            assert_eq!(buffer[(x, y)].bg, app.theme.resolve("muted"));
        }
    }
}

#[test]
fn selecting_user_text_keeps_the_selection_and_surrounding_surface() {
    let mut app = App::new();
    app.push_user("漢字 user request".into(), vec![]);
    app.sel_anchor_row = Some(0);
    app.sel_end_row = Some(0);
    app.sel_anchor_col = Some(2);
    app.sel_end_col = Some(6);
    app.mark_chat_dirty();
    let buffer = render(&mut app, 80, 24);
    let chat = app.chat_content_area;
    assert_eq!(
        buffer[(chat.x + 2, chat.y)].bg,
        app.theme.resolve("selection.background")
    );
    assert_eq!(
        buffer[(chat.x + 3, chat.y)].bg,
        app.theme.resolve("selection.background")
    );
    assert_eq!(buffer[(chat.x + 7, chat.y)].bg, app.theme.resolve("muted"));
    assert_eq!(
        buffer[(chat.right() - 1, chat.y)].bg,
        app.theme.resolve("muted")
    );
}

#[test]
fn user_attachments_and_guidance_share_the_user_surface() {
    let mut app = App::new();
    let image = serde_json::from_value(serde_json::json!({
        "type":"file", "id":"01ARZ3NDEKTSV4RRFFQ69G5FA1",
        "message_id":"01ARZ3NDEKTSV4RRFFQ69G5FA2", "session_id":"01ARZ3NDEKTSV4RRFFQ69G5FA3",
        "mime":"image/png", "filename":"reference.png", "url":"file:///tmp/reference.png"
    }))
    .unwrap();
    app.push_user("> literal quote".into(), vec![image]);
    app.push_guidance("steer this turn".into());
    let buffer = render(&mut app, 80, 24);
    let chat = app.chat_content_area;
    assert_eq!(buffer[(chat.x + 2, chat.y)].symbol(), ">");
    assert_eq!(buffer[(chat.x, chat.y + 3)].symbol(), "→");
    for y in [chat.y, chat.y + 1, chat.y + 3] {
        for x in chat.x..chat.right() {
            assert_eq!(buffer[(x, y)].bg, app.theme.resolve("chat.user.background"));
        }
    }
}

#[test]
fn builtin_user_surfaces_are_distinct_and_readable() {
    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("expected RGB theme color");
        };
        [r, g, b]
            .into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(channel, weight)| {
                let value = f64::from(channel) / 255.0;
                weight
                    * if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../resources/theme_manifest.json")).unwrap();
    for name in manifest["themes"].as_object().unwrap().keys() {
        let theme = Theme::load(name);
        let user_bg = theme.resolve("chat.user.background");
        assert_ne!(user_bg, theme.resolve("chat.surface.background"), "{name}");
        let foreground = luminance(theme.resolve("foreground"));
        let background = luminance(user_bg);
        let contrast = (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05);
        assert!(contrast >= 4.5, "{name}: {contrast}");
    }
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
