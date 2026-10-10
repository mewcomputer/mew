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
    let width_cap = if matches!(view.target, ActivityTarget::Subagent(_)) {
        104
    } else {
        76
    };
    let width = DecisionLayout::content_width(area.width, width_cap);
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
                if let Some(state) = state.filter(|state| {
                    view.transcript.as_ref().is_none_or(|transcript| {
                        state.child_session_id == transcript.child_session_id
                    })
                }) {
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
                    rows.extend(
                        super::chat::wrap_text_to_width(
                            &format!(
                                "Task {} · {}",
                                state.task_id,
                                elapsed(state.started_at, state.finished_at)
                            ),
                            width,
                        )
                        .into_iter()
                        .map(Line::from),
                    );
                    if let Some(transcript) = &mut view.transcript {
                        if let Some(error) = &transcript.error {
                            detail(&mut rows, "Trace unavailable", error, width);
                        }
                        if transcript.messages.is_empty() && transcript.error.is_none() {
                            detail(
                                &mut rows,
                                "Recorded conversation",
                                if transcript.loaded {
                                    "No messages have been recorded."
                                } else {
                                    "Loading recorded turns. The unfinished turn may not appear yet."
                                },
                                width,
                            );
                        } else if !transcript.messages.is_empty() {
                            if !transcript
                                .rendered
                                .as_ref()
                                .is_some_and(|(rendered_width, _)| *rendered_width == width)
                            {
                                let mut lines = Vec::new();
                                transcript_rows(&mut lines, &transcript.messages, width);
                                transcript.rendered = Some((width, lines));
                            }
                            let (_, lines) = transcript.rendered.as_ref().unwrap();
                            rows.extend(lines.iter().map(|line| {
                                Line::from(
                                    line.spans
                                        .iter()
                                        .map(|span| Span::styled(span.content.as_ref(), span.style))
                                        .collect::<Vec<_>>(),
                                )
                            }));
                        }
                    }
                    if let Some(progress) = &state.last_progress {
                        detail(&mut rows, "Latest progress", progress, width);
                    }
                    if let SubagentStatus::Failed { reason } = &state.status {
                        detail(&mut rows, "Failure", reason, width);
                    }
                    ("Subagent", status)
                } else {
                    detail(&mut rows, "Task", id, width);
                    detail(
                        &mut rows,
                        "Activity",
                        if state.is_some() {
                            "The child changed; close and reopen this subagent."
                        } else {
                            "This subagent is no longer available."
                        },
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
    let mut layout = DecisionLayout::new(area, width_cap, rows.len() + 1, 0);
    let title_area = Rect::new(
        layout.body.x,
        layout.body.y,
        layout.body.width,
        u16::from(layout.body.height > 0),
    );
    layout.body.y += title_area.height;
    layout.body.height = layout.body.height.saturating_sub(title_area.height);
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
        Block::default().style(Style::default().bg(surface)),
        layout.popup,
    );
    f.render_widget(
        Paragraph::new(format!("{title} · {status}")).style(
            Style::default()
                .fg(accent)
                .bg(surface)
                .add_modifier(Modifier::BOLD),
        ),
        title_area,
    );
    f.render_widget(
        Paragraph::new(rows)
            .style(Style::default().fg(fg).bg(surface))
            .scroll((view.scroll, 0)),
        layout.body,
    );
    let trace = view.transcript.is_some();
    let hint = if trace && layout.hint.width >= 41 {
        "↑↓/PgUp/PgDn scroll · r refresh · Esc close"
    } else if trace && layout.hint.width >= 25 {
        "↑↓ · r refresh · Esc close"
    } else if trace && layout.hint.width >= 19 {
        "r refresh Esc close"
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

fn transcript_rows(rows: &mut Vec<Line<'static>>, messages: &[mew_message::Message], width: u16) {
    use mew_message::{Part, Role, ToolState};
    for (index, message) in messages.iter().enumerate() {
        for part in &message.parts {
            match part {
                Part::Text(text) if !text.text.is_empty() => detail(
                    rows,
                    match message.role {
                        Role::User if index == 0 => "Parent assignment",
                        Role::User => "Instructions",
                        Role::System => "System instructions",
                        Role::Assistant => "Subagent message",
                    },
                    &text.text,
                    width,
                ),
                Part::Reasoning(reasoning) if !reasoning.text.is_empty() => {
                    detail(rows, "Reasoning", &reasoning.text, width)
                }
                Part::ToolCall(call) => {
                    let state = match call.state {
                        ToolState::Pending(_) => "pending",
                        ToolState::Running(_) => "running",
                        ToolState::Completed(_) => "completed",
                        ToolState::Error(_) => "failed",
                    };
                    detail(
                        rows,
                        &format!("{} · {state}", call.tool_name),
                        &serde_json::to_string_pretty(call.state.input()).unwrap_or_default(),
                        width,
                    );
                    if let Some(output) = call.state.output().filter(|output| !output.is_empty()) {
                        detail(rows, "Result", output, width);
                    }
                    if let ToolState::Error(error) = &call.state {
                        detail(rows, "Error", &error.error, width);
                    }
                }
                Part::File(file) => detail(rows, "File", &file.url, width),
                Part::Compaction(compaction) => detail(
                    rows,
                    "Compaction",
                    compaction.summary.as_deref().unwrap_or("Context compacted"),
                    width,
                ),
                _ => {}
            }
        }
    }
}

fn detail<'a>(rows: &mut Vec<Line<'a>>, label: &str, value: &str, width: u16) {
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
