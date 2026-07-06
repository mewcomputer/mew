use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
    Frame,
};

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
        Style::default().fg(app.theme.tokens.divider),
    )));
    visual_row += 1;

    // Todos
    let todos_collapsed = app.sidebar_collapsed.get("todos").copied().unwrap_or(false);
    let todos_arrow = if todos_collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows.push((visual_row, "todos".into()));
    visual_row += 1;
    let todo_total = app.todos.len();
    let todo_done = app
        .todos
        .iter()
        .filter(|t| t.status == mew_agent::TodoStatus::Done)
        .count();
    text.push_line(Line::from(vec![
        Span::styled(todos_arrow, Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" Todos ({}/{})", todo_done, todo_total),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if !todos_collapsed {
        if app.todos.is_empty() {
            text.push_line(Line::from(Span::styled(
                "  no todos yet",
                Style::default().fg(Color::DarkGray),
            )));
            visual_row += 1;
        } else {
            let max_width = area.width.saturating_sub(8) as usize;
            for t in &app.todos {
                let (mark, color) = match t.status {
                    mew_agent::TodoStatus::Done => ("x", Color::DarkGray),
                    mew_agent::TodoStatus::InProgress => ("~", Color::Yellow),
                    mew_agent::TodoStatus::Pending => (" ", Color::Gray),
                    mew_agent::TodoStatus::Blocked => ("!", Color::Red),
                };
                let content: String = t.content.chars().take(max_width).collect();
                let label = if t.content.chars().count() > max_width {
                    format!("{}…", content)
                } else {
                    content
                };
                text.push_line(Line::from(vec![
                    Span::styled(format!("  [{}] ", mark), Style::default().fg(color)),
                    Span::styled(format!("#{} {}", t.id, label), Style::default().fg(color)),
                ]));
                visual_row += 1;
            }
        }
    }

    text.push_line(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(app.theme.tokens.divider),
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
        Style::default().fg(app.theme.tokens.divider),
    )));
    visual_row += 1;

    // Personas
    let personas_collapsed = app
        .sidebar_collapsed
        .get("personas")
        .copied()
        .unwrap_or(false);
    let personas_arrow = if personas_collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows
        .push((visual_row, "personas".into()));
    visual_row += 1;
    text.push_line(Line::from(vec![
        Span::styled(personas_arrow, Style::default().fg(Color::DarkGray)),
        Span::styled(
            " Personas",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if !personas_collapsed {
        if app.personas.is_empty() {
            text.push_line(Line::from(Span::styled(
                "  No personas loaded",
                Style::default().fg(Color::DarkGray),
            )));
            visual_row += 1;
        } else {
            // Active persona gets a "*" marker and the same purple as the
            // status bar pill so the two surfaces stay in visual sync.
            let active = app.active_persona.as_deref();
            let personas = app.personas.clone();
            for (name, desc) in &personas {
                let is_active = Some(name.as_str()) == active;
                let marker = if is_active { "* " } else { "  " };
                let max_desc = area.width.saturating_sub(8 + name.chars().count() as u16) as usize;
                let desc_clipped: String = desc.chars().take(max_desc).collect();
                let desc_str = if desc.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", desc_clipped)
                };
                // Active persona uses its accent color; inactive ones
                // stay gray.
                let (name_color, marker_color) = if is_active {
                    let accent =
                        crate::theme::persona_accent(name, app.active_persona_color.as_deref());
                    (accent.fg, accent.fg)
                } else {
                    (Color::Gray, Color::DarkGray)
                };
                text.push_line(Line::from(vec![
                    Span::styled(marker, Style::default().fg(marker_color)),
                    Span::styled(name.clone(), Style::default().fg(name_color)),
                    Span::styled(desc_str, Style::default().fg(Color::DarkGray)),
                ]));
                visual_row += 1;
            }
        }
    }

    text.push_line(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(app.theme.tokens.divider),
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
        Style::default().fg(app.theme.tokens.divider),
    )));

    // Sessions (daemon mode only)
    if app.daemon_mode {
        draw_sessions_section(&mut text, &mut visual_row, app, area.width);
    }

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
            Style::default().fg(app.theme.tokens.divider),
        )));
    }

    // Background Jobs
    if !app.background_jobs.is_empty() {
        text.push_line(Line::from(vec![Span::styled(
            "Background Jobs",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));
        for job in &app.background_jobs {
            let elapsed = job.started_at.elapsed().as_secs();
            let (icon, color) = match &job.status {
                crate::app::BackgroundJobStatus::Running => ("▸", Color::Yellow),
                crate::app::BackgroundJobStatus::Completed => ("✓", Color::Green),
                crate::app::BackgroundJobStatus::Failed => ("✗", Color::Red),
                crate::app::BackgroundJobStatus::Cancelled => ("⊘", Color::DarkGray),
            };
            let time_str = if elapsed < 60 {
                format!("{}s", elapsed)
            } else {
                format!("{}m", elapsed / 60)
            };
            let status_label = match &job.status {
                crate::app::BackgroundJobStatus::Running => String::new(),
                crate::app::BackgroundJobStatus::Completed => "  done".to_string(),
                crate::app::BackgroundJobStatus::Failed => "  failed".to_string(),
                crate::app::BackgroundJobStatus::Cancelled => "  cancelled".to_string(),
            };
            // Truncate the command to keep the sidebar narrow.
            let max = area.width.saturating_sub(10) as usize;
            let trimmed: String = job.command.chars().take(max).collect();
            let cmd = if job.command.chars().count() > max {
                format!("{}…", trimmed)
            } else {
                trimmed
            };
            text.push_line(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(cmd, Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("  {}", time_str),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(status_label, Style::default().fg(color)),
            ]));
        }
        text.push_line(Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(2) as usize),
            Style::default().fg(app.theme.tokens.divider),
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

/// Render the daemon "Sessions" sidebar section. Shows each non-archived
/// session from `daemon_sessions` with: state glyph, title (or summary
/// or short id), attention badge, cost. The active session is marked.
fn draw_sessions_section(text: &mut Text, visual_row: &mut u16, app: &mut App, width: u16) {
    let collapsed = app
        .sidebar_collapsed
        .get("sessions")
        .copied()
        .unwrap_or(false);
    let arrow = if collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows
        .push((*visual_row, "sessions".into()));
    *visual_row += 1;

    let count = app.daemon_sessions.iter().filter(|s| !s.archived).count();
    text.push_line(Line::from(vec![
        Span::styled(arrow, Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" Sessions ({})", count),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if collapsed {
        return;
    }

    if app.daemon_sessions.is_empty() {
        text.push_line(Line::from(Span::styled(
            "  no sessions",
            Style::default().fg(Color::DarkGray),
        )));
        *visual_row += 1;
    } else {
        let active_id = app.status.session_id.clone();
        let sessions = app.daemon_sessions.clone();
        let titles = app.session_titles.clone();
        let attention = app.session_attention.clone();
        let max_title = width.saturating_sub(14) as usize;

        for s in sessions.iter().filter(|s| !s.archived) {
            let title = titles
                .get(&s.session_id)
                .cloned()
                .or_else(|| s.summary.clone())
                .unwrap_or_else(|| s.session_id.chars().take(8).collect::<String>());
            let title_display: String = title.chars().take(max_title).collect();
            let title_str = if title.chars().count() > max_title {
                format!("{}…", title_display)
            } else {
                title_display
            };

            let (glyph, glyph_color) = match s.state {
                mew_protocol::SessionState::Running => ("▶", Color::Yellow),
                mew_protocol::SessionState::Active => ("●", Color::Green),
                mew_protocol::SessionState::Idle => ("○", Color::DarkGray),
            };

            let cost = s
                .usage
                .as_ref()
                .map(|u| format!("${:.2}", u.cost))
                .unwrap_or_default();

            // Attention badge: [!] for pending perms, [?] for pending questions.
            let badge = attention.get(&s.session_id);
            let (perm_n, quest_n) = badge.unwrap_or(&(0, 0));
            let badge_str = if *perm_n > 0 && *quest_n > 0 {
                format!(" [{}!{}?]", perm_n, quest_n)
            } else if *perm_n > 0 {
                format!(" [{}!]", perm_n)
            } else if *quest_n > 0 {
                format!(" [{}?]", quest_n)
            } else {
                String::new()
            };
            let badge_color = if *perm_n > 0 {
                Color::Yellow
            } else {
                Color::Cyan
            };

            let is_active = s.session_id == active_id;
            let marker = if is_active { "▸" } else { " " };
            let title_color = if is_active { Color::White } else { Color::Gray };

            text.push_line(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Green)),
                Span::styled(format!(" {} ", glyph), Style::default().fg(glyph_color)),
                Span::styled(title_str, Style::default().fg(title_color)),
                Span::styled(format!("  {}", cost), Style::default().fg(Color::DarkGray)),
                Span::styled(badge_str, Style::default().fg(badge_color)),
            ]));
            *visual_row += 1;
        }
    }

    text.push_line(Line::from(Span::styled(
        "─".repeat(width.saturating_sub(2) as usize),
        Style::default().fg(app.theme.tokens.divider),
    )));
    *visual_row += 1;
}
