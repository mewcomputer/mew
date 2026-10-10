//! Input editor methods for the App state.
//!
//! Character insertion, cursor movement, undo/redo,
//! history navigation, and visual cursor mapping.

use super::*;
use unicode_segmentation::UnicodeSegmentation;

impl App {
    pub fn push_undo(&mut self) {
        self.undo_history.snapshot(&self.input, self.cursor);
    }

    pub fn undo(&mut self) {
        self.undo_history.undo(&mut self.input, &mut self.cursor);
    }

    pub fn redo(&mut self) {
        self.undo_history.redo(&mut self.input, &mut self.cursor);
    }

    pub fn insert_char(&mut self, c: char) {
        self.push_undo();
        editor::insert(&mut self.input, &mut self.cursor, c);
    }

    pub fn insert_newline(&mut self) {
        self.push_undo();
        editor::insert(&mut self.input, &mut self.cursor, '\n');
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.push_undo();
            editor::backspace(&mut self.input, &mut self.cursor);
        }
    }

    pub fn delete_char(&mut self) {
        if self.cursor < self.input.len() {
            self.push_undo();
            editor::delete(&mut self.input, &mut self.cursor);
        }
    }

    pub fn cursor_left(&mut self) {
        editor::cursor_left(&self.input, &mut self.cursor);
    }

    pub fn cursor_right(&mut self) {
        editor::cursor_right(&self.input, &mut self.cursor);
    }

    pub fn cursor_home(&mut self) {
        editor::cursor_home(&self.input, &mut self.cursor);
    }

    pub fn cursor_end(&mut self) {
        editor::cursor_end(&self.input, &mut self.cursor);
    }

    pub fn cursor_visual_up(&mut self, content_width: u16) -> bool {
        let (row, col) = self.cursor_visual_row_col(content_width);
        if row == 0 {
            return false;
        }
        // Move to the same column on the previous visual row.
        let target_row = row - 1;
        if let Some(offset) = self.visual_to_byte_offset_opt(target_row, col, content_width) {
            self.cursor = offset;
        }
        true
    }

    pub fn cursor_visual_down(&mut self, content_width: u16) -> bool {
        let (row, col) = self.cursor_visual_row_col(content_width);
        let total = self.input_visual_line_count(content_width);
        if row >= total.saturating_sub(1) {
            return false;
        }
        let target_row = row + 1;
        if let Some(offset) = self.visual_to_byte_offset_opt(target_row, col, content_width) {
            self.cursor = offset;
        }
        true
    }

    pub fn input_line_count(&self) -> usize {
        self.input.lines().count()
    }

    /// Borrowed wrapped rows: byte start, text, and whether the logical line begins here.
    pub(crate) fn input_visual_rows(
        &self,
        content_width: u16,
    ) -> impl Iterator<Item = (usize, &str, bool)> {
        let width = content_width.max(1) as usize;
        self.input
            .split('\n')
            .scan(0, |offset, line| {
                let start = *offset;
                *offset += line.len() + 1;
                Some((start, line))
            })
            .flat_map(move |(line_start, line)| {
                let mut remaining = line;
                let mut consumed = 0;
                let mut empty = line.is_empty();
                std::iter::from_fn(move || {
                    if empty {
                        empty = false;
                        return Some((line_start, line, true));
                    }
                    if remaining.is_empty() {
                        return None;
                    }
                    let mut end = 0;
                    let mut columns = 0;
                    for grapheme in remaining.graphemes(true) {
                        let grapheme_width = unicode_width::UnicodeWidthStr::width(grapheme);
                        if end > 0 && grapheme_width > 0 && columns + grapheme_width > width {
                            break;
                        }
                        columns += grapheme_width;
                        end += grapheme.len();
                    }
                    let row = (line_start + consumed, &remaining[..end], consumed == 0);
                    remaining = &remaining[end..];
                    consumed += end;
                    Some(row)
                })
            })
    }

    pub fn input_visual_line_count(&self, content_width: u16) -> usize {
        self.input_visual_rows(content_width).count()
    }

    pub fn cursor_visual_row_col(&self, content_width: u16) -> (usize, usize) {
        for (row, (start, text, _)) in self.input_visual_rows(content_width).enumerate() {
            let end = start + text.len();
            let logical_end = self
                .input
                .as_bytes()
                .get(end)
                .is_none_or(|byte| *byte == b'\n');
            if self.cursor < end || (self.cursor == end && logical_end) {
                let column = unicode_width::UnicodeWidthStr::width(
                    &text[..self.cursor.saturating_sub(start)],
                );
                return (row, column.min(content_width.max(1) as usize));
            }
        }
        (0, 0)
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.input[..self.cursor];
        let line = before.bytes().filter(|byte| *byte == b'\n').count();
        let current_line = before.rsplit('\n').next().unwrap_or_default();
        let col = unicode_width::UnicodeWidthStr::width(current_line);
        (line, col)
    }

    pub fn cursor_word_left(&mut self) {
        editor::cursor_word_left(&self.input, &mut self.cursor);
    }

    pub fn cursor_word_right(&mut self) {
        editor::cursor_word_right(&self.input, &mut self.cursor);
    }

    pub fn delete_word_left(&mut self) {
        editor::delete_word_left(&mut self.input, &mut self.cursor);
    }

    pub fn submit_input(&mut self) -> Option<String> {
        let text = self.input.trim();
        if text.is_empty() {
            return None;
        }
        let result = text.to_string();
        self.history.push(result.clone());
        self.history_index = None;
        self.input.clear();
        self.cursor = 0;
        self.mode = Mode::Normal;
        // Re-attach auto-scroll so the user sees the response.
        self.auto_scroll = true;
        Some(result)
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            Some(i) if i > 0 => i - 1,
            Some(_) => return,
            None => self.history.len() - 1,
        };
        if self.history_index.is_none() {
            self.history_draft = Some(self.input.clone());
        }
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    pub fn history_down(&mut self) {
        let idx = match self.history_index {
            Some(i) if i + 1 < self.history.len() => i + 1,
            Some(_) => {
                self.history_index = None;
                self.input = self.history_draft.take().unwrap_or_default();
                self.cursor = self.input.len();
                return;
            }
            None => return,
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }
}
