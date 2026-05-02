use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Mode, PermissionState, ToolDisplayState};
use mew_message::{Part, Role, ToolState};

/// Render the full UI.
pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),      // chat area
            Constraint::Length(3),   // input area
            Constraint::Length(1),   // status line
        ])
        .split(f.area());

    draw_chat(f, app, chunks[0]);
    draw_input(f, app, chunks[1]);
    draw_status(f, app, chunks[2]);

    if app.mode == Mode::PermissionPrompt {
        if let Some(ref perm) = app.permission {
            draw_permission_modal(f, perm, f.area());
        }
    }
}

fn draw_chat(f: &mut Frame, app: &App, area: Rect) {
    let mut text = Text::default();

    for msg in &app.messages {
        let (prefix, style) = match msg.role {
            Role::User => ("> ", Style::default().fg(Color::Cyan)),
            Role::Assistant => ("  ", Style::default()),
        };

        for part in &msg.parts {
            match part {
                Part::Text(tp) => {
                    for line in tp.text.lines() {
                        text.push_line(Line::from(vec![
                            Span::styled(prefix, style),
                            Span::raw(line),
                        ]));
                    }
                }
                Part::Reasoning(rp) => {
                    for line in rp.text.lines() {
                        text.push_line(Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(line, Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                }
                Part::ToolCall(tc) => {
                    let state_label = match app.tool_states.get(&tc.base.id) {
                        Some(ToolDisplayState::Running) => "[running]",
                        Some(ToolDisplayState::Completed(_)) => "[done]",
                        Some(ToolDisplayState::Error(_)) => "[error]",
                        None => match &tc.state {
                            ToolState::Pending(_) => "[pending]",
                            ToolState::Running(_) => "[running]",
                            ToolState::Completed(_) => "[done]",
                            ToolState::Error(_) => "[error]",
                        },
                    };
                    let state_color = match app.tool_states.get(&tc.base.id) {
                        Some(ToolDisplayState::Running) => Color::Yellow,
                        Some(ToolDisplayState::Completed(_)) => Color::Green,
                        Some(ToolDisplayState::Error(_)) => Color::Red,
                        None => match &tc.state {
                            ToolState::Pending(_) => Color::Yellow,
                            ToolState::Running(_) => Color::Yellow,
                            ToolState::Completed(_) => Color::Green,
                            ToolState::Error(_) => Color::Red,
                        },
                    };

                    text.push_line(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(
                            format!("{} {}", state_label, tc.tool_name),
                            Style::default().fg(state_color).add_modifier(Modifier::BOLD),
                        ),
                    ]));

                    // Show output if completed or errored.
                    if let Some(ToolDisplayState::Completed(out)) = app.tool_states.get(&tc.base.id)
                    {
                        if !out.is_empty() {
                            for line in out.lines() {
                                text.push_line(Line::from(vec![
                                    Span::styled("    ", Style::default()),
                                    Span::raw(line),
                                ]));
                            }
                        }
                    }
                    if let Some(ToolDisplayState::Error(err)) = app.tool_states.get(&tc.base.id) {
                        if !err.is_empty() {
                            for line in err.lines() {
                                text.push_line(Line::from(vec![
                                    Span::styled("    ", Style::default()),
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.streaming {
                    " Chat — streaming... "
                } else {
                    " Chat "
                }),
        )
        .wrap(Wrap { trim: true })
        .scroll((app.scroll, 0));

    f.render_widget(paragraph, area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let style = match app.mode {
        Mode::SlashCommand => Style::default().fg(Color::Yellow),
        _ => Style::default(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(" Input ");

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Truncate input to fit the width and show cursor.
    let width = inner.width as usize;
    let visible = if app.input.len() <= width {
        &app.input
    } else {
        let start = app.cursor.saturating_sub(width / 2);
        let end = (start + width).min(app.input.len());
        &app.input[start..end]
    };

    let paragraph = Paragraph::new(visible).style(style);
    f.render_widget(paragraph, inner);

    // Position cursor.
    let cursor_x = inner.x + (app.cursor.min(width) as u16);
    let cursor_y = inner.y;
    f.set_cursor_position((cursor_x, cursor_y));
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let status = &app.status;
    let left = format!(
        " {} | {}",
        status.provider,
        status.model
    );
    let right = format!(
        "tokens: {} in / {} out | cost: ${:.4} | session: {}...",
        status.input_tokens,
        status.output_tokens,
        status.cost,
        &status.session_id[..8.min(status.session_id.len())]
    );

    let spans = vec![
        Span::styled(left, Style::default().fg(Color::White)),
        Span::raw(" "),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ];

    let paragraph = Paragraph::new(Line::from(spans))
        .alignment(Alignment::Left);
    f.render_widget(paragraph, area);
}

fn draw_permission_modal(f: &mut Frame, perm: &PermissionState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 12u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    // Clear background.
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Permission Request ")
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let tool_input = serde_json::to_string_pretty(&perm.input).unwrap_or_default();
    let text = Text::from(vec![
        Line::from(vec![
            Span::styled("Tool: ", Style::default().fg(Color::Yellow)),
            Span::styled(&perm.tool_name, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Input: ", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(tool_input),
        Line::from(""),
        Line::from("Select an option:"),
    ]);

    let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(paragraph, inner);

    // Draw options at the bottom of the popup.
    let options = ["[A]llow once", "[S]ession", "[D]eny"];
    let option_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );

    let mut spans = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let style = if i == perm.selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(*opt, style));
        spans.push(Span::raw("  "));
    }

    let options_para = Paragraph::new(Line::from(spans));
    f.render_widget(options_para, option_area);
}
