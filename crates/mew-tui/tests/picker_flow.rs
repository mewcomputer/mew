use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{
    app::PickerItem,
    harness::{Backend, Harness},
};
use ratatui::{backend::TestBackend, Terminal};

fn display_row(buffer: &ratatui::buffer::Buffer, row: u16) -> String {
    let mut text = String::new();
    let mut column = 0;
    while column < buffer.area.width {
        let symbol = buffer[(column, row)].symbol();
        text.push_str(symbol);
        // TestBackend stores diff cells. A wide glyph covers the next cell,
        // which can still contain a glyph from the previous frame.
        column += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
    }
    text
}

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
fn large_picker_uses_available_height_without_crowding_details_and_navigation() {
    let mut harness = Harness::new(80, 40);
    harness.app.open_model_picker();
    harness.app.picker.as_mut().unwrap().items = (0..80)
        .map(|index| PickerItem {
            id: index.to_string(),
            label: format!("model {index:02}"),
            description: format!("details {index:02}: chosen model"),
            ..Default::default()
        })
        .collect();
    let mut previous = 0;
    for height in [24, 40, 60] {
        harness.resize(80, height);
        let frame = harness.render();
        let picker = harness.app.picker.as_ref().unwrap();
        assert!(picker.visible_items > previous, "{frame}");
        assert!(picker.visible_details > 0);
        for expected in ["details 00", "Enter choose", "Esc close", "█", "│"] {
            assert!(frame.contains(expected), "missing {expected}: {frame}");
        }
        previous = picker.visible_items;
    }
    assert!(previous > 30);
    harness.app.picker.as_mut().unwrap().selected = 79;
    let frame = harness.render();
    assert!(
        frame.contains("model 79") && frame.contains("details 79"),
        "{frame}"
    );
    harness.resize(40, 16);
    let frame = harness.render();
    assert!(
        frame.contains("model 79") && frame.contains("Esc close"),
        "{frame}"
    );
    harness.app.picker.as_mut().unwrap().items.truncate(2);
    let frame = harness.render();
    assert_eq!(harness.app.picker.as_ref().unwrap().visible_items, 2);
    assert!(!frame.contains('│'), "{frame}");
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

#[test]
fn picker_choices_stay_compact_and_only_the_selection_shows_details() {
    let mut harness = Harness::new(80, 24);
    harness.app.open_model_picker();
    harness.app.picker.as_mut().unwrap().items = (0..12)
        .map(|index| PickerItem {
            id: index.to_string(),
            label: format!("model {index:02}"),
            description: format!("details {index:02}: {}", "long explanation ".repeat(8)),
            ..Default::default()
        })
        .collect();
    let frame = harness.render();
    assert!(frame.contains("12 choices"), "{frame}");
    for index in 0..8 {
        assert!(frame.contains(&format!("model {index:02}")), "{frame}");
    }
    assert!(frame.contains("details 00"), "{frame}");
    assert!(!frame.contains("details 01"), "{frame}");
    let rows: Vec<_> = frame.lines().collect();
    let positions: Vec<_> = (0..8)
        .map(|index| {
            rows.iter()
                .position(|row| row.contains(&format!("model {index:02}")))
                .unwrap()
        })
        .collect();
    assert!(positions.windows(2).all(|pair| pair[1] == pair[0] + 1));
    harness.send_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let frame = harness.render();
    assert!(frame.contains("details 01"), "{frame}");
    assert!(!frame.contains("details 00"), "{frame}");
}

#[test]
fn undo_and_redo_reset_details_when_query_length_stays_the_same() {
    let mut harness = Harness::new(40, 24);
    harness.app.open_model_picker();
    let picker = harness.app.picker.as_mut().unwrap();
    picker.items = ["foo", "bar"]
        .into_iter()
        .map(|label| PickerItem {
            id: label.into(),
            label: label.into(),
            description: format!("start {label}\n{}", "long description ".repeat(100)),
            ..Default::default()
        })
        .collect();
    picker.filter = "foo".into();
    picker.cursor = 3;
    harness.send_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    harness.type_str("bar");
    harness.render();
    harness.send_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert!(harness.app.picker.as_ref().unwrap().details_scroll > 0);
    harness.send_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    let frame = harness.render();
    assert_eq!(harness.app.picker.as_ref().unwrap().filter, "foo");
    assert!(frame.contains("start foo"), "{frame}");
    harness.send_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    harness.send_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
    let frame = harness.render();
    assert_eq!(harness.app.picker.as_ref().unwrap().filter, "bar");
    assert!(frame.contains("start bar"), "{frame}");
}

#[test]
fn selected_choice_fills_its_row_including_the_trailing_space() {
    let mut app = mew_tui::App::new();
    app.open_model_picker();
    app.picker.as_mut().unwrap().items = vec![PickerItem {
        id: "first".into(),
        label: "first choice".into(),
        ..Default::default()
    }];
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let (x, y) = (0..24)
        .find_map(|y| {
            (0..79).find_map(|x| {
                (buffer[(x, y)].symbol() == "f" && buffer[(x + 1, y)].symbol() == "i")
                    .then_some((x, y))
            })
        })
        .unwrap();
    let selection = app.theme.resolve("selection.background");
    assert_eq!(buffer[(x, y)].bg, selection);
    assert_eq!(
        buffer[(x + 20, y)].bg,
        selection,
        "trailing row padding must remain selected"
    );
}

#[test]
fn unicode_filter_scrolls_with_its_caret_in_both_directions() {
    let mut app = mew_tui::App::new();
    app.open_model_picker();
    let query = "日本語のとても長い検索条件abcdefghijk終端";
    let picker = app.picker.as_mut().unwrap();
    picker.filter = query.into();
    picker.cursor = query.len();
    let mut terminal = Terminal::new(TestBackend::new(24, 12)).unwrap();
    terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut app))
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    let buffer = terminal.backend().buffer();
    let row = display_row(buffer, cursor.y);
    assert!(
        row.replace(' ', "").contains("終端"),
        "query tail must remain visible: {row}"
    );
    assert!(cursor.x < 24);
    mew_tui::events::handle_key_event(&mut app, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut app))
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    let buffer = terminal.backend().buffer();
    let row = display_row(buffer, cursor.y);
    assert!(
        row.replace(' ', "").contains("日本語"),
        "query start must return with the caret: {row}"
    );
    assert_eq!(buffer[(cursor.x, cursor.y)].symbol(), "日");
}

#[test]
fn query_viewport_keeps_emoji_sequences_whole_and_the_tail_visible() {
    let mut app = mew_tui::App::new();
    app.open_model_picker();
    let picker = app.picker.as_mut().unwrap();
    picker.filter = format!("{}終端", "👩‍💻".repeat(10));
    picker.cursor = picker.filter.len();
    let mut terminal = Terminal::new(TestBackend::new(24, 12)).unwrap();
    terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut app))
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    let buffer = terminal.backend().buffer();
    let row = display_row(buffer, cursor.y);
    assert!(row.replace(' ', "").contains("終端"), "{row}");
    assert!(row.contains("👩‍💻"), "{row}");
    assert_eq!(buffer[(cursor.x, cursor.y)].symbol(), " ");
}

#[test]
fn long_choice_details_are_reachable_without_moving_the_choice_or_footer() {
    let mut harness = Harness::new(80, 24);
    harness.app.open_model_picker();
    harness.app.picker.as_mut().unwrap().items = vec![
        PickerItem {
            id: "first".into(),
            label: "first choice".into(),
            description: (0..20)
                .map(|i| format!("detail line {i:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            ..Default::default()
        },
        PickerItem {
            id: "second".into(),
            label: "second choice".into(),
            description: "second description".into(),
            ..Default::default()
        },
    ];
    let frame = harness.render();
    assert!(frame.contains("PgUp/PgDn details"), "{frame}");
    assert!(frame.contains("detail line 00"), "{frame}");
    assert!(!frame.contains("detail line 19"), "{frame}");
    for _ in 0..20 {
        harness.send_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    }
    let frame = harness.render();
    for expected in [
        "first choice",
        "second choice",
        "detail line 19",
        "Esc close",
    ] {
        assert!(frame.contains(expected), "{frame}");
    }
    assert_eq!(
        harness
            .app
            .picker
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .id,
        "first"
    );
    harness.send_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    harness.send_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert!(harness.render().contains("detail line 00"));
}

#[test]
fn short_picker_can_read_every_detail_line_with_its_actions_visible() {
    let mut harness = Harness::new(24, 12);
    harness.app.open_model_picker();
    harness.app.picker.as_mut().unwrap().items = (0..10)
        .map(|i| PickerItem {
            id: i.to_string(),
            label: format!("model {i:02}"),
            description: "detail zero\ndetail one\ndetail two\ndetail three".into(),
            ..Default::default()
        })
        .collect();
    for expected in ["detail zero", "detail one", "detail two", "detail three"] {
        let frame = harness.render();
        assert!(frame.contains(expected), "{frame}");
        for action in ["Enter", "choose", "Esc", "close", "thinking", "variants"] {
            assert!(frame.contains(action), "{frame}");
        }
        harness.send_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    }
    assert_eq!(harness.app.picker.as_ref().unwrap().selected, 0);
}

#[test]
fn budget_track_fits_the_popup_and_hits_both_range_ends_after_resizing() {
    use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
    for width in [24, 40, 80] {
        let mut harness = Harness::new(width, 16);
        harness.app.status.model = "budget-model".into();
        harness.app.thinking_budget.insert(
            "budget-model".into(),
            mew_protocol::ThinkingBudgetInfo {
                min: 0,
                max: 10000,
                step: 100,
                default: 5000,
                by_effort: vec![],
            },
        );
        harness.app.open_thinking_variant_picker();
        harness.app.picker_down();
        harness.render();
        let rect = harness
            .app
            .picker
            .as_ref()
            .unwrap()
            .budget
            .as_ref()
            .unwrap()
            .track_rect
            .unwrap();
        assert!(rect.right() < width, "{rect:?}");
        for (column, expected) in [(rect.x, 0), (rect.right() - 1, 10000)] {
            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Left),
            ] {
                mew_tui::events::handle_input_event(
                    &mut harness.app,
                    Event::Mouse(MouseEvent {
                        kind,
                        column,
                        row: rect.y,
                        modifiers: KeyModifiers::NONE,
                    }),
                );
            }
            assert_eq!(
                harness
                    .app
                    .picker
                    .as_ref()
                    .unwrap()
                    .budget
                    .as_ref()
                    .unwrap()
                    .snapped(),
                expected
            );
        }
        harness.app.picker.as_mut().unwrap().filter = "off".into();
        harness.app.picker.as_mut().unwrap().cursor = 3;
        harness.render();
        assert!(harness
            .app
            .picker
            .as_ref()
            .unwrap()
            .budget
            .as_ref()
            .unwrap()
            .track_rect
            .is_none());
    }
}

#[test]
fn very_short_and_narrow_pickers_stay_inside_the_frame() {
    for (width, height) in [(1, 1), (4, 4), (10, 6), (18, 10), (24, 12)] {
        let mut app = mew_tui::App::new();
        app.open_command_palette();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| mew_tui::ui::draw(frame, &mut app))
            .unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        assert!(
            cursor.x < width && cursor.y < height,
            "{width}x{height}: {cursor:?}"
        );
    }
}
