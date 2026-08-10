use std::time::{Duration, Instant};

use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use super::display_width;
use crate::app::{
    App, GoalProposalState, PermissionState, PersonaSummary, PersonaSwitchConfirmState,
    PickerBudget, PickerItem, PickerState, PlanApprovalState, SlashCommand, UserQuestionState,
    PICKER_VISIBLE_ITEMS,
};

pub(super) fn draw_slash_autocomplete(f: &mut Frame, app: &App, cmds: &[SlashCommand], area: Rect) {
    let tokens = &app.theme;
    let bg = Block::default().style(Style::default().bg(tokens.resolve("status_bar.background")));
    f.render_widget(bg, area);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    let visible = inner.height as usize;

    let mut text = Text::default();
    let start = app.slash_scroll;
    for (i, cmd) in cmds.iter().enumerate().skip(start).take(visible) {
        let is_selected = i == app.slash_selected;
        let name_style = if is_selected {
            Style::default()
                .fg(tokens.resolve("selection.foreground"))
                .bg(tokens.resolve("selection.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.body"))
                .bg(tokens.resolve("status_bar.background"))
        };
        let desc_style = if is_selected {
            Style::default()
                .fg(tokens.resolve("selection.foreground"))
                .bg(tokens.resolve("selection.background"))
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background"))
        };
        text.push_line(Line::from(vec![
            Span::raw("  "),
            Span::styled(&cmd.name, name_style),
            Span::raw("  "),
            Span::styled(&cmd.description, desc_style),
        ]));
    }

    f.render_widget(Paragraph::new(text), inner);

    if cmds.len() > visible {
        let scrollbar_area = Rect::new(area.x + area.width - 1, area.y, 1, area.height);
        // content_length is the scroll range (max_scroll + 1), not the item
        // count: ratatui only reaches the track end when position ==
        // content_length - 1, and our scroll tops out at len - visible.
        let mut scrollbar_state = ScrollbarState::new(cmds.len().saturating_sub(visible) + 1)
            .viewport_content_length(visible)
            .position(app.slash_scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

pub(super) fn draw_alert(f: &mut Frame, app: &App, area: Rect) {
    let tokens = &app.theme;
    // Render toasts (bottom-right, stacking upward).
    if !app.toasts.is_empty() {
        draw_toasts(f, app, area);
    }

    // Render the legacy centered alert (backward compat).
    if let Some((text, _)) = &app.alert {
        let width = text.len() as u16 + 4;
        let x = (area.width.saturating_sub(width)) / 2;
        let y = area.height.saturating_sub(3);
        let popup = Rect::new(x, y, width, 1);

        let style = Style::default()
            .fg(tokens.resolve("text.inverse"))
            .bg(tokens.resolve("text.success"))
            .add_modifier(Modifier::BOLD);
        let para = Paragraph::new(Line::from(Span::styled(format!(" {} ", text), style)));
        f.render_widget(Clear, popup);
        f.render_widget(para, popup);
    }
}

/// Render toast notifications in the bottom-right corner, stacking upward.
/// Each toast is a small green-on-black pill with the message text.
fn draw_toasts(f: &mut Frame, app: &App, area: Rect) {
    let tokens = &app.theme;
    let max_width: u16 = 50;
    let visible: Vec<&(String, Instant)> = app.toasts.iter().rev().take(3).collect();
    for (i, (text, born)) in visible.into_iter().cloned().enumerate() {
        let elapsed = born.elapsed();
        let ttl = Duration::from_secs(3);
        let remaining = ttl.saturating_sub(elapsed);
        let fading = remaining < Duration::from_millis(500);
        let width = (text.len() as u16 + 4).min(max_width);
        let x = area.width.saturating_sub(width);
        let y = area.height.saturating_sub(3 + i as u16);
        let popup = Rect::new(x, y, width, 1);

        let style = if fading {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("background"))
        } else {
            Style::default()
                .fg(tokens.resolve("text.inverse"))
                .bg(tokens.resolve("text.success"))
                .add_modifier(Modifier::BOLD)
        };
        let para = Paragraph::new(Line::from(Span::styled(format!(" {} ", text), style)));
        f.render_widget(Clear, popup);
        f.render_widget(para, popup);
    }
}

pub(super) fn draw_permission_modal(
    f: &mut Frame,
    perm: &PermissionState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    // Use most of the terminal height (minus a 2-row margin top/bottom) so
    // long tool inputs are readable. Cap width at 60.
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = PermissionState::popup_height(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let bg = Block::default().style(Style::default().bg(tokens.resolve("status_bar.background")));
    f.render_widget(bg, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    let tool_input = serde_json::to_string_pretty(&perm.input).unwrap_or_default();
    let mut content = Text::default();
    content.push_line(Line::from(vec![
        Span::styled(
            "tool  ",
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background")),
        ),
        Span::styled(
            &perm.tool_name,
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    content.push_line(Line::from(""));
    content.push_line(Line::from(vec![Span::styled(
        "input",
        Style::default()
            .fg(tokens.resolve("text.muted"))
            .bg(tokens.resolve("status_bar.background")),
    )]));
    // Pre-wrap the pretty-printed JSON to the content width. Lines are exact
    // display rows (no ratatui `Wrap`), so `max_scroll` — computed from the
    // same wrapping — can reach the true bottom of the content.
    let json_style = Style::default()
        .fg(tokens.resolve("text.placeholder"))
        .bg(tokens.resolve("status_bar.background"));
    for raw in tool_input.lines() {
        for wrapped in wrap_text(raw, inner.width as usize) {
            content.push_line(Line::from(Span::styled(wrapped, json_style)));
        }
    }
    content.push_line(Line::from(""));
    content.push_line(Line::from(Span::styled(
        "choose:",
        Style::default()
            .fg(tokens.resolve("text.muted"))
            .bg(tokens.resolve("status_bar.background")),
    )));
    let content_height = inner.height.saturating_sub(2).max(3);
    let paragraph = Paragraph::new(content).scroll((perm.scroll, 0));
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: content_height,
    };
    f.render_widget(paragraph, content_area);

    let options = [("allow once", 'a'), ("session", 's'), ("deny", 'd')];
    let mut option_lines = Vec::new();
    for (i, (label, key)) in options.iter().enumerate() {
        let is_selected = i == perm.selected;
        let style = if is_selected {
            Style::default()
                .fg(tokens.resolve("selection.foreground"))
                .bg(tokens.resolve("selection.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.placeholder"))
                .bg(tokens.resolve("status_bar.background"))
        };
        option_lines.push(Span::styled(format!(" [{}] {}  ", key, label), style));
    }

    let option_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );
    f.render_widget(Paragraph::new(Line::from(option_lines)), option_area);
}

/// Render the ask_user overlay into the input slot. The caller passes a rect
/// large enough to fit the current page; the slot is already painted with
/// tokens.resolve("status_bar.background"), so this function only emits the text content.
pub(super) fn draw_user_question(
    f: &mut Frame,
    uq: &UserQuestionState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    // Re-derive the rect exactly like draw_input does: 1-cell padding on every
    // side. Matches the visual weight of the regular input box.
    let padded = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if padded.width == 0 || padded.height == 0 {
        return;
    }

    if uq.review {
        draw_user_question_review(f, uq, padded, tokens);
    } else {
        draw_user_question_page(f, uq, padded, tokens);
    }
}

fn draw_user_question_page(
    f: &mut Frame,
    uq: &UserQuestionState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let question = match uq.questions.get(uq.page) {
        Some(q) => q,
        None => return,
    };
    let n = uq.questions.len();
    let page_label = if n > 1 {
        format!("({} / {})", uq.page + 1, n)
    } else {
        String::new()
    };

    // Line 0+: prompt (cyan, bold), wrapped + optional page label on the right.
    let prompt_style = Style::default()
        .fg(tokens.resolve("text.accent"))
        .bg(tokens.resolve("status_bar.background"))
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default()
        .fg(tokens.resolve("text.muted"))
        .bg(tokens.resolve("status_bar.background"));
    let label_w = display_width(&page_label) as u16;
    let label_reserve = if n > 1 { label_w + 1 } else { 0 };
    let prompt_max = area.width.saturating_sub(label_reserve) as usize;

    // Wrap the prompt text to the available width.
    let prompt_lines = wrap_text(&question.prompt, prompt_max);
    let prompt_height = prompt_lines.len() as u16;
    for (pi, pl) in prompt_lines.iter().enumerate() {
        let mut line = Line::default();
        line.spans.push(Span::styled(pl.clone(), prompt_style));
        // Append page label on the last prompt line.
        if n > 1 && pi == prompt_lines.len() - 1 {
            let used = display_width(pl) as u16;
            let pad = area.width.saturating_sub(used + label_reserve);
            if pad > 0 {
                line.spans
                    .push(Span::styled(" ".repeat(pad as usize), prompt_style));
            }
            line.spans
                .push(Span::styled(page_label.clone(), label_style));
        }
        f.render_widget(
            Paragraph::new(line),
            Rect::new(area.x, area.y + pi as u16, area.width, 1),
        );
    }

    // One line per option + one line for freeform. Each row: number, label,
    // and (if present) the description underneath. Labels and descriptions
    // are wrapped instead of truncated.
    let mut row_index = 0usize;
    let mut cursor_target: Option<(u16, u16)> = None;

    // Track the y position as we lay out rows.
    let mut y = area.y + prompt_height + 1; // leave 1 blank line after the prompt
    if y >= area.y + area.height {
        return;
    }

    for (i, opt) in question.options.iter().enumerate() {
        let selected = row_index == uq.selected;
        let number = format!("{}. ", i + 1);
        let number_style = if selected {
            Style::default()
                .fg(tokens.resolve("text.accent"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background"))
        };
        let label_style = if selected {
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.placeholder"))
                .bg(tokens.resolve("status_bar.background"))
        };

        // Wrap the label to the available width (minus number prefix).
        let label_max = area.width.saturating_sub(number.len() as u16) as usize;
        let label_lines = wrap_text(&opt.label, label_max);
        for (li, ll) in label_lines.iter().enumerate() {
            let mut line = Line::default();
            // Only show the number on the first line.
            if li == 0 {
                line.spans.push(Span::styled(number.clone(), number_style));
            } else {
                line.spans
                    .push(Span::styled(" ".repeat(number.len()), number_style));
            }
            line.spans.push(Span::styled(ll.clone(), label_style));
            f.render_widget(Paragraph::new(line), Rect::new(area.x, y, area.width, 1));
            y += 1;
            if y >= area.y + area.height {
                return;
            }
        }
        row_index += 1;

        if y >= area.y + area.height {
            return;
        }
        // Wrap description if present.
        if !opt.description.is_empty() {
            let desc_style = Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background"));
            let indent = "    ";
            let desc_max = area.width.saturating_sub(indent.len() as u16) as usize;
            let desc_lines = wrap_text(&opt.description, desc_max);
            for dl in &desc_lines {
                let desc_line = Line::from(Span::styled(format!("{}{}", indent, dl), desc_style));
                f.render_widget(
                    Paragraph::new(desc_line),
                    Rect::new(area.x, y, area.width, 1),
                );
                y += 1;
                if y >= area.y + area.height {
                    return;
                }
            }
        }
    }

    // Freeform row: "n. Type your own answer" with optional input field.
    if y < area.y + area.height {
        let freeform_index = question.options.len();
        let selected = row_index == uq.selected;
        let number = format!("{}. ", freeform_index + 1);
        let number_style = if selected {
            Style::default()
                .fg(tokens.resolve("text.accent"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background"))
        };
        let prefix = "Type your own answer";
        let prefix_style = if selected {
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.placeholder"))
                .bg(tokens.resolve("status_bar.background"))
        };
        let line = if selected {
            let text_max = area
                .width
                .saturating_sub(number.len() as u16 + prefix.len() as u16 + 2)
                as usize;
            let shown = truncate(&uq.freeform_text, text_max);
            Line::from(vec![
                Span::styled(number.clone(), number_style),
                Span::styled(prefix.to_string(), prefix_style),
                Span::styled(": ", prefix_style),
                Span::styled(shown.clone(), prefix_style),
            ])
        } else {
            Line::from(vec![
                Span::styled(number.clone(), number_style),
                Span::styled(prefix.to_string(), prefix_style),
            ])
        };
        f.render_widget(Paragraph::new(line), Rect::new(area.x, y, area.width, 1));
        if selected {
            let col_offset = number.len() + prefix.len() + 2 + display_width(&uq.freeform_text);
            let col = area.x + col_offset.min(area.width as usize - 1) as u16;
            cursor_target = Some((col, y));
        }
        y += 1;
    }

    // Hint line at the bottom.
    let hint_y = area.y + area.height.saturating_sub(1);
    if hint_y > y {
        let hint = "↑↓ select   1-9 jump   enter next   esc cancel";
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                hint,
                Style::default()
                    .fg(tokens.resolve("text.muted"))
                    .bg(tokens.resolve("status_bar.background")),
            ))),
            Rect::new(area.x, hint_y, area.width, 1),
        );
    }

    if let Some((cx, cy)) = cursor_target {
        f.set_cursor_position((cx, cy));
    }
}

fn draw_user_question_review(
    f: &mut Frame,
    uq: &UserQuestionState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let header_style = Style::default()
        .fg(tokens.resolve("text.accent"))
        .bg(tokens.resolve("status_bar.background"))
        .add_modifier(Modifier::BOLD);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "review your answers",
            header_style,
        ))),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let mut y = area.y + 2;
    let max_y = area.y + area.height;
    for (i, q) in uq.questions.iter().enumerate() {
        if y + 1 >= max_y {
            break;
        }
        let prompt_style = Style::default()
            .fg(tokens.resolve("text.muted"))
            .bg(tokens.resolve("status_bar.background"));
        let answer_style = Style::default()
            .fg(tokens.resolve("foreground"))
            .bg(tokens.resolve("status_bar.background"))
            .add_modifier(Modifier::BOLD);
        let prompt_max = area.width.saturating_sub(2) as usize;
        for pl in wrap_text(&q.prompt, prompt_max) {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(pl, prompt_style))),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;
            if y >= max_y {
                return;
            }
        }
        let answer = uq.answers.get(i).map(|s| s.as_str()).unwrap_or("");
        let answer_max = area.width.saturating_sub(4) as usize;
        for al in wrap_text(answer, answer_max) {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        "  ▸ ",
                        Style::default()
                            .fg(tokens.resolve("text.accent"))
                            .bg(tokens.resolve("status_bar.background")),
                    ),
                    Span::styled(al, answer_style),
                ])),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;
            if y >= max_y {
                return;
            }
        }
        y += 1; // blank line between pairs
    }

    // Action row: [ Submit ]  [ Cancel ]
    if y < max_y {
        let submit_label = "[ Submit ]";
        let cancel_label = "[ Cancel ]";
        let gap = "  ";
        let submit_w = submit_label.chars().count() as u16;
        let cancel_w = cancel_label.chars().count() as u16;
        let gap_w = gap.chars().count() as u16;
        let total = submit_w + gap_w + cancel_w;
        let start_x = area.x + area.width.saturating_sub(total) / 2;
        let submit_style = if uq.review_selected == 0 {
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background"))
        };
        let cancel_style = if uq.review_selected == 1 {
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background"))
        };
        let mut line = Line::default();
        line.spans
            .push(Span::styled(submit_label.to_string(), submit_style));
        line.spans.push(Span::styled(gap, submit_style));
        line.spans
            .push(Span::styled(cancel_label.to_string(), cancel_style));
        f.render_widget(Paragraph::new(line), Rect::new(start_x, y, total, 1));
    }

    let hint_y = area.y + area.height.saturating_sub(1);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "← → switch   y/n shortcut   enter confirm   esc cancel",
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background")),
        ))),
        Rect::new(area.x, hint_y, area.width, 1),
    );
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if display_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw + 1 > max {
            out.push('…');
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// Wrap a string to `max` display columns, returning one `String` per
/// visual line. Delegates to [`super::chat::wrap_text_to_width`] which is
/// the shared wrapping function also used by the chat surface.
fn wrap_text(s: &str, max: usize) -> Vec<String> {
    super::chat::wrap_text_to_width(s, max as u16)
}

/// Width in cells of the budget slider track in the thinking picker.
const BUDGET_TRACK_WIDTH: u16 = 20;

/// Render the thinking picker's budget row as a slider track:
/// `[█████░░░░░░░] 8192 tok`. The fill reflects the snapped draft; the
/// label shows the raw typed digits, with a cursor underscore while the
/// row is selected.
fn budget_track_line(
    budget: &PickerBudget,
    selected: bool,
    tokens: &crate::theme::Theme,
) -> Line<'static> {
    let value = budget.snapped();
    let total = (budget.info.max - budget.info.min).max(1) as f64;
    let filled = ((value - budget.info.min) as f64 / total * BUDGET_TRACK_WIDTH as f64)
        .round()
        .clamp(0.0, BUDGET_TRACK_WIDTH as f64) as usize;
    let bg = tokens.resolve("status_bar.background");
    let muted = Style::default().fg(tokens.resolve("text.muted")).bg(bg);
    let value_style = if selected {
        Style::default()
            .fg(tokens.resolve("selection.foreground"))
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.body")).bg(bg)
    };
    let cursor = if selected { "_" } else { "" };
    Line::from(vec![
        Span::styled("[", muted),
        Span::styled(
            "█".repeat(filled),
            Style::default().fg(tokens.resolve("text.accent")).bg(bg),
        ),
        Span::styled("░".repeat(BUDGET_TRACK_WIDTH as usize - filled), muted),
        Span::styled("]", muted),
        Span::styled(format!(" {} tok{}", budget.draft, cursor), value_style),
    ])
}

pub(super) fn draw_picker(
    f: &mut Frame,
    picker: &mut PickerState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let width = 60u16.min(area.width.saturating_sub(4));

    // `filtered` is cloned to owned items so the budget-track bookkeeping
    // below can mutate the picker while iterating the rows.
    let filtered: Vec<PickerItem> = picker.filtered().into_iter().cloned().collect();

    // The budget track is only hit-testable while its row is visible.
    if let Some(budget) = picker.budget.as_mut() {
        if !filtered.iter().any(|i| i.id == "budget") {
            budget.track_rect = None;
        }
    }

    // Model the list as flat display rows. Each item contributes its wrapped
    // label/description rows; headers and the budget row contribute one. The
    // rows are display-width exact, so popup height, the visible window, the
    // scroll offset, and the scrollbar all agree with what the render emits.
    let list_width = width.saturating_sub(4).saturating_sub(1) as usize;
    let mut flat_rows: Vec<Line<'static>> = Vec::new();
    let mut flat_items: Vec<usize> = Vec::new();
    // Row start of each filtered item, with a trailing sentinel == total_rows.
    let mut starts: Vec<usize> = Vec::with_capacity(filtered.len() + 1);
    let mut acc = 0;
    for (i, item) in filtered.iter().enumerate() {
        starts.push(acc);
        let rows: Vec<Line<'static>> = if item.header {
            // Section headers render as a non-selectable muted label.
            let style = Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background"))
                .add_modifier(Modifier::DIM);
            wrap_text(&item.label, list_width)
                .into_iter()
                .map(|l| Line::from(Span::styled(l, style)))
                .collect()
        } else if item.id == "budget" {
            // The thinking picker's budget row renders as a slider track.
            let line = if let Some(budget) = picker.budget.as_mut() {
                budget_track_line(budget, i == picker.selected, tokens)
            } else {
                Line::from(vec![Span::styled("budget", Style::default())])
            };
            vec![line]
        } else {
            let is_selected = i == picker.selected;
            let label_style = if is_selected {
                Style::default()
                    .fg(tokens.resolve("selection.foreground"))
                    .bg(tokens.resolve("selection.background"))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(tokens.resolve("text.body"))
                    .bg(tokens.resolve("status_bar.background"))
            };
            let desc_style = if is_selected {
                Style::default()
                    .fg(tokens.resolve("selection.foreground"))
                    .bg(tokens.resolve("selection.background"))
            } else {
                Style::default()
                    .fg(tokens.resolve("text.muted"))
                    .bg(tokens.resolve("status_bar.background"))
            };
            let mut rows: Vec<Line<'static>> = wrap_text(&item.label, list_width)
                .into_iter()
                .map(|l| Line::from(Span::styled(l, label_style)))
                .collect();
            if !item.description.is_empty() {
                rows.extend(
                    wrap_text(&item.description, list_width)
                        .into_iter()
                        .map(|l| Line::from(Span::styled(l, desc_style))),
                );
            }
            rows
        };
        acc += rows.len();
        for line in rows {
            flat_items.push(i);
            flat_rows.push(line);
        }
    }
    starts.push(acc);
    let total_rows: usize = flat_rows.len();

    // Cap the popup at a bounded row budget so long lists don't fill the
    // screen; longer content is reached by scrolling.
    let content_height = total_rows.clamp(1, PICKER_VISIBLE_ITEMS * 2) as u16;
    let height = (4 + content_height).min(area.height.saturating_sub(4));
    let list_area_height = height.saturating_sub(4);
    let visible_rows = list_area_height.max(1) as usize;
    picker.visible_items = visible_rows;

    // Keep the selected item fully in view. Runs every draw so arrow keys and
    // wheel-driven selection moves scroll the list along with them.
    if !filtered.is_empty() {
        let sel = picker.selected.min(starts.len() - 2);
        let sel_start = starts[sel];
        let sel_end = starts[sel + 1];
        // Pin to the item's first row when the item is taller than the
        // window or already scrolled past; otherwise bring its end into view.
        if sel_end - sel_start >= visible_rows || sel_start < picker.scroll {
            picker.scroll = sel_start;
        } else if sel_end > picker.scroll + visible_rows {
            picker.scroll = sel_end.saturating_sub(visible_rows);
        }
    }
    picker.scroll = picker.scroll.min(total_rows.saturating_sub(visible_rows));

    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let bg = Block::default().style(Style::default().bg(tokens.resolve("status_bar.background")));
    f.render_widget(bg, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    let filter_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let prefix = Span::styled(
        "> ",
        Style::default()
            .fg(tokens.resolve("text.accent"))
            .bg(tokens.resolve("status_bar.background")),
    );
    let filter_text = Span::styled(
        &picker.filter,
        Style::default()
            .fg(tokens.resolve("text.body"))
            .bg(tokens.resolve("status_bar.background")),
    );
    let filter_para = Paragraph::new(Line::from(vec![prefix, filter_text]));
    f.render_widget(filter_para, filter_area);

    // Show hint when filter is empty and a hint is set.
    if picker.filter.is_empty() {
        if let Some(ref hint) = picker.hint {
            let hint_span = Span::styled(
                format!("  {}", hint),
                Style::default()
                    .fg(tokens.resolve("text.muted"))
                    .bg(tokens.resolve("status_bar.background")),
            );
            let hint_para = Paragraph::new(Line::from(vec![hint_span]));
            let hint_area = Rect::new(
                filter_area.x + 2 + picker.filter.len() as u16,
                filter_area.y,
                filter_area
                    .width
                    .saturating_sub(2 + picker.filter.len() as u16),
                1,
            );
            f.render_widget(hint_para, hint_area);
        }
    }

    let cursor_x = filter_area.x + 2 + (picker.cursor.min(filter_area.width as usize - 2) as u16);
    f.set_cursor_position((cursor_x, filter_area.y));

    let div_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    let div_line = Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(tokens.resolve("divider")),
    ));
    f.render_widget(Paragraph::new(div_line), div_area);

    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width.saturating_sub(1),
        inner.height.saturating_sub(2),
    );

    let mut list_text = Text::default();
    let window_start = picker.scroll;
    for (row_y, (item_idx, line)) in (list_area.y..).zip(
        flat_items
            .iter()
            .zip(flat_rows)
            .skip(window_start)
            .take(visible_rows),
    ) {
        // The budget row records its track rect for mouse hit-testing.
        if filtered.get(*item_idx).map(|it| it.id == "budget") == Some(true) {
            if let Some(budget) = picker.budget.as_mut() {
                budget.track_rect = Some(Rect::new(list_area.x, row_y, BUDGET_TRACK_WIDTH, 1));
            }
        }
        list_text.push_line(line);
    }

    if filtered.is_empty() {
        list_text.push_line(Line::from(Span::styled(
            "no results",
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("status_bar.background")),
        )));
    }

    // Lines are pre-wrapped to the list width, so no ratatui `Wrap`: every
    // emitted row matches the row math above exactly.
    let list_para = Paragraph::new(list_text);
    f.render_widget(list_para, list_area);

    if total_rows > visible_rows {
        let scrollbar_area = list_area.inner(Margin {
            horizontal: 0,
            vertical: 0,
        });
        // content_length is the scroll range (max_scroll + 1) so the thumb
        // travels the full track; the app's scroll tops out at
        // total_rows - visible_rows.
        let mut scrollbar_state = ScrollbarState::new(total_rows.saturating_sub(visible_rows) + 1)
            .viewport_content_length(visible_rows)
            .position(picker.scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

/// Draw the persona-switch confirm modal. Renders a centered box with the
/// diff between the current and target persona, plus [Confirm] [Cancel]
/// buttons that the user navigates with ←/→/Tab and Enter (or y/n for
/// shortcuts).
pub(super) fn draw_persona_confirm_modal(
    f: &mut Frame,
    state: &PersonaSwitchConfirmState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let width = 64u16.min(area.width.saturating_sub(4));
    let height = 18u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    // Use the target persona's accent color for the border and name.
    let accent = crate::theme::persona_accent(&state.target.name, state.target.color.as_deref());

    let block = Block::bordered()
        .title(Span::styled(
            " Switch persona ",
            Style::default()
                .fg(tokens.resolve("foreground"))
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(accent.bg));
    f.render_widget(block, popup);

    let inner = popup.inner(Margin::new(2, 1));
    let mut text = Text::default();

    // From → To
    let from = state
        .current
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("(none)");
    text.push_line(Line::from(vec![
        Span::styled(
            "  from  ",
            Style::default().fg(tokens.resolve("text.muted")),
        ),
        Span::styled(
            from,
            Style::default().fg(tokens.resolve("text.placeholder")),
        ),
        Span::styled(" → ", Style::default().fg(tokens.resolve("text.muted"))),
        Span::styled(
            state.target.name.clone(),
            Style::default().fg(accent.fg).add_modifier(Modifier::BOLD),
        ),
    ]));

    if !state.target.description.is_empty() {
        let max = inner.width.saturating_sub(8) as usize;
        let desc_lines = wrap_text(&state.target.description, max);
        for (di, dl) in desc_lines.iter().enumerate() {
            text.push_line(Line::from(vec![
                Span::styled(
                    if di == 0 { "  desc  " } else { "        " },
                    Style::default().fg(tokens.resolve("text.muted")),
                ),
                Span::styled(
                    dl.clone(),
                    Style::default().fg(tokens.resolve("text.placeholder")),
                ),
            ]));
        }
    }

    text.push_line(Line::from(""));

    // Model
    let model_str = state
        .target
        .model
        .clone()
        .unwrap_or_else(|| "(inherit active)".into());
    let model_changed =
        state.current.as_ref().and_then(|c| c.model.as_ref()) != state.target.model.as_ref();
    text.push_line(Line::from(vec![
        Span::styled(
            "  model ",
            Style::default().fg(tokens.resolve("text.muted")),
        ),
        Span::styled(
            model_str,
            if model_changed {
                Style::default()
                    .fg(tokens.resolve("text.warning"))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(tokens.resolve("text.placeholder"))
            },
        ),
    ]));

    // Tools
    text.push_line(Line::from(vec![
        Span::styled(
            "  tools ",
            Style::default().fg(tokens.resolve("text.muted")),
        ),
        Span::styled(
            format_tools(&state.target.tools),
            tools_style(&state.target, &state.current, false, tokens),
        ),
    ]));
    text.push_line(Line::from(vec![
        Span::styled(
            "  deny  ",
            Style::default().fg(tokens.resolve("text.muted")),
        ),
        Span::styled(
            format_tools(&state.target.tools_deny),
            tools_style(&state.target, &state.current, true, tokens),
        ),
    ]));

    // Skills
    text.push_line(Line::from(vec![
        Span::styled(
            "  skills",
            Style::default().fg(tokens.resolve("text.muted")),
        ),
        Span::styled(" ".to_string(), Style::default()),
        Span::styled(
            format_tools(&state.target.skills),
            skills_style(&state.target, &state.current, tokens),
        ),
    ]));

    text.push_line(Line::from(""));

    // Buttons
    let confirm_label = "[ Confirm ]";
    let cancel_label = "[ Cancel ]";
    let confirm_style = if state.selected == 0 {
        Style::default()
            .bg(tokens.resolve("surface.success"))
            .fg(tokens.resolve("text.inverse"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    };
    let cancel_style = if state.selected == 1 {
        Style::default()
            .bg(tokens.resolve("surface.error"))
            .fg(tokens.resolve("text.inverse"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    };
    let mut buttons = Text::default();
    buttons.push_line(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(confirm_label, confirm_style),
        Span::styled("   ", Style::default()),
        Span::styled(cancel_label, cancel_style),
    ]));
    let para = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(para, inner);
    let buttons_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(2),
        inner.width,
        1,
    );
    f.render_widget(Paragraph::new(buttons), buttons_area);
}

/// Draw the goal proposal modal. A centered modal showing the proposed
/// objective with an accept/reject toggle.
pub(super) fn draw_goal_proposal(
    f: &mut Frame,
    state: &GoalProposalState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let width = 70u16.min(area.width.saturating_sub(4));
    let height = 9u16.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let block = Block::bordered()
        .title(Span::styled(
            " Goal proposed ",
            Style::default()
                .fg(tokens.resolve("foreground"))
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(tokens.resolve("accent")));
    f.render_widget(block, popup);

    let inner = popup.inner(Margin::new(2, 1));

    let objective_wrapped = crate::ui::chat::wrap_text_to_width(&state.objective, inner.width);
    let obj_lines: Vec<Line> = objective_wrapped
        .iter()
        .map(|c| {
            Line::from(vec![Span::styled(
                c.clone(),
                Style::default().fg(tokens.resolve("foreground")),
            )])
        })
        .collect();

    let accept_selected = state.selected == 0;
    let accept_style = if accept_selected {
        Style::default()
            .bg(tokens.resolve("surface.success"))
            .fg(tokens.resolve("text.inverse"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    };
    let reject_style = if !accept_selected {
        Style::default()
            .bg(tokens.resolve("pill.attention.bg"))
            .fg(tokens.resolve("text.inverse"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    };

    let mut text = Text::default();
    text.push_line(Line::from(Span::styled(
        "The agent has proposed a goal:",
        Style::default().fg(tokens.resolve("text.muted")),
    )));
    text.push_line(Line::from(""));
    for line in obj_lines {
        text.push_line(line);
    }
    text.push_line(Line::from(""));
    text.push_line(Line::from(vec![
        Span::styled(" [y] Accept ", accept_style),
        Span::styled("   ", Style::default()),
        Span::styled(" [n] Reject ", reject_style),
    ]));
    text.push_line(Line::from(Span::styled(
        "  Tab toggle · Enter confirm · Esc cancel",
        Style::default().fg(tokens.resolve("text.muted")),
    )));

    f.render_widget(Paragraph::new(text), inner);
}

/// Draw the plan-approval modal (`handoff_plan`). A large centered modal with
/// the plan rendered as markdown, an approve / request-changes footer, and a
/// feedback input line for the request-changes path.
pub(super) fn draw_plan_approval(
    f: &mut Frame,
    state: &PlanApprovalState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let width = 100u16.min(area.width.saturating_sub(4));
    let height = (area.height * 3 / 4).min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let title = format!(" Plan approval → {} ", state.persona);
    let block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default()
                .fg(tokens.resolve("foreground"))
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(tokens.resolve("accent")));
    f.render_widget(block, popup);

    let inner = popup.inner(Margin::new(2, 1));
    if inner.height < 4 {
        return;
    }

    // Reserve the bottom rows for the footer: a blank spacer, the
    // approve/request-changes line, the (optional) feedback input, and a
    // key-hint line.
    let editing = state.editing_feedback;
    // Pre-compute word-wrapped feedback lines so footer height is accurate.
    let feedback_raw: Vec<&str> = if editing {
        state.feedback.split('\n').collect()
    } else {
        Vec::new()
    };
    let wrap_w = inner.width.saturating_sub(2) as usize;
    let label_w = "  feedback: ".len();
    let cont_w = "            ".len();
    let feedback_rows: usize = if editing {
        feedback_raw
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let avail = if i == 0 {
                    wrap_w.saturating_sub(label_w)
                } else {
                    wrap_w.saturating_sub(cont_w)
                };
                wrap_feedback_line(line, avail).len().max(1)
            })
            .sum()
    } else {
        0
    };
    // Footer: blank spacer + button row + feedback rows + hint.
    let footer_height: u16 = if editing {
        2 + feedback_rows as u16 + 1
    } else {
        3
    };
    let body_height = inner.height.saturating_sub(footer_height);

    let body_area = Rect::new(inner.x, inner.y, inner.width, body_height);
    let md_lines =
        ratatui_mdstream::render_markdown(&state.plan_markdown, inner.width, &tokens.md_theme());
    let para = Paragraph::new(md_lines).scroll((state.scroll, 0));
    f.render_widget(para, body_area);

    // Footer.
    let footer_y = inner.y + body_height;
    let approve_selected = state.selected == 0;
    let changes_selected = state.selected == 1;
    let submit_selected = state.selected == 2;
    let approve_style = if approve_selected {
        Style::default()
            .bg(tokens.resolve("surface.success"))
            .fg(tokens.resolve("text.inverse"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    };
    let changes_style = if changes_selected {
        Style::default()
            .bg(tokens.resolve("pill.attention.bg"))
            .fg(tokens.resolve("text.inverse"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    };

    let mut footer = Text::default();
    let mut button_line = vec![
        Span::styled(" [a] Approve ", approve_style),
        Span::styled("   ", Style::default()),
        Span::styled(" [r] Request changes ", changes_style),
    ];
    if editing {
        let submit_style = if submit_selected {
            Style::default()
                .bg(tokens.resolve("accent"))
                .fg(tokens.resolve("text.inverse"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(tokens.resolve("text.placeholder"))
        };
        button_line.push(Span::styled("   ", Style::default()));
        button_line.push(Span::styled(" [Ctrl+S] Submit ", submit_style));
    }
    footer.push_line(Line::from(button_line));
    if editing {
        for (i, line) in feedback_raw.iter().enumerate() {
            let (prefix, avail_w) = if i == 0 {
                ("  feedback: ", wrap_w.saturating_sub(label_w))
            } else {
                ("            ", wrap_w.saturating_sub(cont_w))
            };
            let chunks = wrap_feedback_line(line, avail_w);
            for (j, chunk) in chunks.iter().enumerate() {
                let p = if j == 0 { prefix } else { "            " };
                let mut spans = vec![Span::styled(
                    p,
                    Style::default().fg(tokens.resolve("text.muted")),
                )];
                spans.push(Span::styled(
                    chunk.clone(),
                    Style::default().fg(tokens.resolve("foreground")),
                ));
                if i == feedback_raw.len() - 1 && j == chunks.len() - 1 {
                    spans.push(Span::styled(
                        "▏",
                        Style::default().fg(tokens.resolve("text.placeholder")),
                    ));
                }
                footer.push_line(Line::from(spans));
            }
        }
    }
    let hint = if editing {
        "type feedback · Enter newline · Ctrl+S or Tab→Submit · Esc back"
    } else {
        "↑/↓ scroll · Tab toggle · Enter confirm · Esc cancel"
    };
    footer.push_line(Line::from(Span::styled(
        format!("  {hint}"),
        Style::default().fg(tokens.resolve("text.muted")),
    )));

    let footer_area = Rect::new(inner.x, footer_y, inner.width, footer_height);
    f.render_widget(Paragraph::new(footer), footer_area);
}

/// Word-wrap a single line of feedback text to `max_width` display columns.
/// Returns one or more chunks, each fitting within the width.
fn wrap_feedback_line(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    wrap_text(text, max_width)
}

fn format_tools(list: &Option<Vec<String>>) -> String {
    match list {
        None => "all".into(),
        Some(v) if v.is_empty() => "(none)".into(),
        Some(v) => v.join(", "),
    }
}

fn tools_style(
    target: &PersonaSummary,
    current: &Option<PersonaSummary>,
    deny: bool,
    tokens: &crate::theme::Theme,
) -> Style {
    let target_v = if deny {
        target.tools_deny.as_ref()
    } else {
        target.tools.as_ref()
    };
    let current_v = current.as_ref().and_then(|c| {
        if deny {
            c.tools_deny.as_ref()
        } else {
            c.tools.as_ref()
        }
    });
    if target_v != current_v {
        Style::default()
            .fg(tokens.resolve("text.warning"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    }
}

fn skills_style(
    target: &PersonaSummary,
    current: &Option<PersonaSummary>,
    tokens: &crate::theme::Theme,
) -> Style {
    if target.skills != current.as_ref().and_then(|c| c.skills.clone()) {
        Style::default()
            .fg(tokens.resolve("text.warning"))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(tokens.resolve("text.placeholder"))
    }
}

/// Draw the keyboard shortcuts overlay. A centered modal showing all
/// available shortcuts grouped by category.
pub fn draw_help_overlay(f: &mut Frame, area: Rect, tokens: &crate::theme::Theme) {
    let width = 64.min(area.width.saturating_sub(4));
    let height = 30.min(area.height.saturating_sub(4));
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height - height) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .title(" Keyboard Shortcuts ")
        .borders(ratatui::widgets::Borders::ALL)
        .style(
            Style::default()
                .bg(tokens.resolve("background"))
                .fg(tokens.resolve("foreground")),
        );
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let key_style = Style::default().fg(tokens.resolve("text.accent"));
    let desc_style = Style::default().fg(tokens.resolve("text.placeholder"));
    let header_style = Style::default()
        .fg(tokens.resolve("text.warning"))
        .add_modifier(Modifier::BOLD);

    let lines = vec![
        Line::from(Span::styled("Global", header_style)),
        Line::from(vec![
            Span::styled("  Ctrl+P     ", key_style),
            Span::styled("command palette", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  ?          ", key_style),
            Span::styled("this help overlay", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+L     ", key_style),
            Span::styled("scroll to bottom", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+C     ", key_style),
            Span::styled("cancel stream / clear input / quit (2x)", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Esc        ", key_style),
            Span::styled("cancel stream (2x) / close modal", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+D     ", key_style),
            Span::styled("delete char forward / quit (when input empty)", desc_style),
        ]),
        Line::from(""),
        Line::from(Span::styled("Input Editing", header_style)),
        Line::from(vec![
            Span::styled("  Alt+Enter  ", key_style),
            Span::styled("insert newline (multi-line)", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+A/E   ", key_style),
            Span::styled("cursor to start / end", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+F/B   ", key_style),
            Span::styled("cursor right / left", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+W     ", key_style),
            Span::styled("delete word backward", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+K     ", key_style),
            Span::styled("delete from cursor to end of line", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+U     ", key_style),
            Span::styled("clear input", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+Z     ", key_style),
            Span::styled("undo", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+Shift+Z", key_style),
            Span::styled("redo", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Up/Down    ", key_style),
            Span::styled("cursor between lines / history", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+Up    ", key_style),
            Span::styled("steer current turn (guide queued msg)", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  @          ", key_style),
            Span::styled("file picker (@-mention)", desc_style),
        ]),
        Line::from(""),
        Line::from(Span::styled("Tools & Sidebar", header_style)),
        Line::from(vec![
            Span::styled("  Ctrl+O     ", key_style),
            Span::styled("expand/collapse bash output", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+T     ", key_style),
            Span::styled("toggle reasoning blocks", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+G     ", key_style),
            Span::styled("expand/collapse tool-call batch", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+1     ", key_style),
            Span::styled("toggle sidebar environment section", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+Shift+C", key_style),
            Span::styled("copy selected text", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  x          ", key_style),
            Span::styled("cancel most recent subagent", desc_style),
        ]),
        Line::from(""),
        Line::from(Span::styled("Slash Commands", header_style)),
        Line::from(vec![
            Span::styled("  /clear     ", key_style),
            Span::styled("clear conversation", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /compact   ", key_style),
            Span::styled("force context compaction", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /model     ", key_style),
            Span::styled("switch model", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /persona   ", key_style),
            Span::styled("switch persona", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  Shift+Tab  ", key_style),
            Span::styled("cycle persona (Ctrl+Shift+Tab = back)", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /rewind    ", key_style),
            Span::styled("rewind to earlier message", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /cost      ", key_style),
            Span::styled("session cost breakdown", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /sessions  ", key_style),
            Span::styled("list resumable sessions", desc_style),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(tokens.resolve("text.muted")),
        )),
    ];

    let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

/// Compute a centered rectangle of the given size inside `area`.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let popup_layout = Layout::vertical([
        Constraint::Length(area.height.saturating_sub(h) / 2),
        Constraint::Length(h),
        Constraint::Length(area.height.saturating_sub(h) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Length(area.width.saturating_sub(w) / 2),
        Constraint::Length(w),
        Constraint::Length(area.width.saturating_sub(w) / 2),
    ])
    .split(popup_layout[1])[1]
}
