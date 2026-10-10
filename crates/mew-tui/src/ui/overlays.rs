use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
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

/// Keep compact inline lists usable when the landing screen clips their height.
fn autocomplete_rows(area: Rect) -> Rect {
    let inset = u16::from(area.width > 2);
    let chrome = area.height >= 5;
    Rect::new(
        area.x + inset,
        area.y + if chrome { 2 } else { 0 },
        area.width.saturating_sub(inset * 2),
        area.height.saturating_sub(if chrome { 4 } else { 0 }),
    )
}

fn keep_autocomplete_selection_visible(
    selected: usize,
    scroll: &mut usize,
    len: usize,
    visible: usize,
) {
    let visible = visible.max(1);
    *scroll = (*scroll).min(len.saturating_sub(visible));
    if selected < *scroll {
        *scroll = selected;
    } else if selected >= scroll.saturating_add(visible) {
        *scroll = selected.saturating_sub(visible - 1);
    }
}

/// Render a raised list with its heading and completion keys pinned in place.
fn draw_autocomplete(
    f: &mut Frame,
    tokens: &crate::theme::Theme,
    entries: &[AutocompleteEntry<'_>],
    selected: usize,
    scroll: usize,
    labels: (&str, &str),
    area: Rect,
) {
    let surface = tokens.resolve("panel.overlay");
    let (title, hint) = labels;
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(Style::default().bg(surface)), area);
    let rows = autocomplete_rows(area);
    let visible = rows.height as usize;
    if area.height >= 5 {
        let heading = format!("{title}  {}/{}", selected + 1, entries.len());
        f.render_widget(
            Paragraph::new(truncate(&heading, rows.width as usize)).style(
                Style::default()
                    .fg(tokens.resolve("text.body"))
                    .bg(surface)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(rows.x, area.y + 1, rows.width, 1),
        );
        f.render_widget(
            Paragraph::new(if display_width(hint) <= rows.width as usize {
                hint
            } else if title == "Commands" && rows.width >= 25 {
                "Tab complete · Enter · Esc"
            } else if title == "References" && rows.width >= 22 {
                "↑↓ · Enter insert · Esc"
            } else {
                "Enter · Esc"
            })
            .style(
                Style::default()
                    .fg(tokens.resolve("text.muted"))
                    .bg(surface),
            ),
            Rect::new(rows.x, area.bottom() - 2, rows.width, 1),
        );
    }
    for (row, (i, entry)) in entries
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .enumerate()
    {
        let selected = i == selected;
        let style = if selected {
            Style::default()
                .fg(tokens.resolve("selection.foreground"))
                .bg(tokens.resolve("selection.background"))
        } else {
            Style::default().fg(tokens.resolve("text.body")).bg(surface)
        };
        let name = truncate(entry.name, rows.width as usize);
        let remaining = (rows.width as usize).saturating_sub(display_width(&name) + 2);
        let mut spans = vec![Span::styled(name, style.add_modifier(Modifier::BOLD))];
        if remaining > 0 && !entry.description.is_empty() {
            let description_style = if selected {
                style
            } else {
                style.fg(tokens.resolve("text.muted"))
            };
            spans.push(Span::styled("  ", style));
            spans.push(Span::styled(
                truncate(entry.description, remaining),
                description_style,
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)).style(style),
            Rect::new(rows.x, rows.y + row as u16, rows.width, 1),
        );
    }
    if entries.len() > visible && visible > 0 && area.width > 2 {
        let scrollbar_area = Rect::new(area.right() - 1, rows.y, 1, rows.height);
        let mut state = ScrollbarState::new(entries.len().saturating_sub(visible) + 1)
            .viewport_content_length(visible)
            .position(scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .track_style(
                Style::default()
                    .fg(tokens.resolve("text.muted"))
                    .bg(surface),
            )
            .thumb_style(Style::default().fg(tokens.resolve("text.body")).bg(surface));
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut state);
    }
}

pub(super) fn draw_slash_autocomplete(
    f: &mut Frame,
    app: &mut App,
    cmds: &[SlashCommand],
    area: Rect,
) {
    app.slash_visible = autocomplete_rows(area).height as usize;
    app.slash_selected = app.slash_selected.min(cmds.len().saturating_sub(1));
    keep_autocomplete_selection_visible(
        app.slash_selected,
        &mut app.slash_scroll,
        cmds.len(),
        app.slash_visible,
    );
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
        (
            "Commands",
            "↑↓ choose · Tab complete · Enter run · Esc close",
        ),
        area,
    );
}

/// Render references using the actual allocated viewport above the composer.
pub(super) fn draw_reference_autocomplete(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(picker) = app.picker.as_mut().filter(|p| p.is_at_picker()) else {
        return;
    };
    let filtered_len = picker.filtered().len();
    if filtered_len == 0 {
        return;
    }
    picker.visible_items = autocomplete_rows(area).height as usize;
    picker.selected = picker.selected.min(filtered_len - 1);
    keep_autocomplete_selection_visible(
        picker.selected,
        &mut picker.scroll,
        filtered_len,
        picker.visible_items,
    );
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
        ("References", "↑↓ choose · Enter insert · Esc close"),
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
    track_width: u16,
) -> Line<'static> {
    let value = budget.snapped();
    let total = (budget.info.max - budget.info.min).max(1) as f64;
    let filled = ((value - budget.info.min) as f64 / total * track_width as f64)
        .round()
        .clamp(0.0, track_width as f64) as usize;
    let bg = tokens.resolve(if selected {
        "selection.background"
    } else {
        "panel.overlay"
    });
    let muted = Style::default()
        .fg(if selected {
            tokens.resolve("selection.foreground")
        } else {
            tokens.resolve("text.muted")
        })
        .bg(bg);
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
        Span::styled("░".repeat(track_width as usize - filled), muted),
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
    if let Some(budget) = picker.budget.as_mut() {
        budget.track_rect = None;
    }
    if area.is_empty() {
        return;
    }
    let width = 64u16.min(
        area.width
            .saturating_sub(if area.width >= 20 { 4 } else { 0 }),
    );
    let padding = if width >= 20 {
        2
    } else if width >= 6 {
        1
    } else {
        0
    };
    let inner_width = width.saturating_sub(padding * 2);
    let list_width = inner_width.saturating_sub(1).max(1);
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
    // Own the rows so rendering the slider can update its hit target.
    let filtered: Vec<PickerItem> = picker.filtered().into_iter().cloned().collect();
    if picker.selected >= filtered.len() {
        picker.selected = filtered.iter().position(|item| !item.header).unwrap_or(0);
        picker.details_scroll = 0;
    }
    let choices = filtered.iter().filter(|item| !item.header).count();
    let description = filtered
        .get(picker.selected)
        .map(|item| {
            if item.id == "budget" {
                "Token budget: ←→ adjust, type digits, Enter apply"
            } else {
                item.description.as_str()
            }
        })
        .unwrap_or("");
    let mut details = if description.is_empty() {
        Vec::new()
    } else {
        description
            .split('\n')
            .flat_map(|line| wrap_text(line, inner_width as usize))
            .collect()
    };
    if let Some(item) = filtered.get(picker.selected) {
        if display_width(&item.label) > list_width as usize {
            let mut label = wrap_text(&item.label, inner_width as usize);
            label.append(&mut details);
            details = label;
        }
    }
    if filtered.is_empty() && !picker.filter.is_empty() {
        details = vec!["Try a different search.".into()];
    }
    let surface = tokens.resolve("panel.overlay");
    let body_style = Style::default().fg(tokens.resolve("text.body")).bg(surface);
    let muted_style = Style::default()
        .fg(tokens.resolve("text.muted"))
        .bg(surface);
    let mut footer = wrap_text(
        if inner_width >= 14 {
            "↑↓ move Enter choose Esc close"
        } else {
            "↑↓ Enter Esc"
        },
        inner_width as usize,
    );
    if let Some(hint) = &picker.hint {
        footer.extend(wrap_text(hint, inner_width as usize));
    }
    if details.len() > 3 || (details.len() > 1 && inner_width < 32) {
        footer.extend(wrap_text(
            if inner_width < 20 {
                "Pg↑/↓ details"
            } else {
                "PgUp/PgDn details"
            },
            inner_width as usize,
        ));
    }

    let max_height = area
        .height
        .saturating_sub(if area.height >= 18 { 4 } else { 0 });
    let vertical_padding = u16::from(max_height >= 6);
    let available = max_height.saturating_sub(vertical_padding * 2);
    let header_height = if available >= 4 { 2 } else { 1 };
    let divider_height = u16::from(max_height >= 14);
    let footer_height =
        (footer.len() as u16).min(available.saturating_sub(header_height + divider_height + 1));
    let chrome = vertical_padding * 2 + header_height + divider_height + footer_height;
    let content_budget = max_height.saturating_sub(chrome);
    // Reserve choices before details so a long description never crowds the list.
    let min_choices = filtered.len().clamp(1, if max_height < 14 { 1 } else { 4 }) as u16;
    let details_height = if details.is_empty() || content_budget < min_choices + 2 {
        0
    } else {
        (details.len().min(3) as u16 + 1).min(content_budget - min_choices)
    };
    let list_height = (filtered.len().clamp(1, PICKER_VISIBLE_ITEMS) as u16)
        .min(content_budget.saturating_sub(details_height));
    let height = chrome + list_height + details_height;
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    f.render_widget(Clear, popup);
    f.render_widget(Block::default().style(body_style), popup);
    let inner = Rect::new(
        popup.x + padding,
        popup.y + vertical_padding,
        inner_width,
        height.saturating_sub(vertical_padding * 2),
    );
    let count = format!(
        "{choices} {}",
        if choices == 1 { "choice" } else { "choices" }
    );
    let title_width = display_width(title);
    let title_line = if title_width + display_width(&count) + 2 <= inner.width as usize {
        Line::from(vec![
            Span::styled(title, body_style.add_modifier(Modifier::BOLD)),
            Span::styled(
                " ".repeat(inner.width as usize - title_width - display_width(&count)),
                muted_style,
            ),
            Span::styled(count, muted_style),
        ])
    } else {
        Line::from(Span::styled(
            truncate(title, inner.width as usize),
            body_style.add_modifier(Modifier::BOLD),
        ))
    };
    f.render_widget(
        Paragraph::new(title_line),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    if header_height == 2 {
        let prefix_width = if inner.width > 2 { 2 } else { 0 };
        let query_width = inner.width.saturating_sub(prefix_width) as usize;
        let before = &picker.filter[..picker.cursor];
        let mut start = 0;
        let mut caret_width = display_width(before);
        for (index, grapheme) in before.grapheme_indices(true) {
            if caret_width < query_width {
                break;
            }
            caret_width = caret_width.saturating_sub(display_width(grapheme));
            start = index + grapheme.len();
        }
        let mut query = String::new();
        let mut columns = 0;
        for grapheme in picker.filter[start..].graphemes(true) {
            let width = display_width(grapheme);
            if columns + width > query_width {
                break;
            }
            query.push_str(grapheme);
            columns += width;
        }
        let filter_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if prefix_width > 0 { "> " } else { "" },
                    body_style.fg(tokens.resolve("text.accent")),
                ),
                Span::styled(query, body_style),
            ])),
            filter_area,
        );
        if !filter_area.is_empty() {
            f.set_cursor_position((
                (filter_area.x + prefix_width + caret_width as u16).min(filter_area.right() - 1),
                filter_area.y,
            ));
        }
    }
    let divider = || {
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            muted_style.fg(tokens.resolve("divider")),
        )))
    };
    if divider_height > 0 {
        f.render_widget(
            divider(),
            Rect::new(inner.x, inner.y + header_height, inner.width, 1),
        );
    }
    let list_area = Rect::new(
        inner.x,
        inner.y + header_height + divider_height,
        list_width,
        list_height,
    );
    picker.visible_items = list_height as usize;
    picker.visible_details = details_height.saturating_sub(1) as usize;
    if picker.selected < picker.scroll {
        picker.scroll = picker.selected;
    } else if picker.selected >= picker.scroll.saturating_add(picker.visible_items) {
        picker.scroll = picker
            .selected
            .saturating_sub(picker.visible_items.saturating_sub(1));
    }
    picker.scroll = picker
        .scroll
        .min(filtered.len().saturating_sub(picker.visible_items));
    if filtered.is_empty() {
        f.render_widget(
            Paragraph::new(if picker.filter.is_empty() {
                empty_message
            } else {
                "No matches"
            })
            .style(muted_style),
            list_area,
        );
    } else {
        render_picker_list(f, picker, &filtered, list_area, tokens);
    }
    if details_height > 0 {
        f.render_widget(
            divider(),
            Rect::new(inner.x, list_area.bottom(), inner.width, 1),
        );
        let rows = details_height.saturating_sub(1) as usize;
        picker.details_scroll = picker
            .details_scroll
            .min(details.len().saturating_sub(rows));
        f.render_widget(
            Paragraph::new(Text::from(
                details
                    .into_iter()
                    .skip(picker.details_scroll)
                    .take(rows)
                    .map(|line| Line::from(Span::styled(line, muted_style)))
                    .collect::<Vec<_>>(),
            )),
            Rect::new(inner.x, list_area.bottom() + 1, inner.width, rows as u16),
        );
    }
    f.render_widget(
        Paragraph::new(Text::from(
            footer
                .into_iter()
                .take(footer_height as usize)
                .map(|line| Line::from(Span::styled(line, muted_style)))
                .collect::<Vec<_>>(),
        )),
        Rect::new(
            inner.x,
            list_area.bottom() + details_height,
            inner.width,
            footer_height,
        ),
    );
}

/// Choices occupy one row; only the selected choice has a separate description.
fn render_picker_list(
    f: &mut Frame,
    picker: &mut PickerState,
    filtered: &[PickerItem],
    list_area: Rect,
    tokens: &crate::theme::Theme,
) {
    for (offset, (index, item)) in filtered
        .iter()
        .enumerate()
        .skip(picker.scroll)
        .take(picker.visible_items)
        .enumerate()
    {
        let selected = index == picker.selected && !item.header;
        let bg = tokens.resolve(if selected {
            "selection.background"
        } else {
            "panel.overlay"
        });
        let fg = tokens.resolve(if selected {
            "selection.foreground"
        } else if item.header {
            "text.muted"
        } else {
            "text.body"
        });
        let style = Style::default().fg(fg).bg(bg).add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });
        let row = Rect::new(list_area.x, list_area.y + offset as u16, list_area.width, 1);
        f.render_widget(Block::default().style(style), row);
        let line = if item.id == "budget" {
            if let Some(budget) = picker.budget.as_mut() {
                let value_width = display_width(&format!(
                    " {} tok{}",
                    budget.draft,
                    if selected { "_" } else { "" }
                ));
                let track_width =
                    BUDGET_TRACK_WIDTH.min(list_area.width.saturating_sub(value_width as u16 + 2));
                if track_width >= 2 {
                    budget.track_rect = Some(Rect::new(row.x + 1, row.y, track_width, 1));
                }
                budget_track_line(budget, selected, tokens, track_width)
            } else {
                Line::from(Span::styled(
                    truncate(&item.label, list_area.width as usize),
                    style,
                ))
            }
        } else {
            Line::from(Span::styled(
                truncate(&item.label, list_area.width as usize),
                style,
            ))
        };
        f.render_widget(Paragraph::new(line), row);
    }
    if filtered.len() > picker.visible_items && !list_area.is_empty() {
        let mut state =
            ScrollbarState::new(filtered.len().saturating_sub(picker.visible_items) + 1)
                .viewport_content_length(picker.visible_items)
                .position(picker.scroll);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("█"),
            Rect::new(list_area.right(), list_area.y, 1, list_area.height),
            &mut state,
        );
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
