use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use mew_tui::{
    app::{Mode, PickerItem},
    events::{handle_input_event, handle_key_event},
    ui, App,
};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}
fn find(buffer: &Buffer, text: &str) -> (u16, u16) {
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(x) = line.find(text) {
            return (x as u16, y);
        }
    }
    panic!("missing {text}");
}
fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    handle_input_event(
        app,
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
    );
}
fn picker(app: &mut App) {
    app.open_model_picker();
    app.picker.as_mut().unwrap().items = (0..80)
        .map(|i| PickerItem {
            id: i.to_string(),
            label: format!("choice {i:02}"),
            description: (0..30)
                .map(|n| format!("detail {n:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            ..Default::default()
        })
        .collect();
}
#[test]
fn chat_selection_uses_the_rendered_zero_based_row_after_scroll_and_resize() {
    for width in [24, 80, 120] {
        let mut app = App::new();
        app.push_synthetic_message(
            (0..50)
                .map(|i| format!("row{i:02} abcdef repeated words for wrapping\n"))
                .collect(),
        );
        app.auto_scroll = false;
        app.scroll = 7;
        let buffer = draw(&mut app, width, 24);
        let y = app.chat_area.y + 3;
        let x = 3;
        let expected: String = (x..x + 4).map(|col| buffer[(col, y)].symbol()).collect();
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 4, y);
        assert_eq!(app.selected_text(), expected, "width {width}");
    }
}

#[test]
fn releasing_a_wrapped_chat_selection_copies_text_without_terminal_layout() {
    let text = "first words in a sentence that wraps across several rows, ending last.";
    let mut app = App::new();
    app.push_user(text.into(), vec![]);
    let buffer = draw(&mut app, 40, 30);
    let (start_x, start_y) = find(&buffer, "first");
    let (end_x, end_y) = find(&buffer, "last.");
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        start_x,
        start_y,
    );
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        end_x + 5,
        end_y,
    );
    let action = handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: end_x + 5,
            row: end_y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(
        matches!(action, Some(mew_tui::events::Action::CopySelection(copied)) if copied == text)
    );
}
#[test]
fn scrollbar_click_and_drag_use_the_actual_column_and_reach_both_ends() {
    let mut app = App::new();
    app.push_synthetic_message("many chat rows\n".repeat(100));
    let buffer = draw(&mut app, 80, 24);
    let x = app.chat_area.right() - 1;
    let top = app.chat_area.y;
    let bottom = app.chat_area.bottom() - 1;
    assert!(matches!(buffer[(x, bottom)].symbol(), "█" | "│"));
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, bottom);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x, top);
    assert_eq!(app.scroll, 0);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x, bottom);
    assert_eq!(app.scroll, app.max_scroll);
}
#[test]
fn centered_picker_clicks_select_visible_choices_and_ignore_headers() {
    for (width, height) in [(40, 16), (80, 24), (120, 40)] {
        let mut app = App::new();
        picker(&mut app);
        app.picker.as_mut().unwrap().items[0].header = true;
        let buffer = draw(&mut app, width, height);
        let (x, y) = find(&buffer, "choice 02");
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
        assert_eq!(app.picker.as_ref().unwrap().selected, 2);
        let (x, y) = find(&buffer, "choice 00");
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
        assert_eq!(app.picker.as_ref().unwrap().selected, 2);
        assert!(
            handle_key_event(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).is_some()
        );
    }
}
#[test]
fn picker_wheel_is_local_to_choices_and_details() {
    let mut app = App::new();
    picker(&mut app);
    let buffer = draw(&mut app, 80, 24);
    mouse(&mut app, MouseEventKind::ScrollDown, 0, 0);
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    let (x, y) = find(&buffer, "detail 00");
    mouse(&mut app, MouseEventKind::ScrollDown, x, y);
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    assert_eq!(app.picker.as_ref().unwrap().details_scroll, 1);
    let (x, y) = find(&buffer, "choice 01");
    mouse(&mut app, MouseEventKind::ScrollDown, x, y);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
}
#[test]
fn slash_and_reference_clicks_select_rendered_rows_on_landing_and_chat() {
    for landing in [false, true] {
        for references in [false, true] {
            let mut app = App::new();
            if !landing {
                app.push_synthetic_message("chat".into());
            }
            app.input = if references { "@" } else { "/" }.into();
            app.cursor = 1;
            if references {
                app.open_at_picker();
                app.picker.as_mut().unwrap().items = (0..80)
                    .map(|i| PickerItem {
                        id: format!("file{i:02}"),
                        label: format!("@file{i:02}"),
                        ..Default::default()
                    })
                    .collect();
            } else {
                app.mode = Mode::SlashCommand;
            }
            let buffer = draw(&mut app, 80, 40);
            let (x, y) = find(&buffer, if references { "@file02" } else { "/models" });
            mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
            if references {
                assert_eq!(app.picker.as_ref().unwrap().selected, 2);
            } else {
                assert_eq!(
                    app.filtered_slash_commands()[app.slash_selected].name,
                    "/models"
                );
            }
        }
    }
}

#[test]
fn picker_scrollbar_pages_and_drags_without_losing_the_visible_selection() {
    for inline in [false, true] {
        let mut app = App::new();
        app.push_synthetic_message("chat".into());
        if inline {
            app.input = "/".into();
            app.cursor = 1;
            app.mode = Mode::SlashCommand;
        } else {
            picker(&mut app);
        }
        draw(&mut app, 80, 24);
        let bar = app.picker_mouse.scrollbar;
        let bottom = bar.area.bottom() - 1;
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            bar.area.x,
            bottom,
        );
        let offset = if inline {
            app.slash_scroll
        } else {
            app.picker.as_ref().unwrap().scroll
        };
        assert_eq!(offset, bar.area.height as usize);
        draw(&mut app, 80, 24);
        let bar = app.picker_mouse.scrollbar;
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            bar.thumb.x,
            bar.thumb.y,
        );
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            bar.area.x,
            bar.area.bottom() - 1,
        );
        mouse(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            bar.area.x,
            bar.area.bottom() - 1,
        );
        let buffer = draw(&mut app, 80, 24);
        let bar = app.picker_mouse.scrollbar;
        assert_eq!(buffer[(bar.area.x, bar.area.bottom() - 1)].symbol(), "█");
        draw(&mut app, 40, 16);
        let bar = app.picker_mouse.scrollbar;
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            bar.thumb.x,
            bar.thumb.y,
        );
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            bar.area.x,
            bar.area.bottom() - 1,
        );
        mouse(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            bar.area.x,
            bar.area.bottom() - 1,
        );
        let buffer = draw(&mut app, 40, 16);
        let (selected, offset, visible) = if inline {
            (app.slash_selected, app.slash_scroll, app.slash_visible)
        } else {
            let p = app.picker.as_ref().unwrap();
            (p.selected, p.scroll, p.visible_items)
        };
        assert!(selected >= offset && selected < offset + visible);
        if inline {
            find(&buffer, &app.filtered_slash_commands()[selected].name);
        } else {
            find(&buffer, &format!("choice {selected:02}"));
        }
        let bar = app.picker_mouse.scrollbar;
        assert_eq!(buffer[(bar.area.x, bar.area.bottom() - 1)].symbol(), "█");
    }
}
#[test]
fn clicks_in_blank_chat_and_wheels_over_the_composer_do_not_move_chat() {
    let mut app = App::new();
    app.push_synthetic_message("short".into());
    draw(&mut app, 80, 24);
    let y = app.chat_area.bottom() - 1;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 3, y);
    assert!(app.sel_anchor_row.is_none());
    app.push_user("long\n".repeat(100), vec![]);
    draw(&mut app, 80, 24);
    let scroll = app.scroll;
    let input_y = app.input_area.y + 1;
    mouse(&mut app, MouseEventKind::ScrollUp, 3, input_y);
    assert_eq!(app.scroll, scroll);
}
#[test]
fn a_modal_preempts_a_chat_scrollbar_drag() {
    let mut app = App::new();
    app.push_user("long\n".repeat(100), vec![]);
    draw(&mut app, 80, 24);
    let bar = app.chat_scrollbar;
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        bar.thumb.x,
        bar.thumb.y,
    );
    picker(&mut app);
    draw(&mut app, 80, 24);
    let scroll = app.scroll;
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        bar.area.x,
        bar.area.y,
    );
    assert_eq!(app.scroll, scroll);
}
#[test]
fn composer_clicks_use_the_drawn_wrapped_row_on_landing_and_chat() {
    for landing in [false, true] {
        for width in [24, 80, 120] {
            let mut app = App::new();
            if !landing {
                app.push_synthetic_message("chat".into());
            }
            app.input = "first line\nsecond line".into();
            app.cursor = app.input.len();
            let buffer = draw(&mut app, width, 30);
            let (x, y) = find(&buffer, "second");
            mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x + 3, y);
            handle_key_event(
                &mut app,
                KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
            );
            assert_eq!(app.input, "first line\nsec!ond line");
        }
    }
}

#[test]
fn narrow_reasoning_headers_do_not_shift_the_text_hit_rows_below_them() {
    use mew_message::{Message, Part, PartBase, ReasoningPart, Role, TextPart, Time};
    let mut app = App::new();
    let id = ulid::Ulid::new();
    let session_id = ulid::Ulid::new();
    let reasoning_id = ulid::Ulid::new();
    app.push_message(Message {
        id,
        session_id,
        role: Role::Assistant,
        parts: vec![
            Part::Reasoning(ReasoningPart {
                base: PartBase {
                    id: reasoning_id,
                    message_id: id,
                    session_id,
                },
                text: "some private thinking".into(),
                signature: None,
                encrypted_content: None,
                provider_item_id: None,
            }),
            Part::Text(TextPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: id,
                    session_id,
                },
                text: "answer token".into(),
                synthetic: false,
            }),
        ],
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    });
    let buffer = draw(&mut app, 24, 24);
    let (x, y) = find(&buffer, "thinking");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    assert!(app.reasoning_expanded.contains(&reasoning_id));
    let buffer = draw(&mut app, 24, 24);
    let (x, y) = find(&buffer, "answer");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 6, y);
    assert_eq!(app.selected_text(), "answer");
}

#[test]
fn unicode_chat_selection_keeps_display_columns_and_sidebar_wheels_stay_local() {
    let mut app = App::new();
    app.push_synthetic_message("漢字 tail".into());
    let buffer = draw(&mut app, 120, 24);
    let (_, y) = find(&buffer, "tail");
    let x = 2;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 4, y);
    assert_eq!(app.selected_text(), "漢字");
    app.push_user("long\n".repeat(100), vec![]);
    draw(&mut app, 120, 24);
    let scroll = app.scroll;
    let x = app.sidebar_rect.x;
    let y = app.sidebar_rect.y;
    mouse(&mut app, MouseEventKind::ScrollUp, x, y);
    assert_eq!(app.scroll, scroll);
}
#[test]
fn empty_and_tiny_picker_frames_clear_old_hit_targets() {
    let mut app = App::new();
    picker(&mut app);
    draw(&mut app, 80, 24);
    app.picker.as_mut().unwrap().filter = "missing".into();
    app.picker.as_mut().unwrap().cursor = 7;
    for (width, height) in [(1, 1), (4, 3), (12, 8), (80, 24)] {
        draw(&mut app, width, height);
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 0, 0);
        assert!(app.picker_mouse.rows.is_empty());
        assert!(app.picker_mouse.scrollbar.area.is_empty());
        assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    }
}

#[test]
fn shrinking_past_the_chat_text_width_disables_stale_row_hits() {
    let mut app = App::new();
    app.push_synthetic_message("chat rows from the old wider frame\n".repeat(10));
    draw(&mut app, 80, 24);
    draw(&mut app, 3, 24);
    let y = app.chat_area.y + 1;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 1, y);
    assert!(app.sel_anchor_row.is_none());
}
#[test]
fn budget_drag_releases_and_commits_when_the_pointer_leaves_the_track() {
    let mut app = App::new();
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
    draw(&mut app, 80, 24);
    let rect = app
        .picker
        .as_ref()
        .unwrap()
        .budget
        .as_ref()
        .unwrap()
        .track_rect
        .unwrap();
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        rect.x,
        rect.y,
    );
    let details = app.picker_mouse.details;
    let action = handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: details.x,
            row: details.y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(
        matches!(action,Some(mew_tui::events::Action::SetThinkingVariant(ref value)) if value=="budget:0")
    );
    assert!(
        !app.picker
            .as_ref()
            .unwrap()
            .budget
            .as_ref()
            .unwrap()
            .dragging
    );
}

#[test]
fn wheel_over_each_scrollbar_routes_to_its_list() {
    for mode in [Mode::Normal, Mode::SlashCommand, Mode::CommandPalette] {
        let mut app = App::new();
        app.push_user("many chat rows\n".repeat(100), vec![]);
        if mode == Mode::SlashCommand {
            app.mode = mode;
            app.input = "/".into();
            app.cursor = 1;
            app.slash_selected = 1;
        }
        if mode == Mode::CommandPalette {
            picker(&mut app);
            app.picker.as_mut().unwrap().selected = 1;
        }
        draw(&mut app, 80, 24);
        let bar = if mode == Mode::Normal {
            app.chat_scrollbar
        } else {
            app.picker_mouse.scrollbar
        };
        assert!(!bar.area.is_empty());
        let previous = app.scroll;
        mouse(&mut app, MouseEventKind::ScrollUp, bar.area.x, bar.area.y);
        match mode {
            Mode::Normal => assert_eq!(app.scroll, previous - 1),
            Mode::SlashCommand => assert_eq!(app.slash_selected, 0),
            _ => assert_eq!(app.picker.as_ref().unwrap().selected, 0),
        }
    }
}

#[test]
fn a_release_owned_by_another_modal_cancels_the_previous_scrollbar_grab() {
    let mut app = App::new();
    app.push_user("many rows\n".repeat(100), vec![]);
    draw(&mut app, 80, 24);
    let bar = app.chat_scrollbar;
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        bar.thumb.x,
        bar.thumb.y,
    );
    app.mode = Mode::Help;
    draw(&mut app, 80, 24);
    mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        bar.area.x,
        bar.area.y,
    );
    app.mode = Mode::Normal;
    draw(&mut app, 80, 24);
    let previous = app.scroll;
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        bar.area.x,
        bar.area.y,
    );
    assert_eq!(app.scroll, previous);
}

#[test]
fn scrollbar_paging_does_not_copy_a_previous_text_selection_on_release() {
    let mut app = App::new();
    app.push_user("many rows\n".repeat(100), vec![]);
    draw(&mut app, 80, 24);
    let y = app.chat_area.y + 3;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 4, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 8, y);
    assert!(!app.selected_text().is_empty());
    let bar = app.chat_scrollbar;
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        bar.area.x,
        bar.area.y,
    );
    let action = handle_input_event(
        &mut app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: bar.area.x,
            row: bar.area.y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(action.is_none());
}
