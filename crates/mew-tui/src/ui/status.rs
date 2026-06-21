use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use super::{display_width, DIVIDER, STATUS_BG};
use crate::app::{current_git_branch, App};

pub(super) fn draw_divider(f: &mut Frame, area: Rect) {
    let line = Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(DIVIDER),
    ));
    f.render_widget(Paragraph::new(line), area);
}

fn fmt_tokens(n: u32) -> String {
    if n >= 1_000 {
        format!("{:.1}k", n as f32 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// A pill in the status bar: a single bracketed block of text with its own
/// background color, so each pill reads as a distinct badge against the bar.
/// Brackets are part of the pill style (they're NOT in the text — the pill
/// text is just the label). `build_pills` produces the source list; the
/// segments helpers below interleave gaps and handle the colored marquee.
struct Pill {
    text: String,
    fg: Color,
    bg: Color,
}

/// A single rendered segment: either a pill or a 1-cell gap. The marquee and
/// the static line both render from `Vec<PillSegment>` so a scroll window
/// can keep per-pill colors.
struct PillSegment {
    text: String,
    fg: Color,
    bg: Color,
}

fn gap_segment(width: usize) -> PillSegment {
    PillSegment {
        text: " ".repeat(width),
        fg: Color::Reset,
        bg: STATUS_BG,
    }
}

fn build_pills(app: &App) -> Vec<Pill> {
    let mut pills = Vec::new();

    // model — dark green background, light green text.
    let model_label = if !app.status.provider.is_empty() && !app.status.model.is_empty() {
        format!("{}/{}", app.status.provider, app.status.model)
    } else if !app.status.model.is_empty() {
        app.status.model.clone()
    } else {
        String::new()
    };
    if !model_label.is_empty() {
        pills.push(Pill {
            text: model_label,
            fg: Color::Rgb(150, 230, 160),
            bg: Color::Rgb(25, 70, 35),
        });
    }

    // persona — dark purple background, light purple text.
    if let Some(ref name) = app.active_persona {
        pills.push(Pill {
            text: name.clone(),
            fg: Color::Rgb(200, 170, 240),
            bg: Color::Rgb(55, 35, 75),
        });
    }

    // cwd — dark blue background, light blue text.
    if !app.short_cwd.is_empty() {
        pills.push(Pill {
            text: app.short_cwd.clone(),
            fg: Color::Rgb(150, 190, 240),
            bg: Color::Rgb(30, 55, 90),
        });
    }

    // git — dark amber background, light amber text.
    if let Some(ref branch) = app.git_branch {
        pills.push(Pill {
            text: format!("git: {}", branch),
            fg: Color::Rgb(245, 210, 110),
            bg: Color::Rgb(75, 60, 20),
        });
    }

    // Future pills slot in here: persona, perms, plugin-contributed, etc.
    pills
}

/// Interleave pills with 1-cell gaps, and append a trailing gap of
/// `trailing_gap` width so the marquee cycles don't run together.
fn build_segments(pills: &[Pill], trailing_gap: usize) -> Vec<PillSegment> {
    let mut segs: Vec<PillSegment> = Vec::with_capacity(pills.len() * 2 + 1);
    for (i, p) in pills.iter().enumerate() {
        if i > 0 {
            segs.push(gap_segment(1));
        }
        segs.push(PillSegment {
            text: " ".to_string() + &p.text + " ",
            fg: p.fg,
            bg: p.bg,
        });
    }
    if trailing_gap > 0 {
        segs.push(gap_segment(trailing_gap));
    }
    segs
}

/// Plain concatenated pill text (texts joined by `" "`, no brackets) used
/// for width math.
fn pill_string(pills: &[Pill]) -> String {
    pills
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render segments as a single line (one span per segment). Used for the
/// static (fits) case.
fn segments_line(segs: &[PillSegment]) -> Line<'static> {
    segs.iter()
        .map(|s| Span::styled(s.text.clone(), Style::default().fg(s.fg).bg(s.bg)))
        .collect::<Vec<_>>()
        .into()
}

/// Color-preserving marquee: window `[offset, offset+width)` over the
/// concatenated segment text and emit spans that keep each pill's fg/bg.
/// Consecutive chars from the same segment coalesce into one span. The full
/// sequence already includes a trailing gap (via `build_segments(_, 3)`) so
/// the wrap-around shows a clear cycle separator.
fn segments_window(segs: &[PillSegment], width: usize, offset: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    // Precompute char positions and per-segment char vecs for O(1) lookup.
    let mut starts: Vec<usize> = Vec::with_capacity(segs.len());
    let mut segs_chars: Vec<Vec<char>> = Vec::with_capacity(segs.len());
    let mut pos = 0usize;
    for s in segs {
        starts.push(pos);
        let chars: Vec<char> = s.text.chars().collect();
        pos += chars.len();
        segs_chars.push(chars);
    }
    let total = pos;
    if total == 0 {
        return Line::from("");
    }

    let last_seg = segs.len() - 1;
    let mut spans_v: Vec<(String, Color, Color)> = Vec::new();
    for i in 0..width {
        let global_pos = (offset + i) % total;
        // Find the segment containing global_pos: largest starts[j] <= it.
        let seg_idx = match starts.binary_search(&global_pos) {
            Ok(j) => j,
            Err(j) => j.saturating_sub(1).min(last_seg),
        };
        let local_pos = global_pos - starts[seg_idx];
        let s = &segs[seg_idx];
        let ch = segs_chars[seg_idx][local_pos];
        if let Some(last) = spans_v.last_mut() {
            if last.1 == s.fg && last.2 == s.bg {
                last.0.push(ch);
                continue;
            }
        }
        spans_v.push((ch.to_string(), s.fg, s.bg));
    }
    let line_spans: Vec<Span<'static>> = spans_v
        .into_iter()
        .map(|(t, fg, bg)| Span::styled(t, Style::default().fg(fg).bg(bg)))
        .collect();
    Line::from(line_spans)
}

pub(super) fn draw_status(f: &mut Frame, app: &mut App, area: Rect) {
    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, area);

    // Resolve git branch once, lazily (avoids per-frame and per-test fs reads).
    if !app.git_branch_resolved {
        app.git_branch = current_git_branch();
        app.git_branch_resolved = true;
    }

    let status = &app.status;
    let total = status.input_tokens + status.output_tokens;
    let right = if status.context_window > 0 {
        format!(
            "{} / {}k tok  ·  ${:.2}",
            fmt_tokens(total),
            status.context_window / 1_000,
            status.cost,
        )
    } else {
        format!("{} tok  ·  ${:.2}", fmt_tokens(total), status.cost)
    };

    let inner = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    let right_width = display_width(&right) as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(inner);
    let left_area = chunks[0];

    // Render the right side (tokens/cost), always pinned.
    let right_span = Span::styled(right, Style::default().fg(Color::Gray).bg(STATUS_BG));
    f.render_widget(
        Paragraph::new(Line::from(right_span)).alignment(Alignment::Right),
        chunks[1],
    );

    // Left side: transient status overrides take precedence, then pills.
    let left_line: Line<'static> = if app.esc_cancel_pending.is_some() {
        Line::from(Span::styled(
            "esc again to stop agent",
            Style::default().fg(Color::Yellow).bg(STATUS_BG),
        ))
    } else if app.ctrl_c_quit_pending.is_some() {
        Line::from(Span::styled(
            "ctrl-c again to quit",
            Style::default().fg(Color::Red).bg(STATUS_BG),
        ))
    } else if let Some(ref retry) = app.retry_status {
        Line::from(Span::styled(
            retry.clone(),
            Style::default().fg(Color::LightBlue).bg(STATUS_BG),
        ))
    } else {
        let pills = build_pills(app);
        let pwidth = display_width(&pill_string(&pills)) as u16;
        if pwidth <= left_area.width {
            // Fits: render the pill row, reset the ticker.
            app.status_ticker_offset = 0;
            segments_line(&build_segments(&pills, 0))
        } else {
            // Overflow: color-preserving marquee with a trailing cycle gap.
            let marquee_segs = build_segments(&pills, 3);
            segments_window(
                &marquee_segs,
                left_area.width as usize,
                app.status_ticker_offset,
            )
        }
    };

    f.render_widget(Paragraph::new(left_line), left_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pill(text: &str, fg: Color, bg: Color) -> Pill {
        Pill {
            text: text.into(),
            fg,
            bg,
        }
    }

    #[test]
    fn test_pill_string_joins_with_spaces() {
        let pills = vec![
            pill("a", Color::White, Color::Rgb(0, 0, 0)),
            pill("bb", Color::Cyan, Color::Rgb(0, 0, 0)),
        ];
        assert_eq!(pill_string(&pills), "a bb");
    }

    #[test]
    fn test_pill_string_single() {
        let pills = vec![pill("only", Color::White, Color::Rgb(0, 0, 0))];
        assert_eq!(pill_string(&pills), "only");
    }

    #[test]
    fn test_pill_string_empty() {
        assert_eq!(pill_string(&[]), "");
    }

    #[test]
    fn test_build_segments_interleaves_one_cell_gaps() {
        let pills = vec![
            pill("a", Color::White, Color::Rgb(0, 0, 0)),
            pill("b", Color::Cyan, Color::Rgb(0, 0, 0)),
        ];
        let segs = build_segments(&pills, 0);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].text, "a");
        assert_eq!(segs[1].text, " ");
        assert_eq!(segs[1].bg, STATUS_BG);
        assert_eq!(segs[2].text, "b");
    }

    #[test]
    fn test_build_segments_trailing_gap_for_marquee() {
        let pills = vec![pill("ab", Color::White, Color::Rgb(0, 0, 0))];
        let segs = build_segments(&pills, 3);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "ab");
        assert_eq!(segs[1].text, "   ");
        assert_eq!(segs[1].bg, STATUS_BG);
    }

    #[test]
    fn test_segments_line_emits_one_span_per_segment() {
        let pills = vec![
            pill("a", Color::White, Color::Rgb(10, 0, 0)),
            pill("b", Color::Cyan, Color::Rgb(0, 10, 0)),
        ];
        let segs = build_segments(&pills, 0);
        let line = segments_line(&segs);
        // 3 spans: pill_a, gap, pill_b
        assert_eq!(line.spans.len(), 3);
    }

    #[test]
    fn test_segments_window_width_and_color_preservation() {
        // Two pills with distinct bgs; window of 5 chars should include
        // parts of both and the gap, with each part's style intact.
        let pills = vec![
            pill("AA", Color::White, Color::Rgb(10, 0, 0)),
            pill("BB", Color::Cyan, Color::Rgb(0, 10, 0)),
        ];
        // full sequence: "AA BB" + "   " (trailing) = "AA BB   " (8 chars).
        let segs = build_segments(&pills, 3);
        // offset 0, width 5 → first 5 chars = "AA BB".
        let line = segments_window(&segs, 5, 0);
        let total_text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(total_text, "AA BB");
        // The first span carries the first pill's bg, the gap carries
        // STATUS_BG, the second pill carries its own bg.
        assert_eq!(line.spans[0].style.bg, Some(Color::Rgb(10, 0, 0)));
        assert_eq!(line.spans[1].style.bg, Some(STATUS_BG));
        assert_eq!(line.spans[2].style.bg, Some(Color::Rgb(0, 10, 0)));
    }

    #[test]
    fn test_segments_window_wraps_with_offset() {
        let pills = vec![pill("abc", Color::White, Color::Rgb(0, 0, 0))];
        let segs = build_segments(&pills, 3);
        // full text: "abc" + " " + "   " = "abc    " (7 chars).
        // offset 0, width 3 → "abc".
        assert_eq!(text_of(&segments_window(&segs, 3, 0)), "abc");
        // offset 3, width 3 → gap + 2 trailing spaces → "   " (then wraps to "a").
        // Just check the first 3 chars of offset 3.
        let line = segments_window(&segs, 3, 3);
        let t: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(t.chars().count(), 3);
        // The first char of offset 3 is the first trailing space (gap).
        assert!(t.starts_with(' '));
    }

    #[test]
    fn test_segments_window_cycles_end_to_start() {
        // offset equal to the total char count must wrap to offset 0.
        let pills = vec![pill("ab", Color::White, Color::Rgb(0, 0, 0))];
        let segs = build_segments(&pills, 3);
        // full text = "ab" + "   " = 5 chars.
        let total: usize = segs.iter().map(|s| s.text.chars().count()).sum();
        let a = segments_window(&segs, 2, 0);
        let b = segments_window(&segs, 2, total);
        let ta: String = a.spans.iter().map(|s| s.content.to_string()).collect();
        let tb: String = b.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(ta, tb);
    }

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }
}
