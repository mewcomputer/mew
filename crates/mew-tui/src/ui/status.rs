use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use super::{display_width, DIVIDER, STATUS_BG};
use crate::app::App;

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

pub(super) fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let status = &app.status;

    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, area);

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

    let left_spans = if app.esc_cancel_pending.is_some() {
        vec![Span::styled(
            "esc again to stop agent",
            Style::default().fg(Color::Yellow).bg(STATUS_BG),
        )]
    } else if app.ctrl_c_quit_pending.is_some() {
        vec![Span::styled(
            "ctrl-c again to quit",
            Style::default().fg(Color::Red).bg(STATUS_BG),
        )]
    } else if let Some(ref retry) = app.retry_status {
        vec![Span::styled(
            retry.as_str(),
            Style::default().fg(Color::LightBlue).bg(STATUS_BG),
        )]
    } else {
        vec![
            Span::styled(
                &status.model,
                Style::default().fg(Color::White).bg(STATUS_BG),
            ),
            Span::styled("  ", Style::default().bg(STATUS_BG)),
            Span::styled(
                &status.provider,
                Style::default().fg(Color::DarkGray).bg(STATUS_BG),
            ),
        ]
    };
    let right_width = display_width(&right) as u16;
    let right_span = Span::styled(right, Style::default().fg(Color::Gray).bg(STATUS_BG));
    let left_para = Paragraph::new(Line::from(left_spans));
    let right_para = Paragraph::new(Line::from(right_span)).alignment(Alignment::Right);
    let inner = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(inner);

    f.render_widget(left_para, chunks[0]);
    f.render_widget(right_para, chunks[1]);
}
