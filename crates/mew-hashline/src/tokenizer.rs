//! Line-oriented tokenizer for hashline patches.

use crate::format::{
    HL_DELETE_BLOCK_KEYWORD, HL_DELETE_KEYWORD, HL_FILE_HASH_LENGTH, HL_FILE_PREFIX,
    HL_FILE_SUFFIX, HL_INSERT_AFTER, HL_INSERT_AFTER_BLOCK_KEYWORD, HL_INSERT_BEFORE,
    HL_INSERT_HEAD, HL_INSERT_KEYWORD, HL_INSERT_TAIL, HL_MOVE_KEYWORD, HL_REM_KEYWORD,
    HL_REPLACE_BLOCK_KEYWORD, HL_REPLACE_KEYWORD,
};
use crate::types::Anchor;

/// A classified line from a hashline patch, with its 1-indexed source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineToken {
    pub line: usize,
    pub token: Token,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Blank,
    Header { path: String, hash: Option<String> },
    Op { target: OpTarget },
    PayloadLiteral { text: String },
    Raw { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpTarget {
    Replace { start: usize, end: usize },
    ReplaceBlock { anchor: Anchor },
    Delete { start: usize, end: usize },
    DeleteBlock { anchor: Anchor },
    InsertBefore { anchor: Anchor },
    InsertAfter { anchor: Anchor },
    InsertAfterBlock { anchor: Anchor },
    InsertHead,
    InsertTail,
    Rem,
    Move { dest: String },
}

/// Tokenize a complete patch string into a vector of line-tokens.
pub fn tokenize(text: &str) -> crate::Result<Vec<LineToken>> {
    let mut tokens = Vec::new();
    for (i, line) in split_lines(text).into_iter().enumerate() {
        tokens.push(LineToken {
            line: i + 1,
            token: classify_line(&line)?,
        });
    }
    Ok(tokens)
}

/// Split text into lines, preserving content but stripping `\r` and trailing
/// whitespace the same way the tokenizer does.
pub fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            let mut end = i;
            // Drop a trailing \r if present.
            if end > start {
                let before = &text[..end];
                if before.ends_with('\r') {
                    end -= 1;
                }
            }
            lines.push(text[start..end].to_string());
            start = i + 1;
        }
    }
    if start <= text.len() {
        let mut end = text.len();
        if end > start && text[..end].ends_with('\r') {
            end -= 1;
        }
        lines.push(text[start..end].to_string());
    }
    lines
}

fn classify_line(line: &str) -> crate::Result<Token> {
    if line.trim().is_empty() {
        return Ok(Token::Blank);
    }

    if let Some(header) = try_parse_header(line) {
        return Ok(Token::Header {
            path: header.path,
            hash: header.hash,
        });
    }

    if let Some(target) = try_parse_op(line)? {
        return Ok(Token::Op { target });
    }

    if let Some(text) = line.strip_prefix('+') {
        return Ok(Token::PayloadLiteral {
            text: text.to_string(),
        });
    }

    Ok(Token::Raw {
        text: line.to_string(),
    })
}

struct ParsedHeader {
    path: String,
    hash: Option<String>,
}

fn try_parse_header(line: &str) -> Option<ParsedHeader> {
    let trimmed = line.trim_end();
    if !trimmed.starts_with(HL_FILE_PREFIX) || !trimmed.ends_with(HL_FILE_SUFFIX) {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.is_empty() {
        return None;
    }

    // The tag, if present, is the trailing `#XXXX` block. Paths may contain
    // whitespace but not `#`.
    let mut path_end = inner.len();
    let mut hash: Option<String> = None;
    let trailing_start = inner.len().saturating_sub(HL_FILE_HASH_LENGTH + 1);
    if trailing_start > 0 && inner.as_bytes()[trailing_start] == b'#' {
        let candidate = &inner[trailing_start + 1..];
        if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
            path_end = trailing_start;
            hash = Some(candidate.to_ascii_uppercase());
        }
    }

    if inner[..path_end].contains('#') {
        return None;
    }
    if path_end == 0 {
        return None;
    }

    Some(ParsedHeader {
        path: inner[..path_end].to_string(),
        hash,
    })
}

fn try_parse_op(line: &str) -> crate::Result<Option<OpTarget>> {
    let trimmed = line.trim_start();
    let mut words = trimmed.split_whitespace();
    let Some(first_word) = words.next() else {
        return Ok(None);
    };

    // `INS.PRE`, `INS.POST`, `INS.HEAD`, `INS.TAIL`, and block variants are
    // written without whitespace after the keyword, so split on the dot.
    let (keyword, rest) = if first_word.starts_with(HL_INSERT_KEYWORD)
        && first_word.len() > HL_INSERT_KEYWORD.len()
        && first_word.as_bytes()[HL_INSERT_KEYWORD.len()] == b'.'
    {
        let after = &first_word[HL_INSERT_KEYWORD.len()..];
        let rest = format!("{} {}", after, trimmed[first_word.len()..].trim_start());
        (HL_INSERT_KEYWORD, rest)
    } else {
        (
            first_word,
            trimmed[first_word.len()..].trim_start().to_string(),
        )
    };

    let rest: &str = &rest;

    match keyword {
        HL_REM_KEYWORD => {
            if !rest.is_empty() {
                Err(crate::HashlineError::parse(
                    0,
                    format!("`{HL_REM_KEYWORD}` takes no arguments"),
                ))
            } else {
                Ok(Some(OpTarget::Rem))
            }
        }
        HL_MOVE_KEYWORD => {
            if rest.is_empty() {
                Err(crate::HashlineError::parse(
                    0,
                    format!("`{HL_MOVE_KEYWORD}` requires a destination path"),
                ))
            } else {
                Ok(Some(OpTarget::Move {
                    dest: unquote_path(rest).to_string(),
                }))
            }
        }
        HL_REPLACE_BLOCK_KEYWORD => {
            let (anchor, tail) = parse_lid_and_tail(rest)?;
            require_colon_only(tail)?;
            Ok(Some(OpTarget::ReplaceBlock { anchor }))
        }
        HL_DELETE_BLOCK_KEYWORD => {
            let (anchor, tail) = parse_lid_and_tail(rest)?;
            if !tail.is_empty() {
                return Err(crate::HashlineError::parse(
                    0,
                    format!("`{HL_DELETE_BLOCK_KEYWORD}` takes no body"),
                ));
            }
            Ok(Some(OpTarget::DeleteBlock { anchor }))
        }
        HL_INSERT_AFTER_BLOCK_KEYWORD => {
            let (anchor, tail) = parse_lid_and_tail(rest)?;
            require_colon_only(tail)?;
            Ok(Some(OpTarget::InsertAfterBlock { anchor }))
        }
        HL_REPLACE_KEYWORD => {
            let (start, end, tail) = parse_range(rest, true)?;
            require_colon_only(tail)?;
            Ok(Some(OpTarget::Replace { start, end }))
        }
        HL_DELETE_KEYWORD => {
            let (start, end, tail) = parse_range(rest, true)?;
            if !tail.is_empty() {
                return Err(crate::HashlineError::parse(
                    0,
                    format!("`{HL_DELETE_KEYWORD}` has no body; remove the colon"),
                ));
            }
            Ok(Some(OpTarget::Delete { start, end }))
        }
        HL_INSERT_KEYWORD => parse_insert(rest),
        _ => Ok(None),
    }
}

fn parse_insert(rest: &str) -> crate::Result<Option<OpTarget>> {
    if rest.is_empty() {
        return Ok(None);
    }

    let rest = rest.strip_prefix('.').unwrap_or(rest);

    let sub_end = rest
        .find(|c: char| c.is_whitespace() || c == ':')
        .unwrap_or(rest.len());
    let sub = &rest[..sub_end];
    let after = rest[sub_end..].trim_start();

    match sub {
        HL_INSERT_BEFORE => {
            let (anchor, tail) = parse_lid_and_tail(after)?;
            require_colon_only(tail)?;
            Ok(Some(OpTarget::InsertBefore { anchor }))
        }
        HL_INSERT_AFTER => {
            let (anchor, tail) = parse_lid_and_tail(after)?;
            require_colon_only(tail)?;
            Ok(Some(OpTarget::InsertAfter { anchor }))
        }
        HL_INSERT_HEAD => {
            require_colon_only(after)?;
            Ok(Some(OpTarget::InsertHead))
        }
        HL_INSERT_TAIL => {
            require_colon_only(after)?;
            Ok(Some(OpTarget::InsertTail))
        }
        _ => Ok(None),
    }
}

fn parse_lid_and_tail(s: &str) -> crate::Result<(Anchor, &str)> {
    let s = s.trim_start();
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return Err(crate::HashlineError::parse(0, "expected a line number"));
    }
    let num_str = &s[..end];
    let line: usize = num_str
        .parse()
        .map_err(|_| crate::HashlineError::parse(0, format!("invalid line number: {num_str}")))?;
    if line == 0 {
        return Err(crate::HashlineError::parse(0, "line numbers start at 1"));
    }
    Ok((Anchor { line }, &s[end..]))
}

fn parse_range(s: &str, allow_single: bool) -> crate::Result<(usize, usize, &str)> {
    let s = s.trim_start();
    let (start, after_start) = parse_lid_no_trim(s)?;

    let after = after_start.trim_start();
    if after.is_empty() || after.starts_with(':') {
        if !allow_single {
            return Err(crate::HashlineError::parse(
                0,
                "range must specify start.=end".to_string(),
            ));
        }
        return Ok((start, start, after));
    }

    // Accept `.=` / `..` / `-` / `…` as separators.
    let sep_len = if after.starts_with(".=") || after.starts_with("..") {
        2
    } else if after.starts_with('…') {
        '…'.len_utf8()
    } else if after.starts_with('-') {
        1
    } else {
        return Err(crate::HashlineError::parse(
            0,
            format!("expected range separator after {start}"),
        ));
    };

    let after_sep = after[sep_len..].trim_start();
    let (end, after_end) = parse_lid_no_trim(after_sep)?;
    if end < start {
        return Err(crate::HashlineError::InvalidRange { start, end });
    }

    Ok((start, end, after_end.trim_start()))
}

fn parse_lid_no_trim(s: &str) -> crate::Result<(usize, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return Err(crate::HashlineError::parse(0, "expected a line number"));
    }
    let num_str = &s[..end];
    let line: usize = num_str
        .parse()
        .map_err(|_| crate::HashlineError::parse(0, format!("invalid line number: {num_str}")))?;
    if line == 0 {
        return Err(crate::HashlineError::parse(0, "line numbers start at 1"));
    }
    Ok((line, &s[end..]))
}

fn require_colon_only(tail: &str) -> crate::Result<()> {
    let trimmed = tail.trim();
    if trimmed.is_empty() || trimmed == ":" {
        return Ok(());
    }
    Err(crate::HashlineError::parse(
        0,
        format!("unexpected trailing text: {trimmed}"),
    ))
}

fn unquote_path(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 {
        let first = s.as_bytes()[0];
        let last = s.as_bytes()[s.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return &s[1..s.len() - 1];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_header_with_hash() {
        let tokens = tokenize("[src/lib.rs#A1B2]\n").unwrap();
        assert_eq!(
            tokens,
            vec![
                LineToken {
                    line: 1,
                    token: Token::Header {
                        path: "src/lib.rs".to_string(),
                        hash: Some("A1B2".to_string()),
                    },
                },
                LineToken {
                    line: 2,
                    token: Token::Blank,
                }
            ]
        );
    }

    #[test]
    fn tokenize_ops() {
        let text = r#"[f#ABCD]
SWAP 2.=3:
+hello
DEL 5
INS.POST 1:
+world
REM
MV other.rs
SWAP.BLK 4:
+foo
DEL.BLK 6
INS.BLK.POST 7:
+bar
INS.HEAD:
+baz
INS.TAIL:
+qux
"#;
        let tokens = tokenize(text).unwrap();
        assert!(matches!(
            tokens[1].token,
            Token::Op {
                target: OpTarget::Replace { start: 2, end: 3 }
            }
        ));
        assert!(matches!(
            tokens[3].token,
            Token::Op {
                target: OpTarget::Delete { start: 5, end: 5 }
            }
        ));
        assert!(matches!(
            tokens[4].token,
            Token::Op {
                target: OpTarget::InsertAfter { .. }
            }
        ));
        assert!(matches!(
            tokens[6].token,
            Token::Op {
                target: OpTarget::Rem
            }
        ));
        assert!(matches!(
            tokens[7].token,
            Token::Op {
                target: OpTarget::Move { .. }
            }
        ));
    }

    #[test]
    fn payload_preserves_indentation() {
        let tokens = tokenize("+    indented\n").unwrap();
        assert_eq!(
            tokens[0].token,
            Token::PayloadLiteral {
                text: "    indented".to_string()
            }
        );
    }

    #[test]
    fn rejects_invalid_range() {
        let err = tokenize("[f#ABCD]\nSWAP 5.=2:\n").unwrap_err();
        assert!(err.to_string().contains("ends before it starts"));
    }
}
