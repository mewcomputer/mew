use crate::app::{ActivityTarget, App, BackgroundJobStatus, DecisionLayout, SubagentStatus};
use ansi_to_tui::IntoText;
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
    let surface = app.theme.resolve("panel.overlay");
    let width_cap = 104;
    let width = DecisionLayout::content_width(area.width, width_cap);
    let mut tail = Vec::new();
    let mut include_content = false;
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
                            cache_rows(&mut view.rendered, width, None, |rows| {
                                transcript_rows(rows, &transcript.messages, width, surface)
                            });
                            include_content = true;
                        }
                    }
                    if let Some(progress) = &state.last_progress {
                        detail(&mut tail, "Latest progress", progress, width);
                    }
                    if let SubagentStatus::Failed { reason } = &state.status {
                        detail(&mut tail, "Failure", reason, width);
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
                    if let Some(output) = &view.job_output {
                        if output.output.is_empty() && !output.done && output.error.is_none() {
                            detail(&mut rows, "Output", "Loading job output…", width);
                        } else if output.output.is_empty() && output.done {
                            detail(&mut rows, "Output", "No output was captured.", width);
                        } else {
                            rows.push(Line::default());
                            rows.push(Line::styled(
                                "Output",
                                Style::default().add_modifier(Modifier::BOLD),
                            ));
                            cache_rows(&mut view.rendered, width, None, |rows| {
                                output_rows(rows, &output.output, width, surface)
                            });
                            include_content = true;
                        }
                        if output.incomplete {
                            detail(&mut tail,"Output incomplete","A reader did not reach EOF after process exit. The captured output ends here.",width);
                        }
                        if let Some(error) = &output.error {
                            detail(&mut tail, "Output unavailable", error, width);
                        }
                    }
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
            ActivityTarget::Tool {
                message_id,
                part_id,
            } => {
                let call = app
                    .messages
                    .iter()
                    .find(|message| &message.id == message_id)
                    .and_then(|message| {
                        message.parts.iter().find_map(|part| match part {
                            mew_message::Part::ToolCall(call) if &call.base.id == part_id => {
                                Some(call)
                            }
                            _ => None,
                        })
                    });
                if let Some(call) = call {
                    cache_rows(&mut view.rendered, width, app.chat_dirty, |rows| {
                        tool_rows(rows, call, width, surface)
                    });
                    include_content = true;
                    let status = match &call.state {
                        mew_message::ToolState::Pending(_) => "Pending",
                        mew_message::ToolState::Running(_) => "Running",
                        mew_message::ToolState::Completed(_) => "Completed",
                        mew_message::ToolState::Error(_) => "Failed",
                    };
                    ("Tool", status)
                } else {
                    detail(
                        &mut rows,
                        "Tool",
                        "This tool call is no longer available.",
                        width,
                    );
                    ("Tool", "Unavailable")
                }
            }
            ActivityTarget::Review => {
                if let Some(review) = &view.review {
                    if let Some(error) = &review.error {
                        detail(&mut rows, "Review unavailable", error, width);
                    }
                    if !review.loaded && review.error.is_none() {
                        detail(&mut rows, "Workspace", "Loading workspace changes…", width);
                    } else if review.loaded && review.files.is_empty() {
                        detail(&mut rows, "Workspace", "No workspace changes.", width);
                    }
                    cache_rows(&mut view.rendered, width, None, |rows| {
                        for file in &review.files {
                            detail(
                                rows,
                                &file.path,
                                &format!("{:?} · +{} −{}", file.status, file.added, file.removed),
                                width,
                            );
                            if let Some(old) = &file.old_path {
                                detail(rows, "Renamed from", old, width);
                            }
                            if file.status == mew_diff::FileStatus::Binary {
                                detail(
                                    rows,
                                    "Binary file",
                                    "Content cannot be shown as a text diff.",
                                    width,
                                );
                            }
                            for hunk in &file.hunks {
                                rows.push(Line::from(format!(
                                    "@@ -{},{} +{},{} @@",
                                    hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
                                )));
                                for line in &hunk.lines {
                                    let (prefix, color) = match line.kind {
                                        mew_diff::LineKind::Addition => {
                                            ('+', app.theme.resolve("text.success"))
                                        }
                                        mew_diff::LineKind::Deletion => {
                                            ('-', app.theme.resolve("text.error"))
                                        }
                                        mew_diff::LineKind::Context => {
                                            (' ', app.theme.resolve("text.body"))
                                        }
                                    };
                                    let text = format!(
                                        "{prefix} {:>4} {:>4} {}",
                                        line.old_line.map(|n| n.to_string()).unwrap_or_default(),
                                        line.new_line.map(|n| n.to_string()).unwrap_or_default(),
                                        line.text
                                    );
                                    rows.extend(
                                        super::chat::wrap_text_to_width(&text, width)
                                            .into_iter()
                                            .map(|line| {
                                                Line::styled(line, Style::default().fg(color))
                                            }),
                                    );
                                }
                            }
                        }
                    });
                    include_content = true;
                }
                ("Review", "Workspace changes")
            }
        }
    };
    let content = if include_content {
        view.rendered
            .as_ref()
            .map(|cache| cache.rows.as_slice())
            .unwrap_or_default()
    } else {
        &[]
    };
    let row_count = rows.len() + content.len() + tail.len();
    let mut layout = DecisionLayout::new(area, width_cap, row_count + 1, 0);
    let title_area = Rect::new(
        layout.body.x,
        layout.body.y,
        layout.body.width,
        u16::from(layout.body.height > 0),
    );
    layout.body.y += title_area.height;
    layout.body.height = layout.body.height.saturating_sub(title_area.height);
    view.view.page_rows = layout.body.height;
    view.max_scroll = row_count.saturating_sub(layout.body.height as usize);
    view.view.max_scroll = view.max_scroll.min(u16::MAX as usize) as u16;
    view.scroll = view.scroll.min(view.max_scroll);
    let visible = rows
        .iter()
        .chain(content)
        .chain(tail.iter())
        .skip(view.scroll)
        .take(layout.body.height as usize)
        .cloned()
        .collect::<Vec<_>>();
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
        Paragraph::new(visible).style(Style::default().fg(fg).bg(surface)),
        layout.body,
    );
    let trace = view.transcript.is_some() || view.job_output.is_some() || view.review.is_some();
    let hint = if trace && layout.hint.width >= 41 {
        "↑↓/PgUp/PgDn scroll · r refresh · Esc close"
    } else if trace && layout.hint.width >= 25 {
        "↑↓ · r refresh · Esc close"
    } else if trace && layout.hint.width >= 19 {
        "r refresh Esc close"
    } else if view.max_scroll > 0 && layout.hint.width >= 16 {
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
    if view.max_scroll > 0 && layout.body.height > 0 {
        let mut state = ScrollbarState::new(view.max_scroll + 1).position(view.scroll);
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

fn cache_rows(
    cache: &mut Option<crate::app::RenderedActivity>,
    width: u16,
    dirty: Option<u64>,
    build: impl FnOnce(&mut Vec<Line<'static>>),
) {
    if cache
        .as_ref()
        .is_some_and(|cache| cache.width == width && cache.dirty == dirty)
    {
        return;
    }
    let mut rows = Vec::new();
    build(&mut rows);
    *cache = Some(crate::app::RenderedActivity { width, dirty, rows });
}

fn transcript_rows(
    rows: &mut Vec<Line<'static>>,
    messages: &[mew_message::Message],
    width: u16,
    surface: ratatui::style::Color,
) {
    use mew_message::{Part, Role};
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
                Part::ToolCall(call) => tool_rows(rows, call, width, surface),
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

fn tool_rows(
    rows: &mut Vec<Line<'static>>,
    call: &mew_message::ToolCallPart,
    width: u16,
    surface: ratatui::style::Color,
) {
    use mew_message::ToolState;
    let state = match &call.state {
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
        detail(rows, "Result", "", width);
        output_rows(rows, output, width, surface);
    }
    if let ToolState::Error(error) = &call.state {
        detail(rows, "Error", "", width);
        output_rows(rows, &error.error, width, surface);
    }
}

fn output_rows(
    rows: &mut Vec<Line<'static>>,
    output: &str,
    width: u16,
    surface: ratatui::style::Color,
) {
    let text = output.into_text().unwrap_or_default();
    for line in text.lines {
        rows.extend(super::chat::wrap_tool_line(width, line, "", surface));
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
    if value.is_empty() {
        return;
    }
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
