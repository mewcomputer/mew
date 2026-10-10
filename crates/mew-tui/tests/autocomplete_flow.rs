use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{app::Mode, events::handle_key_event, ui, App};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

#[test]
fn inline_commands_identify_the_list_and_fill_the_selected_row() {
    let mut app = App::new();
    app.push_synthetic_message("conversation".into());
    app.input = "/".into();
    app.cursor = 1;
    app.mode = Mode::SlashCommand;
    let buffer = draw(&mut app, 80, 30);
    let text = rows(&buffer);
    assert!(text.iter().any(|row| row.contains("Commands")));
    assert!(text.iter().any(|row| row.contains("Tab complete")));
    let y = text.iter().position(|row| row.contains("/clear")).unwrap() as u16;
    for x in 1..79 {
        assert_eq!(
            buffer[(x, y)].bg,
            app.theme.resolve("selection.background"),
            "column {x}"
        );
    }
}

#[test]
fn landing_autocomplete_keeps_the_selected_command_visible_while_scrolling() {
    let mut app = App::new();
    app.input = "/".into();
    app.cursor = 1;
    app.mode = Mode::SlashCommand;
    let count = app.filtered_slash_commands().len();
    for _ in 0..count {
        let buffer = draw(&mut app, 40, 24);
        let selected = &app.filtered_slash_commands()[app.slash_selected].name;
        assert!(
            rows(&buffer).iter().any(|row| row.contains(selected)),
            "missing {selected}"
        );
        assert!(app.slash_visible > 0);
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
}

#[test]
fn sidebar_is_available_at_normal_wide_terminal_sizes_and_clears_on_resize() {
    let mut app = App::new();
    app.push_synthetic_message("conversation".into());
    draw(&mut app, 120, 30);
    assert_eq!(app.sidebar_rect.width, 32);
    assert!(app.chat_area.width >= 80);
    draw(&mut app, 80, 24);
    assert_eq!(app.sidebar_rect, ratatui::layout::Rect::default());
    assert!(app.sidebar_header_rows.is_empty());
    draw(&mut app, 120, 30);
    assert_eq!(app.sidebar_rect.width, 32);
}

#[test]
fn reference_choices_use_the_clipped_viewport_and_keep_completion_in_the_composer() {
    let mut app = App::new();
    app.input = "@".into();
    app.cursor = 1;
    app.open_at_picker();
    app.picker.as_mut().unwrap().items = (0..20)
        .map(|i| mew_tui::app::PickerItem {
            id: format!("file{i:02}.rs"),
            label: format!("@file{i:02}.rs"),
            ..Default::default()
        })
        .collect();
    for width in [40, 80] {
        for _ in 0..20 {
            let buffer = draw(&mut app, width, 24);
            let picker = app.picker.as_ref().unwrap();
            let selected = &picker.filtered()[picker.selected].label;
            assert!(
                rows(&buffer).iter().any(|row| row.contains(selected)),
                "missing {selected}"
            );
            assert!(picker.visible_items > 0);
            handle_key_event(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
    }
    let action = handle_key_event(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(action, Some(mew_tui::events::Action::InsertAtMention(path)) if path == "@file00.rs ")
    );
    assert_eq!(app.input, "@");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.picker.is_none());
}

#[test]
fn autocomplete_survives_tiny_areas_and_reclamps_on_resize() {
    let mut app = App::new();
    app.push_synthetic_message("conversation".into());
    app.input = "/".into();
    app.cursor = 1;
    app.mode = Mode::SlashCommand;
    app.slash_selected = app.filtered_slash_commands().len() - 1;
    let selected = app.filtered_slash_commands()[app.slash_selected]
        .name
        .clone();
    app.slash_scroll = usize::MAX;
    for (width, height) in [(1, 1), (2, 4), (10, 8), (40, 16), (80, 30)] {
        let buffer = draw(&mut app, width, height);
        assert!(app.slash_scroll <= app.slash_selected);
        if width >= 40 {
            assert!(rows(&buffer).iter().any(|row| row.contains(&selected)));
        }
    }
}

#[test]
fn narrow_autocomplete_keeps_completion_and_close_keys_visible() {
    let mut app = App::new();
    app.input = "/".into();
    app.cursor = 1;
    app.mode = Mode::SlashCommand;
    let frame = rows(&draw(&mut app, 40, 24)).join("\n");
    assert!(frame.contains("Tab complete"), "{frame}");
    assert!(frame.contains("Enter"), "{frame}");
    assert!(frame.contains("Esc"), "{frame}");
}
