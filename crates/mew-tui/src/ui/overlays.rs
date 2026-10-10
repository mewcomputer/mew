use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use super::display_width;
use crate::app::{
    App, DecisionLayout, DecisionView, GoalComposeState, GoalProposalState, PermissionState,
    PersonaSummary, PersonaSwitchConfirmState, PickerBudget, PickerItem, PickerState, SlashCommand,
    UserQuestionState, PICKER_VISIBLE_ITEMS,
};

/// A single selectable row in an inline autocomplete list (slash commands or
/// `@`-mentions). Name and description share one line, matching the compact
/// look used by both the slash and reference pickers.
struct AutocompleteEntry<'a> {
    name: &'a str,
    description: &'a str,
}

/// Render an inline, borderless autocomplete list. This is the shared
/// component behind both the slash-command list and the `@`-mention list:
/// a raised surface panel with single-line entries and a slim scrollbar.
fn draw_autocomplete(
    f: &mut Frame,
    tokens: &crate::theme::Theme,
    entries: &[AutocompleteEntry<'_>],
    selected: usize,
    scroll: usize,
    area: Rect,
) {
    let surface = tokens.resolve("panel.overlay");
    // Clear any content behind the list (e.g. the landing logo) before
    // painting the surface. A `Block` only sets the background color; the
    // foreground glyphs underneath would otherwise stay visible.
    f.render_widget(Clear, area);
    let bg = Block::default().style(Style::default().bg(surface));
    f.render_widget(bg, area);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    let visible = inner.height as usize;

    let mut text = Text::default();
    for (i, entry) in entries.iter().enumerate().skip(scroll).take(visible) {
        let is_selected = i == selected;
        let name_style = if is_selected {
            Style::default()
                .fg(tokens.resolve("selection.foreground"))
                .bg(tokens.resolve("selection.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(tokens.resolve("text.body")).bg(surface)
        };
        let desc_style = if is_selected {
            Style::default()
                .fg(tokens.resolve("selection.foreground"))
                .bg(tokens.resolve("selection.background"))
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(surface)
        };
        text.push_line(Line::from(vec![
            Span::raw("  "),
            Span::styled(entry.name, name_style),
            Span::raw("  "),
            Span::styled(entry.description, desc_style),
        ]));
    }

    f.render_widget(Paragraph::new(text), inner);

    if entries.len() > visible {
        let scrollbar_area = Rect::new(area.x + area.width - 1, area.y, 1, area.height);
        // content_length is the scroll range (max_scroll + 1), not the item
        // count: ratatui only reaches the track end when position ==
        // content_length - 1, and our scroll tops out at len - visible.
        let mut scrollbar_state = ScrollbarState::new(entries.len().saturating_sub(visible) + 1)
            .viewport_content_length(visible)
            .position(scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

pub(super) fn draw_slash_autocomplete(f: &mut Frame, app: &App, cmds: &[SlashCommand], area: Rect) {
    let entries: Vec<AutocompleteEntry<'_>> = cmds
        .iter()
        .map(|cmd| AutocompleteEntry {
            name: &cmd.name,
            description: &cmd.description,
        })
        .collect();
    draw_autocomplete(
        f,
        &app.theme,
        &entries,
        app.slash_selected,
        app.slash_scroll,
        area,
    );
}

/// Render the inline `@`-mention autocomplete list above the chat input.
/// Reuses the slash-command component with the unified file/model/skill/
/// subagent catalog, and keeps the selected entry in view while scrolling.
pub(super) fn draw_reference_autocomplete(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(picker) = app.picker.as_mut() else {
        return;
    };
    if !picker.is_at_picker() {
        return;
    }

    let filtered_len = picker.filtered().len();
    if filtered_len == 0 {
        return;
    }

    let visible = (area.height.saturating_sub(2) as usize).max(1);
    picker.selected = picker.selected.min(filtered_len - 1);
    if picker.selected < picker.scroll {
        picker.scroll = picker.selected;
    } else if picker.selected >= picker.scroll + visible {
        picker.scroll = picker.selected.saturating_sub(visible - 1);
    }
    picker.scroll = picker.scroll.min(filtered_len.saturating_sub(visible));

    let entries: Vec<AutocompleteEntry<'_>> = picker
        .filtered()
        .into_iter()
        .map(|item| AutocompleteEntry {
            name: &item.label,
            description: &item.description,
        })
        .collect();

    draw_autocomplete(
        f,
        &app.theme,
        &entries,
        picker.selected,
        picker.scroll,
        area,
    );
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
        let para =
            Paragraph::new(Line::from(Span::styled(format!(" {} ", text), style))).style(style);
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
        let para =
            Paragraph::new(Line::from(Span::styled(format!(" {} ", text), style))).style(style);
        f.render_widget(Clear, popup);
        f.render_widget(para, popup);
    }
}

fn draw_decision_frame(
    f: &mut Frame,
    layout: &DecisionLayout,
    title: &str,
    accent: ratatui::style::Color,
    tokens: &crate::theme::Theme,
) {
    f.render_widget(Clear, layout.popup);
    f.render_widget(
        Block::bordered()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(tokens.resolve("foreground"))
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(tokens.resolve("panel.overlay"))),
        layout.popup,
    );
}

fn draw_decision_body(
    f: &mut Frame,
    layout: &DecisionLayout,
    lines: Vec<Line<'_>>,
    scroll: u16,
    view: &DecisionView,
    tokens: &crate::theme::Theme,
) {
    f.render_widget(
        Paragraph::new(lines)
            .style(
                Style::default()
                    .fg(tokens.resolve("foreground"))
                    .bg(tokens.resolve("panel.overlay")),
            )
            .scroll((scroll, 0)),
        layout.body,
    );
    if view.max_scroll > 0 && layout.body.height > 0 {
        let track = Rect::new(
            layout.popup.right().saturating_sub(1),
            layout.body.y,
            1,
            layout.body.height,
        );
        let mut state = ScrollbarState::new(view.max_scroll as usize + 1).position(scroll as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(Style::default().fg(tokens.resolve("panel.overlay")))
                .thumb_style(Style::default().fg(tokens.resolve("text.muted"))),
            track,
            &mut state,
        );
    }
}

fn draw_decision_actions(
    f: &mut Frame,
    layout: &DecisionLayout,
    labels: &[&str],
    selected: usize,
    scrollable: bool,
    tokens: &crate::theme::Theme,
) {
    let mut rows = Vec::new();
    let mut spans = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let style = if i == selected {
            Style::default()
                .fg(tokens.resolve("selection.foreground"))
                .bg(tokens.resolve("selection.background"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.placeholder"))
                .bg(tokens.resolve("panel.overlay"))
        };
        let span = Span::styled(*label, style);
        if layout.actions.height > 1 {
            rows.push(Line::from(span));
        } else {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(span);
        }
    }
    if layout.actions.height <= 1 {
        rows.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(rows), layout.actions);
    let hint = match (layout.hint.width, scrollable) {
        (53.., true) => "PgUp/PgDn scroll · Tab choose · Enter/Esc",
        (36.., true) => "PgUp/PgDn scroll · Enter/Esc",
        (26.., true) => "PgUp/PgDn · Enter/Esc",
        (19.., _) => "Tab/Enter · Esc",
        (9.., _) => "Enter/Esc",
        _ => "Esc",
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(tokens.resolve("text.muted"))),
        layout.hint,
    );
}

pub(super) fn draw_permission_modal(
    f: &mut Frame,
    perm: &mut PermissionState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let rows = perm.body_lines(area.width);
    let layout = DecisionLayout::new(
        area,
        60,
        rows.len(),
        PermissionState::action_rows(area.width),
    );
    layout.clamp_scroll(&mut perm.scroll, &mut perm.view, rows.len());
    draw_decision_frame(
        f,
        &layout,
        " Allow tool? ",
        tokens.resolve("text.warning"),
        tokens,
    );
    let lines = rows.into_iter().map(Line::from).collect();
    draw_decision_body(f, &layout, lines, perm.scroll, &perm.view, tokens);
    draw_decision_actions(
        f,
        &layout,
        &["[a] allow once", "[s] session", "[d] deny"],
        perm.selected,
        perm.view.max_scroll > 0,
        tokens,
    );
}

/// Render the ask_user prompt on a raised surface in the input slot.
pub(super) fn draw_user_question(
    f: &mut Frame,
    uq: &UserQuestionState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default().style(Style::default().bg(tokens.resolve("panel.overlay"))),
        area,
    );
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
        .bg(tokens.resolve("panel.overlay"))
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default()
        .fg(tokens.resolve("text.muted"))
        .bg(tokens.resolve("panel.overlay"));
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
                .bg(tokens.resolve("panel.overlay"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("panel.overlay"))
        };
        let label_style = if selected {
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("panel.overlay"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.placeholder"))
                .bg(tokens.resolve("panel.overlay"))
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
                .bg(tokens.resolve("panel.overlay"));
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
                .bg(tokens.resolve("panel.overlay"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("panel.overlay"))
        };
        let prefix = "Type your own answer";
        let prefix_style = if selected {
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("panel.overlay"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.placeholder"))
                .bg(tokens.resolve("panel.overlay"))
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
            let col_offset = number.len()
                + prefix.len()
                + 2
                + display_width(&uq.freeform_text[..uq.freeform_cursor]);
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
                    .bg(tokens.resolve("panel.overlay")),
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
        .bg(tokens.resolve("panel.overlay"))
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
            .bg(tokens.resolve("panel.overlay"));
        let answer_style = Style::default()
            .fg(tokens.resolve("foreground"))
            .bg(tokens.resolve("panel.overlay"))
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
                            .bg(tokens.resolve("panel.overlay")),
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
                .bg(tokens.resolve("panel.overlay"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("panel.overlay"))
        };
        let cancel_style = if uq.review_selected == 1 {
            Style::default()
                .fg(tokens.resolve("foreground"))
                .bg(tokens.resolve("panel.overlay"))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(tokens.resolve("text.muted"))
                .bg(tokens.resolve("panel.overlay"))
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
                .bg(tokens.resolve("panel.overlay")),
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
    for grapheme in s.graphemes(true) {
        let cw = display_width(grapheme);
        if w + cw + 1 > max {
            out.push('…');
            break;
        }
        out.push_str(grapheme);
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
    let bg = tokens.resolve("panel.overlay");
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
    // Centered pickers (command palette, model, theme, etc.) keep a narrow
    // centered popup; the inline `@`-mention list is rendered by
    // `draw_reference_autocomplete` above the input instead.
    let width = 60u16.min(area.width.saturating_sub(4));
    let (title, empty_message) = match picker.kind.as_str() {
        "command" => ("Commands", "No commands"),
        "model" => ("Models", "No models"),
        "session" => ("Conversations", "No conversations"),
        "project" => ("New conversation in project", "No projects"),
        "theme" => ("Themes", "No themes"),
        "persona" => ("Personas", "No personas"),
        "permission_mode" => ("Permissions", "No permission modes"),
        "thinking_variant" => ("Thinking", "No thinking variants"),
        "rewind" => ("Rewind conversation", "No earlier messages"),
        _ => ("Select", "No options"),
    };
    let inner_width = width.saturating_sub(4);
    let footer_style = Style::default()
        .fg(tokens.resolve("text.muted"))
        .bg(tokens.resolve("panel.overlay"));
    let mut footer: Vec<Line<'static>> =
        wrap_text("↑↓ move  Enter choose  Esc close", inner_width as usize)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, footer_style)))
            .collect();
    if let Some(hint) = &picker.hint {
        if area.height.saturating_sub(4) > 6 + footer.len() as u16 {
            footer.push(Line::from(Span::styled(
                truncate(hint, inner_width as usize),
                footer_style,
            )));
        }
    }

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
                .bg(tokens.resolve("panel.overlay"))
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
                    .bg(tokens.resolve("panel.overlay"))
            };
            let desc_style = if is_selected {
                Style::default()
                    .fg(tokens.resolve("selection.foreground"))
                    .bg(tokens.resolve("selection.background"))
            } else {
                Style::default()
                    .fg(tokens.resolve("text.muted"))
                    .bg(tokens.resolve("panel.overlay"))
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
    // Title, filter, divider, footer, and top/bottom padding surround the list.
    let footer_height = footer.len() as u16;
    let chrome = 5 + footer_height;
    let max_height = area
        .height
        .saturating_sub(4)
        .max((chrome + 1).min(area.height));
    let height = (chrome + content_height).min(max_height);
    let list_area_height = height.saturating_sub(chrome);
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
    let popup = Rect::new(area.x + x, area.y + y, width, height);

    f.render_widget(Clear, popup);

    let bg = Block::default().style(Style::default().bg(tokens.resolve("panel.overlay")));
    f.render_widget(bg, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate(title, inner.width as usize),
            Style::default()
                .fg(tokens.resolve("text.body"))
                .bg(tokens.resolve("panel.overlay"))
                .add_modifier(Modifier::BOLD),
        ))),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let filter_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    let prefix = Span::styled(
        "> ",
        Style::default()
            .fg(tokens.resolve("text.accent"))
            .bg(tokens.resolve("panel.overlay")),
    );
    let filter_text = Span::styled(
        &picker.filter,
        Style::default()
            .fg(tokens.resolve("text.body"))
            .bg(tokens.resolve("panel.overlay")),
    );
    let filter_para = Paragraph::new(Line::from(vec![prefix, filter_text]));
    f.render_widget(filter_para, filter_area);

    let cursor_x = filter_area.x
        + 2
        + (display_width(&picker.filter[..picker.cursor])
            .min(filter_area.width.saturating_sub(2) as usize) as u16);
    if filter_area.width > 2 && filter_area.y < f.area().bottom() {
        f.set_cursor_position((cursor_x.min(filter_area.right() - 1), filter_area.y));
    }

    let div_area = Rect::new(inner.x, inner.y + 2, inner.width, 1);
    let div_line = Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default()
            .fg(tokens.resolve("divider"))
            .bg(tokens.resolve("panel.overlay")),
    ));
    f.render_widget(Paragraph::new(div_line), div_area);

    let list_area = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width.saturating_sub(1),
        list_area_height,
    );

    if filtered.is_empty() {
        let message = if picker.filter.is_empty() {
            empty_message
        } else {
            "No matches"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(message, footer_style))),
            list_area,
        );
    } else {
        render_picker_list(f, picker, &filtered, &flat_items, flat_rows, list_area);
    }
    f.render_widget(
        Paragraph::new(footer),
        Rect::new(inner.x, list_area.bottom(), inner.width, footer_height),
    );
}

/// Render the picker's item list (and scrollbar) into `list_area`.
fn render_picker_list(
    f: &mut Frame,
    picker: &mut PickerState,
    filtered: &[PickerItem],
    flat_items: &[usize],
    flat_rows: Vec<Line<'static>>,
    list_area: Rect,
) {
    let total_rows: usize = flat_rows.len();
    let mut list_text = Text::default();
    let window_start = picker.scroll;
    for (row_y, (item_idx, line)) in (list_area.y..).zip(
        flat_items
            .iter()
            .zip(flat_rows)
            .skip(window_start)
            .take(picker.visible_items),
    ) {
        // The budget row records its track rect for mouse hit-testing.
        if filtered.get(*item_idx).map(|it| it.id == "budget") == Some(true) {
            if let Some(budget) = picker.budget.as_mut() {
                budget.track_rect = Some(Rect::new(list_area.x, row_y, BUDGET_TRACK_WIDTH, 1));
            }
        }
        list_text.push_line(line);
    }

    // Lines are pre-wrapped to the list width, so no ratatui `Wrap`: every
    // emitted row matches the row math above exactly.
    let list_para = Paragraph::new(list_text);
    f.render_widget(list_para, list_area);

    if total_rows > picker.visible_items {
        let scrollbar_area = list_area.inner(Margin {
            horizontal: 0,
            vertical: 0,
        });
        // content_length is the scroll range (max_scroll + 1) so the thumb
        // travels the full track; the app's scroll tops out at
        // total_rows - visible_rows.
        let mut scrollbar_state =
            ScrollbarState::new(total_rows.saturating_sub(picker.visible_items) + 1)
                .viewport_content_length(picker.visible_items)
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
fn decision_text_rows(text: &str, width: u16) -> Vec<String> {
    text.split('\n')
        .flat_map(|line| crate::ui::chat::wrap_text_to_width(line, width))
        .collect()
}

pub(super) fn draw_persona_confirm_modal(
    f: &mut Frame,
    state: &mut PersonaSwitchConfirmState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let width = DecisionLayout::content_width(area.width, 64);
    let muted = Style::default().fg(tokens.resolve("text.muted"));
    let foreground = Style::default().fg(tokens.resolve("foreground"));
    let accent = crate::theme::persona_accent(&state.target.name, state.target.color.as_deref());
    let from = state
        .current
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("(none)");
    let model = state.target.model.as_deref().unwrap_or("(inherit active)");
    let model_changed =
        state.current.as_ref().and_then(|c| c.model.as_ref()) != state.target.model.as_ref();
    let mut rows = Vec::new();
    for (label, value, style) in [
        (
            "from",
            format!("{} → {}", from, state.target.name),
            foreground,
        ),
        ("description", state.target.description.clone(), foreground),
        (
            "model",
            model.to_owned(),
            if model_changed {
                Style::default().fg(tokens.resolve("text.warning"))
            } else {
                foreground
            },
        ),
        (
            "tools",
            format_tools(&state.target.tools),
            tools_style(&state.target, &state.current, false, tokens),
        ),
        (
            "deny",
            format_tools(&state.target.tools_deny),
            tools_style(&state.target, &state.current, true, tokens),
        ),
        (
            "skills",
            format_tools(&state.target.skills),
            skills_style(&state.target, &state.current, tokens),
        ),
    ] {
        if value.is_empty() {
            continue;
        }
        if !rows.is_empty() {
            rows.push(Line::default());
        }
        rows.push(Line::from(Span::styled(label, muted)));
        rows.extend(
            decision_text_rows(&value, width)
                .into_iter()
                .map(|row| Line::from(Span::styled(row, style))),
        );
    }
    let action_rows = if width < 29 { 2 } else { 1 };
    let layout = DecisionLayout::new(area, 64, rows.len(), action_rows);
    layout.clamp_scroll(&mut state.scroll, &mut state.view, rows.len());
    draw_decision_frame(f, &layout, " Switch persona ", accent.bg, tokens);
    draw_decision_body(f, &layout, rows, state.scroll, &state.view, tokens);
    draw_decision_actions(
        f,
        &layout,
        &["[y] Confirm", "[n] Cancel"],
        state.selected,
        state.view.max_scroll > 0,
        tokens,
    );
}

/// Proposed objectives scroll separately from their decision controls.
pub(super) fn draw_goal_proposal(
    f: &mut Frame,
    state: &mut GoalProposalState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let width = DecisionLayout::content_width(area.width, 70);
    let rows = decision_text_rows(&state.objective, width);
    let action_rows = if width < 26 { 2 } else { 1 };
    let layout = DecisionLayout::new(area, 70, rows.len(), action_rows);
    layout.clamp_scroll(&mut state.scroll, &mut state.view, rows.len());
    draw_decision_frame(
        f,
        &layout,
        " Goal proposed ",
        tokens.resolve("accent"),
        tokens,
    );
    draw_decision_body(
        f,
        &layout,
        rows.into_iter().map(Line::from).collect(),
        state.scroll,
        &state.view,
        tokens,
    );
    draw_decision_actions(
        f,
        &layout,
        &["[y] Accept", "[n] Reject"],
        state.selected,
        state.view.max_scroll > 0,
        tokens,
    );
}

/// The goal editor's viewport follows the caret, while confirmation stays pinned.
pub(super) fn draw_goal_compose(
    f: &mut Frame,
    state: &GoalComposeState,
    area: Rect,
    tokens: &crate::theme::Theme,
) {
    let width = DecisionLayout::content_width(area.width, 70);
    let rows: Vec<_> = crate::app::visual_rows(&state.title, width.saturating_sub(1)).collect();
    let cursor = rows
        .iter()
        .enumerate()
        .find_map(|(row, (start, text, _))| {
            let end = start + text.len();
            let logical_end = state
                .title
                .as_bytes()
                .get(end)
                .is_none_or(|byte| *byte == b'\n');
            (state.cursor < end || (state.cursor == end && logical_end)).then(|| {
                (
                    row,
                    display_width(&text[..state.cursor.saturating_sub(*start)]),
                )
            })
        })
        .unwrap_or((0, 0));
    let action_rows = if width < 29 { 2 } else { 1 };
    let layout = DecisionLayout::new(area, 70, rows.len().max(3), action_rows);
    draw_decision_frame(f, &layout, " Set goal ", tokens.resolve("accent"), tokens);
    let first = cursor
        .0
        .saturating_sub(layout.body.height.saturating_sub(1) as usize);
    let lines = rows
        .iter()
        .skip(first)
        .take(layout.body.height as usize)
        .map(|(_, text, _)| {
            Line::from(Span::styled(
                *text,
                Style::default().fg(tokens.resolve("foreground")),
            ))
        })
        .collect::<Vec<_>>();
    f.render_widget(Paragraph::new(lines), layout.body);
    draw_decision_actions(
        f,
        &layout,
        &["[Enter] Set", "[Esc] Cancel"],
        0,
        false,
        tokens,
    );
    let hint = if state.replacing.is_some() && layout.hint.width >= 24 {
        "replaces goal · Enter/Esc"
    } else if layout.hint.width >= 21 {
        "Enter set · Esc cancel"
    } else if layout.hint.width >= 9 {
        "Enter/Esc"
    } else {
        "Esc"
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(tokens.resolve(
            if state.replacing.is_some() {
                "text.warning"
            } else {
                "text.muted"
            },
        ))),
        layout.hint,
    );
    if layout.body.width > 0 && layout.body.height > 0 {
        f.set_cursor_position((
            layout.body.x + (cursor.1 as u16).min(layout.body.width - 1),
            layout.body.y + cursor.0.saturating_sub(first) as u16,
        ));
    }
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
pub fn draw_help_overlay(f: &mut Frame, app: &mut App, area: Rect) {
    let tokens = &app.theme;
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
                .bg(tokens.resolve("panel.overlay"))
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
            Span::styled("  Ctrl+Shift+Z  ", key_style),
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
            Span::styled("reference picker (@-mention)", desc_style),
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
            Span::styled("  Ctrl+Shift+C  ", key_style),
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
    ];

    let content_area = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(1),
        inner.height.saturating_sub(1),
    );
    let mut content = Text::default();
    for line in lines {
        if line.spans.len() == 2 && content_area.width < 48 {
            content.push_line(Line::from(line.spans[0].clone()));
            let description = &line.spans[1];
            for text in wrap_text(&description.content, content_area.width as usize) {
                content.push_line(Line::from(Span::styled(text, description.style)));
            }
        } else if line.spans.len() == 2 && line.spans[0].width() < content_area.width as usize {
            let label = &line.spans[0];
            let description = &line.spans[1];
            let description_width = content_area.width as usize - label.width();
            for (row, text) in wrap_text(&description.content, description_width)
                .into_iter()
                .enumerate()
            {
                let prefix = if row == 0 {
                    label.clone()
                } else {
                    Span::raw(" ".repeat(label.width()))
                };
                content.push_line(Line::from(vec![
                    prefix,
                    Span::styled(text, description.style),
                ]));
            }
        } else if line.spans.is_empty() {
            content.push_line(line);
        } else {
            for span in line.spans {
                for text in wrap_text(&span.content, content_area.width as usize) {
                    content.push_line(Line::from(Span::styled(text, span.style)));
                }
            }
        }
    }
    app.help_page_rows = content_area.height;
    app.help_max_scroll = (content.lines.len() as u16).saturating_sub(content_area.height);
    app.help_scroll = app.help_scroll.min(app.help_max_scroll);
    f.render_widget(
        Paragraph::new(content).scroll((app.help_scroll, 0)),
        content_area,
    );

    if app.help_max_scroll > 0 {
        let mut state = ScrollbarState::new(app.help_max_scroll as usize + 1)
            .position(app.help_scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        f.render_stateful_widget(
            scrollbar,
            Rect {
                height: content_area.height,
                ..inner
            },
            &mut state,
        );
    }
    if inner.height > 0 {
        let footer = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        let hint = if inner.width >= display_width("↑↓/PgUp/PgDn scroll · Esc close") as u16 {
            "↑↓/PgUp/PgDn scroll · Esc close"
        } else if inner.width >= display_width("↑↓ · Esc close") as u16 {
            "↑↓ · Esc close"
        } else {
            "Esc close"
        };
        f.render_widget(
            Paragraph::new(hint).style(Style::default().fg(tokens.resolve("text.muted"))),
            footer,
        );
    }
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

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncated_labels_keep_graphemes_whole_and_use_the_column_budget() {
        assert_eq!(truncate("👩‍💻xy", 3), "👩‍💻…");
        assert_eq!(truncate("e\u{301}xyz", 2), "e\u{301}…");
        assert_eq!(truncate("日本語", 4), "日…");
        assert_eq!(truncate("👩‍💻", 2), "👩‍💻");
        assert_eq!(truncate("text", 0), "");
    }
}
