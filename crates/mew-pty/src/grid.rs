//! A bounded, framework-independent terminal grid.
//!
//! This is intentionally a client-side model. The daemon transports PTY
//! bytes; the grid owns cursor movement, ANSI state, screen updates, and
//! scrollback so a GPUI renderer does not need to understand escape codes.

use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

const DEFAULT_SCROLLBACK: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalStyle {
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCell {
    pub character: char,
    pub style: TerminalStyle,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            character: ' ',
            style: TerminalStyle::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminalPoint {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
enum ParseState {
    Ground,
    Escape,
    Csi(Vec<u8>),
    Osc,
    OscEscape,
}

/// A terminal screen with bounded scrollback and stateful ANSI parsing.
#[derive(Clone, Debug)]
pub struct TerminalGrid {
    rows: usize,
    cols: usize,
    screen: Vec<Vec<TerminalCell>>,
    scrollback: VecDeque<Vec<TerminalCell>>,
    max_scrollback: usize,
    cursor_row: usize,
    cursor_col: usize,
    saved_cursor: Option<(usize, usize, TerminalStyle)>,
    scroll_top: usize,
    scroll_bottom: usize,
    style: TerminalStyle,
    state: ParseState,
    utf8_pending: Vec<u8>,
    pending_wrap: bool,
    tab_stops: Vec<bool>,
}

impl TerminalGrid {
    pub fn new(rows: u16, cols: u16, max_scrollback: usize) -> Self {
        let rows = rows.max(1) as usize;
        let cols = cols.max(1) as usize;
        Self {
            rows,
            cols,
            screen: vec![vec![TerminalCell::default(); cols]; rows],
            scrollback: VecDeque::new(),
            max_scrollback,
            cursor_row: 0,
            cursor_col: 0,
            saved_cursor: None,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            style: TerminalStyle::default(),
            state: ParseState::Ground,
            utf8_pending: Vec::new(),
            pending_wrap: false,
            tab_stops: (0..cols).map(|column| column % 8 == 0).collect(),
        }
    }

    pub fn default_size() -> Self {
        Self::new(24, 80, DEFAULT_SCROLLBACK)
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    pub fn line_count(&self) -> usize {
        self.scrollback.len() + self.screen.len()
    }

    /// Feed an arbitrary chunk of PTY output. Parser state survives chunk
    /// boundaries, including an incomplete UTF-8 character or CSI sequence.
    pub fn process(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.process_byte(byte);
        }
    }

    /// Resize the visible screen, retaining the newest content and cursor.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1) as usize;
        let cols = cols.max(1) as usize;
        if rows == self.rows && cols == self.cols {
            return;
        }

        let mut screen = vec![vec![TerminalCell::default(); cols]; rows];
        let source_start = self.screen.len().saturating_sub(rows);
        for (new_row, source_row) in self.screen[source_start..].iter().enumerate() {
            for (new_col, cell) in source_row.iter().take(cols).enumerate() {
                screen[new_row][new_col] = cell.clone();
            }
        }
        self.cursor_row = self.cursor_row.saturating_sub(source_start).min(rows - 1);
        self.cursor_col = self.cursor_col.min(cols - 1);
        self.rows = rows;
        self.cols = cols;
        self.screen = screen;
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.tab_stops = (0..cols).map(|column| column % 8 == 0).collect();
        self.pending_wrap = false;
    }

    /// Return the bottom viewport when `scroll_offset` is zero. Larger values
    /// reveal older scrollback lines without mutating the live cursor.
    pub fn viewport(&self, scroll_offset: usize) -> Vec<Vec<TerminalCell>> {
        let end = self.line_count().saturating_sub(scroll_offset);
        let start = end.saturating_sub(self.rows);
        (start..end)
            .filter_map(|line| self.line(line).cloned())
            .collect()
    }

    pub fn line_text(&self, line: usize) -> String {
        self.line(line)
            .map(|cells| trim_line(cells))
            .unwrap_or_default()
    }

    pub fn search(&self, query: &str) -> Vec<TerminalMatch> {
        if query.is_empty() {
            return Vec::new();
        }
        self.scrollback
            .iter()
            .chain(self.screen.iter())
            .enumerate()
            .flat_map(|(line, cells)| {
                let text = trim_line(cells);
                text.match_indices(query)
                    .map(move |(start, found)| TerminalMatch {
                        line,
                        start,
                        end: start + found.len(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn copy_range(&self, start: TerminalPoint, end: TerminalPoint) -> String {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let last_line = end.line.min(self.line_count().saturating_sub(1));
        if start.line > last_line || self.line_count() == 0 {
            return String::new();
        }
        let mut output = String::new();
        for line in start.line..=last_line {
            let Some(cells) = self.line(line) else {
                continue;
            };
            let from = if line == start.line {
                start.column.min(cells.len())
            } else {
                0
            };
            let to = if line == last_line {
                end.column.min(cells.len())
            } else {
                cells.len()
            };
            let segment: String = cells[from..to].iter().map(|cell| cell.character).collect();
            if line != last_line {
                output.push_str(segment.trim_end());
            } else {
                output.push_str(&segment);
            }
            if line != last_line {
                output.push('\n');
            }
        }
        output
    }

    fn line(&self, line: usize) -> Option<&Vec<TerminalCell>> {
        self.scrollback
            .get(line)
            .or_else(|| self.screen.get(line.saturating_sub(self.scrollback.len())))
    }

    fn process_byte(&mut self, byte: u8) {
        match self.state.clone() {
            ParseState::Ground => self.process_ground(byte),
            ParseState::Escape => self.process_escape(byte),
            ParseState::Csi(mut params) => {
                if (0x40..=0x7e).contains(&byte) {
                    self.execute_csi(byte as char, &params);
                    self.state = ParseState::Ground;
                } else if params.len() < 128 {
                    params.push(byte);
                    self.state = ParseState::Csi(params);
                } else {
                    self.state = ParseState::Ground;
                }
            }
            ParseState::Osc => {
                if byte == 0x07 {
                    self.state = ParseState::Ground;
                } else if byte == 0x1b {
                    self.state = ParseState::OscEscape;
                }
            }
            ParseState::OscEscape => {
                self.state = if byte == b'\\' {
                    ParseState::Ground
                } else {
                    ParseState::Osc
                };
            }
        }
    }

    fn process_ground(&mut self, byte: u8) {
        match byte {
            0x1b => {
                self.flush_utf8();
                self.state = ParseState::Escape;
            }
            0x00..=0x07 | 0x0e..=0x1f | 0x7f => {
                self.flush_utf8();
                self.control(byte);
            }
            0x08..=0x0d => {
                self.flush_utf8();
                self.control(byte);
            }
            0x20..=0x7e => self.print_char(byte as char),
            _ => self.process_utf8(byte),
        }
    }

    fn process_escape(&mut self, byte: u8) {
        self.state = match byte {
            b'[' => ParseState::Csi(Vec::new()),
            b']' => ParseState::Osc,
            b'7' => {
                self.save_cursor();
                ParseState::Ground
            }
            b'8' => {
                self.restore_cursor();
                ParseState::Ground
            }
            b'D' => {
                self.line_feed();
                ParseState::Ground
            }
            b'M' => {
                self.reverse_index();
                ParseState::Ground
            }
            b'E' => {
                self.carriage_return();
                self.line_feed();
                ParseState::Ground
            }
            b'c' => {
                self.reset();
                ParseState::Ground
            }
            _ => ParseState::Ground,
        };
    }

    fn process_utf8(&mut self, byte: u8) {
        self.utf8_pending.push(byte);
        match std::str::from_utf8(&self.utf8_pending) {
            Ok(text) => {
                let text = text.to_owned();
                self.utf8_pending.clear();
                for character in text.chars() {
                    self.print_char(character);
                }
            }
            Err(error) if error.error_len().is_none() => {}
            Err(_) => {
                self.utf8_pending.clear();
                self.print_char('�');
            }
        }
    }

    fn flush_utf8(&mut self) {
        if !self.utf8_pending.is_empty() {
            self.utf8_pending.clear();
            self.print_char('�');
        }
    }

    fn control(&mut self, byte: u8) {
        match byte {
            0x08 => {
                self.pending_wrap = false;
                self.cursor_col = self.cursor_col.saturating_sub(1);
            }
            0x09 => self.tab(),
            0x0a..=0x0c => self.line_feed(),
            0x0d => self.carriage_return(),
            _ => {}
        }
    }

    fn print_char(&mut self, character: char) {
        let width = UnicodeWidthChar::width(character).unwrap_or(1).max(1);
        if width > self.cols {
            return;
        }
        if self.pending_wrap {
            self.carriage_return();
            self.line_feed();
        }
        if self.cursor_col + width > self.cols {
            self.pending_wrap = true;
            self.carriage_return();
            self.line_feed();
        }
        self.screen[self.cursor_row][self.cursor_col] = TerminalCell {
            character,
            style: self.style,
        };
        for offset in 1..width {
            self.screen[self.cursor_row][self.cursor_col + offset] = TerminalCell {
                character: ' ',
                style: self.style,
            };
        }
        self.cursor_col += width;
        if self.cursor_col >= self.cols {
            self.cursor_col = self.cols - 1;
            self.pending_wrap = true;
        }
    }

    fn carriage_return(&mut self) {
        self.pending_wrap = false;
        self.cursor_col = 0;
    }

    fn line_feed(&mut self) {
        self.pending_wrap = false;
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up(1);
        } else {
            self.cursor_row = (self.cursor_row + 1).min(self.rows - 1);
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor_row == self.scroll_top {
            self.scroll_down(1);
        } else {
            self.cursor_row = self.cursor_row.saturating_sub(1);
        }
    }

    fn tab(&mut self) {
        self.pending_wrap = false;
        let next = ((self.cursor_col + 8) / 8 * 8).min(self.cols - 1);
        self.cursor_col = (next..self.cols)
            .find(|column| self.tab_stops[*column])
            .unwrap_or(self.cols - 1);
    }

    fn scroll_up(&mut self, count: usize) {
        for _ in 0..count {
            let row = self.screen.remove(self.scroll_top);
            if self.scroll_top == 0
                && self.scroll_bottom == self.rows - 1
                && self.max_scrollback > 0
            {
                self.scrollback.push_back(row);
                while self.scrollback.len() > self.max_scrollback {
                    self.scrollback.pop_front();
                }
            }
            self.screen
                .insert(self.scroll_bottom, vec![TerminalCell::default(); self.cols]);
        }
    }

    fn scroll_down(&mut self, count: usize) {
        for _ in 0..count {
            self.screen.remove(self.scroll_bottom);
            self.screen
                .insert(self.scroll_top, vec![TerminalCell::default(); self.cols]);
        }
    }

    fn execute_csi(&mut self, final_byte: char, raw: &[u8]) {
        let private = raw.first() == Some(&b'?');
        let raw = if private { &raw[1..] } else { raw };
        let params = parse_params(raw);
        let first = |default: usize| {
            params
                .first()
                .copied()
                .filter(|value| *value != 0)
                .map(usize::from)
                .unwrap_or(default)
        };
        match final_byte {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(first(1)),
            'B' | 'e' => self.cursor_row = (self.cursor_row + first(1)).min(self.rows - 1),
            'C' | 'a' => self.cursor_col = (self.cursor_col + first(1)).min(self.cols - 1),
            'D' => self.cursor_col = self.cursor_col.saturating_sub(first(1)),
            'G' | '`' => self.cursor_col = first(1).saturating_sub(1).min(self.cols - 1),
            'd' => self.cursor_row = first(1).saturating_sub(1).min(self.rows - 1),
            'H' | 'f' => {
                self.cursor_row = params.first().copied().unwrap_or(1).max(1) as usize;
                self.cursor_row = self.cursor_row.saturating_sub(1).min(self.rows - 1);
                self.cursor_col = params.get(1).copied().unwrap_or(1).max(1) as usize;
                self.cursor_col = self.cursor_col.saturating_sub(1).min(self.cols - 1);
                self.pending_wrap = false;
            }
            'J' => self.erase_display(params.first().copied().unwrap_or(0)),
            'K' => self.erase_line(params.first().copied().unwrap_or(0)),
            'm' => self.apply_sgr(&params),
            'r' => self.set_scroll_region(&params),
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            'S' => self.scroll_up(first(1)),
            'T' => self.scroll_down(first(1)),
            '@' => self.insert_chars(first(1)),
            'P' => self.delete_chars(first(1)),
            'L' => self.insert_lines(first(1)),
            'M' => self.delete_lines(first(1)),
            'h' | 'l' if private => {}
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                for row in (self.cursor_row + 1)..self.rows {
                    self.screen[row].fill(TerminalCell::default());
                }
            }
            1 => {
                self.erase_line(1);
                for row in 0..self.cursor_row {
                    self.screen[row].fill(TerminalCell::default());
                }
            }
            2 => self.screen.fill(vec![TerminalCell::default(); self.cols]),
            3 => {
                self.screen.fill(vec![TerminalCell::default(); self.cols]);
                self.scrollback.clear();
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        match mode {
            0 => self.screen[self.cursor_row][self.cursor_col..].fill(TerminalCell::default()),
            1 => self.screen[self.cursor_row][..=self.cursor_col].fill(TerminalCell::default()),
            2 => self.screen[self.cursor_row].fill(TerminalCell::default()),
            _ => {}
        }
    }

    fn insert_chars(&mut self, count: usize) {
        let count = count.min(self.cols - self.cursor_col);
        let row = &mut self.screen[self.cursor_row];
        for _ in 0..count {
            row.insert(self.cursor_col, TerminalCell::default());
            row.pop();
        }
    }

    fn delete_chars(&mut self, count: usize) {
        let count = count.min(self.cols - self.cursor_col);
        let row = &mut self.screen[self.cursor_row];
        for _ in 0..count {
            row.remove(self.cursor_col);
            row.push(TerminalCell::default());
        }
    }

    fn insert_lines(&mut self, count: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let count = count.min(self.scroll_bottom - self.cursor_row + 1);
        for _ in 0..count {
            self.screen
                .insert(self.cursor_row, vec![TerminalCell::default(); self.cols]);
            self.screen.remove(self.scroll_bottom + 1);
        }
    }

    fn delete_lines(&mut self, count: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let count = count.min(self.scroll_bottom - self.cursor_row + 1);
        for _ in 0..count {
            self.screen.remove(self.cursor_row);
            self.screen
                .insert(self.scroll_bottom, vec![TerminalCell::default(); self.cols]);
        }
    }

    fn set_scroll_region(&mut self, params: &[u16]) {
        let top = params.first().copied().unwrap_or(1).max(1) as usize - 1;
        let bottom = params.get(1).copied().unwrap_or(self.rows as u16).max(1) as usize - 1;
        if top < bottom && bottom < self.rows {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            self.cursor_row = top;
            self.cursor_col = 0;
        }
    }

    fn apply_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.style = TerminalStyle::default();
            return;
        }
        let mut index = 0;
        while index < params.len() {
            match params[index] {
                0 => self.style = TerminalStyle::default(),
                1 => self.style.bold = true,
                3 => self.style.italic = true,
                4 => self.style.underline = true,
                7 => self.style.inverse = true,
                22 => self.style.bold = false,
                23 => self.style.italic = false,
                24 => self.style.underline = false,
                27 => self.style.inverse = false,
                30..=37 => {
                    self.style.foreground = TerminalColor::Indexed((params[index] - 30) as u8)
                }
                39 => self.style.foreground = TerminalColor::Default,
                40..=47 => {
                    self.style.background = TerminalColor::Indexed((params[index] - 40) as u8)
                }
                49 => self.style.background = TerminalColor::Default,
                90..=97 => {
                    self.style.foreground = TerminalColor::Indexed((params[index] - 90 + 8) as u8)
                }
                100..=107 => {
                    self.style.background = TerminalColor::Indexed((params[index] - 100 + 8) as u8)
                }
                38 | 48 => {
                    let target_foreground = params[index] == 38;
                    match params.get(index + 1).copied() {
                        Some(5) => {
                            if let Some(color) = params.get(index + 2).copied() {
                                set_color(
                                    &mut self.style,
                                    target_foreground,
                                    TerminalColor::Indexed(color as u8),
                                );
                                index += 2;
                            }
                        }
                        Some(2) => {
                            if let (Some(red), Some(green), Some(blue)) = (
                                params.get(index + 2),
                                params.get(index + 3),
                                params.get(index + 4),
                            ) {
                                set_color(
                                    &mut self.style,
                                    target_foreground,
                                    TerminalColor::Rgb(*red as u8, *green as u8, *blue as u8),
                                );
                                index += 4;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn save_cursor(&mut self) {
        self.saved_cursor = Some((self.cursor_row, self.cursor_col, self.style));
    }

    fn restore_cursor(&mut self) {
        if let Some((row, col, style)) = self.saved_cursor {
            self.cursor_row = row.min(self.rows - 1);
            self.cursor_col = col.min(self.cols - 1);
            self.style = style;
            self.pending_wrap = false;
        }
    }

    fn reset(&mut self) {
        self.screen.fill(vec![TerminalCell::default(); self.cols]);
        self.scrollback.clear();
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.saved_cursor = None;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.style = TerminalStyle::default();
        self.state = ParseState::Ground;
        self.utf8_pending.clear();
        self.pending_wrap = false;
    }
}

fn parse_params(raw: &[u8]) -> Vec<u16> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split(|byte| *byte == b';')
        .map(|part| {
            if part.is_empty() {
                0
            } else {
                std::str::from_utf8(part)
                    .ok()
                    .and_then(|part| part.parse().ok())
                    .unwrap_or(0)
            }
        })
        .collect()
}

fn set_color(style: &mut TerminalStyle, foreground: bool, color: TerminalColor) {
    if foreground {
        style.foreground = color;
    } else {
        style.background = color;
    }
}

fn trim_line(cells: &[TerminalCell]) -> String {
    cells
        .iter()
        .map(|cell| cell.character)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(grid: &TerminalGrid) -> Vec<String> {
        grid.viewport(0)
            .into_iter()
            .map(|line| trim_line(&line))
            .collect()
    }

    #[test]
    fn parses_cursor_movement_and_erase_commands() {
        let mut grid = TerminalGrid::new(3, 8, 16);
        grid.process(b"hello\x1b[2DXY\x1b[2;1Hthere\x1b[0K");
        assert_eq!(text(&grid), vec!["helXY", "there", ""]);
    }

    #[test]
    fn preserves_ansi_state_across_chunks() {
        let mut grid = TerminalGrid::new(2, 8, 16);
        grid.process(b"\x1b[38;2;255;");
        grid.process(b"10;20mred\x1b[0m");
        let line = grid.viewport(0).remove(0);
        assert_eq!(line[0].style.foreground, TerminalColor::Rgb(255, 10, 20));
        assert_eq!(line[0].character, 'r');
        assert_eq!(line[3].style, TerminalStyle::default());
    }

    #[test]
    fn scrollback_is_bounded_and_copy_search_use_logical_lines() {
        let mut grid = TerminalGrid::new(2, 12, 2);
        grid.process(b"one\r\ntwo\r\nthree\r\nfour\r\n");
        assert_eq!(grid.scrollback_len(), 2);
        assert_eq!(grid.search("three").len(), 1);
        assert_eq!(
            grid.copy_range(
                TerminalPoint { line: 0, column: 0 },
                TerminalPoint { line: 1, column: 3 }
            ),
            "two\nthr"
        );
    }

    #[test]
    fn resize_keeps_newest_content_and_utf8_chunks() {
        let mut grid = TerminalGrid::new(2, 8, 16);
        grid.process("café".as_bytes());
        grid.resize(3, 10);
        assert_eq!(grid.line_text(0), "café");
        grid.process(b"\x1b[2J\x1b[3;1Hok");
        assert_eq!(grid.line_text(2), "ok");
    }
}
