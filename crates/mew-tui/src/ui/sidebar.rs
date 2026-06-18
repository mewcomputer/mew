use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
    Frame,
};

use super::DIVIDER;
use crate::app::App;

pub(super) fn draw_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    app.sidebar_rect = area;
    app.sidebar_header_rows.clear();
    let mut visual_row = area.y;
    let mut text = Text::default();

    // Context
    let ctx_collapsed = app
        .sidebar_collapsed
        .get("context")
        .copied()
        .unwrap_or(false);
    let ctx_arrow = if ctx_collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows.push((visual_row, "context".into()));
    visual_row += 1;
    text.push_line(Line::from(vec![
        Span::styled(ctx_arrow, Style::default().fg(Color::DarkGray)),
        Span::styled(
            " Context",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if !ctx_collapsed {
        if app.context_files.is_empty() {
            text.push_line(Line::from(Span::styled(
                "  No context files loaded",
                Style::default().fg(Color::DarkGray),
            )));
            visual_row += 1;
        } else {
            let ctx = app.context_files.clone();
            for path in &ctx {
                text.push_line(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(path.clone(), Style::default().fg(Color::Gray)),
                ]));
            }
            visual_row += ctx.len() as u16;
        }
    }

    text.push_line(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(DIVIDER),
    )));
    visual_row += 1;

    // Companion (buddy plugin)
    draw_companion_section(&mut text, &mut visual_row, app, area.width);

    // Tools
    let tools_collapsed = app.sidebar_collapsed.get("tools").copied().unwrap_or(false);
    let tools_arrow = if tools_collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows.push((visual_row, "tools".into()));
    visual_row += 1;
    text.push_line(Line::from(vec![
        Span::styled(tools_arrow, Style::default().fg(Color::DarkGray)),
        Span::styled(
            " Tools",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if !tools_collapsed {
        if app.tools.is_empty() {
            text.push_line(Line::from(Span::styled(
                "  No tools available",
                Style::default().fg(Color::DarkGray),
            )));
            visual_row += 1;
        } else {
            let tools: Vec<String> = app.tools.clone();
            for tool in &tools {
                text.push_line(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(tool.clone(), Style::default().fg(Color::Gray)),
                ]));
            }
            visual_row += tools.len() as u16;
        }
    }

    text.push_line(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(DIVIDER),
    )));
    visual_row += 1;

    // MCP Servers
    let mcp_collapsed = app.sidebar_collapsed.get("mcp").copied().unwrap_or(false);
    let mcp_arrow = if mcp_collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows.push((visual_row, "mcp".into()));
    text.push_line(Line::from(vec![
        Span::styled(mcp_arrow, Style::default().fg(Color::DarkGray)),
        Span::styled(
            " MCP Servers",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if !mcp_collapsed {
        if app.mcp_status.is_empty() {
            text.push_line(Line::from(Span::styled(
                "  No MCP servers configured",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let mcp = app.mcp_status.clone();
            for (name, ok, count) in &mcp {
                let (icon, style) = if *ok {
                    ("✓", Style::default().fg(Color::Green))
                } else {
                    ("✗", Style::default().fg(Color::Red))
                };
                let label = if *ok && *count > 0 {
                    format!("{} ({} tools)", name, count)
                } else if *ok {
                    name.clone()
                } else {
                    format!("{} (offline)", name)
                };
                text.push_line(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(icon, style),
                    Span::styled(" ", Style::default()),
                    Span::styled(label, Style::default().fg(Color::Gray)),
                ]));
            }
        }
    }

    text.push_line(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(DIVIDER),
    )));

    // Subagents
    if !app.subagents.is_empty() {
        text.push_line(Line::from(vec![Span::styled(
            "Subagents",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));
        for sa in &app.subagents {
            let elapsed = sa.started_at.elapsed().as_secs();
            let (icon, color) = match &sa.status {
                crate::app::SubagentStatus::Running => ("▸", Color::Yellow),
                crate::app::SubagentStatus::Completed => ("✓", Color::Green),
                crate::app::SubagentStatus::Failed { .. } => ("✗", Color::Red),
                crate::app::SubagentStatus::Cancelled => ("⊘", Color::DarkGray),
            };
            let time_str = if elapsed < 60 {
                format!("{}s", elapsed)
            } else {
                format!("{}m", elapsed / 60)
            };
            let status_label = match &sa.status {
                crate::app::SubagentStatus::Running => String::new(),
                crate::app::SubagentStatus::Completed => "  done".to_string(),
                crate::app::SubagentStatus::Failed { reason } => {
                    let one_line = reason.lines().next().unwrap_or("").to_string();
                    format!("  failed: {}", one_line)
                }
                crate::app::SubagentStatus::Cancelled => "  cancelled".to_string(),
            };
            text.push_line(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(
                    sa.display_name.as_deref().unwrap_or(&sa.name).to_string(),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    match &sa.display_name {
                        Some(_) => format!("  ({})", sa.name),
                        None => "".to_string(),
                    },
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("  {}", time_str),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(status_label, Style::default().fg(color)),
            ]));
            if let Some(progress) = &sa.last_progress {
                // Truncate to keep the sidebar narrow. The full message
                // is still in self.messages if the user needs it.
                let max = area.width.saturating_sub(8) as usize;
                let truncated: String = progress.chars().take(max).collect();
                let line = if progress.chars().count() > max {
                    format!("    ↳ {}…", truncated)
                } else {
                    format!("    ↳ {}", truncated)
                };
                text.push_line(Line::from(Span::styled(
                    line,
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        text.push_line(Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(2) as usize),
            Style::default().fg(DIVIDER),
        )));
    }

    // Session
    text.push_line(Line::from(vec![Span::styled(
        "Session",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));
    text.push_line(Line::from(""));
    text.push_line(Line::from(vec![
        Span::styled("  id  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &app.status.session_id[..8.min(app.status.session_id.len())],
            Style::default().fg(Color::Gray),
        ),
    ]));
    text.push_line(Line::from(vec![
        Span::styled("  msg ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", app.messages.len()),
            Style::default().fg(Color::Gray),
        ),
    ]));

    let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    f.render_widget(paragraph, inner);
}

fn draw_companion_section(text: &mut Text, visual_row: &mut u16, app: &mut App, _width: u16) {
    let info = app.plugin_ui.get("buddy/info").cloned().unwrap_or_default();
    let collapsed = app
        .sidebar_collapsed
        .get("companion")
        .copied()
        .unwrap_or(false);

    let arrow = if collapsed { "▶" } else { "▼" };

    app.sidebar_header_rows
        .push((*visual_row, "companion".into()));
    *visual_row += 1;
    text.push_line(Line::from(vec![
        Span::styled(arrow, Style::default().fg(Color::DarkGray)),
        Span::styled(
            " Companion",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if !collapsed && info.is_empty() {
        text.push_line(Line::from(Span::styled(
            "  no companion loaded",
            Style::default().fg(Color::DarkGray),
        )));
        *visual_row += 1;
    } else if !collapsed {
        for line in info.lines().take(12) {
            text.push_line(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(Color::Gray),
            )));
        }
        *visual_row += info.lines().count().min(12) as u16;
    }
}
