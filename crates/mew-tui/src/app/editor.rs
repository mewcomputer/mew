//! Shared readline-style line editing.
//!
//! Every text field in the TUI (chat input, picker filters, the question
//! overlay's freeform answer, plan feedback, history search, settings
//! fields) routes character editing through the functions here so the key
//! bindings and cursor semantics stay identical across surfaces.
//!
//! Cursor positions are byte offsets into the text and always sit on a
//! `char` boundary. Word boundaries treat alphanumeric runs as words,
//! matching the chat input's historical behavior.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// Undo/redo history for a text field.
///
/// `snapshot` records the state *before* a mutation; rapid successive
/// snapshots (within 500 ms) coalesce into a single undo step, matching
/// the chat input's long-standing behavior.
#[derive(Debug, Default)]
pub struct UndoHistory {
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    last_push: Option<Instant>,
}

impl UndoHistory {
    /// Record `(text, cursor)` as the state to revert to. Coalesces when a
    /// snapshot happened less than 500 ms ago; any snapshot (coalesced or
    /// not) clears the redo stack.
    pub fn snapshot(&mut self, text: &str, cursor: usize) {
        let now = Instant::now();
        let coalesce = self
            .last_push
            .map(|t| now.duration_since(t) < Duration::from_millis(500))
            .unwrap_or(false);
        if !coalesce {
            self.undo.push((text.to_owned(), cursor));
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
        }
        self.last_push = Some(now);
        self.redo.clear();
    }

    /// Revert to the most recent snapshot, if any.
    pub fn undo(&mut self, text: &mut String, cursor: &mut usize) {
        if let Some((prev_text, prev_cursor)) = self.undo.pop() {
            self.redo.push((text.clone(), *cursor));
            *text = prev_text;
            *cursor = prev_cursor;
            self.last_push = None;
        }
    }

    /// Re-apply the most recently undone edit, if any.
    pub fn redo(&mut self, text: &mut String, cursor: &mut usize) {
        if let Some((next_text, next_cursor)) = self.redo.pop() {
            self.undo.push((text.clone(), *cursor));
            *text = next_text;
            *cursor = next_cursor;
            self.last_push = None;
        }
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn undo_is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    pub fn redo_is_empty(&self) -> bool {
        self.redo.is_empty()
    }
}

/// Insert a character at the cursor.
pub fn insert(text: &mut String, cursor: &mut usize, c: char) {
    text.insert(*cursor, c);
    *cursor += c.len_utf8();
}

/// Delete the character before the cursor (Backspace).
pub fn backspace(text: &mut String, cursor: &mut usize) {
    if *cursor > 0 {
        let prev = text[..*cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        text.remove(prev);
        *cursor = prev;
    }
}

/// Delete the character at the cursor (Delete / Ctrl+D).
pub fn delete(text: &mut String, cursor: &mut usize) {
    if *cursor < text.len() {
        text.remove(*cursor);
    }
}

/// Delete from the cursor to the end of the text (readline Ctrl+K).
pub fn kill_to_end(text: &mut String, cursor: &mut usize) {
    if *cursor < text.len() {
        text.replace_range(*cursor.., "");
    }
}

/// Clear the whole text (readline Ctrl+U in this app's convention).
pub fn clear(text: &mut String, cursor: &mut usize) {
    text.clear();
    *cursor = 0;
}

/// Move the cursor to the previous character.
pub fn cursor_left(text: &str, cursor: &mut usize) {
    if *cursor > 0 {
        *cursor = text[..*cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }
}

/// Move the cursor to the next character.
pub fn cursor_right(text: &str, cursor: &mut usize) {
    if *cursor < text.len() {
        *cursor = text[*cursor..]
            .chars()
            .next()
            .map(|c| *cursor + c.len_utf8())
            .unwrap_or(text.len());
    }
}

/// Move the cursor to the start of the current line.
pub fn cursor_home(text: &str, cursor: &mut usize) {
    let before = &text[..*cursor];
    *cursor = before.rfind('\n').map(|pos| pos + 1).unwrap_or(0);
}

/// Move the cursor to the end of the current line.
pub fn cursor_end(text: &str, cursor: &mut usize) {
    *cursor = text[*cursor..]
        .find('\n')
        .map(|pos| *cursor + pos)
        .unwrap_or(text.len());
}

/// Move the cursor to the start of the previous word.
pub fn cursor_word_left(text: &str, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let chars: Vec<(usize, char)> = text[..*cursor].char_indices().collect();
    if chars.len() < 2 {
        *cursor = 0;
        return;
    }

    // Start from the character before the cursor.
    let start_is_word = chars[chars.len() - 1].1.is_alphanumeric();
    let mut i = chars.len() - 1;
    while i > 0 && chars[i].1.is_alphanumeric() == start_is_word {
        i -= 1;
    }
    *cursor = if chars[i].1.is_alphanumeric() != start_is_word {
        chars[i + 1].0
    } else {
        chars[i].0
    };
}

/// Move the cursor to the start of the next word.
pub fn cursor_word_right(text: &str, cursor: &mut usize) {
    if *cursor >= text.len() {
        return;
    }
    let after: Vec<(usize, char)> = text[*cursor..].char_indices().collect();
    if after.is_empty() {
        return;
    }

    let start_is_word = after[0].1.is_alphanumeric();
    let mut i = 0;
    while i + 1 < after.len() && after[i + 1].1.is_alphanumeric() == start_is_word {
        i += 1;
    }
    *cursor = *cursor + after[i].0 + after[i].1.len_utf8();
}

/// Delete the word before the cursor (Alt+Backspace / Ctrl+W).
pub fn delete_word_left(text: &mut String, cursor: &mut usize) {
    let old_cursor = *cursor;
    cursor_word_left(text, cursor);
    text.replace_range(*cursor..old_cursor, "");
}

/// Apply a readline-style editing key to `(text, cursor)`. Returns `true`
/// when the key was consumed by the editor.
///
/// Handled: printable characters, Backspace (Alt = delete word, Meta =
/// clear), Delete, Left/Right (Alt = word), Home/End (line-aware),
/// Ctrl+A/E/F/B/D/K/U/W. Enter, Esc, Tab, Up/Down, and other key chords
/// are left to the caller, whose meaning differs per surface.
pub fn handle_key(text: &mut String, cursor: &mut usize, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let meta = key.modifiers.contains(KeyModifiers::META);
    match key.code {
        KeyCode::Char(c) if !ctrl && !alt && !meta => {
            insert(text, cursor, c);
        }
        KeyCode::Char('a') if ctrl => cursor_home(text, cursor),
        KeyCode::Char('e') if ctrl => cursor_end(text, cursor),
        KeyCode::Char('f') if ctrl => cursor_right(text, cursor),
        KeyCode::Char('b') if ctrl => cursor_left(text, cursor),
        KeyCode::Char('d') if ctrl => delete(text, cursor),
        KeyCode::Char('k') if ctrl => kill_to_end(text, cursor),
        KeyCode::Char('u') if ctrl => clear(text, cursor),
        KeyCode::Char('w') if ctrl => delete_word_left(text, cursor),
        KeyCode::Backspace if alt => delete_word_left(text, cursor),
        KeyCode::Backspace if meta => clear(text, cursor),
        KeyCode::Backspace => backspace(text, cursor),
        KeyCode::Delete => delete(text, cursor),
        KeyCode::Left if alt => cursor_word_left(text, cursor),
        KeyCode::Left => cursor_left(text, cursor),
        KeyCode::Right if alt => cursor_word_right(text, cursor),
        KeyCode::Right => cursor_right(text, cursor),
        KeyCode::Home => cursor_home(text, cursor),
        KeyCode::End => cursor_end(text, cursor),
        _ => return false,
    }
    true
}

/// Route an editing key through [`handle_key`], recording an undo snapshot
/// whenever the text changes, and bind Ctrl+Z / Ctrl+Y to undo/redo.
///
/// Surfaces with an [`UndoHistory`] should call this instead of
/// [`handle_key`] so every edit (including un-done ones) participates in
/// the same undo path.
pub fn handle_key_with_undo(
    text: &mut String,
    cursor: &mut usize,
    key: KeyEvent,
    history: &mut UndoHistory,
) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('z') => {
                history.undo(text, cursor);
                return true;
            }
            KeyCode::Char('y') => {
                history.redo(text, cursor);
                return true;
            }
            _ => {}
        }
    }
    let before = text.clone();
    let before_cursor = *cursor;
    if !handle_key(text, cursor, key) {
        return false;
    }
    if *text != before {
        history.snapshot(&before, before_cursor);
    }
    true
}

/// Insert at the cursor with a snapshot of the prior state (for surfaces
/// whose Enter inserts a newline outside the key router).
pub fn insert_with_undo(text: &mut String, cursor: &mut usize, c: char, history: &mut UndoHistory) {
    let before = text.clone();
    let before_cursor = *cursor;
    insert(text, cursor, c);
    history.snapshot(&before, before_cursor);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_respect_cursor() {
        let mut text = String::from("ac");
        let mut cursor = 1;
        insert(&mut text, &mut cursor, 'b');
        assert_eq!((&text, cursor), (&"abc".to_string(), 2));
        backspace(&mut text, &mut cursor);
        assert_eq!((&text, cursor), (&"ac".to_string(), 1));
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut text = String::new();
        let mut cursor = 0;
        backspace(&mut text, &mut cursor);
        assert_eq!(text, "");
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut text = String::from("abc");
        let mut cursor = 1;
        delete(&mut text, &mut cursor);
        assert_eq!(text, "ac");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn cursor_moves_on_char_boundaries() {
        let text = String::from("aé");
        let mut cursor = 1;
        cursor_left(&text, &mut cursor);
        assert_eq!(cursor, 0);
        cursor_right(&text, &mut cursor);
        assert_eq!(cursor, 1);
        cursor_right(&text, &mut cursor);
        assert_eq!(cursor, 3);
    }

    #[test]
    fn home_end_are_line_aware() {
        let text = String::from("one\ntwo");
        let mut cursor = text.len();
        cursor_home(&text, &mut cursor);
        assert_eq!(cursor, 4);
        cursor_end(&text, &mut cursor);
        assert_eq!(cursor, text.len());
    }

    #[test]
    fn word_moves_skip_alphanumeric_runs() {
        let text = String::from("foo bar baz");
        let mut cursor = text.len();
        cursor_word_left(&text, &mut cursor);
        assert_eq!(&text[cursor..], "baz");
        // The trailing whitespace is part of the skipped run, so the second
        // hop lands on the space before "bar" (matches the chat input).
        cursor_word_left(&text, &mut cursor);
        assert_eq!(&text[cursor..], " baz");
        cursor_word_right(&text, &mut cursor);
        assert_eq!(&text[..cursor], "foo bar ");
    }

    #[test]
    fn kill_and_clear() {
        let mut text = String::from("hello world");
        let mut cursor = 5;
        kill_to_end(&mut text, &mut cursor);
        assert_eq!(text, "hello");
        clear(&mut text, &mut cursor);
        assert!(text.is_empty());
        assert_eq!(cursor, 0);
    }

    #[test]
    fn delete_word_left_removes_previous_word() {
        let mut text = String::from("foo bar");
        let mut cursor = text.len();
        delete_word_left(&mut text, &mut cursor);
        assert_eq!(text, "foo ");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn handle_key_routes_readline_bindings() {
        let mut text = String::from("hello");
        let mut cursor = text.len();
        assert!(handle_key(&mut text, &mut cursor, key('b')));
        assert!(handle_key(
            &mut text,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
        ));
        // "hellob" with cursor at the end, ^B moves back to the 'b'.
        assert_eq!(cursor, 5);
        assert!(handle_key(
            &mut text,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
        ));
        assert_eq!(text, "");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn handle_key_leaves_control_keys_to_caller() {
        let mut text = String::from("x");
        let mut cursor = 1;
        for code in [
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::Up,
            KeyCode::Down,
        ] {
            assert!(!handle_key(
                &mut text,
                &mut cursor,
                KeyEvent::new(code, KeyModifiers::NONE)
            ));
        }
        assert_eq!(text, "x");
    }

    #[test]
    fn undo_history_coalesces_rapid_snapshots() {
        let mut history = UndoHistory::default();
        let mut text = String::new();
        let mut cursor = 0;
        handle_key_with_undo(&mut text, &mut cursor, key('a'), &mut history);
        handle_key_with_undo(&mut text, &mut cursor, key('b'), &mut history);
        assert_eq!(text, "ab");
        assert_eq!(history.undo_len(), 1, "rapid keystrokes coalesce");
        history.undo(&mut text, &mut cursor);
        assert_eq!(text, "");
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut history = UndoHistory::default();
        let mut text = String::from("hi");
        let mut cursor = 2;
        history.snapshot("hi", 2);
        assert_eq!(cursor, 2);
        text.push('!');
        cursor = 3;
        history.undo(&mut text, &mut cursor);
        assert_eq!(text, "hi");
        assert_eq!(cursor, 2);
        history.redo(&mut text, &mut cursor);
        assert_eq!(text, "hi!");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn handle_key_with_undo_binds_ctrl_z_y() {
        let mut history = UndoHistory::default();
        let mut text = String::new();
        let mut cursor = 0;
        handle_key_with_undo(&mut text, &mut cursor, key('x'), &mut history);
        assert!(handle_key_with_undo(
            &mut text,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
            &mut history,
        ));
        assert_eq!(text, "");
        assert!(handle_key_with_undo(
            &mut text,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
            &mut history,
        ));
        assert_eq!(text, "x");
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
}
