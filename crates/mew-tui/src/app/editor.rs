//! Shared readline-style line editing.
//!
//! Every text field in the TUI (chat input, picker filters, the question
//! overlay's freeform answer, plan feedback, history search, settings
//! fields) routes character editing through the functions here so the key
//! bindings and cursor semantics stay identical across surfaces.
//!
//! Cursor positions are byte offsets into the text and always sit on a
//! grapheme boundary. Word movement treats alphanumeric runs as words;
//! Ctrl+W uses whitespace boundaries, as in a Unix shell.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};
use std::{collections::VecDeque, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

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
    pub fn break_coalescing(&mut self) {
        self.last_push = None;
    }

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

/// Killed text shared across input fields. Navigation ends the current
/// command sequence but keeps the ring available for the next yank.
#[derive(Debug, Default)]
pub struct KillRing {
    entries: VecDeque<String>,
    consecutive: bool,
    yank: Option<(Range<usize>, usize)>,
}

impl KillRing {
    pub fn end_sequence(&mut self) {
        self.consecutive = false;
        self.yank = None;
    }

    fn save(&mut self, killed: &str, backward: bool) {
        if self.consecutive {
            let entry = self.entries.front_mut().expect("preceding kill");
            if backward {
                entry.insert_str(0, killed);
            } else {
                entry.push_str(killed);
            }
        } else {
            self.entries.push_front(killed.to_owned());
            self.entries.truncate(20);
        }
        self.consecutive = true;
        self.yank = None;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditResult {
    Unhandled,
    Consumed,
    Changed,
}

impl EditResult {
    pub fn handled(&self) -> bool {
        *self != Self::Unhandled
    }

    pub fn changed(&self) -> bool {
        *self == Self::Changed
    }
}

fn kill_range(text: &str, cursor: usize, key: KeyEvent) -> Option<Range<usize>> {
    let mut boundary = cursor;
    match (key.code, key.modifiers) {
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            cursor_end(text, &mut boundary);
            if boundary == cursor && boundary < text.len() {
                boundary += 1;
            }
            Some(cursor..boundary)
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) | (KeyCode::Backspace, KeyModifiers::META) => {
            cursor_home(text, &mut boundary);
            Some(boundary..cursor)
        }
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            let before = text[..cursor].trim_end_matches(char::is_whitespace);
            boundary = before
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
                .map_or(0, |(offset, c)| offset + c.len_utf8());
            Some(boundary..cursor)
        }
        (KeyCode::Backspace, KeyModifiers::ALT) => {
            cursor_word_left(text, &mut boundary);
            Some(boundary..cursor)
        }
        (KeyCode::Char('d'), KeyModifiers::ALT) => {
            cursor_word_right(text, &mut boundary);
            Some(cursor..boundary)
        }
        _ => None,
    }
}

/// Whether the key can continue a kill/yank sequence across event dispatch.
pub fn is_kill_or_yank(key: KeyEvent) -> bool {
    matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('k' | 'u' | 'w' | 'y'), KeyModifiers::CONTROL)
            | (
                KeyCode::Char('d' | 'y') | KeyCode::Backspace,
                KeyModifiers::ALT
            )
            | (KeyCode::Backspace, KeyModifiers::META)
    )
}

pub fn is_newline(key: KeyEvent) -> bool {
    (key.code == KeyCode::Enter && matches!(key.modifiers, KeyModifiers::ALT | KeyModifiers::SHIFT))
        || (key.code == KeyCode::Char('j') && key.modifiers == KeyModifiers::CONTROL)
}

/// Insert a character at the cursor.
pub fn insert(text: &mut String, cursor: &mut usize, c: char) {
    text.insert(*cursor, c);
    *cursor += c.len_utf8();
}

/// Delete the grapheme before the cursor (Backspace).
pub fn backspace(text: &mut String, cursor: &mut usize) {
    if *cursor > 0 {
        let end = *cursor;
        cursor_left(text, cursor);
        text.replace_range(*cursor..end, "");
    }
}

/// Delete the grapheme at the cursor (Delete / Ctrl+D).
pub fn delete(text: &mut String, cursor: &mut usize) {
    if *cursor < text.len() {
        let mut end = *cursor;
        cursor_right(text, &mut end);
        text.replace_range(*cursor..end, "");
    }
}

/// Delete to the end of this line, or its newline when already at the end.
pub fn kill_to_end(text: &mut String, cursor: &mut usize) {
    let mut end = *cursor;
    cursor_end(text, &mut end);
    if end == *cursor && end < text.len() {
        end += 1;
    }
    text.replace_range(*cursor..end, "");
}

/// Clear the whole text.
pub fn clear(text: &mut String, cursor: &mut usize) {
    text.clear();
    *cursor = 0;
}

/// Move the cursor to the previous grapheme.
pub fn cursor_left(text: &str, cursor: &mut usize) {
    if *cursor > 0 {
        *cursor = text[..*cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }
}

/// Move the cursor to the next grapheme.
pub fn cursor_right(text: &str, cursor: &mut usize) {
    if *cursor < text.len() {
        *cursor = text[*cursor..]
            .graphemes(true)
            .next()
            .map(|grapheme| *cursor + grapheme.len())
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
    let mut in_word = false;
    for (offset, grapheme) in text[..*cursor].grapheme_indices(true).rev() {
        let word = grapheme.chars().any(char::is_alphanumeric);
        if in_word && !word {
            break;
        }
        in_word |= word;
        *cursor = offset;
    }
}

/// Move the cursor to the end of the next word.
pub fn cursor_word_right(text: &str, cursor: &mut usize) {
    let start = *cursor;
    let mut in_word = false;
    for (offset, grapheme) in text[start..].grapheme_indices(true) {
        let word = grapheme.chars().any(char::is_alphanumeric);
        if in_word && !word {
            break;
        }
        in_word |= word;
        *cursor = start + offset + grapheme.len();
    }
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
    let ctrl = key.modifiers == KeyModifiers::CONTROL;
    let alt = key.modifiers == KeyModifiers::ALT;
    let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
    if let Some(range) = kill_range(text, *cursor, key) {
        *cursor = range.start;
        text.replace_range(range, "");
        return true;
    }
    match key.code {
        KeyCode::Char(c) if plain => {
            insert(text, cursor, c);
        }
        KeyCode::Char('a') if ctrl => cursor_home(text, cursor),
        KeyCode::Char('e') if ctrl => cursor_end(text, cursor),
        KeyCode::Char('f') if ctrl => cursor_right(text, cursor),
        KeyCode::Char('b') if ctrl => cursor_left(text, cursor),
        KeyCode::Char('d') if ctrl => delete(text, cursor),
        KeyCode::Char('h') if ctrl => backspace(text, cursor),
        KeyCode::Char('b') if alt => cursor_word_left(text, cursor),
        KeyCode::Char('f') if alt => cursor_word_right(text, cursor),
        KeyCode::Char('<') if alt || key.modifiers == (KeyModifiers::ALT | KeyModifiers::SHIFT) => {
            *cursor = 0
        }
        KeyCode::Char('>') if alt || key.modifiers == (KeyModifiers::ALT | KeyModifiers::SHIFT) => {
            *cursor = text.len()
        }
        KeyCode::Backspace if plain => backspace(text, cursor),
        KeyCode::Delete if plain => delete(text, cursor),
        KeyCode::Left if alt => cursor_word_left(text, cursor),
        KeyCode::Left if plain => cursor_left(text, cursor),
        KeyCode::Right if alt => cursor_word_right(text, cursor),
        KeyCode::Right if plain => cursor_right(text, cursor),
        KeyCode::Home if plain => cursor_home(text, cursor),
        KeyCode::End if plain => cursor_end(text, cursor),
        _ => return false,
    }
    true
}

/// Route editing through the field's undo history and the shared kill ring.
///
/// Surfaces with an [`UndoHistory`] should call this instead of
/// [`handle_key`] so every edit (including un-done ones) participates in
/// the same undo path.
pub fn handle_key_with_undo(
    text: &mut String,
    cursor: &mut usize,
    key: KeyEvent,
    history: &mut UndoHistory,
    ring: &mut KillRing,
) -> EditResult {
    if let Some(range) = kill_range(text, *cursor, key) {
        if range.is_empty() {
            ring.end_sequence();
            return EditResult::Consumed;
        }
        history.break_coalescing();
        history.snapshot(text, *cursor);
        ring.save(&text[range.clone()], range.start < *cursor);
        *cursor = range.start;
        text.replace_range(range, "");
        history.break_coalescing();
        return EditResult::Changed;
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
            ring.end_sequence();
            let Some(entry) = ring.entries.front() else {
                return EditResult::Consumed;
            };
            history.break_coalescing();
            history.snapshot(text, *cursor);
            let start = *cursor;
            text.insert_str(start, entry);
            *cursor += entry.len();
            ring.yank = Some((start..*cursor, 0));
            history.break_coalescing();
            return EditResult::Changed;
        }
        (KeyCode::Char('y'), KeyModifiers::ALT) => {
            let Some((range, index)) = ring.yank.as_mut() else {
                return EditResult::Consumed;
            };
            // A paste, mouse move or field change must not replace unrelated text.
            if *cursor != range.end
                || text.get(range.clone()) != ring.entries.get(*index).map(String::as_str)
            {
                ring.end_sequence();
                return EditResult::Consumed;
            }
            if ring.entries.len() < 2 {
                return EditResult::Consumed;
            }
            history.break_coalescing();
            history.snapshot(text, *cursor);
            *index = (*index + 1) % ring.entries.len();
            let entry = &ring.entries[*index];
            text.replace_range(range.clone(), entry);
            range.end = range.start + entry.len();
            *cursor = range.end;
            history.break_coalescing();
            return EditResult::Changed;
        }
        _ => ring.end_sequence(),
    }

    let undo = matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('z' | '_' | '7'), KeyModifiers::CONTROL)
    ) || (key.code == KeyCode::Char('_')
        && key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT));
    let redo = key.modifiers == KeyModifiers::CONTROL | KeyModifiers::SHIFT
        && matches!(key.code, KeyCode::Char('z' | 'Z'));
    if undo || redo {
        let available = if undo {
            !history.undo_is_empty()
        } else {
            !history.redo_is_empty()
        };
        if undo {
            history.undo(text, cursor);
        } else {
            history.redo(text, cursor);
        }
        return if available {
            EditResult::Changed
        } else {
            EditResult::Consumed
        };
    }

    let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
    let transpose = key.code == KeyCode::Char('t') && key.modifiers == KeyModifiers::CONTROL;
    let mutates = transpose
        || (plain
            && matches!(
                key.code,
                KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
            ))
        || (key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('d' | 'h')));
    if !mutates {
        history.break_coalescing();
        return if handle_key(text, cursor, key) {
            EditResult::Consumed
        } else {
            EditResult::Unhandled
        };
    }
    if transpose {
        let mut middle = *cursor;
        let mut end = *cursor;
        if text[*cursor..].starts_with('\n') || *cursor == text.len() {
            cursor_left(text, &mut middle);
        } else {
            cursor_right(text, &mut end);
        }
        let mut start = middle;
        cursor_left(text, &mut start);
        history.break_coalescing();
        if start == middle
            || text[start..end].contains('\n')
            || text[start..middle] == text[middle..end]
        {
            return EditResult::Consumed;
        }
        history.snapshot(text, *cursor);
        let left = text[start..middle].to_owned();
        text.replace_range(start..middle, "");
        text.insert_str(end - left.len(), &left);
        *cursor = end;
        history.break_coalescing();
        return EditResult::Changed;
    }
    let backward = key.code == KeyCode::Backspace
        || (key.code == KeyCode::Char('h') && key.modifiers == KeyModifiers::CONTROL);
    let forward = key.code == KeyCode::Delete
        || (key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::CONTROL);
    if (backward && *cursor == 0) || (forward && *cursor == text.len()) {
        return EditResult::Consumed;
    }
    history.snapshot(text, *cursor);
    handle_key(text, cursor, key);
    EditResult::Changed
}

/// Insert at the cursor with a snapshot of the prior state (for surfaces
/// whose Enter inserts a newline outside the key router).
pub fn insert_with_undo(text: &mut String, cursor: &mut usize, c: char, history: &mut UndoHistory) {
    history.snapshot(text, *cursor);
    insert(text, cursor, c);
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
        cursor_word_left(&text, &mut cursor);
        assert_eq!(&text[cursor..], "bar baz");
        cursor_word_right(&text, &mut cursor);
        assert_eq!(&text[..cursor], "foo bar");
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
        assert_eq!(text, "b");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn line_kills_preserve_the_other_lines_and_the_text_after_the_cursor() {
        let mut text = String::from("first\nhello world\nlast");
        let mut cursor = 11;
        handle_key(&mut text, &mut cursor, chord('k', KeyModifiers::CONTROL));
        assert_eq!(text, "first\nhello\nlast");
        handle_key(&mut text, &mut cursor, chord('k', KeyModifiers::CONTROL));
        assert_eq!(text, "first\nhellolast");
        handle_key(&mut text, &mut cursor, chord('u', KeyModifiers::CONTROL));
        assert_eq!(text, "first\nlast");
        assert_eq!(cursor, 6);
    }

    #[test]
    fn movement_and_deletion_keep_combining_text_and_emoji_intact() {
        let mut text = String::from("e\u{301}👩‍💻x");
        let mut cursor = text.len();
        cursor_left(&text, &mut cursor);
        assert_eq!(&text[cursor..], "x");
        backspace(&mut text, &mut cursor);
        assert_eq!(text, "e\u{301}x");
        cursor_left(&text, &mut cursor);
        assert_eq!(cursor, 0);
        delete(&mut text, &mut cursor);
        assert_eq!(text, "x");
    }

    #[test]
    fn unix_word_rubout_uses_whitespace_while_alt_word_keys_skip_punctuation() {
        let mut text = String::from("one /tmp/foo-bar  ");
        let mut cursor = text.len();
        handle_key(&mut text, &mut cursor, chord('w', KeyModifiers::CONTROL));
        assert_eq!(text, "one ");
        text = "one /tmp/foo-bar  ".into();
        cursor = text.len();
        handle_key(&mut text, &mut cursor, chord('b', KeyModifiers::ALT));
        assert_eq!(&text[cursor..], "bar  ");
        handle_key(&mut text, &mut cursor, chord('b', KeyModifiers::ALT));
        assert_eq!(&text[cursor..], "foo-bar  ");
        handle_key(&mut text, &mut cursor, chord('f', KeyModifiers::ALT));
        assert_eq!(&text[..cursor], "one /tmp/foo");
    }

    fn chord(c: char, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), modifiers)
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
        let mut ring = KillRing::default();
        let mut text = String::new();
        let mut cursor = 0;
        handle_key_with_undo(&mut text, &mut cursor, key('a'), &mut history, &mut ring);
        handle_key_with_undo(&mut text, &mut cursor, key('b'), &mut history, &mut ring);
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
    fn handle_key_with_undo_binds_ctrl_z_and_shift_z() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = String::new();
        let mut cursor = 0;
        handle_key_with_undo(&mut text, &mut cursor, key('x'), &mut history, &mut ring);
        assert!(handle_key_with_undo(
            &mut text,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        )
        .handled());
        assert_eq!(text, "");
        assert!(handle_key_with_undo(
            &mut text,
            &mut cursor,
            KeyEvent::new(
                KeyCode::Char('Z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            &mut history,
            &mut ring,
        )
        .handled());
        assert_eq!(text, "x");
    }

    #[test]
    fn consecutive_kills_yank_as_one_piece_and_yank_is_undoable() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = "one two three".to_owned();
        let mut cursor = text.len();
        for c in ['w', 'w', 'y'] {
            handle_key_with_undo(
                &mut text,
                &mut cursor,
                chord(c, KeyModifiers::CONTROL),
                &mut history,
                &mut ring,
            );
        }
        assert_eq!(text, "one two three");
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('z', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, "one ");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn yank_pop_rotates_kills_and_stops_after_cursor_movement() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = "one two".to_owned();
        let mut cursor = text.len();
        for (c, modifiers) in [
            ('w', KeyModifiers::CONTROL),
            ('a', KeyModifiers::CONTROL),
            ('k', KeyModifiers::CONTROL),
            ('y', KeyModifiers::CONTROL),
            ('y', KeyModifiers::ALT),
        ] {
            handle_key_with_undo(
                &mut text,
                &mut cursor,
                chord(c, modifiers),
                &mut history,
                &mut ring,
            );
        }
        assert_eq!(text, "two");
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('b', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('y', KeyModifiers::ALT),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, "two");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn transpose_swaps_whole_graphemes_and_stays_on_this_line() {
        for (input, position, expected, expected_cursor) in [
            ("ab", 2, "ba", 2),
            ("ab\ncd", 2, "ba\ncd", 2),
            ("a\nb", 2, "a\nb", 2),
            ("e\u{301}👩‍💻", 3, "👩‍💻e\u{301}", 14),
        ] {
            let mut text = input.to_owned();
            let mut cursor = position;
            let mut history = UndoHistory::default();
            let mut ring = KillRing::default();
            handle_key_with_undo(
                &mut text,
                &mut cursor,
                chord('t', KeyModifiers::CONTROL),
                &mut history,
                &mut ring,
            );
            assert_eq!(text, expected, "{input:?}");
            assert_eq!(cursor, expected_cursor);
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn printable_delete_binding_letters_still_type_at_empty_boundaries() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = String::new();
        let mut cursor = 0;
        for c in ['h', 'd'] {
            assert!(
                handle_key_with_undo(&mut text, &mut cursor, key(c), &mut history, &mut ring)
                    .changed()
            );
        }
        assert_eq!(text, "hd");
    }

    #[test]
    fn word_delete_and_control_backspace_can_be_undone_without_splitting_graphemes() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = "e\u{301} /tmp/foo".to_owned();
        let mut cursor = 3;
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('d', KeyModifiers::ALT),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, "e\u{301}/foo");
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('y', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, "e\u{301} /tmp/foo");
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('a', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('f', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('h', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, " /tmp/foo");
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('_', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, "e\u{301} /tmp/foo");
    }

    #[test]
    fn empty_kills_and_yanks_do_not_create_undo_or_ring_entries() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = String::new();
        let mut cursor = 0;
        for c in ['k', 'u', 'w', 'y', 't', 'h', 'd'] {
            assert!(!handle_key_with_undo(
                &mut text,
                &mut cursor,
                chord(c, KeyModifiers::CONTROL),
                &mut history,
                &mut ring
            )
            .changed());
        }
        assert!(history.undo_is_empty());
        assert!(ring.entries.is_empty());
        assert_eq!(cursor, 0);
    }

    #[test]
    fn cursor_movement_separates_typing_undo_steps() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = String::new();
        let mut cursor = 0;
        handle_key_with_undo(&mut text, &mut cursor, key('a'), &mut history, &mut ring);
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('b', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        handle_key_with_undo(&mut text, &mut cursor, key('b'), &mut history, &mut ring);
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('z', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, "a");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn undo_accepts_the_control_underscore_event_from_legacy_terminals() {
        let mut history = UndoHistory::default();
        let mut ring = KillRing::default();
        let mut text = String::new();
        let mut cursor = 0;
        handle_key_with_undo(&mut text, &mut cursor, key('x'), &mut history, &mut ring);
        handle_key_with_undo(
            &mut text,
            &mut cursor,
            chord('7', KeyModifiers::CONTROL),
            &mut history,
            &mut ring,
        );
        assert_eq!(text, "");
    }

    #[test]
    fn shifted_alt_angle_keys_move_across_the_whole_multiline_field() {
        let mut text = "one\ntwo".to_owned();
        let mut cursor = 2;
        assert!(handle_key(
            &mut text,
            &mut cursor,
            chord('>', KeyModifiers::ALT | KeyModifiers::SHIFT)
        ));
        assert_eq!(cursor, text.len());
        assert!(handle_key(
            &mut text,
            &mut cursor,
            chord('<', KeyModifiers::ALT | KeyModifiers::SHIFT)
        ));
        assert_eq!(cursor, 0);
    }
}
