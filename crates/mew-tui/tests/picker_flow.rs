use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{
    app::PickerItem,
    harness::{Backend, Harness},
};
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn centered_pickers_identify_the_task_and_keep_navigation_visible_while_searching() {
    for (kind, title) in [
        ("command", "Commands"),
        ("model", "Models"),
        ("session", "Conversations"),
        ("project", "New conversation in project"),
        ("theme", "Themes"),
        ("persona", "Personas"),
        ("permission_mode", "Permissions"),
        ("thinking_variant", "Thinking"),
        ("rewind", "Rewind conversation"),
    ] {
        for width in [40, 80] {
            let mut harness = Harness::new(width, 24);
            harness.app.open_command_palette();
            let picker = harness.app.picker.as_mut().unwrap();
            picker.kind = kind.into();
            picker.items = vec![PickerItem {
                id: "first".into(),
                label: "first choice".into(),
                ..Default::default()
            }];
            picker.filter = "choice".into();
            picker.cursor = picker.filter.len();
            picker.hint = Some("special action".into());
            let frame = harness.render();
            for expected in [
                title,
                "choice",
                "first choice",
                "↑↓ move",
                "Enter choose",
                "Esc close",
                "special action",
            ] {
                assert!(
                    frame.contains(expected),
                    "missing {expected} for {kind} at width {width}:\n{frame}"
                );
            }
        }
    }
}

#[test]
fn empty_picker_search_keeps_the_query_and_a_way_out() {
    for width in [40, 80] {
        let mut harness = Harness::new(width, 24);
        harness.app.open_model_picker();
        let picker = harness.app.picker.as_mut().unwrap();
        picker.items = vec![PickerItem {
            id: "first".into(),
            label: "first choice".into(),
            ..Default::default()
        }];
        picker.filter = "missing model".into();
        picker.cursor = picker.filter.len();
        let frame = harness.render();
        for expected in ["Models", "missing model", "No matches", "Esc close"] {
            assert!(
                frame.contains(expected),
                "missing {expected} at width {width}:\n{frame}"
            );
        }
        assert!(!frame.contains("first choice"));
        assert_eq!(harness.app.picker.as_ref().unwrap().filter, "missing model");
    }
}

#[test]
fn empty_unfiltered_picker_explains_the_state() {
    let mut harness = Harness::new(40, 24);
    harness.app.open_session_picker();
    let frame = harness.render();
    assert!(frame.contains("Conversations"), "{frame}");
    assert!(frame.contains("No conversations"), "{frame}");
    assert!(frame.contains("Esc close"), "{frame}");
}

#[test]
fn short_picker_keeps_the_selected_wrapped_row_and_footer_reachable() {
    for (width, height) in [(24, 12), (40, 16), (80, 24)] {
        let mut harness = Harness::new(width, height);
        harness.app.open_model_picker();
        let picker = harness.app.picker.as_mut().unwrap();
        picker.items = (0..20)
            .map(|index| PickerItem {
                id: index.to_string(),
                label: format!("model {index:02}"),
                description: "a wrapped description explaining the model".into(),
                ..Default::default()
            })
            .collect();
        for _ in 0..19 {
            harness.render();
            harness.send_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        let frame = harness.render();
        for expected in [
            "Models", "model 19", "move", "Enter", "choose", "Esc", "close",
        ] {
            assert!(
                frame.contains(expected),
                "missing {expected} at {width}x{height}:\n{frame}"
            );
        }
        assert!(harness.app.picker.as_ref().unwrap().scroll > 0);
    }
}

#[test]
fn tiny_picker_never_places_the_caret_outside_the_terminal() {
    let mut app = mew_tui::App::new();
    app.open_command_palette();
    let mut terminal = Terminal::new(TestBackend::new(4, 12)).unwrap();
    terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut app))
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(cursor.x < 4 && cursor.y < 12, "{cursor:?}");
}
