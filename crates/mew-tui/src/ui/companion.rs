use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use super::display_width;
use crate::app::App;

/// Check whether the companion should shrink to the input bar because chat
/// content occupies the bottom rows.
pub fn should_use_compact(app: &App, chat_area: Rect) -> bool {
    let info = app.plugin_ui.get("buddy/info").cloned().unwrap_or_default();
    let bubble = app
        .plugin_ui
        .get("buddy/bubble")
        .cloned()
        .unwrap_or_default();
    if info.is_empty() && bubble.is_empty() {
        return false;
    }

    // Estimate the panel height.
    let sprite_h = info.lines().take(5).count() as u16;
    let bubble_h = if bubble.is_empty() {
        0
    } else {
        bubble.lines().count() as u16 + 2
    };
    let panel_h = bubble_h + sprite_h;

    if panel_h >= chat_area.height {
        return true;
    }

    // Scan the exact bottom region of the chat for non-empty content.
    // Only switch to compact if multiple rows would be overlapped.
    let scroll = app.scroll as usize;
    let chat_h = chat_area.height as usize;
    if scroll + chat_h > app.chat_rows.len() {
        return false; // not enough content to matter
    }

    let overlap_start = (scroll + chat_h).saturating_sub(panel_h as usize);
    let mut text_rows = 0usize;
    for row_idx in overlap_start..(scroll + chat_h).min(app.chat_rows.len()) {
        if let Some(line) = app.chat_rows.get(row_idx) {
            let t = line.trim();
            // Count only rows with actual visible content (not dividers or blanks).
            if !t.is_empty() && !t.chars().all(|c| c == '-' || c == '=' || c == ' ') {
                text_rows += 1;
            }
        }
    }

    // Switch to compact only if 3+ rows would be obscured.
    text_rows >= 3
}

/// Render the companion as a right-aligned float at the bottom of the chat
/// area. Speech bubble sits directly above the sprite, both flush-right.
/// Bubble auto-dismisses after text-based duration.
pub fn draw_companion_float(f: &mut Frame, app: &mut App, chat_area: Rect) {
    let info = app.plugin_ui.get("buddy/info").cloned().unwrap_or_default();
    let bubble = app
        .plugin_ui
        .get("buddy/bubble")
        .cloned()
        .unwrap_or_default();

    if info.is_empty() && bubble.is_empty() {
        return;
    }

    let chat_bg = Color::Rgb(22, 22, 26);

    // Sprite: first 5 lines of info.
    let sprite_lines: Vec<&str> = info.lines().take(5).collect();
    let sprite_w = sprite_lines
        .iter()
        .map(|l| display_width(l).max(1))
        .max()
        .unwrap_or(1) as u16;
    let sprite_h = sprite_lines.len() as u16;

    // Bubble: check if expired.
    let expired = app
        .companion_bubble_since
        .map(|since| since.elapsed() > App::companion_bubble_ttl(&bubble))
        .unwrap_or(false);
    if expired {
        app.plugin_ui.remove("buddy/bubble");
        app.companion_bubble_since = None;
    }

    let bubble_lines: Vec<&str> = if bubble.is_empty() || expired {
        vec![]
    } else {
        bubble.lines().collect()
    };
    let has_bubble = !bubble_lines.is_empty();
    let bubble_text_w = bubble_lines
        .iter()
        .map(|l| display_width(l))
        .max()
        .unwrap_or(0) as u16;
    let bubble_w = if has_bubble {
        (bubble_text_w + 4).clamp(10, 36)
    } else {
        0
    };
    let bubble_h = if has_bubble {
        bubble_lines.len() as u16 + 2
    } else {
        0
    };

    let panel_w = sprite_w.max(bubble_w + 2).max(2);
    let panel_h = bubble_h + sprite_h;

    if panel_w >= chat_area.width || panel_h >= chat_area.height {
        return;
    }

    let x = chat_area.x + chat_area.width.saturating_sub(panel_w);
    let y = chat_area.y + chat_area.height.saturating_sub(panel_h);
    let panel_rect = Rect::new(x, y, panel_w, panel_h);

    // Opaque background so chat text doesn't bleed through.
    f.render_widget(
        Block::default().style(Style::default().bg(chat_bg)),
        panel_rect,
    );

    let mut row = y;

    // Bubble — right-aligned.
    if has_bubble {
        let bw = bubble_w.min(panel_w);

        let top = format!("\u{256d}{}\u{256e}", "\u{2500}".repeat(bw as usize));
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                top,
                Style::default().fg(Color::DarkGray).bg(chat_bg),
            ))),
            Rect::new(x, row, bw + 2, 1),
        );
        row += 1;

        for bline in &bubble_lines {
            if row >= y + panel_h {
                break;
            }
            let truncated: String = bline
                .chars()
                .chain(std::iter::repeat(' '))
                .take(bw as usize)
                .collect();
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("\u{2502}", Style::default().fg(Color::DarkGray).bg(chat_bg)),
                    Span::styled(truncated, Style::default().fg(Color::Yellow).bg(chat_bg)),
                    Span::styled("\u{2502}", Style::default().fg(Color::DarkGray).bg(chat_bg)),
                ])),
                Rect::new(x, row, bw + 2, 1),
            );
            row += 1;
        }

        let bottom = format!("\u{2570}{}\u{25bc}", "\u{2500}".repeat(bw as usize));
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                bottom,
                Style::default().fg(Color::DarkGray).bg(chat_bg),
            ))),
            Rect::new(x, row, bw + 2, 1),
        );
        row += 1;
    }

    // Sprite, right-aligned.
    for line in &sprite_lines {
        if row >= y + panel_h {
            break;
        }
        let lw = display_width(line) as u16;
        let lx = x + panel_w.saturating_sub(lw);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                *line,
                Style::default().fg(Color::Green).bg(chat_bg),
            ))),
            Rect::new(lx, row, lw, 1),
        );
        row += 1;
    }
}
