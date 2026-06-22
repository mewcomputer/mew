use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::display_width;
use crate::app::App;

/// The cat ASCII art shown on the landing screen, sitting to the left of the
/// "mew" wordmark. Four lines; widths are a mix of half- and full-width glyphs,
/// so layout is driven by `display_width`, not byte length.
const CAT: &str = " ╱|、\n(˚。7\n|、˜〵\nじしˍ,)ノ";
const LOGO_M: &[&str] = &["███▄███▄", "██ ██ ██", "██ ██ ██"];
const LOGO_E: &[&str] = &["▄█▀█▄", "██▄█▀", "▀█▄▄▄"];
const LOGO_W: &[&str] = &["██   ██", "██ █ ██", " ██▀██"];

/// Build the three logo rows by concatenating the per-letter rows with a
/// single-column gap between letters.
fn logo_lines() -> Vec<String> {
    (0..3)
        .map(|i| format!("{} {} {}", LOGO_M[i], LOGO_E[i], LOGO_W[i]))
        .collect()
}

/// Render the landing (start) screen and return the rect the caller should
/// render the input into.
///
/// Draws the cat + block "mew" wordmark centered as a hero, with room reserved
/// below it for a centered input field (and the slash-autocomplete list above
/// that field, when active). The hero and the input are centered together as a
/// single block so the cat reads as sitting directly above the field. The
/// caller renders the input itself via `draw_input` and draws the slash list
/// into the `slash_height` rows directly above the returned rect.
pub(super) fn draw_landing(f: &mut Frame, app: &App, area: Rect, slash_height: u16) -> Rect {
    let cat_lines: Vec<&str> = CAT.lines().collect();
    let logo_lines = logo_lines();

    let green = Style::default().fg(Color::Green);
    let wordmark = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    // Fixed column widths so the cat and the multi-row logo stay aligned across
    // rows regardless of per-line glyph widths.
    let cat_col_w = cat_lines
        .iter()
        .map(|l| display_width(l))
        .max()
        .unwrap_or(0);
    let logo_w = logo_lines
        .iter()
        .map(|l| display_width(l))
        .max()
        .unwrap_or(0);
    let gap = "  ";
    let gap_w = display_width(gap);

    // Vertically align the cat and logo by their baselines (bottom edges) so
    // the wordmark reads as sitting on the same line as the cat.
    let cat_h = cat_lines.len() as u16;
    let logo_h = logo_lines.len() as u16;
    let hero_h = cat_h.max(logo_h);
    let cat_offset = hero_h - cat_h;
    let logo_offset = hero_h - logo_h;

    let mut hero_rows: Vec<Line> = Vec::with_capacity(hero_h as usize);
    for i in 0..hero_h as usize {
        let mut spans: Vec<Span> = Vec::new();

        // Cat column, padded to cat_col_w so the logo column always starts at
        // the same x on every row.
        let ci = i as isize - cat_offset as isize;
        let cat_line = if ci >= 0 {
            cat_lines.get(ci as usize).copied()
        } else {
            None
        };
        if let Some(cl) = cat_line {
            spans.push(Span::styled(cl.to_string(), green));
            let pad = cat_col_w.saturating_sub(display_width(cl));
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
        } else {
            spans.push(Span::raw(" ".repeat(cat_col_w)));
        }

        spans.push(Span::raw(gap));

        // Logo column.
        let li = i as isize - logo_offset as isize;
        if li >= 0 {
            if let Some(ll) = logo_lines.get(li as usize) {
                spans.push(Span::styled(ll.clone(), wordmark));
            }
        }

        hero_rows.push(Line::from(spans));
    }

    let hero_w = (cat_col_w + gap_w + logo_w) as u16;

    // Centered input width: ~60% of the area, clamped to a sane range and to
    // the available width. The content width (minus the 1-cell inset on each
    // side that `draw_input` applies) drives wrapping/height.
    let input_w = (area.width * 3 / 5)
        .clamp(30, 80)
        .min(area.width.saturating_sub(2));
    let content_w = input_w.saturating_sub(2);
    let input_h = (app.input_visual_line_count(content_w).clamp(1, 12) + 2) as u16;

    // Vertical layout: hero, a 1-row gap, the slash list, then the input — all
    // centered as one block so the cat hovers just above the field.
    let gap_rows: u16 = 1;
    let block_h = hero_h
        .saturating_add(gap_rows)
        .saturating_add(slash_height)
        .saturating_add(input_h);

    let top_pad = if area.height > block_h {
        (area.height - block_h) / 2
    } else {
        0
    };
    let block_y = area.y + top_pad;
    let center_x = area.x + area.width / 2;

    // Hero: centered as a fixed-width, left-aligned block.
    let hero_w = hero_w.min(area.width);
    let hero_x = center_x
        .saturating_sub(hero_w / 2)
        .clamp(area.x, area.right().saturating_sub(hero_w));
    let hero_rect = Rect::new(hero_x, block_y, hero_w, hero_h);
    f.render_widget(Paragraph::new(hero_rows), hero_rect);

    // Input: horizontally centered, sitting below the hero + slash list.
    let input_y = block_y
        .saturating_add(hero_h)
        .saturating_add(gap_rows)
        .saturating_add(slash_height);
    let input_x = center_x
        .saturating_sub(input_w / 2)
        .clamp(area.x, area.right().saturating_sub(input_w));
    let remaining = area.bottom().saturating_sub(input_y);
    Rect::new(input_x, input_y, input_w, input_h.min(remaining))
}
