use ansi_to_tui::IntoText as _;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};
use ratatui_mdstream::Theme as MdTheme;
use std::rc::Rc;

use crate::app::{
    App, Mode, PermissionState, PickerState, SlashCommand, ToolDisplayState, PICKER_VISIBLE_ITEMS,
    SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH,
};
use mew_message::{Part, Role, ToolState};

/// Background color for the input and status line surface.
const STATUS_BG: Color = Color::Rgb(30, 30, 33);
/// Background color for the sidebar surface.
const SIDEBAR_BG: Color = Color::Rgb(28, 28, 31);
/// Background color for tool call blocks.
const TOOL_BG: Color = Color::Rgb(34, 34, 38);
/// Subtle divider color.
const DIVIDER: Color = Color::Rgb(50, 50, 55);
/// Max lines of bash output shown when collapsed.
const BASH_LINES_COLLAPSED: usize = 10;
/// Max lines of bash output shown when expanded.
const BASH_LINES_EXPANDED: usize = 50;
/// Max lines shown for non-bash tool output.
const TOOL_LINES_MAX: usize = 20;
/// Max lines of live streaming output shown while a tool is running.
const TOOL_LINES_LIVE: usize = 5;
/// Max lines of diff shown inline.
const DIFF_LINES_MAX: usize = 30;

/// Render the full UI.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Decide if sidebar fits.
    let show_sidebar = area.width >= SIDEBAR_MIN_WIDTH + SIDEBAR_WIDTH;

    let main_chunks: Vec<Rect> = if show_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(SIDEBAR_MIN_WIDTH),
                Constraint::Length(SIDEBAR_WIDTH),
            ])
            .split(area);
        // Fill sidebar background.
        let sidebar_bg = Block::default().style(Style::default().bg(SIDEBAR_BG));
        f.render_widget(sidebar_bg, chunks[1]);
        draw_sidebar(f, app, chunks[1]);
        chunks.to_vec()
    } else {
        vec![area]
    };

    let main_area = main_chunks[0];

    // Check if slash autocomplete should show.
    let slash_cmds = app.filtered_slash_commands();
    let show_slash = app.mode == Mode::SlashCommand && !slash_cmds.is_empty();
    let slash_height = if show_slash {
        (slash_cmds.len() as u16 + 2).min(5)
    } else {
        0
    };

    let input_height = (app.input_line_count().clamp(1, 12) + 2) as u16;

    // Vertical layout inside main area.
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),               // chat
            Constraint::Length(1),            // divider
            Constraint::Length(slash_height), // slash autocomplete
            Constraint::Length(input_height), // input
            Constraint::Length(1),            // status
        ])
        .split(main_area);

    draw_chat(f, app, vert[0]);
    draw_divider(f, vert[1]);
    if show_slash {
        draw_slash_autocomplete(f, app, &slash_cmds, vert[2]);
    }
    draw_input(f, app, vert[3]);
    draw_status(f, app, vert[4]);

    if app.mode == Mode::PermissionPrompt {
        if let Some(ref perm) = app.permission {
            draw_permission_modal(f, perm, main_area);
        }
    }

    if app.mode == Mode::CommandPalette {
        if let Some(ref picker) = app.picker {
            draw_picker(f, picker, main_area);
        }
    }
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let mut text = Text::default();
    let tool_bg_style = Style::default().bg(TOOL_BG);
    let msg_count = app.messages.len();
    let md_width = area.width.saturating_sub(2);

    // Invalidate markdown cache when terminal width changes.
    if app.last_md_width != md_width {
        app.rendered_md_cache.clear();
        app.last_md_width = md_width;
    }

    for (msg_idx, msg) in app.messages.iter().enumerate() {
        let is_last = msg_idx + 1 == msg_count;
        let is_streaming = app.streaming && is_last;
        let (prefix, prefix_color, content_style) = match msg.role {
            Role::User => (">", Color::Cyan, Style::default().fg(Color::White)),
            Role::Assistant => ("", Color::Gray, Style::default().fg(Color::White)),
        };

        let mut message_had_content = false;

        // Model attribution for assistant messages.
        if msg.role == Role::Assistant {
            if let Some(ref meta) = msg.assistant {
                if !meta.provider_id.is_empty() && !meta.model_id.is_empty() {
                    text.push_line(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            format!("{} / {}", meta.provider_id, meta.model_id),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                    message_had_content = true;
                }
            }
        }

        // Only the final text part in a streaming message uses live md_state.
        // Earlier text parts (before tool calls) are already complete and should
        // render from their accumulated text, not the streaming state.
        let last_text_part_idx = msg.parts.iter().rposition(|p| matches!(p, Part::Text(_)));

        for (part_idx, part) in msg.parts.iter().enumerate() {
            match part {
                Part::Text(tp) => {
                    if prefix.is_empty() {
                        // Assistant text — render as markdown.
                        let use_streaming = is_streaming && Some(part_idx) == last_text_part_idx;
                        let md_lines: Vec<ratatui::text::Line<'static>> = if use_streaming {
                            // Use the incremental markdown stream for the active text part.
                            let mut highlighter =
                                ratatui_mdstream::highlight::SyntectHighlighter::new();
                            fix_em_dashes(ratatui_mdstream::render_streaming(
                                &app.md_state,
                                md_width,
                                &MdTheme::dark(),
                                &mut highlighter,
                            ))
                        } else {
                            // Use cached rendering for completed messages.
                            if app.pending_md_rerender == Some(msg.id) {
                                app.rendered_md_cache.remove(&msg.id);
                                app.pending_md_rerender = None;
                            }
                            let cache = &mut app.rendered_md_cache;
                            if let Some((cached_width, cached_text, cached_lines)) =
                                cache.get(&msg.id)
                            {
                                if cached_text == &tp.text && *cached_width == md_width {
                                    Rc::unwrap_or_clone(Rc::clone(cached_lines))
                                } else {
                                    cache.remove(&msg.id);
                                    let lines = fix_em_dashes(ratatui_mdstream::render_markdown(
                                        &tp.text,
                                        md_width,
                                        &MdTheme::dark(),
                                    ));
                                    let rc = Rc::new(lines);
                                    cache.insert(
                                        msg.id,
                                        (md_width, tp.text.clone(), Rc::clone(&rc)),
                                    );
                                    Rc::unwrap_or_clone(rc)
                                }
                            } else {
                                let lines = fix_em_dashes(ratatui_mdstream::render_markdown(
                                    &tp.text,
                                    md_width,
                                    &MdTheme::dark(),
                                ));
                                let rc = Rc::new(lines);
                                cache.insert(msg.id, (md_width, tp.text.clone(), Rc::clone(&rc)));
                                Rc::unwrap_or_clone(rc)
                            }
                        };
                        for line in md_lines {
                            let mut new_line = Line::from(line.spans);
                            new_line.spans.insert(0, Span::raw("  "));
                            new_line.style = line.style;
                            text.push_line(new_line);
                        }
                    } else {
                        // User text — keep the > prefix. Em dash → two-em-dash for width.
                        for line in tp.text.lines() {
                            let display_line = line.replace('\u{2014}', "— ");
                            let spans = vec![
                                Span::styled(
                                    format!("{} ", prefix),
                                    Style::default().fg(prefix_color),
                                ),
                                Span::styled(display_line, content_style),
                            ];
                            text.push_line(Line::from(spans));
                        }
                    }
                    message_had_content = true;
                }
                Part::Reasoning(rp) => {
                    for line in rp.text.lines() {
                        text.push_line(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(line, Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                    message_had_content = true;
                }
                Part::ToolCall(tc) => {
                    let (state_label, state_color) = tool_call_label_and_color(app, tc);

                    // Top edge: half default bg, half tool bg.
                    push_tool_edge(&mut text, area.width, true, TOOL_BG);

                    // Tool name line.
                    push_tool_line(
                        &mut text,
                        area.width,
                        vec![
                            Span::styled("  ", tool_bg_style),
                            Span::styled(
                                format!("{} {}", state_label, tc.tool_name),
                                Style::default()
                                    .fg(state_color)
                                    .bg(TOOL_BG)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ],
                        tool_bg_style,
                    );

                    // Args line.
                    if let Some(args) = tool_call_args_summary(tc) {
                        push_tool_line(
                            &mut text,
                            area.width,
                            vec![
                                Span::styled("      ", tool_bg_style),
                                Span::styled(
                                    args,
                                    Style::default().fg(Color::DarkGray).bg(TOOL_BG),
                                ),
                            ],
                            tool_bg_style,
                        );
                    }
                    message_had_content = true;

                    // Partial output while running.
                    if let ToolState::Running(ref running) = tc.state {
                        if !running.output.is_empty() {
                            let parsed = running.output.as_str().into_text().unwrap_or_default();
                            let lines = parsed.lines;
                            let skip = lines.len().saturating_sub(TOOL_LINES_LIVE);
                            for line in lines.into_iter().skip(skip) {
                                push_ansi_line(&mut text, area.width, line, "      ", TOOL_BG);
                            }
                        }
                    }

                    // Completed output.
                    if let Some(ToolDisplayState::Completed { output, diff }) =
                        app.tool_states.get(&tc.base.id)
                    {
                        if !output.is_empty() {
                            let parsed = output.as_str().into_text().unwrap_or_default();
                            let is_bash = tc.tool_name == "bash";
                            let lines = parsed.lines;
                            let line_count = lines.len();

                            if is_bash {
                                let limit = if app.bash_expanded {
                                    BASH_LINES_EXPANDED
                                } else {
                                    BASH_LINES_COLLAPSED
                                };
                                let skip = line_count.saturating_sub(limit);
                                if skip > 0 {
                                    push_tool_line(
                                        &mut text,
                                        area.width,
                                        vec![
                                            Span::styled("      ", tool_bg_style),
                                            Span::styled(
                                                format!("... ({} earlier lines)", skip),
                                                Style::default().fg(Color::DarkGray).bg(TOOL_BG),
                                            ),
                                        ],
                                        tool_bg_style,
                                    );
                                }
                                for line in lines.into_iter().skip(skip) {
                                    push_ansi_line(&mut text, area.width, line, "      ", TOOL_BG);
                                }
                            } else {
                                for line in lines.into_iter().take(TOOL_LINES_MAX) {
                                    push_ansi_line(&mut text, area.width, line, "      ", TOOL_BG);
                                }
                                if line_count > TOOL_LINES_MAX {
                                    push_tool_line(
                                        &mut text,
                                        area.width,
                                        vec![
                                            Span::styled("      ", tool_bg_style),
                                            Span::styled(
                                                format!(
                                                    "... ({} more lines)",
                                                    line_count - TOOL_LINES_MAX
                                                ),
                                                Style::default().fg(Color::DarkGray).bg(TOOL_BG),
                                            ),
                                        ],
                                        tool_bg_style,
                                    );
                                }
                            }
                        }
                        if let Some(diff) = diff {
                            for line in diff.lines().take(DIFF_LINES_MAX) {
                                let style = if line.starts_with('+') {
                                    Style::default().fg(Color::Green).bg(TOOL_BG)
                                } else if line.starts_with('-') {
                                    Style::default().fg(Color::Red).bg(TOOL_BG)
                                } else {
                                    Style::default().fg(Color::DarkGray).bg(TOOL_BG)
                                };
                                push_tool_line(
                                    &mut text,
                                    area.width,
                                    vec![
                                        Span::styled("      ", tool_bg_style),
                                        Span::styled(line, style),
                                    ],
                                    tool_bg_style,
                                );
                            }
                            let diff_lines = diff.lines().count();
                            if diff_lines > DIFF_LINES_MAX {
                                push_tool_line(
                                    &mut text,
                                    area.width,
                                    vec![
                                        Span::styled("      ", tool_bg_style),
                                        Span::styled(
                                            format!(
                                                "... ({} more lines)",
                                                diff_lines - DIFF_LINES_MAX
                                            ),
                                            Style::default().fg(Color::DarkGray).bg(TOOL_BG),
                                        ),
                                    ],
                                    tool_bg_style,
                                );
                            }
                        }
                    }
                    if let Some(ToolDisplayState::Error(err)) = app.tool_states.get(&tc.base.id) {
                        if !err.is_empty() {
                            let parsed = err.as_str().into_text().unwrap_or_default();
                            for line in parsed.lines {
                                push_ansi_line(&mut text, area.width, line, "      ", TOOL_BG);
                            }
                        }
                    }

                    // Bottom edge: half tool bg, half default bg.
                    push_tool_edge(&mut text, area.width, false, TOOL_BG);
                }
                Part::File(fp) => {
                    let name = fp
                        .filename
                        .as_deref()
                        .or_else(|| fp.url.rsplit('/').next())
                        .unwrap_or("file");
                    text.push_line(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("[image]  ", Style::default().fg(Color::DarkGray)),
                        Span::styled(name.to_string(), Style::default().fg(Color::Cyan)),
                    ]));
                    message_had_content = true;
                }
                Part::ToolResult(_) => {}
                _ => {}
            }
        }

        if message_had_content {
            text.push_line(Line::from(""));
        }
    }

    // Compute scroll offset. app.scroll is "lines scrolled up from bottom".
    // Compute scroll. app.scroll is "lines from top", auto-scroll to bottom.
    let total_lines = text.lines.len() as u16;
    let max_scroll = total_lines.saturating_sub(area.height);
    app.max_scroll = max_scroll;

    if app.auto_scroll {
        app.scroll = max_scroll;
    }
    let scroll_offset = app.scroll.min(max_scroll);

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));

    f.render_widget(paragraph, area);

    // Scrollbar (only when content overflows).
    if total_lines > area.height {
        let mut scrollbar_state = ScrollbarState::new(total_lines as usize)
            .viewport_content_length(area.height as usize)
            .position(scroll_offset as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }

    // Scroll indicators.
    if app.scroll > 0 {
        let indicator = Span::styled("↑", Style::default().fg(Color::Yellow));
        let indicator_area = Rect::new(area.x, area.y, 2, 1);
        f.render_widget(Paragraph::new(Line::from(indicator)), indicator_area);
    }
    if app.scroll < max_scroll {
        let indicator = Span::styled("↓", Style::default().fg(Color::Yellow));
        let indicator_area = Rect::new(area.x + area.width - 2, area.y + area.height - 1, 2, 1);
        f.render_widget(Paragraph::new(Line::from(indicator)), indicator_area);
    }
}

/// Push a line into `text`, padding with spaces to `width` using `pad_style`.
fn push_tool_line<'a>(text: &mut Text<'a>, width: u16, spans: Vec<Span<'a>>, pad_style: Style) {
    let mut line = Line::from(spans);
    let used = line.width() as u16;
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat((width - used) as usize), pad_style));
    }
    text.push_line(line);
}

/// Pre-process markdown lines: convert em dashes to double-width,
/// preserving span styles and line-level backgrounds.
fn fix_em_dashes(lines: Vec<ratatui::text::Line<'static>>) -> Vec<ratatui::text::Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let spans: Vec<Span> = line
                .spans
                .into_iter()
                .map(|s| {
                    let content: String = if s.content.contains('\u{2014}') {
                        s.content
                            .chars()
                            .map(|c| {
                                if c == '\u{2014}' {
                                    "— ".to_string()
                                } else {
                                    c.to_string()
                                }
                            })
                            .collect()
                    } else {
                        s.content.to_string()
                    };
                    Span::styled(content, s.style)
                })
                .collect();
            let mut new_line = Line::from(spans);
            new_line.style = line.style;
            new_line
        })
        .collect()
}

/// Compute display width of a string using Unicode standard widths.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

/// Push a half-block edge line to transition between the terminal default
/// background and the tool call background.
fn push_tool_edge(text: &mut Text<'_>, width: u16, is_top: bool, tool_bg: Color) {
    if width == 0 {
        return;
    }
    let ch = if is_top { '▄' } else { '▀' };
    let style = Style::default().fg(tool_bg).bg(Color::Reset);
    text.push_line(Line::from(vec![Span::styled(
        ch.to_string().repeat(width as usize),
        style,
    )]));
}

/// Push a single ANSI-parsed line with indent and tool background padding.
fn push_ansi_line<'a>(
    text: &mut Text<'a>,
    width: u16,
    mut line: Line<'static>,
    indent: &str,
    tool_bg: Color,
) {
    let mut spans = vec![Span::styled(
        indent.to_string(),
        Style::default().bg(tool_bg),
    )];
    spans.extend(line.spans);
    line.spans = spans;
    line.style = Style::default().bg(tool_bg);
    let used = line.width() as u16;
    if used < width {
        line.spans.push(Span::styled(
            " ".repeat((width - used) as usize),
            Style::default().bg(tool_bg),
        ));
    }
    text.push_line(line);
}

fn tool_call_label_and_color(app: &App, tc: &mew_message::ToolCallPart) -> (&'static str, Color) {
    match app.tool_states.get(&tc.base.id) {
        Some(ToolDisplayState::Running) => ("▶", Color::Yellow),
        Some(ToolDisplayState::Completed { .. }) => ("✓", Color::Green),
        Some(ToolDisplayState::Error(_)) => ("✗", Color::Red),
        None => match &tc.state {
            ToolState::Pending(_) => ("○", Color::Yellow),
            ToolState::Running(_) => ("▶", Color::Yellow),
            ToolState::Completed(_) => ("✓", Color::Green),
            ToolState::Error(_) => ("✗", Color::Red),
        },
    }
}

/// Extract a short summary of the tool call's key argument for display.
fn tool_call_args_summary(tc: &mew_message::ToolCallPart) -> Option<String> {
    let input = tc.state.input();
    match tc.tool_name.as_str() {
        "read" | "write" | "edit" => input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "bash" => input.get("command").and_then(|v| v.as_str()).map(|s| {
            if s.len() > 50 {
                format!("{}...", &s[..50])
            } else {
                s.to_string()
            }
        }),
        "grep" => input.get("pattern").and_then(|v| v.as_str()).map(|s| {
            if s.len() > 40 {
                format!("'{}...'", &s[..40])
            } else {
                format!("'{}'", s)
            }
        }),
        "glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "echo" => input.get("input").and_then(|v| v.as_str()).map(|s| {
            if s.len() > 40 {
                format!("'{}...'", &s[..40])
            } else {
                format!("'{}'", s)
            }
        }),
        _ => None,
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
        Mode::SlashCommand => Style::default().fg(Color::Yellow).bg(STATUS_BG),
        _ => Style::default().fg(Color::White).bg(STATUS_BG),
    };

    let line_count = app.input_line_count().max(1);
    // Clamp input area height: use as many rows as needed, up to a max.
    let input_height = line_count.min(12) as u16 + 2; // +2 for padding
    let input_area = Rect::new(area.x, area.y, area.width, input_height.min(area.height));

    // Fill background.
    let bg_block = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg_block, input_area);

    let content_area = Rect::new(
        input_area.x + 1,
        input_area.y + 1,
        input_area.width.saturating_sub(2),
        input_area.height.saturating_sub(2),
    );

    let prefix_width = 2usize; // "> " or "… "
    let (cursor_line, cursor_col) = app.cursor_line_col();

    let lines: Vec<&str> = app.input.split('\n').collect();
    for (li, line) in lines.iter().enumerate() {
        let y = content_area.y + li as u16;
        if y >= content_area.y + content_area.height {
            break;
        }

        let available = content_area.width as usize;
        let (visible, _col) = if display_width(line) <= available {
            (*line, cursor_col.min(available))
        } else {
            let cursor_col_in_text = if li == cursor_line { cursor_col } else { 0 };
            let start_col = cursor_col_in_text.saturating_sub(available / 2);
            let start_byte = byte_at_display_offset(line, start_col);
            let end_byte = byte_at_display_offset(line, start_col + available);
            (&line[start_byte..end_byte], cursor_col_in_text - start_col)
        };

        let prefix = if app.streaming {
            Span::styled("… ", Style::default().fg(Color::Yellow).bg(STATUS_BG))
        } else {
            Span::styled("> ", Style::default().fg(Color::Cyan).bg(STATUS_BG))
        };

        let text = Text::from(Line::from(vec![prefix, Span::styled(visible, style)]));
        let row = Rect::new(content_area.x, y, content_area.width, 1);
        f.render_widget(Paragraph::new(text), row);
    }

    // Position cursor on the active line.
    let cursor_x =
        content_area.x + prefix_width as u16 + cursor_col.min(content_area.width as usize) as u16;
    let cursor_y = content_area.y + cursor_line.min(content_area.height as usize) as u16;
    f.set_cursor_position((cursor_x, cursor_y));
}

/// Find the byte offset into `s` that corresponds to the given display column.
fn byte_at_display_offset(s: &str, target_col: usize) -> usize {
    let mut col = 0usize;
    for (i, ch) in s.char_indices() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if col + w > target_col {
            return i;
        }
        col += w;
    }
    s.len()
}

fn fmt_tokens(n: u32) -> String {
    if n >= 1_000 {
        format!("{:.1}k", n as f32 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
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

fn draw_slash_autocomplete(f: &mut Frame, app: &App, cmds: &[SlashCommand], area: Rect) {
    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, area);

    let mut text = Text::default();
    for (i, cmd) in cmds.iter().enumerate() {
        let is_selected = i == app.slash_selected;
        let name_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(STATUS_BG)
        };
        let desc_style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray).bg(STATUS_BG)
        };
        text.push_line(Line::from(vec![
            Span::raw("  "),
            Span::styled(&cmd.name, name_style),
            Span::raw("  "),
            Span::styled(&cmd.description, desc_style),
        ]));
    }

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    f.render_widget(Paragraph::new(text), inner);
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let mut text = Text::default();

    // Header
    text.push_line(Line::from(vec![Span::styled(
        "Context",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));
    text.push_line(Line::from(""));

    // Context files
    if app.context_files.is_empty() {
        text.push_line(Line::from(Span::styled(
            "No context files loaded",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for path in &app.context_files {
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            text.push_line(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(name, Style::default().fg(Color::Gray)),
            ]));
        }
    }

    text.push_line(Line::from(""));
    text.push_line(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(DIVIDER),
    )));
    text.push_line(Line::from(""));

    // Tools
    text.push_line(Line::from(vec![Span::styled(
        "Tools",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));
    text.push_line(Line::from(""));

    if app.tools.is_empty() {
        text.push_line(Line::from(Span::styled(
            "No tools available",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for tool in &app.tools {
            text.push_line(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(tool, Style::default().fg(Color::Gray)),
            ]));
        }
    }

    text.push_line(Line::from(""));
    text.push_line(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(DIVIDER),
    )));
    text.push_line(Line::from(""));

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

fn draw_permission_modal(f: &mut Frame, perm: &PermissionState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 14u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    // Dim the background.
    f.render_widget(Clear, popup);

    // Solid background block, no border.
    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    let tool_input = serde_json::to_string_pretty(&perm.input).unwrap_or_default();
    let text = Text::from(vec![
        Line::from(vec![
            Span::styled("tool  ", Style::default().fg(Color::DarkGray).bg(STATUS_BG)),
            Span::styled(
                &perm.tool_name,
                Style::default()
                    .fg(Color::White)
                    .bg(STATUS_BG)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "input",
            Style::default().fg(Color::DarkGray).bg(STATUS_BG),
        )]),
        Line::from(Span::styled(
            tool_input,
            Style::default().fg(Color::Gray).bg(STATUS_BG),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "choose:",
            Style::default().fg(Color::DarkGray).bg(STATUS_BG),
        )),
    ]);

    let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(paragraph, inner);

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
            Style::default().fg(Color::Gray).bg(STATUS_BG)
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

fn draw_picker(f: &mut Frame, picker: &PickerState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let max_items = PICKER_VISIBLE_ITEMS as u16;
    let height = (4 + max_items).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    // Solid background, no border.
    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    // Filter input at top.
    let filter_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let prefix = Span::styled("> ", Style::default().fg(Color::Cyan).bg(STATUS_BG));
    let filter_text = Span::styled(
        &picker.filter,
        Style::default().fg(Color::White).bg(STATUS_BG),
    );
    let filter_para = Paragraph::new(Line::from(vec![prefix, filter_text]));
    f.render_widget(filter_para, filter_area);

    // Cursor in filter.
    let cursor_x = filter_area.x + 2 + (picker.cursor.min(filter_area.width as usize - 2) as u16);
    f.set_cursor_position((cursor_x, filter_area.y));

    // Divider under filter.
    let div_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    let div_line = Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(DIVIDER),
    ));
    f.render_widget(Paragraph::new(div_line), div_area);

    // Item list.
    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width.saturating_sub(1),
        inner.height.saturating_sub(2),
    );

    let filtered = picker.filtered();
    let mut list_text = Text::default();

    let start = picker.scroll;
    for (i, item) in filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(list_area.height as usize)
    {
        let is_selected = i == picker.selected;
        let label_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(STATUS_BG)
        };
        let desc_style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray).bg(STATUS_BG)
        };

        list_text.push_line(Line::from(vec![Span::styled(&item.label, label_style)]));
        if !item.description.is_empty() {
            list_text.push_line(Line::from(vec![Span::styled(
                &item.description,
                desc_style,
            )]));
        }
    }

    if filtered.is_empty() {
        list_text.push_line(Line::from(Span::styled(
            "no results",
            Style::default().fg(Color::DarkGray).bg(STATUS_BG),
        )));
    }

    let list_para = Paragraph::new(list_text).wrap(Wrap { trim: true });
    f.render_widget(list_para, list_area);

    // Scrollbar.
    if filtered.len() > list_area.height as usize {
        let scrollbar_area = list_area.inner(Margin {
            horizontal: 0,
            vertical: 0,
        });
        let visible = list_area.height as usize;
        let mut scrollbar_state = ScrollbarState::new(filtered.len())
            .viewport_content_length(visible)
            .position(picker.scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}
