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

/// A single bracketed pill in the status bar. Each pill renders as `[text]`
/// on its own background color, so they read as separate "boxes" against the
/// status bar. Collect them in priority order; later pills (cwd, git, persona,
/// perms) drop first if width is tight, and the whole row marquees when even
/// the model pill alone overflows.
struct Pill {
    text: String,
    fg: Color,
    bg: Color,
}

fn build_pills(app: &App) -> Vec<Pill> {
    let mut pills = Vec::new();

    // [provider/model] — dark green background, light green text.
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

    // [~/code/mew] — dark blue background, light blue text.
    if !app.short_cwd.is_empty() {
        pills.push(Pill {
            text: app.short_cwd.clone(),
            fg: Color::Rgb(150, 190, 240),
            bg: Color::Rgb(30, 55, 90),
        });
    }

    // [git: main] — dark yellow/amber background, light yellow text.
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

/// Plain concatenated pill text (`[a] [b] [c]`) for width math and marquee.
fn pill_string(pills: &[Pill]) -> String {
    pills
        .iter()
        .map(|p| format!("[{}]", p.text))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Styled line of pills for the static (fits) case. Each pill is one span
/// with its own bg/fg, and gaps between pills are 1 cell of the status bar
/// background so they read as separate boxes.
fn pill_line(pills: &[Pill]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, p) in pills.iter().enumerate() {
        if i > 0 {
            // 1-cell gap: status bar background showing through.
            spans.push(Span::styled(" ", Style::default().bg(STATUS_BG)));
        }
        spans.push(Span::styled(
            format!("[{}]", p.text),
            Style::default().fg(p.fg).bg(p.bg),
        ));
    }
    Line::from(spans)
}

/// Scroll a window across `text` (with a gap appended so cycles don't run
/// together). Used when the pills don't fit and the bar acts as a ticker.
fn marquee(text: &str, width: usize, offset: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut seq: Vec<char> = text.chars().collect();
    if seq.is_empty() {
        return String::new();
    }
    seq.extend("   ".chars());
    let n = seq.len();
    (0..width).map(|i| seq[(offset + i) % n]).collect()
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
        let pstr = pill_string(&pills);
        let pwidth = display_width(&pstr) as u16;
        if pwidth <= left_area.width {
            // Fits: render styled pills, reset the ticker.
            app.status_ticker_offset = 0;
            pill_line(&pills)
        } else {
            // Overflow: marquee. Drop per-pill colors for the scrolled window.
            let scrolled = marquee(&pstr, left_area.width as usize, app.status_ticker_offset);
            Line::from(Span::styled(
                scrolled,
                Style::default().fg(Color::Gray).bg(STATUS_BG),
            ))
        }
    };

    f.render_widget(Paragraph::new(left_line), left_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pill_string_formats_brackets() {
        let pills = vec![
            Pill {
                text: "a".into(),
                fg: Color::White,
                bg: Color::Rgb(0, 0, 0),
            },
            Pill {
                text: "bb".into(),
                fg: Color::Cyan,
                bg: Color::Rgb(0, 0, 0),
            },
        ];
        assert_eq!(pill_string(&pills), "[a] [bb]");
    }

    #[test]
    fn test_pill_string_single() {
        let pills = vec![Pill {
            text: "only".into(),
            fg: Color::White,
            bg: Color::Rgb(0, 0, 0),
        }];
        assert_eq!(pill_string(&pills), "[only]");
    }

    #[test]
    fn test_pill_string_empty() {
        assert_eq!(pill_string(&[]), "");
    }

    #[test]
    fn test_marquee_returns_exact_width() {
        let out = marquee("hello", 3, 0);
        assert_eq!(out.chars().count(), 3);
    }

    #[test]
    fn test_marquee_advances_with_offset() {
        // seq = "abc" + "   " = "abc   " (6 chars). offset 0 → first 4 = "abc ".
        assert_eq!(marquee("abc", 4, 0), "abc ");
        // offset 1 → chars 1..5 = "bc  ".
        assert_eq!(marquee("abc", 4, 1), "bc  ");
    }

    #[test]
    fn test_marquee_wraps_around() {
        // offset past the end wraps via modulo.
        let seq_len = "abc".chars().count() + 3; // 6
                                                 // offset == seq_len should equal offset 0.
        assert_eq!(marquee("abc", 4, seq_len), marquee("abc", 4, 0));
    }

    #[test]
    fn test_marquee_zero_width_returns_empty() {
        assert_eq!(marquee("abc", 0, 0), "");
    }
}
