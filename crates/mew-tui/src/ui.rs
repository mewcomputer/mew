use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Mode, PermissionState, ToolDisplayState};
use mew_message::{Part, Role, ToolState};

/// Background color for the input surface (slightly different from default terminal bg).
const INPUT_BG: Color = Color::Rgb(35, 35, 38);
/// Background color for the status line surface.
const STATUS_BG: Color = Color::Rgb(30, 30, 33);
/// Subtle divider color.
const DIVIDER: Color = Color::Rgb(50, 50, 55);

/// Render the full UI.
pub fn draw(f: &mut Frame, app: &App) {
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),     // chat area
            Constraint::Length(1),  // divider
            Constraint::Length(1),  // input area
            Constraint::Length(1),  // status line
        ])
        .split(f.area());

    draw_chat(f, app, main[0]);
    draw_divider(f, main[1]);
    draw_input(f, app, main[2]);
    draw_status(f, app, main[3]);

    if app.mode == Mode::PermissionPrompt {
        if let Some(ref perm) = app.permission {
            draw_permission_modal(f, perm, f.area());
        }
    }
}

fn draw_chat(f: &mut Frame, app: &App, area: Rect) {
    let mut text = Text::default();

    for msg in &app.messages {
        let (prefix, prefix_color, content_style) = match msg.role {
            Role::User => (">", Color::Cyan, Style::default().fg(Color::White)),
            Role::Assistant => ("", Color::Gray, Style::default().fg(Color::White)),
        };

        for part in &msg.parts {
            match part {
                Part::Text(tp) => {
                    for line in tp.text.lines() {
                        let mut spans = Vec::new();
                        if !prefix.is_empty() {
                            spans.push(Span::styled(
                                format!("{} ", prefix),
                                Style::default().fg(prefix_color),
                            ));
                        } else {
                            spans.push(Span::raw("  "));
                        }
                        spans.push(Span::styled(line, content_style));
                        text.push_line(Line::from(spans));
                    }
                }
                Part::Reasoning(rp) => {
                    for line in rp.text.lines() {
                        text.push_line(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(line, Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                }
                Part::ToolCall(tc) => {
                    let (state_label, state_color) = tool_call_label_and_color(app, tc);

                    text.push_line(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            format!("{} {}", state_label, tc.tool_name),
                            Style::default().fg(state_color).add_modifier(Modifier::BOLD),
                        ),
                    ]));

                    // Show output if completed or errored.
                    if let Some(ToolDisplayState::Completed(out)) = app.tool_states.get(&tc.base.id)
                    {
                        if !out.is_empty() {
                            for line in out.lines().take(20) {
                                // cap displayed lines
                                text.push_line(Line::from(vec![
                                    Span::raw("    "),
                                    Span::raw(line),
                                ]));
                            }
                            let line_count = out.lines().count();
                            if line_count > 20 {
                                text.push_line(Line::from(vec![
                                    Span::raw("    "),
                                    Span::styled(
                                        format!("... ({} more lines)", line_count - 20),
                                        Style::default().fg(Color::DarkGray),
                                    ),
                                ]));
                            }
                        }
                    }
                    if let Some(ToolDisplayState::Error(err)) = app.tool_states.get(&tc.base.id) {
                        if !err.is_empty() {
                            for line in err.lines() {
                                text.push_line(Line::from(vec![
                                    Span::raw("    "),
                                    Span::styled(line, Style::default().fg(Color::Red)),
                                ]));
                            }
                        }
                    }
                }
                Part::ToolResult(_) => {
                    // Tool results are markers; don't render separately.
                }
                _ => {}
            }
        }

        // Add spacing between messages.
        text.push_line(Line::from(""));
    }

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: true })
        .scroll((app.scroll, 0));

    f.render_widget(paragraph, area);
}

fn tool_call_label_and_color(app: &App, tc: &mew_message::ToolCallPart) -> (&'static str, Color) {
    match app.tool_states.get(&tc.base.id) {
        Some(ToolDisplayState::Running) => ("▶", Color::Yellow),
        Some(ToolDisplayState::Completed(_)) => ("✓", Color::Green),
        Some(ToolDisplayState::Error(_)) => ("✗", Color::Red),
        None => match &tc.state {
            ToolState::Pending(_) => ("○", Color::Yellow),
            ToolState::Running(_) => ("▶", Color::Yellow),
            ToolState::Completed(_) => ("✓", Color::Green),
            ToolState::Error(_) => ("✗", Color::Red),
        },
    }
}

fn draw_divider(f: &mut Frame, area: Rect) {
    let line = Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(DIVIDER),
    ));
    f.render_widget(Paragraph::new(line), area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let style = match app.mode {
        Mode::SlashCommand => Style::default().fg(Color::Yellow).bg(INPUT_BG),
        _ => Style::default().fg(Color::White).bg(INPUT_BG),
    };

    // Fill the input area background.
    let bg_block = Block::default().style(Style::default().bg(INPUT_BG));
    f.render_widget(bg_block, area);

    // Truncate input to fit the width and show cursor.
    let width = area.width.saturating_sub(2) as usize;
    let visible = if app.input.len() <= width {
        &app.input
    } else {
        let start = app.cursor.saturating_sub(width / 2);
        let end = (start + width).min(app.input.len());
        &app.input[start..end]
    };

    // Add a prefix indicator.
    let prefix = if app.streaming {
        Span::styled("… ", Style::default().fg(Color::Yellow).bg(INPUT_BG))
    } else {
        Span::styled("> ", Style::default().fg(Color::Cyan).bg(INPUT_BG))
    };

    let text = Text::from(Line::from(vec![prefix, Span::styled(visible, style)]));
    let paragraph = Paragraph::new(text);

    let inner = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    f.render_widget(paragraph, inner);

    // Position cursor.
    let cursor_x = inner.x + (app.cursor.min(width) as u16);
    let cursor_y = inner.y;
    f.set_cursor_position((cursor_x, cursor_y));
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let status = &app.status;

    // Background fill.
    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, area);

    let left = format!("{}  ·  {}", status.provider, status.model);
    let right = format!(
        "{} tok  ·  ${:.2}",
        status.input_tokens + status.output_tokens,
        status.cost,
    );

    let left_span = Span::styled(left, Style::default().fg(Color::Gray).bg(STATUS_BG));
    let right_span = Span::styled(right, Style::default().fg(Color::Gray).bg(STATUS_BG));

    let left_para = Paragraph::new(Line::from(left_span));
    let right_para = Paragraph::new(Line::from(right_span)).alignment(Alignment::Right);

    let left_area = Rect::new(area.x + 1, area.y, area.width / 2, 1);
    let right_area = Rect::new(area.x + area.width / 2, area.y, area.width / 2 - 1, 1);

    f.render_widget(left_para, left_area);
    f.render_widget(right_para, right_area);
}

fn draw_permission_modal(f: &mut Frame, perm: &PermissionState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 14u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    // Dim the background.
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIVIDER))
        .style(Style::default().bg(INPUT_BG))
        .title(" Permission ");

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let tool_input = serde_json::to_string_pretty(&perm.input).unwrap_or_default();
    let text = Text::from(vec![
        Line::from(vec![
            Span::styled("tool  ", Style::default().fg(Color::DarkGray).bg(INPUT_BG)),
            Span::styled(&perm.tool_name, Style::default().fg(Color::White).bg(INPUT_BG).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("input", Style::default().fg(Color::DarkGray).bg(INPUT_BG)),
        ]),
        Line::from(Span::styled(tool_input, Style::default().fg(Color::Gray).bg(INPUT_BG))),
        Line::from(""),
        Line::from(Span::styled(
            "choose:",
            Style::default().fg(Color::DarkGray).bg(INPUT_BG),
        )),
    ]);

    let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(paragraph, inner);

    // Draw options.
    let options = [("allow once", 'a'), ("session", 's'), ("deny", 'd')];
    let mut option_lines = Vec::new();
    for (i, (label, key)) in options.iter().enumerate() {
        let is_selected = i == perm.selected;
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray).bg(INPUT_BG)
        };
        option_lines.push(Span::styled(format!(" [{}] {}  ", key, label), style));
    }

    let option_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(2),
        inner.width,
        1,
    );
    f.render_widget(Paragraph::new(Line::from(option_lines)), option_area);
}
