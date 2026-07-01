//! Executor that turns classified hashline tokens into a flat list of edits.

use crate::tokenizer::{LineToken, OpTarget, Token};
use crate::types::{Anchor, Cursor, Edit, FileOp, InsertMode};

/// Parse the body of one hashline section (everything after the header) into
/// edits, an optional file-level op, and any warnings.
pub fn parse_section(
    tokens: &[LineToken],
) -> crate::Result<(Vec<Edit>, Option<FileOp>, Vec<String>)> {
    let mut executor = Executor::new();
    for token in tokens {
        executor.feed(token)?;
    }
    Ok(executor.finish())
}

struct Executor {
    edits: Vec<Edit>,
    warnings: Vec<String>,
    pending: Option<Pending>,
    file_op: Option<FileOp>,
}

struct Pending {
    target: OpTarget,
    payloads: Vec<String>,
    deferred_blanks: Vec<String>,
}

impl Executor {
    fn new() -> Self {
        Self {
            edits: Vec::new(),
            warnings: Vec::new(),
            pending: None,
            file_op: None,
        }
    }

    fn feed(&mut self, token: &LineToken) -> crate::Result<()> {
        match &token.token {
            Token::Blank => self.handle_blank(token.line),
            Token::Header { .. } => {
                // Sections are split before calling parse_section; a stray
                // header inside a body is a parse error.
                return Err(crate::HashlineError::parse(
                    token.line,
                    "unexpected file header inside section body",
                ));
            }
            Token::Op { target } => {
                self.flush_pending()?;
                self.start_op(target.clone(), token.line)?;
            }
            Token::PayloadLiteral { text } => {
                self.commit_deferred_blanks();
                self.add_payload(text.clone());
            }
            Token::Raw { text } => {
                self.handle_raw(text, token.line)?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> (Vec<Edit>, Option<FileOp>, Vec<String>) {
        // A trailing delete op with no body is valid; flush it.
        let _ = self.flush_pending();
        (self.edits, self.file_op, self.warnings)
    }

    fn start_op(&mut self, target: OpTarget, _line: usize) -> crate::Result<()> {
        match target {
            OpTarget::Rem => {
                if self.file_op.is_some() {
                    return Err(crate::HashlineError::parse(
                        _line,
                        "only one file-level op (REM or MV) allowed per section",
                    ));
                }
                if !self.edits.is_empty() {
                    return Err(crate::HashlineError::parse(
                        _line,
                        "REM cannot be combined with line ops",
                    ));
                }
                self.file_op = Some(FileOp::Rem);
            }
            OpTarget::Move { dest } => {
                if self.file_op.is_some() {
                    return Err(crate::HashlineError::parse(
                        _line,
                        "only one file-level op (REM or MV) allowed per section",
                    ));
                }
                self.file_op = Some(FileOp::Move { dest });
            }
            _ => {
                if self.file_op.is_some() {
                    return Err(crate::HashlineError::parse(
                        _line,
                        "line ops cannot follow REM or MV",
                    ));
                }
                self.pending = Some(Pending {
                    target,
                    payloads: Vec::new(),
                    deferred_blanks: Vec::new(),
                });
            }
        }
        Ok(())
    }

    fn flush_pending(&mut self) -> crate::Result<()> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };

        match pending.target {
            OpTarget::Replace { start, end } => {
                for text in pending.payloads {
                    self.edits.push(Edit::Insert {
                        cursor: Cursor::BeforeAnchor {
                            anchor: Anchor { line: start },
                        },
                        text,
                        mode: InsertMode::Replacement,
                        block_start: None,
                    });
                }
                for line in start..=end {
                    self.edits.push(Edit::Delete {
                        anchor: Anchor { line },
                    });
                }
            }
            OpTarget::ReplaceBlock { anchor } => {
                self.edits.push(Edit::Block {
                    anchor,
                    payloads: pending.payloads,
                    mode: crate::types::BlockMode::Replace,
                });
            }
            OpTarget::Delete { start, end } => {
                for line in start..=end {
                    self.edits.push(Edit::Delete {
                        anchor: Anchor { line },
                    });
                }
            }
            OpTarget::DeleteBlock { anchor } => {
                self.edits.push(Edit::Block {
                    anchor,
                    payloads: Vec::new(),
                    mode: crate::types::BlockMode::Delete,
                });
            }
            OpTarget::InsertBefore { anchor } => {
                if pending.payloads.is_empty() {
                    return Err(crate::HashlineError::parse(0, "empty INSERT body"));
                }
                for text in pending.payloads {
                    self.edits.push(Edit::Insert {
                        cursor: Cursor::BeforeAnchor { anchor },
                        text,
                        mode: InsertMode::Normal,
                        block_start: None,
                    });
                }
            }
            OpTarget::InsertAfter { anchor } => {
                if pending.payloads.is_empty() {
                    return Err(crate::HashlineError::parse(0, "empty INSERT body"));
                }
                for text in pending.payloads {
                    self.edits.push(Edit::Insert {
                        cursor: Cursor::AfterAnchor { anchor },
                        text,
                        mode: InsertMode::Normal,
                        block_start: None,
                    });
                }
            }
            OpTarget::InsertAfterBlock { anchor } => {
                self.edits.push(Edit::Block {
                    anchor,
                    payloads: pending.payloads,
                    mode: crate::types::BlockMode::InsertAfter,
                });
            }
            OpTarget::InsertHead => {
                if pending.payloads.is_empty() {
                    return Err(crate::HashlineError::parse(0, "empty INSERT body"));
                }
                for text in pending.payloads {
                    self.edits.push(Edit::Insert {
                        cursor: Cursor::Bof,
                        text,
                        mode: InsertMode::Normal,
                        block_start: None,
                    });
                }
            }
            OpTarget::InsertTail => {
                if pending.payloads.is_empty() {
                    return Err(crate::HashlineError::parse(0, "empty INSERT body"));
                }
                for text in pending.payloads {
                    self.edits.push(Edit::Insert {
                        cursor: Cursor::Eof,
                        text,
                        mode: InsertMode::Normal,
                        block_start: None,
                    });
                }
            }
            OpTarget::Rem | OpTarget::Move { .. } => unreachable!(),
        }

        Ok(())
    }

    fn add_payload(&mut self, text: String) {
        if let Some(pending) = self.pending.as_mut() {
            pending.payloads.push(text);
        }
    }

    fn handle_blank(&mut self, _line: usize) {
        if let Some(pending) = self.pending.as_mut() {
            if !pending.payloads.is_empty() {
                pending.deferred_blanks.push(String::new());
            }
        }
    }

    fn commit_deferred_blanks(&mut self) {
        if let Some(pending) = self.pending.as_mut() {
            if !pending.deferred_blanks.is_empty() {
                pending
                    .payloads
                    .extend(std::mem::take(&mut pending.deferred_blanks));
            }
        }
    }

    fn handle_raw(&mut self, text: &str, line: usize) -> crate::Result<()> {
        // Detect common contamination.
        let trimmed = text.trim_start();
        if trimmed.starts_with("@@") {
            return Err(crate::HashlineError::parse(
                line,
                "unified-diff hunk headers are not valid in hashline",
            ));
        }
        if trimmed.starts_with("*** Update File:")
            || trimmed.starts_with("*** Add File:")
            || trimmed.starts_with("*** Delete File:")
            || trimmed.starts_with("*** Move to:")
        {
            return Err(crate::HashlineError::parse(
                line,
                "apply_patch sentinels are not valid in hashline",
            ));
        }
        if let Some(pending) = self.pending.as_mut() {
            if pending.target.is_delete() {
                return Err(crate::HashlineError::parse(
                    line,
                    "DELETE takes no body rows",
                ));
            }
            if trimmed.is_empty() {
                // Already handled as blank; raw with only whitespace is also blank.
                return Ok(());
            }
            if trimmed.starts_with('-') {
                return Err(crate::HashlineError::parse(
                    line,
                    "`-old` rows are not valid; the range does the deleting",
                ));
            }
            // Bare body row: treat as literal content. Auto-strip line-number
            // prefixes (e.g. "12:content") that models paste from read output.
            self.commit_deferred_blanks();
            let payload = if let Some(stripped) = strip_line_number_prefix(trimmed) {
                self.warnings
                    .push("auto-stripped line-number prefix from bare body row".to_string());
                stripped.to_string()
            } else {
                text.to_string()
            };
            self.add_payload(payload);
        } else {
            return Err(crate::HashlineError::parse(
                line,
                format!("payload line has no preceding hunk header: {text}"),
            ));
        }
        Ok(())
    }
}

/// Check if a line starts with a line-number prefix like `123:` or `123 :`.
/// Returns the content after the prefix (leading whitespace preserved).
///
/// This handles the most common copy-paste contamination from read output,
/// where the model accidentally includes the `N:` prefix on body rows.
/// Conservative: only strips when the prefix is a bare number followed by a
/// colon, with no other sigil (`+`, `-`, `@@`, etc.) before it.
fn strip_line_number_prefix(line: &str) -> Option<&str> {
    let mut chars = line.chars();
    let first = chars.next()?;
    if !first.is_ascii_digit() {
        return None;
    }
    // Consume remaining digits.
    let rest_after_digits: &str = line[first.len_utf8()..]
        .trim_start_matches(|c: char| c.is_ascii_digit());
    // Optional space before the colon.
    let rest_after_space = rest_after_digits.trim_start();
    if !rest_after_space.starts_with(':') {
        return None;
    }
    let after_colon = &rest_after_space[1..];
    // Only strip if there's content after the colon (not just "123:").
    // We don't strip a leading space — the read output format is `N:content`
    // with no separator, so any leading whitespace is part of the payload.
    if after_colon.is_empty() {
        return None;
    }
    Some(after_colon)
}

impl OpTarget {
    fn is_delete(&self) -> bool {
        matches!(self, OpTarget::Delete { .. } | OpTarget::DeleteBlock { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::tokenize;
    use crate::types::Cursor;

    fn parse_body(text: &str) -> crate::Result<(Vec<Edit>, Option<FileOp>, Vec<String>)> {
        let tokens = tokenize(text)?;
        // Skip leading header if present; tests that include it will remove it.
        parse_section(&tokens)
    }

    #[test]
    fn replace_with_payload() {
        let (edits, _, _) = parse_body("SWAP 2.=3:\n+a\n+b").unwrap();
        // 2 payload inserts + 2 deletes for lines 2 and 3.
        assert_eq!(edits.len(), 4);
        assert!(matches!(
            &edits[0],
            Edit::Insert {
                cursor: Cursor::BeforeAnchor { anchor, .. },
                mode: InsertMode::Replacement,
                text,
                ..
            } if anchor.line == 2 && text == "a"
        ));
        assert!(matches!(&edits[2], Edit::Delete { anchor } if anchor.line == 2));
    }

    #[test]
    fn delete_block_no_body() {
        let (edits, _, _) = parse_body("DEL 5").unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(&edits[0], Edit::Delete { anchor } if anchor.line == 5));
    }

    #[test]
    fn insert_head() {
        let (edits, _, _) = parse_body("INS.HEAD:\n// header").unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(
            &edits[0],
            Edit::Insert { cursor: Cursor::Bof, text, mode: InsertMode::Normal, .. } if text == "// header"
        ));
    }

    #[test]
    fn rejects_minus_row() {
        let err = parse_body("SWAP 1.=1:\n-old\n+new").unwrap_err();
        assert!(err.to_string().contains("`-old` rows are not valid"));
    }

    #[test]
    fn rem_no_body() {
        let (_, op, _) = parse_body("REM").unwrap();
        assert!(matches!(op, Some(FileOp::Rem)));
    }

    #[test]
    fn move_op() {
        let (_, op, _) = parse_body("MV other.rs").unwrap();
        assert!(matches!(op, Some(FileOp::Move { dest }) if dest == "other.rs"));
    }

    #[test]
    fn bare_row_strips_line_number_prefix() {
        let (edits, _, warnings) = parse_body("INS.POST 1:\n12:hello world").unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(
            &edits[0],
            Edit::Insert { text, .. } if text == "hello world"
        ));
        assert!(warnings.iter().any(|w| w.contains("auto-stripped")));
    }

    #[test]
    fn bare_row_preserves_non_prefixed_content() {
        let (edits, _, warnings) = parse_body("INS.POST 1:\nhello world").unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(
            &edits[0],
            Edit::Insert { text, .. } if text == "hello world"
        ));
        assert!(!warnings.iter().any(|w| w.contains("auto-stripped")));
    }

    #[test]
    fn bare_row_strips_prefix_with_space_after_colon() {
        let (edits, _, _) = parse_body("INS.POST 1:\n42:    indented code").unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(
            &edits[0],
            Edit::Insert { text, .. } if text == "    indented code"
        ));
    }

    #[test]
    fn bare_row_does_not_strip_when_only_digits_and_colon() {
        // "123:" with no content after → should not be treated as a payload
        // (it would fail "payload line has no preceding hunk header" or similar).
        // Instead it should still be treated as raw text and error.
        let result = parse_body("INS.POST 1:\n123:");
        // The line "123:" has content after stripping would be empty, so we
        // don't strip. It becomes a raw payload with text "123:".
        let (edits, _, _) = result.unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(
            &edits[0],
            Edit::Insert { text, .. } if text == "123:"
        ));
    }

    #[test]
    fn bare_row_strips_multidigit_prefix() {
        let (edits, _, _) = parse_body("INS.POST 1:\n999:fn main() {}").unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(
            &edits[0],
            Edit::Insert { text, .. } if text == "fn main() {}"
        ));
    }
}
