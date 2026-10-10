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

