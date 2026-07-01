//! Hashline format primitives: hash computation, normalization, and helpers.

/// File-section header delimiters.
pub const HL_FILE_PREFIX: char = '[';
pub const HL_FILE_SUFFIX: char = ']';

/// Separator between path and hash in a file-section header.
pub const HL_FILE_HASH_SEP: char = '#';

/// Separator between two line numbers in a range (`5.=10`).
pub const HL_RANGE_SEP: &str = ".=";

/// Payload sigil for literal body rows.
pub const HL_PAYLOAD_REPLACE: char = '+';

/// Hunk-header keywords.
pub const HL_REPLACE_KEYWORD: &str = "SWAP";
pub const HL_DELETE_KEYWORD: &str = "DEL";
pub const HL_INSERT_KEYWORD: &str = "INS";
pub const HL_REM_KEYWORD: &str = "REM";
pub const HL_MOVE_KEYWORD: &str = "MV";

/// Block-aware hunk-header keywords.
pub const HL_REPLACE_BLOCK_KEYWORD: &str = "SWAP.BLK";
pub const HL_DELETE_BLOCK_KEYWORD: &str = "DEL.BLK";
pub const HL_INSERT_AFTER_BLOCK_KEYWORD: &str = "INS.BLK.POST";

/// Insert position keywords.
pub const HL_INSERT_BEFORE: &str = "PRE";
pub const HL_INSERT_AFTER: &str = "POST";
pub const HL_INSERT_HEAD: &str = "HEAD";
pub const HL_INSERT_TAIL: &str = "TAIL";

/// Number of hex characters in a content-derived file-hash tag.
pub const HL_FILE_HASH_LENGTH: usize = 4;

/// Normalize text before hashing.
///
/// Trailing spaces, tabs, and carriage returns are stripped from every line
/// (and the final line) so cosmetic whitespace does not invalidate the tag.
/// Unlike the previous mew implementation, this preserves a trailing newline
/// so that `file\n` and `file` hash differently, matching oh-my-pi/hashline.
pub fn normalize_file_hash_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut trailing_ws_start: Option<usize> = None;

    for ch in text.chars() {
        if ch == '\n' {
            // Drop trailing whitespace accumulated since the last newline.
            if let Some(start) = trailing_ws_start {
                out.truncate(start);
            }
            trailing_ws_start = None;
            out.push('\n');
        } else if ch == ' ' || ch == '\t' || ch == '\r' {
            if trailing_ws_start.is_none() {
                trailing_ws_start = Some(out.len());
            }
        } else {
            trailing_ws_start = None;
            out.push(ch);
        }
    }

    // Drop trailing whitespace at EOF.
    if let Some(start) = trailing_ws_start {
        out.truncate(start);
    }

    out
}

/// Compute the 4-hex uppercase content hash used in hashline headers.
///
/// Uses xxHash32 with seed 0, matching oh-my-pi/hashline, and keeps the low
/// 16 bits. This is stable across processes and compiler versions.
pub fn compute_file_hash(text: &str) -> String {
    let normalized = normalize_file_hash_text(text);
    let hash = xxhash_rust::xxh32::xxh32(normalized.as_bytes(), 0);
    format!("{:04X}", hash & 0xFFFF)
}

/// Format a hashline section header for a file path and snapshot tag.
pub fn format_hashline_header(file_path: &str, file_hash: &str) -> String {
    format!("[{file_path}{HL_FILE_HASH_SEP}{file_hash}]")
}

/// Format a numbered line as returned by read/search tools.
pub fn format_numbered_line(line_number: usize, line: &str) -> String {
    format!("{line_number}:{line}")
}

/// Format file text with hashline-mode line-number prefixes.
pub fn format_numbered_lines(text: &str, start_line: usize) -> String {
    text.lines()
        .enumerate()
        .map(|(i, line)| format_numbered_line(start_line + i, line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Detect the dominant line ending of a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

pub fn detect_line_ending(text: &str) -> LineEnding {
    if text.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}

/// Strip a leading UTF-8 BOM if present.
pub fn strip_bom(text: &str) -> (&str, bool) {
    if let Some(rest) = text.strip_prefix('\u{FEFF}') {
        (rest, true)
    } else {
        (text, false)
    }
}

/// Normalize a text to LF for hashing and editing.
pub fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Restore the original line ending style after editing.
pub fn restore_line_endings(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Lf => text.to_string(),
        LineEnding::Crlf => text.replace('\n', "\r\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_four_hex() {
        let h1 = compute_file_hash("hello\nworld\n");
        let h2 = compute_file_hash("hello\nworld\n");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), HL_FILE_HASH_LENGTH);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_is_uppercase_hex() {
        let h = compute_file_hash("x");
        assert_eq!(h, h.to_ascii_uppercase());
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn trailing_whitespace_is_normalized() {
        let h1 = compute_file_hash("hello\nworld\n");
        let h2 = compute_file_hash("hello  \nworld\t\r\n");
        assert_eq!(h1, h2, "trailing spaces/tabs/cr should not change hash");
    }

    #[test]
    fn trailing_newline_matters() {
        let h1 = compute_file_hash("hello\nworld");
        let h2 = compute_file_hash("hello\nworld\n");
        assert_ne!(h1, h2, "trailing newline should change hash");
    }

    #[test]
    fn header_formatting() {
        assert_eq!(
            format_hashline_header("src/lib.rs", "A1B2"),
            "[src/lib.rs#A1B2]"
        );
    }

    #[test]
    fn numbered_lines() {
        assert_eq!(format_numbered_lines("a\nb", 1), "1:a\n2:b");
    }
}
