use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{events::handle_key_event, ui, App};
use ratatui::{backend::TestBackend, Terminal};

fn chat(width: u16, height: u16) -> (App, Terminal<TestBackend>) {
    let mut app = App::new();
    app.push_synthetic_message("conversation".into());
    let terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    (app, terminal)
}

fn draw(app: &mut App, terminal: &mut Terminal<TestBackend>) -> Vec<String> {
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            let mut row = String::new();
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                row.push_str(symbol);
                x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
            }
            row
        })
        .collect()
}

fn key(app: &mut App, code: KeyCode) {
    handle_key_event(app, KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn composer_has_no_prompt_gutter_on_landing_or_chat_while_idle_or_streaming() {
    for landing in [false, true] {
        let (mut app, mut terminal) = chat(40, 24);
        if landing {
            app.rewind_to(0);
        }
        app.input = "hello\nworld".into();
        app.cursor = 0;
        for streaming in [false, true, false] {
            app.streaming = streaming;
            draw(&mut app, &mut terminal);
            let cursor = terminal.get_cursor_position().unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(cursor.x, app.input_area.x + 1);
            assert_eq!(buffer[(cursor.x, cursor.y)].symbol(), "h");
            assert_eq!(buffer[(cursor.x, cursor.y + 1)].symbol(), "w");
            assert_eq!(
                buffer[(cursor.x - 1, cursor.y)].symbol(),
                if streaming { "⠋" } else { " " }
            );
            assert_eq!(
                buffer[(cursor.x, cursor.y)].bg,
                app.theme.resolve("chat.user.background")
            );
        }
    }
}

#[test]
fn text_that_fits_the_composer_surface_does_not_wrap_into_the_old_prompt_slot() {
    let (mut app, mut terminal) = chat(20, 24);
    app.input = "abcdefghijklmnopqr".into();
    app.cursor = app.input.len();
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(cursor.y, app.input_area.y + 1);
    assert_eq!(cursor.x, app.input_area.x + 19);
    assert_eq!(rows[cursor.y as usize].trim(), app.input);
}

#[test]
fn queued_follow_up_stays_visible_above_the_editable_prompt() {
    let (mut app, mut terminal) = chat(48, 24);
    app.queued_messages = vec!["check the tests".into(), "then review".into()];
    app.input = "current prompt".into();
    app.cursor = app.input.len();

    let rows = draw(&mut app, &mut terminal);

    assert!(rows[app.input_area.y as usize].contains("check the tests (+1)"));
    assert_eq!(rows[app.input_area.y as usize + 2].trim(), "current prompt");
}

#[test]
fn narrow_queue_preview_preserves_the_badge_after_wide_characters() {
    let (mut app, mut terminal) = chat(16, 24);
    app.queued_messages = vec!["漢漢漢漢漢漢漢漢".into(), "next".into()];

    let rows = draw(&mut app, &mut terminal);

    assert_eq!(
        rows[app.input_area.y as usize].trim_end(),
        "⏳ 漢漢漢漢 (+1)"
    );
}

#[test]
fn long_prompt_scrolls_to_the_row_being_edited_and_back_up() {
    let (mut app, mut terminal) = chat(48, 30);
    app.input = (1..=15)
        .map(|n| format!("line {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.cursor = app.input.len();

    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains("line 15"));
    assert!(cursor.y < app.input_area.bottom().saturating_sub(1));
    key(&mut app, KeyCode::Char('!'));
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains("line 15!"));

    for _ in 0..14 {
        key(&mut app, KeyCode::Up);
    }
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains("line 01"));
    for _ in 0..14 {
        key(&mut app, KeyCode::Down);
    }
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains("line 15!"));
}

#[test]
fn history_recall_scrolls_to_the_end_and_restores_the_draft() {
    let (mut app, mut terminal) = chat(48, 30);
    app.input = "draft".into();
    app.cursor = app.input.len();
    app.history.push(
        (1..=15)
            .map(|n| format!("history {n:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    draw(&mut app, &mut terminal);

    key(&mut app, KeyCode::Up);
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains("history 15"));
    key(&mut app, KeyCode::Down);
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(app.input, "draft");
    assert_eq!(rows[cursor.y as usize].trim(), "draft");
}

#[test]
fn wrapped_up_moves_by_the_composer_text_width() {
    let (mut app, mut terminal) = chat(20, 30);
    app.input = "abcdefghijklmnopqrstuvwx".into();
    app.cursor = app.input.len();
    draw(&mut app, &mut terminal);

    key(&mut app, KeyCode::Up);

    assert_eq!(app.cursor, 6);
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(cursor.x, app.input_area.x + 1 + 6);
    assert!(rows[cursor.y as usize].contains("abcdefghijklmnopqr"));
}

#[test]
fn wide_text_and_empty_trailing_line_have_visible_carets() {
    let (mut app, mut terminal) = chat(20, 30);
    for (input, expected_x) in [("漢字abc", 8), ("first\n", 1)] {
        app.input = input.into();
        app.cursor = app.input.len();
        draw(&mut app, &mut terminal);
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!(cursor.x, app.input_area.x + expected_x, "{input:?}");
        assert_eq!(
            cursor.y,
            app.input_area.y + 1 + u16::from(input.ends_with('\n'))
        );
    }
}

#[test]
fn clicking_a_scrolled_draft_edits_the_visible_row_and_ignores_the_queue() {
    use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
    use mew_tui::events::handle_input_event;

    let (mut app, mut terminal) = chat(48, 30);
    app.input = (1..=15)
        .map(|n| format!("line {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.cursor = app.input.len();
    app.queued_messages.push("queued follow-up".into());
    let rows = draw(&mut app, &mut terminal);
    let first_row = app.input_area.y + 2;
    let first_line = 15 - app.input_area.height.saturating_sub(3) as usize;
    assert!(rows[first_row as usize].contains(&format!("line {:02}", first_line + 1)));
    let click = |app: &mut App, row| {
        handle_input_event(
            app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: app.input_area.x + 2,
                row,
                modifiers: KeyModifiers::NONE,
            }),
        );
    };

    let queue_row = app.input_area.y;
    click(&mut app, queue_row);
    assert_eq!(app.cursor, app.input.len());
    click(&mut app, first_row);
    key(&mut app, KeyCode::Char('!'));
    assert_eq!(
        app.input.lines().nth(first_line),
        Some(format!("l!ine {:02}", first_line + 1).as_str())
    );
}

#[test]
fn a_caret_at_an_exact_wrap_boundary_stays_on_its_logical_line() {
    let (mut app, mut terminal) = chat(20, 30);
    app.input = "abcdefghijklmnopqr\nnext".into();
    app.cursor = 18;

    draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(cursor.y, app.input_area.y + 1);
    assert_eq!(cursor.x, app.input_area.x + 19);
}

#[test]
fn narrow_reverse_history_search_keeps_the_edited_query_and_caret_visible() {
    let (mut app, mut terminal) = chat(20, 24);
    app.mode = mew_tui::Mode::HistorySearch;
    app.history_search_query = "あいうえお abcdefghijklmnopqrstuvwxyz".into();
    app.history_search_cursor = app.history_search_query.len();

    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(cursor.x < 20);
    assert_eq!(cursor.y, app.input_area.bottom() - 1);
    assert!(rows[cursor.y as usize].contains("uvwxyz"));

    key(&mut app, KeyCode::Left);
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(cursor.x < 20);
    assert_eq!(rows[cursor.y as usize].chars().last(), Some('z'));
}

#[test]
fn wrapped_draft_keeps_its_caret_visible_after_a_narrow_short_resize() {
    let (mut app, mut terminal) = chat(24, 30);
    app.input = "a".repeat(270) + "current";
    app.cursor = app.input.len();
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains("current"));

    terminal.backend_mut().resize(20, 8);
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(cursor.x < 20 && cursor.y < 8);
    assert!(rows[cursor.y as usize].contains("current"));
    assert!(cursor.y < app.input_area.bottom() - 1);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Char('!'));
    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains('!'));
    assert!(cursor.x < 20 && cursor.y < 8);
}

#[test]
fn wide_glyph_wraps_before_the_edge_and_cursor_and_movement_use_that_row() {
    let (mut app, mut terminal) = chat(19, 24);
    app.input = "a".repeat(16) + "漢b";
    app.cursor = app.input.len();

    let rows = draw(&mut app, &mut terminal);
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(rows[cursor.y as usize].contains("漢b"));
    assert_eq!(cursor.x, app.input_area.x + 4);
    assert_eq!(app.visual_to_byte_offset(1, 2, 17), 19);
    key(&mut app, KeyCode::Up);
    assert_eq!(app.cursor, 3);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.cursor, app.input.len());

    use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
    mew_tui::events::handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: cursor.x - 1,
            row: cursor.y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    key(&mut app, KeyCode::Char('!'));
    assert!(app.input.ends_with("漢!b"));
}

#[test]
fn one_column_composer_marks_a_wide_glyph_without_losing_the_input() {
    let (mut app, mut terminal) = chat(1, 24);
    app.input = "漢b".into();
    app.cursor = app.input.len();

    let rows = draw(&mut app, &mut terminal);
    assert_eq!(app.input_visual_line_count(1), 2);
    assert!(rows[app.input_area.y as usize + 1].contains('�'));
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(cursor.x, 0);
    assert_eq!(cursor.y, app.input_area.y + 2);
    assert_eq!(app.input, "漢b");
}

#[test]
fn up_from_a_full_wrapped_row_moves_to_the_previous_visible_row() {
    let (mut app, mut terminal) = chat(20, 24);
    app.input = "a".repeat(36);
    app.cursor = app.input.len();
    draw(&mut app, &mut terminal);

    key(&mut app, KeyCode::Up);

    assert_eq!(app.cursor_visual_row_col(18).0, 0);
    assert_eq!(app.cursor, 17);
}

#[test]
fn joined_emoji_and_combining_marks_wrap_as_displayed_graphemes() {
    for (suffix, expected_x) in [("👨‍👩‍👧‍👦b", 4), ("e\u{301}b", 2)] {
        let (mut app, mut terminal) = chat(19, 24);
        app.input = "a".repeat(16) + suffix;
        app.cursor = app.input.len();

        let rows = draw(&mut app, &mut terminal);
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!(app.input_visual_line_count(17), 2);
        assert_eq!(cursor.x, expected_x);
        assert_eq!(cursor.y, app.input_area.y + 2);
        assert!(rows.join("\n").contains(suffix.strip_suffix('b').unwrap()));
        if suffix.starts_with('👨') {
            let after_emoji = 16 + suffix.strip_suffix('b').unwrap().len();
            assert_eq!(app.visual_to_byte_offset(1, 2, 17), after_emoji);
        } else {
            assert_eq!(app.visual_to_byte_offset(0, 17, 17), 16);
        }
    }
}
