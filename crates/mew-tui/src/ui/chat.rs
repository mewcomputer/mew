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

use super::display_width;

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
    // Reserve the rightmost 1 column for the scrollbar so tool blocks (which
    // paint TOOL_BG across the full paragraph width) can't cover it. The
    // down-indicator then overlays the last column of the chat (like the
    // up-indicator at the top-left).
    let scrollbar_area = Rect {
        x: area.x + area.width.saturating_sub(1),
        y: area.y,
        width: 1.min(area.width),
        height: area.height,
    };
    let chat_inner = Rect {
        x: area.x,
        y: area.y,
        width: area.width.saturating_sub(scrollbar_area.width),
        height: area.height,
    };
    // The chat indents every text part by 2 spaces ("  ") for a left
    // margin. The markdown renderer should produce lines that, after
    // prepending that indent, are exactly `chat_inner.width` wide —
    // otherwise the paragraph's `.wrap(Wrap { trim: false })` wraps the
    // line a second time at render and the continuation row spills past
    // the right edge (or, with the old off-by-one, got cut off entirely).
    let md_width = chat_inner.width.saturating_sub(2);
    // Tool lines pad to this width so the bg fill matches the paragraph
    // render area. Using `area.width` here would make each line 1 col wider
    // than the render area, and `wrapped_height` (which uses
    // `chat_inner.width`) would count every tool line as 2 visual rows —
    // doubling the tool block's height and leaving a row of bg-only "empty
    // space" under every line.
    let tool_width = chat_inner.width;

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

                    if let Some(line) = push_tool_edge(tool_width, true, TOOL_BG) {
                        sel_ctx.push_line(line);
                    }

                    sel_ctx.push_line(push_tool_line(
                        tool_width,
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
                            tool_width,
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
                                for wrapped in wrap_tool_line(tool_width, line, "      ", TOOL_BG) {
                                    sel_ctx.push_line(wrapped);
                                }
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
                                        tool_width,
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
                                    for wrapped in
                                        wrap_tool_line(tool_width, line, "      ", TOOL_BG)
                                    {
                                        sel_ctx.push_line(wrapped);
                                    }
                                }
                            } else {
                                for line in lines.into_iter().take(TOOL_LINES_MAX) {
                                    for wrapped in
                                        wrap_tool_line(tool_width, line, "      ", TOOL_BG)
                                    {
                                        sel_ctx.push_line(wrapped);
                                    }
                                }
                                if line_count > TOOL_LINES_MAX {
                                    sel_ctx.push_line(push_tool_line(
                                        tool_width,
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
                                    tool_width,
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
                                    tool_width,
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
                                for wrapped in wrap_tool_line(tool_width, line, "      ", TOOL_BG) {
                                    sel_ctx.push_line(wrapped);
                                }
                            }
                        }
                    }

                    if let Some(line) = push_tool_edge(tool_width, false, TOOL_BG) {
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

    let total_lines = wrapped_height(&text, chat_inner.width);
    let max_scroll = total_lines.saturating_sub(chat_inner.height);
    app.max_scroll = max_scroll;

    if app.auto_scroll {
        app.scroll = max_scroll;
    }
    let scroll_offset = app.scroll.min(max_scroll);

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));

    f.render_widget(paragraph, chat_inner);

    if total_lines > chat_inner.height {
        let mut scrollbar_state = ScrollbarState::new(total_lines as usize)
            .viewport_content_length(chat_inner.height as usize)
            .position(scroll_offset as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }

    if app.scroll > 0 {
        let indicator = Span::styled("↑", Style::default().fg(Color::Yellow));
        let indicator_area = Rect::new(area.x, area.y, 2, 1);
        f.render_widget(Paragraph::new(Line::from(indicator)), indicator_area);
    }
    if app.scroll < max_scroll {
        let indicator = Span::styled("↓", Style::default().fg(Color::Yellow));
        let indicator_area = Rect::new(
            chat_inner.x + chat_inner.width - 1,
            area.y + area.height - 1,
            1,
            1,
        );
        f.render_widget(Paragraph::new(Line::from(indicator)), indicator_area);
    }
}

fn push_tool_line<'a>(width: u16, mut spans: Vec<Span<'a>>, pad_style: Style) -> Line<'a> {
    // Force pad_style's bg on every span so wrapped continuation rows
    // inherit the tool fill (see `push_ansi_line` for the full rationale).
    let tool_bg = pad_style.bg;
    for span in &mut spans {
        if let Some(bg) = tool_bg {
            span.style = span.style.bg(bg);
        }
    }
    let mut line = Line::from(spans);
    let used = line.width() as u16;
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat((width - used) as usize), pad_style));
    }
    line
}

/// Count the visual rows `text` occupies when wrapped to `width` columns.
/// `Paragraph::scroll` advances by wrapped rows, not raw `Line` entries, so
/// the scroll ceiling must be derived from the wrapped height — otherwise any
/// line that wraps makes the bottom unreachable (the classic "can't scroll to
/// the last line" bug).
fn wrapped_height(text: &ratatui::text::Text, width: u16) -> u16 {
    if width == 0 {
        return text.lines.len() as u16;
    }
    let w = width as usize;
    text.lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            (line_width.div_ceil(w)).max(1) as u16
        })
        .sum()
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

/// Wrap a tool output line to `width` columns with the given indent and
/// tool bg. Word-aware: prefers breaking at whitespace, falls back to
/// hard-breaking if a single word is wider than the available content
/// width. Returns one `Line` per visual row, each padded to exactly
/// `width` so the paragraph never wraps a tool line (which would leak
/// the chat bg on the continuation row's trailing cells).
fn wrap_tool_line<'a>(width: u16, line: Line<'a>, indent: &str, tool_bg: Color) -> Vec<Line<'a>> {
    let indent_w = display_width(indent) as u16;
    let content_w = width.saturating_sub(indent_w);
    if content_w == 0 {
        return vec![blank_tool_line(width, indent, tool_bg)];
    }

    // Collect the full text and remember the first span's style (tool
    // output is usually monochrome; for multi-color ANSI output we lose
    // the per-word color but keep the fill, which is the visible bug).
    let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let base_style = line
        .spans
        .first()
        .map(|s| s.style.bg(tool_bg))
        .unwrap_or_else(|| Style::default().bg(tool_bg));

    let chunks = wrap_text_to_width(&full, content_w);
    if chunks.is_empty() {
        return vec![blank_tool_line(width, indent, tool_bg)];
    }

    chunks
        .into_iter()
        .map(|chunk| {
            // Every row (including wrapped continuations) carries the
            // indent so wrapped text aligns with the first row's content
            // rather than flush left.
            build_tool_row(width, indent, &chunk, base_style, tool_bg)
        })
        .collect()
}

fn blank_tool_line(width: u16, indent: &str, tool_bg: Color) -> Line<'static> {
    build_tool_row(width, indent, "", Style::default().bg(tool_bg), tool_bg)
}

fn build_tool_row(
    width: u16,
    prefix: &str,
    content: &str,
    content_style: Style,
    tool_bg: Color,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(
            prefix.to_string(),
            Style::default().bg(tool_bg),
        ));
    }
    if !content.is_empty() {
        spans.push(Span::styled(content.to_string(), content_style));
    }
    let used: u16 = spans
        .iter()
        .map(|s| display_width(s.content.as_ref()) as u16)
        .sum();
    if used < width {
        spans.push(Span::styled(
            " ".repeat((width - used) as usize),
            Style::default().bg(tool_bg),
        ));
    }
    Line::from(spans)
}

/// Word-aware wrap `text` into chunks of at most `max_width` display
/// columns each. Prefers breaking at whitespace; hard-breaks a single
/// oversized word across chunks.
fn wrap_text_to_width(text: &str, max_width: u16) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let max = max_width as usize;
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for word in text.split_inclusive(' ') {
        let word_w = display_width(word);

        // If the word alone overflows, flush current and hard-break.
        if word_w > max {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_w = 0;
            }
            let mut buf = String::new();
            let mut buf_w = 0;
            for ch in word.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if buf_w + cw > max {
                    chunks.push(std::mem::take(&mut buf));
                    buf_w = 0;
                }
                buf.push(ch);
                buf_w += cw;
            }
            if !buf.is_empty() {
                current.push_str(&buf);
                current_w = buf_w;
            }
            continue;
        }

        // If adding the word (plus a space if not first) overflows, flush.
        let needed = if current.is_empty() {
            word_w
        } else {
            current_w + word_w
        };
        if needed > max {
            chunks.push(std::mem::take(&mut current));
            current.push_str(word);
            current_w = word_w;
        } else {
            current.push_str(word);
            current_w += word_w;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrapped_height_no_wrap_when_lines_fit() {
        let text = Text::from(vec![Line::from("short"), Line::from("also short")]);
        assert_eq!(wrapped_height(&text, 80), 2);
    }

    #[test]
    fn test_wrapped_height_wraps_long_line() {
        // 25 chars at width 10 -> ceil(25/10) = 3 rows
        let text = Text::from(vec![Line::from("a".repeat(25))]);
        assert_eq!(wrapped_height(&text, 10), 3);
    }

    #[test]
    fn test_wrapped_height_empty_line_counts_as_one() {
        let text = Text::from(vec![
            Line::from("short"),        // 1 row
            Line::from("b".repeat(25)), // 3 rows at width 10
            Line::from(""),             // 1 row
        ]);
        assert_eq!(wrapped_height(&text, 10), 5);
    }

    #[test]
    fn test_wrapped_height_zero_width_falls_back_to_line_count() {
        let text = Text::from(vec![Line::from("x"), Line::from("y")]);
        assert_eq!(wrapped_height(&text, 0), 2);
    }

    #[test]
    fn test_wrapped_height_exact_width_line_is_one_row() {
        // A line exactly `width` cols wide must count as 1 row. A line
        // `width + 1` cols wide must count as 2 rows. Tool blocks pad
        // their lines to the render area width — if they pad to a width
        // that's 1 col wider than the render area, every tool line
        // becomes 2 rows and the tool block's height doubles.
        let text_exact = Text::from(vec![Line::from("a".repeat(10))]);
        assert_eq!(wrapped_height(&text_exact, 10), 1);
        let text_over = Text::from(vec![Line::from("a".repeat(11))]);
        assert_eq!(wrapped_height(&text_over, 10), 2);
    }

    #[test]
    fn test_wrap_tool_line_short_returns_one_row() {
        let tool_bg = Color::Rgb(50, 50, 56);
        let content = Line::from("hello world");
        let rows = wrap_tool_line(80, content, "      ", tool_bg);
        assert_eq!(rows.len(), 1);
        // Every row is exactly `width` wide so the paragraph never wraps.
        assert_eq!(rows[0].width(), 80);
    }

    #[test]
    fn test_wrap_tool_line_long_wraps_at_word_boundary() {
        let tool_bg = Color::Rgb(50, 50, 56);
        // "hello world foo bar" is 19 chars; with indent 6 and width 16,
        // content_w = 10. Should wrap to: "hello " (6) + "world " (6) +
        // "foo bar" (7) → fits in chunks of ≤10. "hello world " is 12 > 10
        // so it breaks to "hello " (6) then "world foo " (10) then "bar".
        let content = Line::from("hello world foo bar");
        let rows = wrap_tool_line(16, content, "      ", tool_bg);
        // Every row exactly `width` wide, with the first row carrying the
        // indent and continuation rows left-aligned to the content.
        assert!(rows.len() >= 2);
        for row in &rows {
            assert_eq!(row.width(), 16);
        }
    }

    #[test]
    fn test_wrap_tool_line_hard_breaks_oversized_word() {
        let tool_bg = Color::Rgb(50, 50, 56);
        // A 20-char word at width 10 must hard-break into chunks of 10.
        let content = Line::from("a".repeat(20));
        let rows = wrap_tool_line(10, content, "", tool_bg);
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.width(), 10);
        }
    }

    #[test]
    fn test_wrap_tool_line_continuation_rows_have_indent() {
        let tool_bg = Color::Rgb(50, 50, 56);
        // Indent is 6, width 16, content_w 10. "one two three" is 14
        // chars — wraps to "one two " (8) and "three" (5). Both rows
        // start with the indent so wrapped text aligns with the first
        // row's content rather than flush left.
        let content = Line::from("one two three");
        let rows = wrap_tool_line(16, content, "      ", tool_bg);
        assert!(rows.len() >= 2);
        for row in &rows {
            assert_eq!(row.spans[0].content, "      ");
        }
    }

    #[test]
    fn test_wrap_tool_line_every_row_has_tool_bg() {
        let tool_bg = Color::Rgb(50, 50, 56);
        let content = Line::from("hello world foo bar baz qux");
        let rows = wrap_tool_line(14, content, "      ", tool_bg);
        for row in &rows {
            for span in &row.spans {
                assert_eq!(
                    span.style.bg,
                    Some(tool_bg),
                    "span missing tool bg: {:?}",
                    span.content
                );
            }
        }
    }

    #[test]
    fn test_wrap_text_to_width_basic() {
        let chunks = wrap_text_to_width("the quick brown fox", 10);
        // "the quick " is 10 → fits. "brown fox" is 9 → fits.
        assert_eq!(chunks, vec!["the quick ", "brown fox"]);
    }

    #[test]
    fn test_wrap_text_to_width_hard_break() {
        let chunks = wrap_text_to_width("aaaaaaaaaa", 5);
        assert_eq!(chunks, vec!["aaaaa", "aaaaa"]);
    }
}
