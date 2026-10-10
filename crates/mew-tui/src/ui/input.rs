use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph},
    Frame,
};

use super::display_width;

use crate::app::{byte_at_display_offset, App, Mode};

/// The editor, queue and auxiliary prompt share the available input slot.
pub(crate) struct InputLayout {
    pub input: Rect,
    pub content: Rect,
    pub queue: Rect,
    pub prompt: Rect,
    pub wrap_width: u16,
}

impl InputLayout {
    pub fn new(app: &App, area: Rect) -> Self {
        let prompt_height = u16::from(matches!(app.mode, Mode::HistorySearch | Mode::PasteConfirm))
            .min(area.height);
        let queue_height =
            u16::from(!app.queued_messages.is_empty() && area.height > prompt_height + 1);
        let input = Rect::new(
            area.x,
            area.y + queue_height,
            area.width,
            area.height.saturating_sub(queue_height + prompt_height),
        );
        let pad_x = u16::from(input.width >= 5);
        let content_width = input.width.saturating_sub(pad_x * 2);
        let wrap_width = content_width.max(1);
        let pad_y = u16::from(
            input.height >= 3 && (input.height > 3 || app.input_visual_line_count(wrap_width) <= 1),
        );
        let content = Rect::new(
            input.x + pad_x,
            input.y + pad_y,
            content_width,
            input.height.saturating_sub(pad_y * 2),
        );
        Self {
            input,
            content,
            queue: Rect::new(area.x, area.y, area.width, queue_height),
            prompt: Rect::new(area.x, input.bottom(), area.width, prompt_height),
            wrap_width,
        }
    }
}

pub(super) fn desired_height(app: &App, width: u16) -> u16 {
    if app.mode == Mode::UserQuestion {
        if app
            .user_question
            .as_ref()
            .is_some_and(|question| question.review)
        {
            return 12;
        }
        return app
            .user_question
            .as_ref()
            .and_then(|question| question.questions.get(question.page))
            .map(|question| {
                (question.options.len().min(u16::MAX as usize) as u16)
                    .saturating_mul(2)
                    .saturating_add(6)
            })
            .unwrap_or(6);
    }
    let layout = InputLayout::new(app, Rect::new(0, 0, width, 14));
    (app.input_visual_line_count(layout.wrap_width).clamp(1, 12) + 2) as u16
        + u16::from(!app.queued_messages.is_empty())
        + u16::from(matches!(app.mode, Mode::HistorySearch | Mode::PasteConfirm))
}

pub(super) fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let input_bg = app.theme.resolve("chat.user.background");
    let style = match app.mode {
        Mode::SlashCommand => Style::default()
            .fg(app.theme.resolve("text.warning"))
            .bg(input_bg),
        _ => Style::default()
            .fg(app.theme.resolve("foreground"))
            .bg(input_bg),
    };

    let layout = InputLayout::new(app, area);
    let queue_height = layout.queue.height;
    if queue_height > 0 {
        let queue_area = layout.queue;
        let queue_bg = input_bg;
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

    let input_area = layout.input;
    let content_area = layout.content;
    let wrap_w = layout.wrap_width as usize;
    f.render_widget(
        Block::default().style(Style::default().bg(input_bg)),
        input_area,
    );

    if app.streaming && content_area.x > input_area.x && content_area.height > 0 {
        let frames = crate::ui::spinner::spinner_frames();
        let frame = frames
            .chars()
            .nth(app.spinner_frame % frames.chars().count())
            .unwrap_or(' ');
        f.render_widget(
            Paragraph::new(Span::styled(
                frame.to_string(),
                Style::default()
                    .fg(app.theme.resolve("text.warning"))
                    .bg(input_bg),
            )),
            Rect::new(content_area.x - 1, content_area.y, 1, 1),
        );
    }

    let (cursor_visual_row, cursor_visual_col) = app.cursor_visual_row_col(wrap_w as u16);

    let first_visual_row =
        cursor_visual_row.saturating_sub(content_area.height.saturating_sub(1) as usize);
    for (visual_row, (_, text, _)) in app
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
        let text = Line::from(Span::styled(visible, style));
        let row = Rect::new(content_area.x, y, content_area.width, 1);
        f.render_widget(Paragraph::new(text), row);
    }

    let cursor_x = content_area.x + cursor_visual_col.min(wrap_w) as u16;
    let cursor_y = content_area.y + cursor_visual_row.saturating_sub(first_visual_row) as u16;
    let editing_composer = matches!(app.mode, Mode::Normal | Mode::SlashCommand)
        || (app.mode == Mode::CommandPalette
            && app
                .picker
                .as_ref()
                .is_some_and(|picker| picker.is_at_picker()));
    if editing_composer && content_area.height > 0 && input_area.width > 0 {
        f.set_cursor_position((cursor_x.min(input_area.right() - 1), cursor_y));
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
        let search_area = layout.prompt;
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
            let count = pending.chars().count();
            let full = format!("paste {} chars? [y/N]", count);
            let compact = format!("paste {}? y/N", count);
            let prompt = if display_width(&full) <= layout.prompt.width as usize {
                full
            } else if display_width(&compact) <= layout.prompt.width as usize {
                compact
            } else {
                "y/N".into()
            };
            let confirm_area = layout.prompt;
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
    let input_bg = app.theme.resolve("chat.user.background");
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
