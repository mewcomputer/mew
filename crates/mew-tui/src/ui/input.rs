use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph},
    Frame,
};

use super::display_width;

use crate::app::{byte_at_display_offset, App, Mode};

pub(super) fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let input_bg = app.theme.resolve("muted");
    let style = match app.mode {
        Mode::SlashCommand => Style::default()
            .fg(app.theme.resolve("text.warning"))
            .bg(input_bg),
        _ => Style::default()
            .fg(app.theme.resolve("foreground"))
            .bg(input_bg),
    };

    // If there are queued messages, render a dimmed preview strip above the
    // input area showing the oldest queued message with a (+N) badge.
    let queue_height = u16::from(!app.queued_messages.is_empty()).min(area.height);
    if queue_height > 0 {
        let queue_area = Rect::new(area.x, area.y, area.width, queue_height);
        let queue_bg = app.theme.resolve("muted");
        let muted_style = Style::default()
            .fg(app.theme.resolve("text.muted"))
            .bg(queue_bg)
            .add_modifier(Modifier::DIM);
        let badge_style = Style::default()
            .fg(app.theme.resolve("text.warning"))
            .bg(queue_bg);

        let oldest = app.queued_messages[0].as_str();
        let extra = app.queued_messages.len().saturating_sub(1);
        let badge = if extra > 0 {
            format!(" (+{})", extra)
        } else {
            String::new()
        };
        // Truncate the preview to fit the available width minus prefix/badge.
        let prefix_str = "⏳ ";
        let available = area
            .width
            .saturating_sub(display_width(prefix_str) as u16)
            .saturating_sub(display_width(&badge) as u16);
        let first_line = oldest.lines().next().unwrap_or_default();
        let preview = &first_line[..byte_at_display_offset(first_line, available as usize)];
        let line = Line::from(vec![
            Span::styled(prefix_str, badge_style),
            Span::styled(preview, muted_style),
            Span::styled(badge, badge_style),
        ]);
        f.render_widget(Paragraph::new(line), queue_area);
    }

    let content_width = area.width.saturating_sub(2);
    // The prefix ("> " or spinner + space) takes 2 columns, so the
    // effective text width is content_width minus the prefix.
    let prefix_width = 2usize;
    let wrap_w = content_width.saturating_sub(prefix_width as u16).max(1) as usize;
    let visual_line_count = app.input_visual_line_count(wrap_w as u16).max(1);
    let input_height = (visual_line_count as u16).min(12) + 2;
    let prompt_height = u16::from(matches!(app.mode, Mode::HistorySearch | Mode::PasteConfirm));
    let input_area = Rect::new(
        area.x,
        area.y + queue_height,
        area.width,
        input_height.min(area.height.saturating_sub(queue_height + prompt_height)),
    );

    let bg_block = Block::default().style(Style::default().bg(input_bg));
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
                .fg(app.theme.resolve("text.warning"))
                .bg(input_bg),
        )
    } else {
        Span::styled(
            "> ",
            Style::default()
                .fg(app.theme.resolve("text.accent"))
                .bg(input_bg),
        )
    };
    let indent = Span::styled("  ", Style::default().bg(input_bg));

    let (cursor_visual_row, cursor_visual_col) = app.cursor_visual_row_col(wrap_w as u16);

    let first_visual_row =
        cursor_visual_row.saturating_sub(content_area.height.saturating_sub(1) as usize);
    for (visual_row, (_, text, first)) in app
        .input_visual_rows(wrap_w as u16)
        .enumerate()
        .skip(first_visual_row)
        .take(content_area.height as usize)
    {
        let y = content_area.y + (visual_row - first_visual_row) as u16;
        let visible = if display_width(text) > wrap_w {
            "�"
        } else {
            text
        };
        let row_prefix = if first {
            prefix.clone()
        } else {
            indent.clone()
        };
        let spans = vec![row_prefix, Span::styled(visible, style)];
        let text = Text::from(Line::from(spans));
        let row = Rect::new(content_area.x, y, content_area.width, 1);
        f.render_widget(Paragraph::new(text), row);
    }

    let cursor_x = content_area.x + prefix_width as u16 + cursor_visual_col.min(wrap_w) as u16;
    let cursor_y = content_area.y + cursor_visual_row.saturating_sub(first_visual_row) as u16;
    if content_area.height > 0 && cursor_x < input_area.right() {
        f.set_cursor_position((cursor_x, cursor_y));
    }

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
                .fg(app.theme.resolve("text.success"))
                .bg(input_bg)
        } else {
            Style::default()
                .fg(app.theme.resolve("text.error"))
                .bg(input_bg)
        };
        let text = Text::from(Line::from(Span::styled(prompt, search_style)));
        let query_column = display_width("reverse-i-search: ")
            + display_width(&app.history_search_query[..app.history_search_cursor]);
        let first_column =
            query_column.saturating_sub(search_area.width.saturating_sub(1) as usize);
        f.render_widget(
            Paragraph::new(text).scroll((0, first_column.min(u16::MAX as usize) as u16)),
            search_area,
        );
        if search_area.width > 0 && search_y < area.bottom() {
            let query_cursor_x = search_area.x + (query_column - first_column) as u16;
            f.set_cursor_position((query_cursor_x, search_y));
        }
    }

    // Large paste confirmation prompt.
    if app.mode == Mode::PasteConfirm {
        if let Some(ref pending) = app.pending_paste {
            let prompt = format!("paste {} chars? [y/N]", pending.len());
            let confirm_y = input_area.y + input_area.height;
            let confirm_area = Rect::new(input_area.x, confirm_y, input_area.width, 1);
            let style = Style::default()
                .fg(app.theme.resolve("text.warning"))
                .bg(input_bg);
            let text = Text::from(Line::from(Span::styled(prompt, style)));
            f.render_widget(Paragraph::new(text), confirm_area);
        }
    }
}

/// Compact companion render when chat content would be overlapped.
/// Shows bubble text and a single sprite face line at the right of the input area.
pub(super) fn draw_companion_compact(f: &mut Frame, app: &App, _input_area: Rect) {
    let input_bg = app.theme.resolve("muted");
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
                    .fg(app.theme.resolve("text.warning"))
                    .bg(input_bg),
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
                    .fg(app.theme.resolve("text.success"))
                    .bg(input_bg),
            ))),
            Rect::new(base_x + max_w.saturating_sub(lw), _input_area.y + 2, lw, 1),
        );
    }
}
