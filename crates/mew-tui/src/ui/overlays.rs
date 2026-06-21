use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use super::{DIVIDER, STATUS_BG};
use crate::app::{
    App, PermissionState, PersonaSummary, PersonaSwitchConfirmState, PickerState, SlashCommand,
    UserQuestionState, PICKER_VISIBLE_ITEMS,
};

pub(super) fn draw_slash_autocomplete(f: &mut Frame, app: &App, cmds: &[SlashCommand], area: Rect) {
    let bg = Block::default().style(Style::default().bg(STATUS_BG));
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

    f.render_widget(Paragraph::new(text), inner);

    if cmds.len() > visible {
        let scrollbar_area = Rect::new(area.x + area.width - 1, area.y, 1, area.height);
        let mut scrollbar_state = ScrollbarState::new(cmds.len())
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
    let (text, _) = app.alert.as_ref().unwrap();
    let width = text.len() as u16 + 4;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = area.height.saturating_sub(3);
    let popup = Rect::new(x, y, width, 1);

    let style = Style::default()
        .fg(Color::Black)
        .bg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let para = Paragraph::new(Line::from(Span::styled(format!(" {} ", text), style)));
    f.render_widget(Clear, popup);
    f.render_widget(para, popup);
}

pub(super) fn draw_permission_modal(f: &mut Frame, perm: &PermissionState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 14u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

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

pub(super) fn draw_user_question_modal(f: &mut Frame, uq: &UserQuestionState, area: Rect) {
    // Each question takes 3 lines (prompt, answer, blank) + a hint line.
    let per_q: u16 = 3;
    let width = 64u16.min(area.width.saturating_sub(4));
    let height = (3 + per_q * uq.questions.len() as u16).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);
    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    let mut text = Text::default();
    for (i, prompt) in uq.questions.iter().enumerate() {
        let focused = i == uq.current;
        let prompt_style = if focused {
            Style::default()
                .fg(Color::Cyan)
                .bg(STATUS_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray).bg(STATUS_BG)
        };
        let answer_style = if focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Gray).bg(STATUS_BG)
        };
        let answer = uq.answers.get(i).map(|s| s.as_str()).unwrap_or("");
        text.push_line(Line::from(Span::styled(
            format!(" {}", prompt),
            prompt_style,
        )));
        text.push_line(Line::from(Span::styled(
            format!("  {}", answer),
            answer_style,
        )));
        text.push_line(Line::from(""));
    }

    f.render_widget(Paragraph::new(text), inner);

    // Hint at the bottom.
    let hint_style = Style::default().fg(Color::DarkGray).bg(STATUS_BG);
    let hint_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "enter submit · tab next · esc cancel",
            hint_style,
        ))),
        hint_area,
    );

    // Place the cursor at the end of the focused answer.
    if let Some(answer) = uq.answers.get(uq.current) {
        let row = inner.y + (uq.current as u16) * per_q + 1;
        let col = inner.x + 2 + (answer.chars().count() as u16).min(inner.width.saturating_sub(4));
        f.set_cursor_position((col, row));
    }
}

pub(super) fn draw_picker(f: &mut Frame, picker: &mut PickerState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));

    let item_lines: u16 =
        picker
            .items
            .first()
            .map_or(1, |i| if i.description.is_empty() { 1 } else { 2 });

    let max_items = PICKER_VISIBLE_ITEMS as u16;
    let content_height = max_items * item_lines;
    let height = (4 + content_height).min(area.height.saturating_sub(4));

    let list_area_height = height.saturating_sub(4);
    let visible_items = (list_area_height / item_lines).max(1) as usize;

    picker.visible_items = visible_items;

    let filtered = picker.filtered();

    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let bg = Block::default().style(Style::default().bg(STATUS_BG));
    f.render_widget(bg, popup);

    let inner = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );

    let filter_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let prefix = Span::styled("> ", Style::default().fg(Color::Cyan).bg(STATUS_BG));
    let filter_text = Span::styled(
        &picker.filter,
        Style::default().fg(Color::White).bg(STATUS_BG),
    );
    let filter_para = Paragraph::new(Line::from(vec![prefix, filter_text]));
    f.render_widget(filter_para, filter_area);

    let cursor_x = filter_area.x + 2 + (picker.cursor.min(filter_area.width as usize - 2) as u16);
    f.set_cursor_position((cursor_x, filter_area.y));

    let div_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    let div_line = Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(DIVIDER),
    ));
    f.render_widget(Paragraph::new(div_line), div_area);

    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width.saturating_sub(1),
        inner.height.saturating_sub(2),
    );

    let mut list_text = Text::default();

    let start = picker.scroll;
    for (i, item) in filtered.iter().enumerate().skip(start).take(visible_items) {
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

    if filtered.len() > visible_items {
        let scrollbar_area = list_area.inner(Margin {
            horizontal: 0,
            vertical: 0,
        });
        let mut scrollbar_state = ScrollbarState::new(filtered.len())
            .viewport_content_length(visible_items)
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
) {
    let width = 64u16.min(area.width.saturating_sub(4));
    let height = 18u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let block = Block::bordered()
        .title(Span::styled(
            " Switch persona ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Magenta));
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
        Span::styled("  from  ", Style::default().fg(Color::DarkGray)),
        Span::styled(from, Style::default().fg(Color::Gray)),
        Span::styled(" → ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.target.name.clone(),
            Style::default()
                .fg(Color::Rgb(200, 170, 240))
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if !state.target.description.is_empty() {
        let max = inner.width.saturating_sub(4) as usize;
        let desc: String = state.target.description.chars().take(max).collect();
        text.push_line(Line::from(vec![
            Span::styled("  desc  ", Style::default().fg(Color::DarkGray)),
            Span::styled(desc, Style::default().fg(Color::Gray)),
        ]));
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
        Span::styled("  model ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            model_str,
            if model_changed {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            },
        ),
    ]));

    // Tools
    text.push_line(Line::from(vec![
        Span::styled("  tools ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_tools(&state.target.tools),
            tools_style(&state.target, &state.current, false),
        ),
    ]));
    text.push_line(Line::from(vec![
        Span::styled("  deny  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_tools(&state.target.tools_deny),
            tools_style(&state.target, &state.current, true),
        ),
    ]));

    // Skills
    text.push_line(Line::from(vec![
        Span::styled("  skills", Style::default().fg(Color::DarkGray)),
        Span::styled(" ".to_string(), Style::default()),
        Span::styled(
            format_tools(&state.target.skills),
            skills_style(&state.target, &state.current),
        ),
    ]));

    text.push_line(Line::from(""));

    // Buttons
    let confirm_label = "[ Confirm ]";
    let cancel_label = "[ Cancel ]";
    let confirm_style = if state.selected == 0 {
        Style::default()
            .bg(Color::Rgb(35, 90, 50))
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let cancel_style = if state.selected == 1 {
        Style::default()
            .bg(Color::Rgb(90, 35, 35))
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
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

fn format_tools(list: &Option<Vec<String>>) -> String {
    match list {
        None => "all".into(),
        Some(v) if v.is_empty() => "(none)".into(),
        Some(v) => v.join(", "),
    }
}

fn tools_style(target: &PersonaSummary, current: &Option<PersonaSummary>, deny: bool) -> Style {
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
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    }
}

fn skills_style(target: &PersonaSummary, current: &Option<PersonaSummary>) -> Style {
    if target.skills != current.as_ref().and_then(|c| c.skills.clone()) {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    }
}
