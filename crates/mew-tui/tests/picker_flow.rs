use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
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

fn project_conversation(id: &str, path: &std::path::Path) -> mew_protocol::SessionInfo {
    serde_json::from_value(serde_json::json!({
        "session_id": id, "state": "idle", "created_at": 10, "client_count": 0,
        "summary": format!("work in {id}"), "cwd": path,
        "last_message_at": 1_728_556_200_000_i64
    }))
    .unwrap()
}

#[test]
fn conversation_rows_show_each_project_and_both_lines_choose_their_owner() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut harness = Harness::new(80, 40);
    harness.app.daemon_sessions = vec![
        project_conversation("alpha", &home.join("code/alpha")),
        project_conversation("beta", &home.join("code/beta")),
        project_conversation("outside", std::path::Path::new("/tmp/outside")),
    ];
    harness.app.input = "draft across projects".into();
    harness.app.cursor = harness.app.input.len();
    harness.app.open_session_picker();
    let frame = harness.render();
    assert_eq!(harness.app.picker_mouse.rows.len(), 6, "{frame}");
    let list = harness.app.picker_mouse.list;
    let lines: Vec<_> = frame.lines().collect();
    for (index, path) in ["~/code/alpha", "~/code/beta", "/tmp/outside"]
        .into_iter()
        .enumerate()
    {
        let y = list.y + index as u16 * 2;
        assert!(lines[y as usize + 1].contains(path), "{frame}");
        assert!(lines[y as usize + 1].contains("10-10 10:30"), "{frame}");
        assert!(harness.app.picker_mouse.rows.contains(&(y + 1, index)));
    }
    for (index, id) in ["alpha", "beta", "outside"].into_iter().enumerate() {
        for (offset, column) in [(0, list.x + 2), (1, list.x + 2), (1, list.right() - 1)] {
            harness.app.open_session_picker();
            harness.render();
            let list = harness.app.picker_mouse.list;
            let action = mew_tui::events::handle_input_event(
                &mut harness.app,
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row: list.y + index as u16 * 2 + offset,
                    modifiers: KeyModifiers::NONE,
                }),
            );
            assert!(action.is_none());
            assert_eq!(harness.app.picker.as_ref().unwrap().selected, index);
            let action = mew_tui::events::handle_input_event(
                &mut harness.app,
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            );
            assert!(
                matches!(&action, Some(mew_tui::events::Action::AttachSession(owner)) if owner == id),
                "{id} row {offset}, col {column}, action {action:?}"
            );
            assert_eq!(harness.app.input, "draft across projects");
        }
    }
}

#[test]
fn conversation_rows_keep_metadata_right_aligned_and_selection_on_both_lines() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut app = mew_tui::App::new();
    app.daemon_sessions = vec![project_conversation("alpha", &home.join("code/alpha"))];
    app.open_session_picker();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let completed = terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut app))
        .unwrap();
    let list = app.picker_mouse.list;
    assert_eq!(completed.buffer[(list.x + 2, list.y + 1)].symbol(), "~");
    assert_eq!(
        completed.buffer[(list.right() - 1, list.y + 1)].symbol(),
        "0"
    );
    let selection = app.theme.resolve("selection.background");
    assert_eq!(completed.buffer[(list.x, list.y)].bg, selection);
    assert_eq!(completed.buffer[(list.x, list.y + 1)].bg, selection);
    assert!(completed.buffer[(list.x, list.y)]
        .modifier
        .contains(ratatui::style::Modifier::BOLD));
    assert!(!completed.buffer[(list.x + 2, list.y + 1)]
        .modifier
        .contains(ratatui::style::Modifier::BOLD));
}

#[test]
fn clipped_conversation_context_is_readable_in_selected_details() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut harness = Harness::new(40, 30);
    harness.app.daemon_sessions = vec![project_conversation(
        "alpha",
        &home.join("code/identifying-parent/project"),
    )];
    harness.app.open_session_picker();
    harness.app.picker.as_mut().unwrap().items[0].label = "○ task".into();
    let frame = harness.render();
    assert!(frame.contains("identifying-parent"), "{frame}");
    harness.resize(80, 30);
    harness.render();
    assert!(harness.app.picker_mouse.details.is_empty());
}

#[test]
fn sidebar_conversation_context_routes_both_lines_after_scrolling() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut harness = Harness::new(128, 24);
    harness.app.daemon_mode = true;
    harness.app.daemon_sessions = vec![
        project_conversation("alpha", &home.join("code/alpha")),
        project_conversation("beta", &home.join("code/beta")),
    ];
    harness.app.input = "keep draft".into();
    harness.app.cursor = harness.app.input.len();
    for scroll in [0, 2] {
        harness.app.sidebar_scroll = scroll;
        let frame = harness.render();
        assert!(frame.contains("~/code/beta"), "{frame}");
        for id in ["alpha", "beta"] {
            let rows: Vec<_> = harness.app.sidebar_item_rows.iter()
                .filter(|(_, item)| matches!(item, mew_tui::app::SidebarItem::Session(owner) if owner == id))
                .map(|(y, _)| *y).collect();
            assert_eq!(rows.len(), 2);
            let column = harness.app.sidebar_rect.x + 2;
            for y in rows {
                let action = mew_tui::events::handle_input_event(
                    &mut harness.app,
                    Event::Mouse(MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column,
                        row: y,
                        modifiers: KeyModifiers::NONE,
                    }),
                );
                assert!(
                    matches!(action, Some(mew_tui::events::Action::AttachSession(owner)) if owner == id)
                );
                assert_eq!(harness.app.input, "keep draft");
            }
        }
    }
}

#[test]
fn short_conversation_picker_reserves_scrollable_context_and_failure_details() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut harness = Harness::new(40, 16);
    harness.app.daemon_sessions = (0..6)
        .map(|i| {
            project_conversation(
                &format!("project-{i}"),
                &home.join("code/a-long-project-parent/workspace"),
            )
        })
        .collect();
    harness.app.session_turn_outcomes.insert(
        "project-0".into(),
        mew_client_core::TurnOutcome::Failed {
            detail: Some("credentials unavailable".into()),
        },
    );
    harness.app.open_session_picker();
    for (width, height) in [(40, 16), (24, 12), (20, 12), (80, 18)] {
        harness.resize(width, height);
        let mut frames = harness.render();
        assert!(
            harness.app.picker.as_ref().unwrap().visible_details > 0,
            "{frames}"
        );
        if width == 40 {
            assert!(frames.contains("PgUp/PgDn details"), "{frames}");
        }
        for _ in 0..12 {
            harness.key(KeyCode::PageDown.into());
            frames.push_str(&harness.render());
        }
        assert!(frames.contains("credentials"), "{frames}");
        assert_eq!(harness.app.picker.as_ref().unwrap().selected, 0);
        harness.key(KeyCode::PageUp.into());
    }
}

#[test]
fn tiny_conversation_picker_keeps_a_title_and_escape_reachable() {
    let mut harness = Harness::new(20, 4);
    harness.app.daemon_sessions = vec![project_conversation(
        "alpha",
        std::path::Path::new("/tmp/alpha"),
    )];
    harness.app.open_session_picker();
    let frame = harness.render();
    assert!(frame.contains("○ work in"), "{frame}");
    assert_eq!(harness.app.picker_mouse.rows.len(), 1);
    assert_eq!(harness.app.picker.as_ref().unwrap().visible_items, 1);
    harness.key(KeyCode::Esc.into());
    assert!(harness.app.picker.is_none());
}

#[test]
fn conversation_scrollbars_page_by_choices_and_keep_complete_pairs_after_resize() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut harness = Harness::new(80, 24);
    harness.app.daemon_sessions = (0..40)
        .map(|i| {
            project_conversation(
                &format!("project-{i:02}"),
                &home.join(format!("code/project-{i:02}")),
            )
        })
        .collect();
    harness.app.open_session_picker();
    harness.render();
    let visible = harness.app.picker.as_ref().unwrap().visible_items;
    let bar = harness.app.picker_mouse.scrollbar;
    assert_eq!(bar.area.height as usize, visible * 2);
    mew_tui::events::handle_input_event(
        &mut harness.app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: bar.area.x,
            row: bar.area.bottom() - 1,
            modifiers: KeyModifiers::NONE,
        }),
    );
    harness.render();
    assert_eq!(harness.app.picker.as_ref().unwrap().scroll, visible);
    for (width, height) in [(80, 40), (40, 16), (24, 12)] {
        harness.resize(width, height);
        harness.app.picker.as_mut().unwrap().selected = 39;
        let frame = harness.render();
        let picker = harness.app.picker.as_ref().unwrap();
        let list = harness.app.picker_mouse.list;
        assert_eq!(list.height as usize, picker.visible_items * 2, "{frame}");
        assert!(harness.app.picker_mouse.rows.iter().any(|(_, i)| *i == 39));
        assert!(harness
            .app
            .picker_mouse
            .rows
            .iter()
            .all(|(y, _)| *y < list.bottom()));
        assert!(
            frame.contains(if width >= 40 { "project-39" } else { "t-39" }),
            "{frame}"
        );
        assert_eq!(
            harness
                .app
                .picker_mouse
                .rows
                .iter()
                .filter(|(_, i)| *i == 39)
                .count(),
            2
        );
    }
}

#[test]
fn centered_pickers_identify_the_task_and_show_only_special_actions_while_searching() {
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
            for expected in [title, "choice", "first choice", "special action"] {
                assert!(
                    frame.contains(expected),
                    "missing {expected} for {kind} at width {width}:\n{frame}"
                );
            }
            assert!(
                !frame.contains("↑↓ move")
                    && !frame.contains("Enter choose")
                    && !frame.contains("Esc close"),
                "{frame}"
            );
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
        for expected in ["Models", "missing model", "No matches"] {
            assert!(
                frame.contains(expected),
                "missing {expected} at width {width}:\n{frame}"
            );
        }
        assert!(!frame.contains("first choice"));
        assert_eq!(harness.app.picker.as_ref().unwrap().filter, "missing model");
        harness.send_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(harness.app.mode, mew_tui::app::Mode::Normal);
    }
}

#[test]
fn empty_unfiltered_picker_explains_the_state() {
    let mut harness = Harness::new(40, 24);
    harness.app.open_session_picker();
    let frame = harness.render();
    assert!(frame.contains("Conversations"), "{frame}");
    assert!(frame.contains("No conversations"), "{frame}");
    harness.send_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(harness.app.mode, mew_tui::app::Mode::Normal);
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
        for expected in ["details 00", "█", "│"] {
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
    assert!(frame.contains("model 79"), "{frame}");
    harness.app.picker.as_mut().unwrap().items.truncate(2);
    let frame = harness.render();
    assert_eq!(harness.app.picker.as_ref().unwrap().visible_items, 2);
    assert!(!frame.contains('│'), "{frame}");
}

#[test]
fn short_picker_keeps_the_selected_wrapped_row_and_escape_reachable() {
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
        for expected in ["Models", "model 19"] {
            assert!(
                frame.contains(expected),
                "missing {expected} at {width}x{height}:\n{frame}"
            );
        }
        assert!(harness.app.picker.as_ref().unwrap().scroll > 0);
        harness.send_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(harness.app.mode, mew_tui::app::Mode::Normal);
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
    picker.items = ["foo", "ofo"]
        .into_iter()
        .map(|label| PickerItem {
            id: label.into(),
            label: label.into(),
            description: format!("start {label}\n{}", "long description ".repeat(100)),
            ..Default::default()
        })
        .collect();
    picker.filter = "foo".into();
    picker.cursor = 1;
    harness.send_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    harness.render();
    harness.send_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert!(harness.app.picker.as_ref().unwrap().details_scroll > 0);
    harness.send_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    let frame = harness.render();
    assert_eq!(harness.app.picker.as_ref().unwrap().filter, "foo");
    assert!(frame.contains("start foo"), "{frame}");
    harness.send_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    harness.send_key(KeyEvent::new(
        KeyCode::Char('Z'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    let frame = harness.render();
    assert_eq!(harness.app.picker.as_ref().unwrap().filter, "ofo");
    assert!(frame.contains("start ofo"), "{frame}");
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
        "→ thinking variants",
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
    let mut seen = String::new();
    for _ in 0..4 {
        let frame = harness.render();
        seen.push_str(&frame);
        for action in ["thinking", "variants"] {
            assert!(frame.contains(action), "{frame}");
        }
        harness.send_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    }
    for expected in ["detail zero", "detail one", "detail two", "detail three"] {
        assert!(seen.contains(expected), "{seen}");
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
