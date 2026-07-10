//! Input editor methods for the App state.
//!
//! Character insertion, cursor movement, undo/redo,
//! history navigation, and visual cursor mapping.

use super::*;

impl App {
    pub fn push_undo(&mut self) {
        let now = Instant::now();
        let should_coalesce = self
            .last_undo_push
            .map(|t| now.duration_since(t) < Duration::from_millis(500))
            .unwrap_or(false);
        if !should_coalesce {
            self.undo_stack.push((self.input.clone(), self.cursor));
            if self.undo_stack.len() > 100 {
                self.undo_stack.remove(0);
            }
        }
        self.last_undo_push = Some(now);
        // Any new mutation clears the redo stack.
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        if let Some((prev_input, prev_cursor)) = self.undo_stack.pop() {
            self.redo_stack.push((self.input.clone(), self.cursor));
            self.input = prev_input;
            self.cursor = prev_cursor;
            self.last_undo_push = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some((next_input, next_cursor)) = self.redo_stack.pop() {
            self.undo_stack.push((self.input.clone(), self.cursor));
            self.input = next_input;
            self.cursor = next_cursor;
            self.last_undo_push = None;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.push_undo();
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_newline(&mut self) {
        self.push_undo();
        self.input.insert(self.cursor, '\n');
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.push_undo();
            let prev = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(prev);
            self.cursor = prev;
        }
    }

    pub fn delete_char(&mut self) {
        if self.cursor < self.input.len() {
            self.push_undo();
            self.input.remove(self.cursor);
        }
    }

    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = self.input[self.cursor..]
                .chars()
                .next()
                .map(|c| self.cursor + c.len_utf8())
                .unwrap_or(self.input.len());
        }
    }

    pub fn cursor_home(&mut self) {
        // Move to start of current line.
        let before = &self.input[..self.cursor];
        if let Some(ln_pos) = before.rfind('\n') {
            self.cursor = ln_pos + 1;
        } else {
            self.cursor = 0;
        }
    }

    pub fn cursor_end(&mut self) {
        if let Some(ln_pos) = self.input[self.cursor..].find('\n') {
            self.cursor += ln_pos;
        } else {
            self.cursor = self.input.len();
        }
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

    pub fn input_visual_line_count(&self, content_width: u16) -> usize {
        let w = content_width.max(1) as usize;
        self.input
            .split('\n')
            .map(|line| {
                let dw = unicode_width::UnicodeWidthStr::width(line);
                dw.div_ceil(w).max(1)
            })
            .sum()
    }

    pub fn cursor_visual_row_col(&self, content_width: u16) -> (usize, usize) {
        let w = content_width.max(1) as usize;
        let (logical_line, col_in_line) = self.cursor_line_col();
        let mut visual_row = 0;
        for (li, line) in self.input.split('\n').enumerate() {
            if li == logical_line {
                let dw = unicode_width::UnicodeWidthStr::width(line);
                let col_clamped = col_in_line.min(dw);
                let row_in_line = col_clamped.checked_div(w).unwrap_or(0);
                return (visual_row + row_in_line, col_clamped - row_in_line * w);
            }
            let dw = unicode_width::UnicodeWidthStr::width(line);
            let rows = if w == 0 { 1 } else { dw.div_ceil(w).max(1) };
            visual_row += rows;
        }
        (visual_row, 0)
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.input[..self.cursor];
        let line = before.lines().count().saturating_sub(1);
        let col = if let Some(ln_pos) = before.rfind('\n') {
            self.cursor - ln_pos - 1
        } else {
            self.cursor
        };
        (line, col)
    }

    pub fn cursor_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.input[..self.cursor];
        let chars: Vec<(usize, char)> = before.char_indices().collect();
        if chars.len() < 2 {
            self.cursor = 0;
            return;
        }

        // Start from the character before cursor.
        let mut i = chars.len() - 1;
        let start_is_word = chars[i].1.is_alphanumeric();

        // Skip word chars if we started on one, or skip non-word chars if we started on one.
        while i > 0 && chars[i].1.is_alphanumeric() == start_is_word {
            i -= 1;
        }

        // If we ended on a different kind, land on the boundary.
        if chars[i].1.is_alphanumeric() != start_is_word {
            self.cursor = chars[i + 1].0;
        } else {
            self.cursor = chars[i].0;
        }
    }

    pub fn cursor_word_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let after: Vec<(usize, char)> = self.input[self.cursor..].char_indices().collect();
        if after.is_empty() {
            return;
        }

        let start_is_word = after[0].1.is_alphanumeric();
        let mut i = 0;

        // Skip chars of the same kind.
        while i + 1 < after.len() && after[i + 1].1.is_alphanumeric() == start_is_word {
            i += 1;
        }

        self.cursor = self.cursor + after[i].0 + after[i].1.len_utf8();
    }

    pub fn delete_word_left(&mut self) {
        let old_cursor = self.cursor;
        self.cursor_word_left();
        self.input.replace_range(self.cursor..old_cursor, "");
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
