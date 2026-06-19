use ansi_to_tui::IntoText as _;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};
use ratatui_mdstream::Theme as MdTheme;
use std::rc::Rc;

use super::{
    BASH_LINES_COLLAPSED, BASH_LINES_EXPANDED, DIFF_LINES_MAX, TOOL_BG, TOOL_LINES_LIVE,
    TOOL_LINES_MAX,
};
use crate::app::{byte_at_display_offset, App, ToolDisplayState};
use mew_message::{Part, Role, ToolState};

use super::welcome::draw_welcome;

/// State for tracking visual rows during chat rendering for drag-to-select.
struct ChatLineCtx<'a, 't> {
    text: &'t mut Text<'a>,
    chat_rows: &'t mut Vec<String>,
    visual_row: usize,
    has_sel: bool,
    anchor_row: usize,
    anchor_col: usize,
    end_row: usize,
    end_col: usize,
}

impl<'a, 't> ChatLineCtx<'a, 't> {
    fn push_line(&mut self, line: Line<'a>) {
        let line = if self.has_sel {
            self.apply_selection(line)
        } else {
            line
        };
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        self.chat_rows.push(plain);
        self.text.push_line(line);
        self.visual_row += 1;
    }

    fn sel_range(&self) -> Option<(usize, usize)> {
        if !self.has_sel {
            return None;
        }
        let lo = self.anchor_row.min(self.end_row);
        let hi = self.anchor_row.max(self.end_row);
        let row = self.visual_row;
        if row < lo || row > hi {
            return None;
        }
        if lo == hi {
            let (a, b) = (self.anchor_col, self.end_col);
            Some((a.min(b), a.max(b)))
        } else if row == lo {
            let start = if self.anchor_row < self.end_row {
                self.anchor_col
            } else {
                self.end_col
            };
            Some((start, usize::MAX))
        } else if row == hi {
            let end = if self.anchor_row > self.end_row {
                self.anchor_col
            } else {
                self.end_col
            };
            Some((0, end))
        } else {
            Some((0, usize::MAX))
        }
    }

    fn apply_selection(&self, mut line: Line<'a>) -> Line<'a> {
        let (start_disp, end_disp) = match self.sel_range() {
            Some(r) => r,
            None => return line,
        };
        let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let start = byte_at_display_offset(&full, start_disp);
        let end_excl = if end_disp == usize::MAX {
            full.len()
        } else {
            byte_at_display_offset(&full, end_disp)
        };
        if start >= end_excl {
            return line;
        }
        if start == 0 && end_excl == full.len() {
            for s in line.spans.iter_mut() {
                s.style = s.style.fg(Color::Black).bg(Color::White);
            }
            return line;
        }
        let mut new_spans: Vec<Span<'a>> = Vec::new();
        let mut offset = 0usize;
        let sel_style = Style::default().fg(Color::Black).bg(Color::White);
        for s in line.spans {
            let len = s.content.len();
            let span_end = offset + len;
            if span_end <= start || offset >= end_excl {
                new_spans.push(s);
            } else if offset >= start && span_end <= end_excl {
                new_spans.push(Span::styled(s.content, sel_style));
            } else {
                if offset < start && span_end <= end_excl {
                    let before = s.content[..start - offset].to_string();
                    let inside = s.content[start - offset..].to_string();
                    new_spans.push(Span::styled(before, s.style));
                    new_spans.push(Span::styled(inside, sel_style));
                } else if offset < start {
                    let before = s.content[..start - offset].to_string();
                    let inside = s.content[start - offset..end_excl - offset].to_string();
                    let after = s.content[end_excl - offset..].to_string();
                    new_spans.push(Span::styled(before, s.style));
                    new_spans.push(Span::styled(inside, sel_style));
                    new_spans.push(Span::styled(after, s.style));
                } else {
                    let inside = s.content[..end_excl - offset].to_string();
                    let after = s.content[end_excl - offset..].to_string();
                    new_spans.push(Span::styled(inside, sel_style));
                    new_spans.push(Span::styled(after, s.style));
                }
            }
            offset = span_end;
        }
        line.spans = new_spans;
        line
    }
}

pub(super) fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let mut text = Text::default();
    let tool_bg_style = Style::default().bg(TOOL_BG);
    let msg_count = app.messages.len();
    let md_width = area.width.saturating_sub(2);

    let mut chat_rows = std::mem::take(&mut app.chat_rows);
    chat_rows.clear();
    let mut sel_ctx = ChatLineCtx {
        text: &mut text,
        chat_rows: &mut chat_rows,
        visual_row: 0,
        has_sel: app.sel_anchor_row.is_some() && app.sel_end_row.is_some(),
        anchor_row: app.sel_anchor_row.unwrap_or(0),
        anchor_col: app.sel_anchor_col.unwrap_or(0),
        end_row: app.sel_end_row.unwrap_or(0),
        end_col: app.sel_end_col.unwrap_or(0),
    };

    if app.last_md_width != md_width {
        app.rendered_md_cache.clear();
        app.last_md_width = md_width;
    }

    if app.messages.is_empty() {
        draw_welcome(f, area);
        app.chat_rows = chat_rows;
        return;
    }

    for (msg_idx, msg) in app.messages.iter().enumerate() {
        let is_last = msg_idx + 1 == msg_count;
        let is_streaming = app.streaming && is_last;
        let (prefix, prefix_color, content_style) = match msg.role {
            Role::User => (">", Color::Cyan, Style::default().fg(Color::White)),
            Role::Assistant => ("", Color::Gray, Style::default().fg(Color::White)),
        };

        let mut message_had_content = false;

        if msg.role == Role::Assistant {
            if let Some(ref meta) = msg.assistant {
                if !meta.provider_id.is_empty() && !meta.model_id.is_empty() {
                    sel_ctx.push_line(Line::from(vec![
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

        let last_text_part_idx = msg.parts.iter().rposition(|p| matches!(p, Part::Text(_)));

        for (part_idx, part) in msg.parts.iter().enumerate() {
            match part {
                Part::Text(tp) => {
                    if prefix.is_empty() {
                        let use_streaming = is_streaming && Some(part_idx) == last_text_part_idx;
                        let md_lines: Vec<ratatui::text::Line<'static>> = if use_streaming {
                            let mut highlighter =
                                ratatui_mdstream::highlight::SyntectHighlighter::new();
                            fix_em_dashes(ratatui_mdstream::render_streaming(
                                &app.md_state,
                                md_width,
                                &MdTheme::dark(),
                                &mut highlighter,
                            ))
                        } else {
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
                            sel_ctx.push_line(new_line);
                        }
                    } else {
                        for line in tp.text.lines() {
                            let display_line = line.replace('\u{2014}', "— ");
                            let spans = vec![
                                Span::styled(
                                    format!("{} ", prefix),
                                    Style::default().fg(prefix_color),
                                ),
                                Span::styled(display_line, content_style),
                            ];
                            sel_ctx.push_line(Line::from(spans));
                        }
                    }
                    message_had_content = true;
                }
                Part::Reasoning(rp) => {
                    let line_count = rp.text.lines().count();
                    sel_ctx.push_line(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            if app.reasoning_expanded {
                                format!("[thinking — Ctrl+T to collapse] ({} lines)", line_count)
                            } else {
                                format!("[thinking — Ctrl+T to expand] ({} lines)", line_count)
                            },
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                    if app.reasoning_expanded {
                        for line in rp.text.lines() {
                            sel_ctx.push_line(Line::from(vec![
                                Span::raw("  "),
                                Span::styled(line, Style::default().fg(Color::DarkGray)),
                            ]));
                        }
                    }
                    message_had_content = true;
                }
                Part::ToolCall(tc) => {
                    let (state_label, state_color) = tool_call_label_and_color(app, tc);

                    if let Some(line) = push_tool_edge(area.width, true, TOOL_BG) {
                        sel_ctx.push_line(line);
                    }

                    sel_ctx.push_line(push_tool_line(
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
                    ));

                    if let Some(args) = tool_call_args_summary(tc) {
                        sel_ctx.push_line(push_tool_line(
                            area.width,
                            vec![
                                Span::styled("      ", tool_bg_style),
                                Span::styled(
                                    args,
                                    Style::default().fg(Color::DarkGray).bg(TOOL_BG),
                                ),
                            ],
                            tool_bg_style,
                        ));
                    }
                    message_had_content = true;

                    if let ToolState::Running(ref running) = tc.state {
                        if !running.output.is_empty() {
                            let parsed = running.output.as_str().into_text().unwrap_or_default();
                            let lines = parsed.lines;
                            let skip = lines.len().saturating_sub(TOOL_LINES_LIVE);
                            for line in lines.into_iter().skip(skip) {
                                sel_ctx
                                    .push_line(push_ansi_line(area.width, line, "      ", TOOL_BG));
                            }
                        }
                    }

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
                                    sel_ctx.push_line(push_tool_line(
                                        area.width,
                                        vec![
                                            Span::styled("      ", tool_bg_style),
                                            Span::styled(
                                                format!("... ({} earlier lines)", skip),
                                                Style::default().fg(Color::DarkGray).bg(TOOL_BG),
                                            ),
                                        ],
                                        tool_bg_style,
                                    ));
                                }
                                for line in lines.into_iter().skip(skip) {
                                    sel_ctx.push_line(push_ansi_line(
                                        area.width, line, "      ", TOOL_BG,
                                    ));
                                }
                            } else {
                                for line in lines.into_iter().take(TOOL_LINES_MAX) {
                                    sel_ctx.push_line(push_ansi_line(
                                        area.width, line, "      ", TOOL_BG,
                                    ));
                                }
                                if line_count > TOOL_LINES_MAX {
                                    sel_ctx.push_line(push_tool_line(
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
                                    ));
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
                                sel_ctx.push_line(push_tool_line(
                                    area.width,
                                    vec![
                                        Span::styled("      ", tool_bg_style),
                                        Span::styled(line, style),
                                    ],
                                    tool_bg_style,
                                ));
                            }
                            let diff_lines = diff.lines().count();
                            if diff_lines > DIFF_LINES_MAX {
                                sel_ctx.push_line(push_tool_line(
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
                                ));
                            }
                        }
                    }
                    if let Some(ToolDisplayState::Error(err)) = app.tool_states.get(&tc.base.id) {
                        if !err.is_empty() {
                            let parsed = err.as_str().into_text().unwrap_or_default();
                            for line in parsed.lines {
                                sel_ctx
                                    .push_line(push_ansi_line(area.width, line, "      ", TOOL_BG));
                            }
                        }
                    }

                    if let Some(line) = push_tool_edge(area.width, false, TOOL_BG) {
                        sel_ctx.push_line(line);
                    }
                }
                Part::File(fp) => {
                    let name = fp
                        .filename
                        .as_deref()
                        .or_else(|| fp.url.rsplit('/').next())
                        .unwrap_or("file");
                    sel_ctx.push_line(Line::from(vec![
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
            sel_ctx.push_line(Line::from(""));
        }
    }
    #[allow(clippy::drop_non_drop)]
    drop(sel_ctx);
    app.chat_rows = chat_rows;

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

fn push_tool_line<'a>(width: u16, spans: Vec<Span<'a>>, pad_style: Style) -> Line<'a> {
    let mut line = Line::from(spans);
    let used = line.width() as u16;
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat((width - used) as usize), pad_style));
    }
    line
}

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

fn push_tool_edge(width: u16, is_top: bool, tool_bg: Color) -> Option<Line<'static>> {
    if width == 0 {
        return None;
    }
    let ch = if is_top { '▄' } else { '▀' };
    let style = Style::default().fg(tool_bg).bg(Color::Reset);
    Some(Line::from(vec![Span::styled(
        ch.to_string().repeat(width as usize),
        style,
    )]))
}

fn push_ansi_line<'a>(width: u16, mut line: Line<'a>, indent: &str, tool_bg: Color) -> Line<'a> {
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
    line
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
