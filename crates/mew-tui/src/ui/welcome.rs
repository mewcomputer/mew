use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
    Frame,
};

pub(super) fn draw_welcome(f: &mut Frame, area: Rect) {
    let logo = vec![
        Line::from(Span::styled(
            "mew",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "terminal agent harness",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let prompts = vec![
        Line::from(Span::styled(
            "try asking:",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  \"explain this codebase\"",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  \"fix the bug in src/main.rs\"",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  \"add a test for the login endpoint\"",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "type /help for commands",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let all_lines: Vec<Line> = logo.into_iter().chain(prompts).collect();
    let content_height = all_lines.len() as u16;

    let top_pad = if area.height > content_height + 4 {
        (area.height - content_height) / 2
    } else {
        2
    };

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_pad),
            Constraint::Length(content_height),
            Constraint::Min(0),
        ])
        .split(area);

    let inner = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(vert[1]);

    let welcome_text = Text::from(all_lines);
    let para = Paragraph::new(welcome_text)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(para, inner[1]);
}
