use crate::app::{visual_rows, App, DecisionLayout};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

pub(super) fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(state) = app.todo_editor.as_mut() else {
        return;
    };
    let width = DecisionLayout::content_width(area.width, 76).max(1);
    let mut rows = Vec::new();
    let mut hits = Vec::new();
    let mut selected_rows = None;
    let mut caret = None;
    let foreground = app.theme.resolve("foreground");
    let background = app.theme.resolve("panel.overlay");
    let base = Style::default().fg(foreground).bg(background);
    let selected = base
        .fg(app.theme.resolve("selection.foreground"))
        .bg(app.theme.resolve("selection.background"));
    if let Some(draft) = &state.draft {
        for (row, (start, text, _)) in
            visual_rows(&draft.content, width.saturating_sub(1).max(1)).enumerate()
        {
            let end = start + text.len();
            if draft.cursor >= start
                && (draft.cursor < end
                    || (draft.cursor == end
                        && draft
                            .content
                            .as_bytes()
                            .get(end)
                            .is_none_or(|byte| *byte == b'\n')))
            {
                caret = Some((row, UnicodeWidthStr::width(&text[..draft.cursor - start])));
            }
            rows.push(Line::raw(text));
        }
    } else if state.session_id != app.status.session_id {
        rows.push(Line::raw("session changed; reopen todos"));
    } else if app.todos.is_empty() {
        rows.push(Line::raw("no todos in this session"));
        state.selected = None;
    } else {
        if !app.todos.iter().any(|todo| Some(todo.id) == state.selected) {
            state.selected = Some(app.todos[0].id);
            state.reveal_selection = true;
        }
        for todo in &app.todos {
            let first = rows.len();
            let heading = format!("#{}  {}", todo.id, todo.status.as_str());
            rows.push(Line::raw(heading));
            for (_, text, _) in visual_rows(&todo.content, width) {
                rows.push(Line::raw(text));
            }
            if !todo.depends_on.is_empty() {
                let deps = todo
                    .depends_on
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                rows.extend(
                    super::chat::wrap_text_to_width(&format!("depends on {deps}"), width)
                        .into_iter()
                        .map(Line::raw),
                );
            }
            if Some(todo.id) == state.selected {
                selected_rows = Some((first, rows.len()));
            }
            hits.extend((first..rows.len()).map(|row| (row, todo.id)));
            rows.push(Line::raw(""));
        }
    }
    let error_rows = state
        .error
        .as_ref()
        .map(|error| super::chat::wrap_text_to_width(error, width));
    let error_height = error_rows
        .as_ref()
        .map_or(0, |rows| rows.len().min(3) as u16);
    let action_rows = 1 + error_height;
    let layout = DecisionLayout::new(area, 76, rows.len().max(3), action_rows);
    state.body = layout.body;
    state.max_scroll =
        (rows.len().min(u16::MAX as usize) as u16).saturating_sub(layout.body.height);
    if let Some((row, _)) = caret {
        state.scroll = (row.min(u16::MAX as usize) as u16)
            .saturating_sub(layout.body.height.saturating_sub(1));
    } else if let Some((first, last)) = selected_rows {
        let first = first.min(u16::MAX as usize) as u16;
        if state.reveal_selection && first < state.scroll {
            state.scroll = first;
        } else if state.reveal_selection && first >= state.scroll.saturating_add(layout.body.height)
        {
            state.scroll = (last.min(u16::MAX as usize) as u16)
                .saturating_sub(layout.body.height)
                .min(first);
        }
        state.reveal_selection = false;
        for row in &mut rows[first as usize..last] {
            row.style = selected;
        }
    }
    state.scroll = state.scroll.min(state.max_scroll);
    state.row_hits.clear();
    for (row, id) in hits {
        if let Some(visible) = row
            .checked_sub(state.scroll as usize)
            .filter(|row| *row < layout.body.height as usize)
        {
            state.row_hits.push((layout.body.y + visible as u16, id));
        }
    }
    f.render_widget(Clear, layout.popup);
    f.render_widget(Block::default().style(base), layout.popup);
    let title = state
        .draft
        .as_ref()
        .map(|draft| format!(" edit todo #{} ", draft.original.id))
        .unwrap_or_else(|| " todos ".into());
    f.render_widget(
        Paragraph::new(title).style(base.add_modifier(Modifier::BOLD)),
        Rect::new(layout.body.x, layout.popup.y, layout.body.width, 1),
    );
    f.render_widget(
        Paragraph::new(rows).style(base).scroll((state.scroll, 0)),
        layout.body,
    );
    let status = state
        .draft
        .as_ref()
        .map(|draft| format!("{} · tab change", draft.status.as_str()))
        .unwrap_or_else(|| format!("{} tasks  enter edit", app.todos.len()));
    let mut actions = vec![Line::raw(status)];
    if let Some(errors) = error_rows {
        actions.extend(errors.into_iter().take(3).map(|text| {
            Line::from(Span::styled(
                text,
                base.fg(app.theme.resolve("text.warning")),
            ))
        }));
    }
    f.render_widget(Paragraph::new(actions).style(base), layout.actions);
    let hint = if state.draft.is_some() {
        if layout.hint.width >= 20 {
            "^s save · Esc cancel"
        } else if layout.hint.width >= 12 {
            "^s save Esc"
        } else {
            "^s · Esc"
        }
    } else if layout.hint.width >= 32 {
        "↑↓ select · PgUp/Dn scroll · Esc"
    } else if layout.hint.width >= 22 {
        "↑↓ select · Esc close"
    } else {
        "↑↓ · Esc"
    };
    f.render_widget(
        Paragraph::new(hint).style(base.fg(app.theme.resolve("text.muted"))),
        layout.hint,
    );
    if let Some((row, column)) = caret {
        let row = row.saturating_sub(state.scroll as usize);
        if row < layout.body.height as usize {
            f.set_cursor_position((
                layout.body.x + (column as u16).min(layout.body.width.saturating_sub(1)),
                layout.body.y + row as u16,
            ));
        }
    }
}
