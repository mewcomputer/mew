use super::{editor, App, Mode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_agent::{Todo, TodoStatus};

pub struct TodoEditor {
    pub session_id: String,
    pub selected: Option<usize>,
    pub draft: Option<TodoDraft>,
    pub error: Option<String>,
    pub scroll: u16,
    pub max_scroll: u16,
    pub row_hits: Vec<(u16, usize)>,
    pub body: ratatui::layout::Rect,
    pub reveal_selection: bool,
}

pub struct TodoDraft {
    pub original: Todo,
    pub content: String,
    pub cursor: usize,
    pub status: TodoStatus,
    pub undo: editor::UndoHistory,
}

impl App {
    pub fn open_todos(&mut self, selected: Option<usize>) {
        if self.todo_editor.as_ref().is_some_and(|state| {
            state.session_id == self.status.session_id && state.draft.is_some()
        }) {
            self.mode = Mode::TodoEditor;
            return;
        }
        self.todo_editor = Some(TodoEditor {
            session_id: self.status.session_id.clone(),
            selected: selected.or_else(|| self.todos.first().map(|todo| todo.id)),
            draft: None,
            error: None,
            scroll: 0,
            max_scroll: 0,
            row_hits: Vec::new(),
            body: Default::default(),
            reveal_selection: true,
        });
        self.mode = Mode::TodoEditor;
    }

    pub fn finish_todo_save(&mut self, result: Result<Vec<Todo>, String>) {
        let Some(state) = self.todo_editor.as_mut() else {
            return;
        };
        if state.session_id != self.status.session_id {
            state.error = Some("session changed; reopen todos".into());
            return;
        }
        match result {
            Ok(todos) => {
                self.todos = todos;
                state.draft = None;
                state.error = None;
                state.scroll = 0;
                state.reveal_selection = true;
            }
            Err(error) => state.error = Some(error),
        }
    }

    pub fn handle_todo_key(&mut self, key: KeyEvent) -> Option<crate::events::Action> {
        let state = self.todo_editor.as_mut()?;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('c')) {
            if state.draft.take().is_some() {
                state.error = None;
                state.scroll = 0;
                state.reveal_selection = true;
            } else {
                self.todo_editor = None;
                self.mode = Mode::Normal;
            }
            return None;
        }
        if state.session_id != self.status.session_id {
            state.error = Some("session changed; reopen todos".into());
            return None;
        }
        if let Some(draft) = state.draft.as_mut() {
            if ctrl && key.code == KeyCode::Char('s') {
                if draft.content.trim().is_empty() {
                    state.error = Some("task description cannot be empty".into());
                    return None;
                }
                return Some(crate::events::Action::UpdateTodo {
                    session_id: state.session_id.clone(),
                    edit: mew_protocol::TodoEdit {
                        todo_id: draft.original.id,
                        expected_content: draft.original.content.clone(),
                        expected_status: draft.original.status.as_str().into(),
                        content: draft.content.clone(),
                        status: draft.status.as_str().into(),
                    },
                });
            }
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    let statuses = [
                        TodoStatus::Pending,
                        TodoStatus::InProgress,
                        TodoStatus::Blocked,
                        TodoStatus::Done,
                    ];
                    let index = statuses
                        .iter()
                        .position(|status| *status == draft.status)
                        .unwrap();
                    let delta = if key.code == KeyCode::BackTab { 3 } else { 1 };
                    draft.status = statuses[(index + delta) % statuses.len()];
                }
                _ if (key.code == KeyCode::Enter && key.modifiers.is_empty())
                    || editor::is_newline(key) =>
                {
                    editor::insert_with_undo(
                        &mut draft.content,
                        &mut draft.cursor,
                        '\n',
                        &mut draft.undo,
                    )
                }
                KeyCode::Up | KeyCode::Down => {
                    let width = state.body.width.saturating_sub(1).max(1);
                    let mut position = None;
                    for (row, (start, text, _)) in
                        super::visual_rows(&draft.content, width).enumerate()
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
                            position = Some((
                                row,
                                unicode_width::UnicodeWidthStr::width(
                                    &text[..draft.cursor - start],
                                ),
                            ));
                            break;
                        }
                    }
                    if let Some((row, column)) = position {
                        let target = if key.code == KeyCode::Up {
                            row.saturating_sub(1)
                        } else {
                            row + 1
                        };
                        if let Some((start, text, _)) =
                            super::visual_rows(&draft.content, width).nth(target)
                        {
                            draft.cursor = start + super::byte_at_display_offset(text, column);
                        }
                    }
                }
                _ => {
                    editor::handle_key_with_undo(
                        &mut draft.content,
                        &mut draft.cursor,
                        key,
                        &mut draft.undo,
                        &mut self.kill_ring,
                    );
                }
            }
            return None;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(todo) = state
                    .selected
                    .and_then(|id| self.todos.iter().find(|todo| todo.id == id))
                {
                    state.draft = Some(TodoDraft {
                        original: todo.clone(),
                        content: todo.content.clone(),
                        cursor: todo.content.len(),
                        status: todo.status,
                        undo: Default::default(),
                    });
                    state.scroll = 0;
                    state.error = None;
                }
            }
            KeyCode::Up | KeyCode::Down => {
                if !self.todos.is_empty() {
                    let index = self
                        .todos
                        .iter()
                        .position(|todo| Some(todo.id) == state.selected)
                        .unwrap_or(0);
                    let index = if key.code == KeyCode::Up {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(self.todos.len() - 1)
                    };
                    state.selected = Some(self.todos[index].id);
                    state.reveal_selection = true;
                    state.error = None;
                }
            }
            KeyCode::PageUp => state.scroll = state.scroll.saturating_sub(5),
            KeyCode::PageDown => {
                state.scroll = state.scroll.saturating_add(5).min(state.max_scroll)
            }
            _ => {}
        }
        None
    }

    pub fn paste_todo(&mut self, text: &str) {
        if let Some(draft) = self
            .todo_editor
            .as_mut()
            .and_then(|state| state.draft.as_mut())
        {
            draft.undo.snapshot(&draft.content, draft.cursor);
            draft.content.insert_str(draft.cursor, text);
            draft.cursor += text.len();
        }
    }
}
