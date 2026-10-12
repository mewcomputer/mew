use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use mew_tui::{
    app::Mode,
    events::handle_input_event,
    harness::{Backend, Harness},
};

fn key(harness: &mut Harness, code: KeyCode) {
    harness.send_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn help_content_is_reachable_at_narrow_normal_and_wide_sizes() {
    for (width, height) in [(40, 24), (80, 24), (160, 36)] {
        let mut harness = Harness::new(width, height);
        key(&mut harness, KeyCode::Char('?'));
        let mut frames = String::new();
        for _ in 0..100 {
            let frame = harness.render();
            assert!(
                frame.contains("Esc close"),
                "missing pinned hint at {width}x{height}:\n{frame}"
            );
            frames.push_str(&frame);
            key(&mut harness, KeyCode::Down);
        }
        for content in [
            "Global",
            "Input Editing",
            "Tools & Sidebar",
            "Slash Commands",
            "/sessions",
            "list resumable sessions",
        ] {
            assert!(
                frames.contains(content),
                "unreachable {content} at {width}x{height}"
            );
        }
        assert!(!frames.contains("Ctrl+Shift+Zredo"));
        assert!(!frames.contains("Ctrl+Shift+Ccopy"));
    }
}

#[test]
fn help_page_navigation_reaches_both_ends_and_reopening_starts_at_top() {
    let mut harness = Harness::new(80, 24);
    key(&mut harness, KeyCode::Char('?'));
    let top = harness.render();
    key(&mut harness, KeyCode::PageDown);
    let page = harness.render();
    assert_ne!(page, top);
    key(&mut harness, KeyCode::PageUp);
    assert_eq!(harness.render(), top);
    key(&mut harness, KeyCode::End);
    let bottom = harness.render();
    assert!(bottom.contains("/sessions"));
    key(&mut harness, KeyCode::Down);
    assert_eq!(
        harness.render(),
        bottom,
        "scroll must stop at last visible page"
    );
    key(&mut harness, KeyCode::Home);
    assert_eq!(harness.render(), top);
    key(&mut harness, KeyCode::Up);
    assert_eq!(harness.render(), top);
    key(&mut harness, KeyCode::End);
    key(&mut harness, KeyCode::Esc);
    assert_eq!(harness.app.mode, Mode::Normal);
    key(&mut harness, KeyCode::Char('?'));
    assert_eq!(harness.render(), top);
}

#[test]
fn help_scroll_reclamps_after_resize_and_wheel_keeps_chat_position() {
    let mut harness = Harness::new(40, 24);
    harness.app.scroll = 7;
    key(&mut harness, KeyCode::Char('?'));
    harness.render();
    for _ in 0..100 {
        handle_input_event(
            &mut harness.app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 20,
                row: 12,
                modifiers: KeyModifiers::NONE,
            }),
        );
        harness.render();
    }
    assert!(harness.render().contains("/sessions"));
    assert_eq!(harness.app.scroll, 7);
    harness.resize(160, 36);
    let frame = harness.render();
    assert!(
        frame.contains("/sessions"),
        "resize must preserve a reachable bottom page:\n{frame}"
    );
    assert!(frame.contains("Esc close"));
}

#[test]
fn very_narrow_help_keeps_the_close_hint_visible() {
    let mut harness = Harness::new(24, 12);
    key(&mut harness, KeyCode::Char('?'));
    assert!(harness.render().contains("Esc close"));
}
