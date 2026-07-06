use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph},
    Frame,
};

use super::display_width;

use crate::app::{byte_at_display_offset, App, Mode};

pub(super) fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let style = match app.mode {
        Mode::SlashCommand => Style::default()
            .fg(Color::Yellow)
            .bg(app.theme.tokens.status_bg),
        _ => Style::default()
            .fg(Color::White)
            .bg(app.theme.tokens.status_bg),
    };

    let content_width = area.width.saturating_sub(2);
    // The prefix ("> " or spinner + space) takes 2 columns, so the
    // effective text width is content_width minus the prefix.
    let prefix_width = 2usize;
    let wrap_w = content_width.saturating_sub(prefix_width as u16).max(1) as usize;
    let visual_line_count = app.input_visual_line_count(wrap_w as u16).max(1);
    let input_height = (visual_line_count as u16).min(12) + 2;
    let input_area = Rect::new(area.x, area.y, area.width, input_height.min(area.height));

    let bg_block = Block::default().style(Style::default().bg(app.theme.tokens.status_bg));
    f.render_widget(bg_block, input_area);

    let content_area = Rect::new(
        input_area.x + 1,
        input_area.y + 1,
        input_area.width.saturating_sub(2),
        input_area.height.saturating_sub(2),
    );

    let prefix = if app.streaming {
        let frames = crate::ui::spinner::spinner_frames();
        let frame = frames
            .chars()
            .nth(app.spinner_frame % frames.chars().count())
            .unwrap_or(' ');
        Span::styled(
            format!("{} ", frame),
            Style::default()
                .fg(Color::Yellow)
                .bg(app.theme.tokens.status_bg),
        )
    } else {
        Span::styled(
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .bg(app.theme.tokens.status_bg),
        )
    };
    let indent = Span::styled("  ", Style::default().bg(app.theme.tokens.status_bg));

    let (cursor_visual_row, cursor_visual_col) = app.cursor_visual_row_col(wrap_w as u16);

    let mut visual_row = 0usize;
    for line in app.input.split('\n') {
        let dw = display_width(line);
        let rows = if wrap_w == 0 {
            1
        } else {
            dw.div_ceil(wrap_w).max(1)
        };
        for r in 0..rows {
            let y = content_area.y + visual_row as u16;
            if y >= content_area.y + content_area.height {
                break;
            }
            let start_col = r * wrap_w;
            let end_col = ((r + 1) * wrap_w).min(dw);
            let start_byte = byte_at_display_offset(line, start_col);
            let end_byte = byte_at_display_offset(line, end_col);
            let visible = &line[start_byte..end_byte];

            let spans = if r == 0 {
                vec![prefix.clone(), Span::styled(visible, style)]
            } else {
                vec![indent.clone(), Span::styled(visible, style)]
            };
            let text = Text::from(Line::from(spans));
            let row = Rect::new(content_area.x, y, content_area.width, 1);
            f.render_widget(Paragraph::new(text), row);
            visual_row += 1;
        }
    }

    let cursor_x = content_area.x + prefix_width as u16 + cursor_visual_col.min(wrap_w) as u16;
    let cursor_y = content_area.y + cursor_visual_row.min(content_area.height as usize) as u16;
    f.set_cursor_position((cursor_x, cursor_y));

    // History search prompt (Ctrl+R).
    if app.mode == Mode::HistorySearch {
        let match_text = app.history_search_current_match().unwrap_or_default();
        let matches = app.history_search_matches();
        let match_count = matches.len();
        let prompt = format!(
            "reverse-i-search: {}  →  {}",
            app.history_search_query,
            if match_text.is_empty() {
                "(no match)".to_string()
            } else {
                match_text
            }
        );
        let search_y = input_area.y + input_area.height;
        let search_area = Rect::new(input_area.x, search_y, input_area.width, 1);
        let search_style = if match_count > 0 {
            Style::default()
                .fg(Color::Green)
                .bg(app.theme.tokens.status_bg)
        } else {
            Style::default()
                .fg(Color::Red)
                .bg(app.theme.tokens.status_bg)
        };
        let text = Text::from(Line::from(Span::styled(prompt, search_style)));
        f.render_widget(Paragraph::new(text), search_area);
        // Move cursor to end of search query.
        let query_cursor_x = input_area.x
            + 1
            + "reverse-i-search: ".len() as u16
            + app.history_search_query.len() as u16;
        f.set_cursor_position((query_cursor_x, search_y));
    }

    // Large paste confirmation prompt.
    if app.mode == Mode::PasteConfirm {
        if let Some(ref pending) = app.pending_paste {
            let prompt = format!("paste {} chars? [y/N]", pending.len());
            let confirm_y = input_area.y + input_area.height;
            let confirm_area = Rect::new(input_area.x, confirm_y, input_area.width, 1);
            let style = Style::default()
                .fg(Color::Yellow)
                .bg(app.theme.tokens.status_bg);
            let text = Text::from(Line::from(Span::styled(prompt, style)));
            f.render_widget(Paragraph::new(text), confirm_area);
        }
    }
}

/// Compact companion render when chat content would be overlapped.
/// Shows bubble text and a single sprite face line at the right of the input area.
pub(super) fn draw_companion_compact(f: &mut Frame, app: &App, _input_area: Rect) {
    let bubble = app
        .plugin_ui
        .get("buddy/bubble")
        .cloned()
        .unwrap_or_default();
    let info = app.plugin_ui.get("buddy/info").cloned().unwrap_or_default();

    // Pick the most "face-like" line from the sprite (line with eye/face).
    let face = info
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();

    let bubble_line = bubble.lines().next().unwrap_or(&bubble);
    let bubble_text = format!("\u{25c0} {}", bubble_line);

    let max_w = bubble_text.len().max(face.len()) as u16;
    let base_x = _input_area.x + _input_area.width.saturating_sub(max_w + 3);

    // Bubble line.
    if !bubble.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &bubble_text,
                Style::default()
                    .fg(Color::Yellow)
                    .bg(app.theme.tokens.status_bg),
            ))),
            Rect::new(base_x, _input_area.y + 1, bubble_text.len() as u16 + 1, 1),
        );
    }

    // Face line below bubble.
    if !face.is_empty() {
        let lw = face.len() as u16;
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                face,
                Style::default()
                    .fg(Color::Green)
                    .bg(app.theme.tokens.status_bg),
            ))),
            Rect::new(base_x + max_w.saturating_sub(lw), _input_area.y + 2, lw, 1),
        );
    }
}
