use mew_tui::App;

fn render(app: &mut App, width: u16) {
    app.ensure_chat_rendered(width - 4, width, 100);
}

fn select_all(app: &mut App, reverse: bool) -> String {
    let end = app.chat_rows.len() - 1;
    let (start, finish) = if reverse {
        ((end, 1000), (0, 0))
    } else {
        ((0, 0), (end, 1000))
    };
    app.sel_anchor_row = Some(start.0);
    app.sel_anchor_col = Some(start.1);
    app.sel_end_row = Some(finish.0);
    app.sel_end_col = Some(finish.1);
    app.selected_text()
}

#[test]
fn copied_prose_omits_surface_padding_and_soft_wraps_after_resize() {
    let text = "some words with **emphasis** and a longer sentence that wraps several times";
    let expected = text.replace("**", "");
    let mut app = App::new();
    app.push_synthetic_message(text.into());
    for width in [24, 80, 120, 24] {
        render(&mut app, width);
        assert_eq!(select_all(&mut app, false), expected, "width {width}");
        assert_eq!(
            select_all(&mut app, true),
            expected,
            "reverse width {width}"
        );
    }
}

#[test]
fn copied_user_text_preserves_source_spaces_and_newlines() {
    let text = "  indented words with  two spaces and a very long line\nnext source line\n\nlast paragraph";
    let mut app = App::new();
    app.push_user(text.into(), Vec::new());
    render(&mut app, 24);
    assert_eq!(select_all(&mut app, false), text);
}

#[test]
fn copied_code_keeps_indentation_and_hard_breaks_without_background_fill() {
    let text = "before\n\n```rust\nfn main() {\n    println!(\"long code line that wraps across the terminal\");\n}\n```\n\nafter";
    let expected = "before\n\nfn main() {\n    println!(\"long code line that wraps across the terminal\");\n}\n\nafter";
    let mut app = App::new();
    app.push_synthetic_message(text.into());
    render(&mut app, 24);
    assert_eq!(select_all(&mut app, false), expected);
}

#[test]
fn copied_unbroken_unicode_words_do_not_gain_spaces_at_wraps() {
    let text = "漢字café👩‍💻abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
    for user in [false, true] {
        let mut app = App::new();
        if user {
            app.push_user(text.into(), Vec::new());
        } else {
            app.push_synthetic_message(text.into());
        }
        render(&mut app, 24);
        assert_eq!(select_all(&mut app, false), text);
    }
}

#[test]
fn copied_partial_wrap_omits_only_layout_spaces() {
    let mut app = App::new();
    app.push_user("first second third fourth fifth sixth".into(), Vec::new());
    render(&mut app, 24);
    app.sel_anchor_row = Some(0);
    app.sel_anchor_col = Some(8);
    app.sel_end_row = Some(1);
    app.sel_end_col = Some(7);
    assert_eq!(app.selected_text(), "second third");
}

#[test]
fn copying_only_the_margin_or_surface_fill_returns_empty() {
    let mut app = App::new();
    app.push_user("hello".into(), Vec::new());
    render(&mut app, 40);
    for (start, end) in [(0, 2), (7, 40)] {
        app.sel_anchor_row = Some(0);
        app.sel_anchor_col = Some(start);
        app.sel_end_row = Some(0);
        app.sel_end_col = Some(end);
        assert_eq!(app.selected_text(), "");
    }
}

#[test]
fn copied_list_and_quote_continuations_do_not_repeat_display_prefixes() {
    let mut app = App::new();
    app.push_synthetic_message("- a list item that wraps over multiple rows\n- another item\n\n> a quotation that wraps over multiple rows".into());
    render(&mut app, 24);
    assert_eq!(select_all(&mut app, false), "• a list item that wraps over multiple rows\n• another item\n\n> a quotation that wraps over multiple rows");
}

#[test]
fn streaming_and_completed_markdown_copy_the_same_text() {
    for text in [
        "first paragraph with words that wrap\n\nsecond paragraph also wraps across rows",
        "- a list item\n\n> a quotation that wraps over multiple rows",
    ] {
        assert_streaming_copy(text);
    }
}

fn assert_streaming_copy(text: &str) {
    let expected = text.replace("- ", "• ");
    let mut app = App::new();
    app.push_synthetic_message(text.into());
    let mut stream = mdstream::MdStream::new(mdstream::Options::default());
    app.md_state.apply(stream.append(text));
    app.streaming = true;
    render(&mut app, 24);
    assert_eq!(select_all(&mut app, false), expected);
    app.streaming = false;
    app.mark_chat_dirty();
    render(&mut app, 24);
    assert_eq!(select_all(&mut app, false), expected);
}

#[test]
fn copied_tool_output_omits_generated_indent_and_preserves_source_indent() {
    let text = "    output words that wrap across several screen rows\n    another output line";
    let mut harness = mew_tui::harness::Harness::new(40, 30);
    harness.say_tool_call("bash", text, None);
    render(&mut harness.app, 24);
    let start = harness
        .app
        .chat_rows
        .iter()
        .position(|row| row.contains("output "))
        .unwrap();
    let end = harness
        .app
        .chat_rows
        .iter()
        .rposition(|row| row.contains("line"))
        .unwrap();
    harness.app.sel_anchor_row = Some(start);
    harness.app.sel_anchor_col = Some(0);
    harness.app.sel_end_row = Some(end);
    harness.app.sel_end_col = Some(1000);
    assert_eq!(harness.app.selected_text(), text);
}
