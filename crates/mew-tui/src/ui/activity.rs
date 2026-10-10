use crate::app::{ActivityTarget, App, BackgroundJobStatus, DecisionLayout, SubagentStatus};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

pub(super) fn draw_activity(f: &mut Frame, app: &mut App, area: Rect) {
    app.prune_sidebar_state();
    let Some(view) = app.activity_view.as_mut() else {
        return;
    };
    let width = DecisionLayout::content_width(area.width, 76);
    let mut rows = Vec::new();
    let (title, status) = if view.session_id != app.status.session_id {
        detail(
            &mut rows,
            "Activity",
            "Session changed; close and reopen activity.",
            width,
        );
        ("Activity", "Unavailable")
    } else {
        match &view.target {
            ActivityTarget::Subagent(id) => {
                let state = app.subagents.iter().find(|state| &state.task_id == id);
                if let Some(state) = state {
                    let name = state
                        .display_name
                        .as_ref()
                        .map(|name| format!("{name} · {}", state.name));
                    detail(
                        &mut rows,
                        "Agent",
                        name.as_deref().unwrap_or(&state.name),
                        width,
                    );
                    let status = match &state.status {
                        SubagentStatus::Running => "Running",
                        SubagentStatus::Completed => "Completed",
                        SubagentStatus::Cancelled => "Cancelled",
                        SubagentStatus::Failed { .. } => "Failed",
                    };
                    if let Some(progress) = &state.last_progress {
                        detail(&mut rows, "Latest progress", progress, width);
                    }
                    if let SubagentStatus::Failed { reason } = &state.status {
                        detail(&mut rows, "Failure", reason, width);
                    }
                    detail(
                        &mut rows,
                        "Elapsed",
                        &elapsed(state.started_at, state.finished_at),
                        width,
                    );
                    detail(&mut rows, "Task", &state.task_id, width);
                    ("Subagent", status)
                } else {
                    detail(&mut rows, "Task", id, width);
                    detail(
                        &mut rows,
                        "Activity",
                        "This subagent is no longer available.",
                        width,
                    );
                    ("Subagent", "Unavailable")
                }
            }
            ActivityTarget::Job(id) => {
                let state = app.background_jobs.iter().find(|state| &state.job_id == id);
                if let Some(state) = state {
                    detail(&mut rows, "Command", &state.command, width);
                    detail(&mut rows, "Job", &state.job_id, width);
                    let status = match state.status {
                        BackgroundJobStatus::Running => "Running",
                        BackgroundJobStatus::Completed => "Completed",
                        BackgroundJobStatus::Cancelled => "Cancelled",
                        BackgroundJobStatus::Failed => "Failed",
                    };
                    detail(
                        &mut rows,
                        "Elapsed",
                        &elapsed(state.started_at, state.finished_at),
                        width,
                    );
                    ("Job", status)
                } else {
                    detail(&mut rows, "Job", id, width);
                    detail(
                        &mut rows,
                        "Activity",
                        "This job is no longer available.",
                        width,
                    );
                    ("Job", "Unavailable")
                }
            }
        }
    };
    let layout = DecisionLayout::new(area, 76, rows.len(), 0);
    layout.clamp_scroll(&mut view.scroll, &mut view.view, rows.len());
    let surface = app.theme.resolve("panel.overlay");
    let fg = app.theme.resolve("text.body");
    let accent = app.theme.resolve(if status == "Failed" {
        "text.error"
    } else {
        "text.accent"
    });
    f.render_widget(Clear, layout.popup);
    f.render_widget(
        Block::bordered()
            .title(Span::styled(
                format!(" {title} · {status} "),
                Style::default().fg(fg).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(surface)),
        layout.popup,
    );
    f.render_widget(
        Paragraph::new(rows)
            .style(Style::default().fg(fg).bg(surface))
            .scroll((view.scroll, 0)),
        layout.body,
    );
    let hint = if view.view.max_scroll > 0 && layout.hint.width >= 30 {
        "↑↓/PgUp/PgDn scroll · Esc close"
    } else if view.view.max_scroll > 0 && layout.hint.width >= 16 {
        "↑↓ · Esc close"
    } else {
        "Esc close"
    };
    f.render_widget(
        Paragraph::new(hint).style(
            Style::default()
                .fg(app.theme.resolve("text.muted"))
                .bg(surface),
        ),
        layout.hint,
    );
    if view.view.max_scroll > 0 && layout.body.height > 0 {
        let mut state =
            ScrollbarState::new(view.view.max_scroll as usize + 1).position(view.scroll as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(Style::default().bg(surface))
                .thumb_style(
                    Style::default()
                        .fg(app.theme.resolve("text.muted"))
                        .bg(surface),
                ),
            Rect::new(
                layout.popup.right().saturating_sub(1),
                layout.body.y,
                1,
                layout.body.height,
            ),
            &mut state,
        );
    }
}

fn detail(rows: &mut Vec<Line<'static>>, label: &str, value: &str, width: u16) {
    if !rows.is_empty() {
        rows.push(Line::default());
    }
    rows.push(Line::from(Span::styled(
        label.to_owned(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for logical in value.split('\n') {
        rows.extend(
            super::chat::wrap_text_to_width(logical, width)
                .into_iter()
                .map(Line::from),
        );
    }
}

fn elapsed(start: std::time::Instant, end: Option<std::time::Instant>) -> String {
    let seconds = end
        .unwrap_or_else(std::time::Instant::now)
        .saturating_duration_since(start)
        .as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}
