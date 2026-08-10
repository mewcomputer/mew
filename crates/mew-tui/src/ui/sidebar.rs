use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::app::App;

pub(super) fn draw_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    app.sidebar_rect = area;
    app.sidebar_header_rows.clear();
    // Content renders into the padded inner rect below, so row tracking
    // starts one line down to stay aligned with what the user sees (the
    // click handler compares these rows against mouse coordinates). Every
    // pushed line is truncated to the inner width so `Wrap` below never
    // produces extra visual lines; if that invariant breaks, headers drift
    // away from their recorded rows and clicks miss.
    let mut visual_row = area.y + 1;
    let mut text = Text::default();

    // Session header: always on top.
    draw_session_header(&mut text, &mut visual_row, app, area.width);

    // Activity sections render only when they have content.
    if !app.todos.is_empty() {
        push_divider(&mut text, &mut visual_row, app, area.width);
        draw_todos_section(&mut text, &mut visual_row, app, area.width);
    }

    draw_companion_section(&mut text, &mut visual_row, app, area.width);

    if !app.subagents.is_empty() {
        push_divider(&mut text, &mut visual_row, app, area.width);
        draw_subagents_section(&mut text, &mut visual_row, app, area.width);
    }

    if !app.background_jobs.is_empty() {
        push_divider(&mut text, &mut visual_row, app, area.width);
        draw_jobs_section(&mut text, &mut visual_row, app, area.width);
    }

    if !app.change_stats.files.is_empty() || !app.flagged_files.is_empty() {
        push_divider(&mut text, &mut visual_row, app, area.width);
        draw_changes_section(&mut text, &mut visual_row, app, area.width);
    }

    draw_environment_section(&mut text, &mut visual_row, app, area.width);

    let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    f.render_widget(paragraph, inner);
}

/// Truncate `s` to at most `max_cols` display columns, appending an ellipsis
/// inside the budget when truncation happens. Display width (not char count)
/// drives the cut so wide glyphs can't overflow the sidebar.
fn truncate_to_fit(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if super::display_width(s) <= max_cols {
        return s.to_string();
    }
    let limit = max_cols - 1; // reserve one column for the ellipsis
    let mut out = String::new();
    let mut cols = 0;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if cols + w > limit {
            break;
        }
        out.push(ch);
        cols += w;
    }
    out.push('…');
    out
}

fn push_divider(text: &mut Text, visual_row: &mut u16, app: &mut App, width: u16) {
    text.push_line(Line::from(Span::styled(
        "─".repeat(width.saturating_sub(2) as usize),
        Style::default().fg(app.theme.resolve("divider")),
    )));
    *visual_row += 1;
}

/// Session header at the top of the sidebar: the current session's title
/// (when known), then id and message count.
fn draw_session_header(text: &mut Text, visual_row: &mut u16, app: &mut App, width: u16) {
    let active_id = app.status.session_id.clone();
    let title = app
        .session_titles
        .get(&active_id)
        .cloned()
        .or_else(|| {
            app.daemon_sessions
                .iter()
                .find(|s| s.session_id == active_id)
                .and_then(|s| s.summary.clone().or_else(|| s.first_message.clone()))
        })
        .filter(|t| !t.is_empty());
    let max = width.saturating_sub(2) as usize;
    if let Some(title) = title {
        text.push_line(Line::from(Span::styled(
            truncate_to_fit(&title, max),
            Style::default()
                .fg(app.theme.resolve("text.body"))
                .add_modifier(Modifier::BOLD),
        )));
        *visual_row += 1;
    }
    text.push_line(Line::from(vec![
        Span::styled(
            "  id  ",
            Style::default().fg(app.theme.resolve("text.muted")),
        ),
        Span::styled(
            active_id[..8.min(active_id.len())].to_string(),
            Style::default().fg(app.theme.resolve("text.placeholder")),
        ),
    ]));
    *visual_row += 1;
    text.push_line(Line::from(vec![
        Span::styled(
            "  msg ",
            Style::default().fg(app.theme.resolve("text.muted")),
        ),
        Span::styled(
            format!("{}", app.messages.len()),
            Style::default().fg(app.theme.resolve("text.placeholder")),
        ),
    ]));
    *visual_row += 1;
}

fn draw_todos_section(text: &mut Text, visual_row: &mut u16, app: &mut App, area_width: u16) {
    let collapsed = app.sidebar_collapsed.get("todos").copied().unwrap_or(false);
    let arrow = if collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows.push((*visual_row, "todos".into()));
    let todo_total = app.todos.len();
    let todo_done = app
        .todos
        .iter()
        .filter(|t| t.status == mew_agent::TodoStatus::Done)
        .count();
    text.push_line(Line::from(vec![
        Span::styled(arrow, Style::default().fg(app.theme.resolve("text.muted"))),
        Span::styled(
            format!(" Todos ({}/{})", todo_done, todo_total),
            Style::default()
                .fg(app.theme.resolve("text.body"))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    *visual_row += 1;

    if collapsed {
        return;
    }

    // `  [x] ` (6) + `#` + id + ` ` precede the label.
    let inner = area_width.saturating_sub(2) as usize;
    for t in &app.todos {
        let (mark, color) = match t.status {
            mew_agent::TodoStatus::Done => ("x", app.theme.resolve("text.muted")),
            mew_agent::TodoStatus::InProgress => ("~", app.theme.resolve("text.warning")),
            mew_agent::TodoStatus::Pending => (" ", app.theme.resolve("text.placeholder")),
            mew_agent::TodoStatus::Blocked => ("!", app.theme.resolve("text.error")),
        };
        let id_str = t.id.to_string();
        let fixed = 6 + 1 + id_str.len() + 1;
        let budget = inner.saturating_sub(fixed);
        text.push_line(Line::from(vec![
            Span::styled(format!("  [{}] ", mark), Style::default().fg(color)),
            Span::styled(
                format!("#{} {}", id_str, truncate_to_fit(&t.content, budget)),
                Style::default().fg(color),
            ),
        ]));
        *visual_row += 1;
    }
}

fn draw_companion_section(text: &mut Text, visual_row: &mut u16, app: &mut App, width: u16) {
    let info = app.plugin_ui.get("buddy/info").cloned().unwrap_or_default();
    if info.trim().is_empty() {
        return;
    }
    push_divider(text, visual_row, app, width);

    let collapsed = app
        .sidebar_collapsed
        .get("companion")
        .copied()
        .unwrap_or(false);
    let arrow = if collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows
        .push((*visual_row, "companion".into()));
    text.push_line(Line::from(vec![
        Span::styled(arrow, Style::default().fg(app.theme.resolve("text.muted"))),
        Span::styled(
            " Companion",
            Style::default()
                .fg(app.theme.resolve("text.body"))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    *visual_row += 1;

    if !collapsed {
        let budget = width.saturating_sub(2) as usize - 2; // two-space indent
        for line in info.lines().take(12) {
            text.push_line(Line::from(Span::styled(
                format!("  {}", truncate_to_fit(line, budget)),
                Style::default().fg(app.theme.resolve("text.placeholder")),
            )));
        }
        *visual_row += info.lines().count().min(12) as u16;
    }
}

fn draw_subagents_section(text: &mut Text, visual_row: &mut u16, app: &mut App, area_width: u16) {
    let inner = area_width.saturating_sub(2) as usize;
    text.push_line(Line::from(vec![Span::styled(
        "Subagents",
        Style::default()
            .fg(app.theme.resolve("text.body"))
            .add_modifier(Modifier::BOLD),
    )]));
    *visual_row += 1;
    for sa in &app.subagents {
        let elapsed = sa.started_at.elapsed().as_secs();
        let (icon, color) = match &sa.status {
            crate::app::SubagentStatus::Running => ("▸", app.theme.resolve("text.warning")),
            crate::app::SubagentStatus::Completed => ("✓", app.theme.resolve("text.success")),
            crate::app::SubagentStatus::Failed { .. } => ("✗", app.theme.resolve("text.error")),
            crate::app::SubagentStatus::Cancelled => ("⊘", app.theme.resolve("text.muted")),
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
        let name_suffix = match &sa.display_name {
            Some(_) => format!("  ({})", sa.name),
            None => String::new(),
        };
        let time_suffix = format!("  {}", time_str);
        // `  ▸ ` prefix (4), then the name gets whatever remains after the
        // suffix, time, and status columns.
        let fixed = 4
            + super::display_width(&name_suffix)
            + super::display_width(&time_suffix)
            + super::display_width(&status_label);
        let shown = sa.display_name.as_deref().unwrap_or(&sa.name);
        text.push_line(Line::from(vec![
            Span::styled(format!("  {} ", icon), Style::default().fg(color)),
            Span::styled(
                truncate_to_fit(shown, inner.saturating_sub(fixed)),
                Style::default().fg(app.theme.resolve("text.placeholder")),
            ),
            Span::styled(
                name_suffix,
                Style::default().fg(app.theme.resolve("text.muted")),
            ),
            Span::styled(
                time_suffix,
                Style::default().fg(app.theme.resolve("text.muted")),
            ),
            Span::styled(status_label, Style::default().fg(color)),
        ]));
        *visual_row += 1;
        if let Some(progress) = &sa.last_progress {
            // The full message is still in self.messages if the user needs it.
            // `    ↳ ` prefix is 6 columns.
            let budget = inner.saturating_sub(6);
            text.push_line(Line::from(Span::styled(
                format!("    ↳ {}", truncate_to_fit(progress, budget)),
                Style::default().fg(app.theme.resolve("text.muted")),
            )));
            *visual_row += 1;
        }
    }
}

fn draw_jobs_section(text: &mut Text, visual_row: &mut u16, app: &mut App, area_width: u16) {
    let inner = area_width.saturating_sub(2) as usize;
    text.push_line(Line::from(vec![Span::styled(
        "Background Jobs",
        Style::default()
            .fg(app.theme.resolve("text.body"))
            .add_modifier(Modifier::BOLD),
    )]));
    *visual_row += 1;
    for job in &app.background_jobs {
        let elapsed = job.started_at.elapsed().as_secs();
        let (icon, color) = match &job.status {
            crate::app::BackgroundJobStatus::Running => ("▸", app.theme.resolve("text.warning")),
            crate::app::BackgroundJobStatus::Completed => ("✓", app.theme.resolve("text.success")),
            crate::app::BackgroundJobStatus::Failed => ("✗", app.theme.resolve("text.error")),
            crate::app::BackgroundJobStatus::Cancelled => ("⊘", app.theme.resolve("text.muted")),
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
        let time_suffix = format!("  {}", time_str);
        // `  ▸ ` prefix (4), then the command gets the remainder.
        let fixed = 4 + super::display_width(&time_suffix) + super::display_width(&status_label);
        text.push_line(Line::from(vec![
            Span::styled(format!("  {} ", icon), Style::default().fg(color)),
            Span::styled(
                truncate_to_fit(&job.command, inner.saturating_sub(fixed)),
                Style::default().fg(app.theme.resolve("text.placeholder")),
            ),
            Span::styled(
                time_suffix,
                Style::default().fg(app.theme.resolve("text.muted")),
            ),
            Span::styled(status_label, Style::default().fg(color)),
        ]));
        *visual_row += 1;
    }
}

fn draw_changes_section(text: &mut Text, visual_row: &mut u16, app: &mut App, area_width: u16) {
    let inner = area_width.saturating_sub(2) as usize;
    text.push_line(Line::from(vec![
        Span::styled(
            "Changes ",
            Style::default()
                .fg(app.theme.resolve("text.body"))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "(+{} −{})",
                app.change_stats.added, app.change_stats.removed
            ),
            Style::default().fg(app.theme.resolve("text.muted")),
        ),
    ]));
    *visual_row += 1;
    // Union of changed files and flagged files (flagged first, with ⚑).
    let mut rows: Vec<(String, bool)> = Vec::new();
    for f in &app.flagged_files {
        rows.push((f.path.clone(), true));
    }
    for f in &app.change_stats.files {
        if !app.flagged_files.iter().any(|ff| &ff.path == f) {
            rows.push((f.clone(), false));
        }
    }
    // `  ⚑ ` / `    ` marker prefix is 4 columns.
    let budget = inner.saturating_sub(4);
    for (path, flagged) in rows.iter().take(10) {
        let (marker, color) = if *flagged {
            ("⚑ ", app.theme.resolve("text.warning"))
        } else {
            ("  ", app.theme.resolve("text.placeholder"))
        };
        text.push_line(Line::from(vec![
            Span::styled(format!("  {}", marker), Style::default().fg(color)),
            Span::styled(truncate_to_fit(path, budget), Style::default().fg(color)),
        ]));
        *visual_row += 1;
    }
    if rows.len() > 10 {
        text.push_line(Line::from(Span::styled(
            format!("    … {} more", rows.len() - 10),
            Style::default().fg(app.theme.resolve("text.muted")),
        )));
        *visual_row += 1;
    }
}

/// Static environment info (loaded context files, MCP server status) in a
/// single section, collapsed by default so it stays out of the way.
fn draw_environment_section(text: &mut Text, visual_row: &mut u16, app: &mut App, width: u16) {
    if app.context_files.is_empty() && app.mcp_status.is_empty() {
        return;
    }
    push_divider(text, visual_row, app, width);

    let collapsed = app
        .sidebar_collapsed
        .get("environment")
        .copied()
        .unwrap_or_else(|| App::sidebar_default_collapsed("environment"));
    let arrow = if collapsed { "▶" } else { "▼" };
    app.sidebar_header_rows
        .push((*visual_row, "environment".into()));
    text.push_line(Line::from(vec![
        Span::styled(arrow, Style::default().fg(app.theme.resolve("text.muted"))),
        Span::styled(
            " Environment",
            Style::default()
                .fg(app.theme.resolve("text.body"))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    *visual_row += 1;

    if collapsed {
        return;
    }

    let inner = width.saturating_sub(2) as usize;
    for path in &app.context_files.clone() {
        text.push_line(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                truncate_to_fit(path, inner.saturating_sub(2)),
                Style::default().fg(app.theme.resolve("text.placeholder")),
            ),
        ]));
        *visual_row += 1;
    }

    // `  ✓ ` prefix is 4 columns.
    let budget = inner.saturating_sub(4);
    for (name, ok, count) in &app.mcp_status.clone() {
        let (icon, style) = if *ok {
            ("✓", Style::default().fg(app.theme.resolve("text.success")))
        } else {
            ("✗", Style::default().fg(app.theme.resolve("text.error")))
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
            Span::styled(
                truncate_to_fit(&label, budget),
                Style::default().fg(app.theme.resolve("text.placeholder")),
            ),
        ]));
        *visual_row += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render the sidebar into a test buffer and return it as text lines.
    fn render_lines(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("create test backend");
        terminal
            .draw(|f| {
                let area = f.area();
                draw_sidebar(f, app, area);
            })
            .expect("draw sidebar");
        let buffer = terminal.backend().buffer().clone();
        let mut lines = Vec::new();
        for y in 0..height {
            let mut line = String::new();
            for x in 0..width {
                line.push_str(buffer[(x, y)].symbol());
            }
            lines.push(line.trim_end().to_string());
        }
        lines
    }

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let mut out = render_lines(app, width, height).join("\n");
        out.push('\n');
        out
    }

    #[test]
    fn test_truncate_to_fit() {
        assert_eq!(truncate_to_fit("short", 10), "short");
        assert_eq!(truncate_to_fit("exactly-10", 10), "exactly-10");
        assert_eq!(truncate_to_fit("overlong content", 10), "overlong …");
        assert_eq!(super::super::display_width("overlong …"), 10);
        assert_eq!(truncate_to_fit("anything", 0), "");
        // Wide glyphs must not overflow the budget.
        assert_eq!(truncate_to_fit("日本語のテキスト", 6), "日本…");
        assert_eq!(super::super::display_width("日本…"), 5);
    }

    #[test]
    fn test_sidebar_empty_app_hides_all_sections() {
        let mut app = App::new();
        app.status.session_id = "abcdef1234567890".into();
        let out = render(&mut app, 32, 40);
        // Session header is always present.
        assert!(out.contains("abcdef12"), "missing session id:\n{out}");
        // Empty sections render nothing at all.
        for absent in [
            "Todos",
            "Tools",
            "Personas",
            "MCP Servers",
            "Context",
            "Sessions",
            "Environment",
            "Companion",
            "no todos yet",
            "No context files loaded",
        ] {
            assert!(!out.contains(absent), "unexpected '{absent}':\n{out}");
        }
    }

    #[test]
    fn test_sidebar_session_header_shows_title_and_count() {
        let mut app = App::new();
        app.status.session_id = "abcdef1234567890".into();
        app.session_titles
            .insert("abcdef1234567890".into(), "refactor sidebar".into());
        let out = render(&mut app, 32, 40);
        assert!(out.contains("refactor sidebar"), "missing title:\n{out}");
        assert!(out.contains("msg 0"), "missing message count:\n{out}");
    }

    #[test]
    fn test_sidebar_session_header_falls_back_to_summary() {
        let mut app = App::new();
        app.status.session_id = "abcdef1234567890".into();
        app.daemon_sessions.push(mew_protocol::SessionInfo {
            session_id: "abcdef1234567890".into(),
            state: mew_protocol::SessionState::Idle,
            model: None,
            provider: None,
            created_at: 0,
            last_message_at: None,
            summary: Some("summarized turn".into()),
            client_count: 0,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            pinned: false,
            group_id: None,
            change_stats: None,
            usage: None,
            context_tokens: None,
            pending_permissions: 0,
            pending_questions: 0,
            first_message: None,
        });
        let out = render(&mut app, 32, 40);
        assert!(out.contains("summarized turn"), "missing summary:\n{out}");
    }

    #[test]
    fn test_sidebar_todos_visible_when_present() {
        let mut app = App::new();
        app.status.session_id = "abcdef1234567890".into();
        app.todos.push(mew_agent::Todo {
            id: 1,
            content: "write tests".into(),
            status: mew_agent::TodoStatus::InProgress,
            depends_on: Vec::new(),
        });
        let out = render(&mut app, 32, 40);
        assert!(out.contains("Todos (0/1)"), "missing header:\n{out}");
        assert!(out.contains("write tests"), "missing todo:\n{out}");
    }

    #[test]
    fn test_sidebar_environment_collapsed_by_default_and_togglable() {
        let mut app = App::new();
        app.status.session_id = "abcdef1234567890".into();
        app.context_files = vec!["AGENTS.md".into()];
        app.mcp_status = vec![("context7".into(), true, 3)];

        let collapsed = render(&mut app, 32, 40);
        assert!(
            collapsed.contains("Environment"),
            "missing header:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("AGENTS.md"),
            "content should be hidden:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("context7"),
            "content should be hidden:\n{collapsed}"
        );

        app.toggle_sidebar_section("environment");
        let expanded = render(&mut app, 32, 40);
        assert!(
            expanded.contains("AGENTS.md"),
            "missing context file:\n{expanded}"
        );
        assert!(
            expanded.contains("context7 (3 tools)"),
            "missing mcp row:\n{expanded}"
        );
    }

    #[test]
    fn test_sidebar_tools_and_personas_never_rendered() {
        let mut app = App::new();
        app.status.session_id = "abcdef1234567890".into();
        app.tools = vec!["read".into(), "write".into()];
        app.personas = vec![("builder".into(), "builds things".into())];
        let out = render(&mut app, 32, 40);
        assert!(
            !out.contains("Tools"),
            "tools section should be gone:\n{out}"
        );
        assert!(
            !out.contains("Personas"),
            "personas section should be gone:\n{out}"
        );
        assert!(
            !out.contains("builder"),
            "persona list should be gone:\n{out}"
        );
    }

    /// The click handler toggles the section whose recorded header row equals
    /// the clicked row, so every recorded row must point at the rendered
    /// header line. Long content above a header must not shift it down via
    /// wrapping.
    #[test]
    fn test_sidebar_header_rows_align_with_rendered_headers() {
        let mut app = App::new();
        app.status.session_id = "abcdef1234567890".into();
        // Fill every section with content long enough to wrap if the
        // truncation budgets were wrong.
        app.todos.push(mew_agent::Todo {
            id: 42,
            content: "a very long todo item that would wrap if not truncated properly".into(),
            status: mew_agent::TodoStatus::Pending,
            depends_on: Vec::new(),
        });
        app.plugin_ui.insert(
            "buddy/info".into(),
            "a companion line that is also far too long for the sidebar width".into(),
        );
        app.change_stats
            .files
            .push("some/deeply/nested/path/to/a/file/that/keeps/going/and/going.rs".into());
        app.context_files
            .push("a/really/long/context/file/path/AGENTS.md".into());
        app.mcp_status = vec![("a-server-with-a-very-long-name".into(), true, 42)];
        app.toggle_sidebar_section("environment");

        let lines = render_lines(&mut app, 32, 60);
        let headers = app.sidebar_header_rows.clone();
        assert!(
            headers.len() >= 3,
            "expected todos/companion/environment headers, got {headers:?}"
        );
        for (row, section) in &headers {
            let line = lines
                .get(*row as usize)
                .unwrap_or_else(|| panic!("row {row} out of bounds"));
            let expected = match section.as_str() {
                "todos" => "Todos",
                "companion" => "Companion",
                "environment" => "Environment",
                other => panic!("unexpected section {other}"),
            };
            assert!(
                line.contains(expected),
                "header for '{section}' recorded at row {row}, but that row is: {line:?}"
            );
        }
    }
}
