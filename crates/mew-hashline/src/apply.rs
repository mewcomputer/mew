//! Apply a parsed list of concrete edits to a text body.

use crate::types::{ApplyResult, Cursor, Edit, InsertMode};

/// Apply a list of concrete (non-block) edits to `text` and return the new
/// text plus the first changed line and any warnings.
///
/// Edits are applied bottom-up so line numbers stay stable. The input `text`
/// is expected to be LF-normalized and BOM-stripped.
pub fn apply_edits(text: &str, edits: &[Edit]) -> crate::Result<ApplyResult> {
    if edits.is_empty() {
        return Ok(ApplyResult {
            text: text.to_string(),
            first_changed_line: None,
            warnings: Vec::new(),
            block_resolutions: Vec::new(),
        });
    }

    for edit in edits {
        if matches!(edit, Edit::Block { .. }) {
            return Err(crate::HashlineError::execution(
                "unresolved block edit reached apply_edits",
            ));
        }
    }

    let mut file_lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    let phantom_line = trailing_phantom_line(&file_lines);

    validate_line_bounds(edits, &file_lines, phantom_line)?;

    let mut first_changed_line: Option<usize> = None;
    let mut track_change = |line: usize| {
        if first_changed_line.is_none_or(|f| line < f) {
            first_changed_line = Some(line);
        }
    };

    // Collect lines to insert at BOF and EOF separately.
    let mut bof_lines: Vec<String> = Vec::new();
    let mut eof_lines: Vec<String> = Vec::new();
    let mut anchor_edits: Vec<IndexedEdit> = Vec::new();

    for (index, edit) in edits.iter().enumerate() {
        match edit {
            Edit::Insert {
                cursor: Cursor::Bof,
                text,
                ..
            } => {
                bof_lines.push(text.clone());
            }
            Edit::Insert {
                cursor: Cursor::Eof,
                text,
                ..
            } => {
                eof_lines.push(text.clone());
            }
            Edit::Insert {
                cursor,
                mode,
                block_start,
                ..
            } => {
                anchor_edits.push(IndexedEdit {
                    edit: edit.clone(),
                    index,
                });
                let _ = (cursor, mode, block_start);
            }
            Edit::Delete { .. } => {
                anchor_edits.push(IndexedEdit {
                    edit: edit.clone(),
                    index,
                });
            }
            Edit::Block { .. } => unreachable!(),
        }
    }

    // Bucket by anchor line and apply bottom-up.
    let buckets = bucket_anchor_edits(anchor_edits);
    let mut line_keys: Vec<usize> = buckets.keys().copied().collect();
    line_keys.sort_unstable_by(|a, b| b.cmp(a));

    for line in line_keys {
        let Some(bucket) = buckets.get(&line) else {
            continue;
        };
        apply_line_bucket(line, bucket, &mut file_lines, &mut track_change)?;
    }

    if !bof_lines.is_empty() {
        insert_at_start(&mut file_lines, &bof_lines);
        track_change(1);
    }
    if let Some(line) = insert_at_end(&mut file_lines, &eof_lines) {
        track_change(line);
    }

    Ok(ApplyResult {
        text: file_lines.join("\n"),
        first_changed_line,
        warnings: Vec::new(),
        block_resolutions: Vec::new(),
    })
}

#[derive(Clone)]
struct IndexedEdit {
    edit: Edit,
    index: usize,
}

fn trailing_phantom_line(file_lines: &[String]) -> Option<usize> {
    // `split('\n')` on a newline-terminated file yields a trailing empty
    // sentinel. It is addressable for inserts (append-past-end), but it is
    // not real content. Deleting it only strips the file's final newline.
    if file_lines.len() > 1 && file_lines.last().is_some_and(|l| l.is_empty()) {
        Some(file_lines.len())
    } else {
        None
    }
}

fn validate_line_bounds(
    edits: &[Edit],
    file_lines: &[String],
    phantom_line: Option<usize>,
) -> crate::Result<()> {
    let len = file_lines.len();
    for edit in edits {
        match edit {
            Edit::Delete { anchor } => {
                if anchor.line == 0 || anchor.line > len {
                    return Err(crate::HashlineError::LineOutOfBounds {
                        line: anchor.line,
                        file_lines: len,
                    });
                }
                if Some(anchor.line) == phantom_line {
                    return Err(crate::HashlineError::LineOutOfBounds {
                        line: anchor.line,
                        file_lines: len - 1,
                    });
                }
            }
            Edit::Insert { cursor, .. } => match cursor {
                Cursor::BeforeAnchor { anchor } | Cursor::AfterAnchor { anchor } => {
                    if anchor.line == 0 || anchor.line > len {
                        return Err(crate::HashlineError::LineOutOfBounds {
                            line: anchor.line,
                            file_lines: len,
                        });
                    }
                }
                _ => {}
            },
            Edit::Block { .. } => {}
        }
    }
    Ok(())
}

fn bucket_anchor_edits(
    edits: Vec<IndexedEdit>,
) -> std::collections::HashMap<usize, Vec<IndexedEdit>> {
    let mut map: std::collections::HashMap<usize, Vec<IndexedEdit>> =
        std::collections::HashMap::new();
    for item in edits {
        let line = match &item.edit {
            Edit::Delete { anchor } => anchor.line,
            Edit::Insert { cursor, .. } => match cursor {
                Cursor::BeforeAnchor { anchor } | Cursor::AfterAnchor { anchor } => anchor.line,
                _ => continue,
            },
            Edit::Block { .. } => continue,
        };
        map.entry(line).or_default().push(item);
    }
    map
}

fn apply_line_bucket(
    line: usize,
    bucket: &[IndexedEdit],
    file_lines: &mut Vec<String>,
    track_change: &mut dyn FnMut(usize),
) -> crate::Result<()> {
    let mut sorted = bucket.to_vec();
    sorted.sort_by_key(|item| item.index);

    let mut before_inserts: Vec<String> = Vec::new();
    let mut replacement_inserts: Vec<String> = Vec::new();
    let mut after_inserts: Vec<String> = Vec::new();
    let mut delete = false;

    for item in sorted {
        match &item.edit {
            Edit::Insert {
                cursor, text, mode, ..
            } => match cursor {
                Cursor::BeforeAnchor { .. } => {
                    if *mode == InsertMode::Replacement {
                        replacement_inserts.push(text.clone());
                    } else {
                        before_inserts.push(text.clone());
                    }
                }
                Cursor::AfterAnchor { .. } => {
                    after_inserts.push(text.clone());
                }
                _ => {}
            },
            Edit::Delete { .. } => {
                delete = true;
            }
            _ => {}
        }
    }

    if before_inserts.is_empty()
        && replacement_inserts.is_empty()
        && after_inserts.is_empty()
        && !delete
    {
        return Ok(());
    }

    let idx = line - 1;
    let current = file_lines.get(idx).cloned().unwrap_or_default();

    let replacement: Vec<String> = if delete {
        before_inserts
            .into_iter()
            .chain(replacement_inserts)
            .chain(after_inserts)
            .collect()
    } else {
        before_inserts
            .into_iter()
            .chain(replacement_inserts)
            .chain(std::iter::once(current))
            .chain(after_inserts)
            .collect()
    };

    file_lines.splice(idx..idx + 1, replacement);
    track_change(line);
    Ok(())
}

fn insert_at_start(file_lines: &mut Vec<String>, lines: &[String]) {
    if file_lines.len() == 1 && file_lines[0].is_empty() {
        // Empty file represented by a single empty sentinel.
        file_lines.clear();
    }
    for line in lines.iter().rev() {
        file_lines.insert(0, line.clone());
    }
}

fn insert_at_end(file_lines: &mut Vec<String>, lines: &[String]) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let has_trailing_newline =
        file_lines.len() > 1 && file_lines.last().is_some_and(|l| l.is_empty());
    let insert_index = if has_trailing_newline {
        file_lines.len() - 1
    } else {
        file_lines.len()
    };
    for line in lines.iter().rev() {
        file_lines.insert(insert_index, line.clone());
    }
    Some(insert_index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Anchor, InsertMode};

    fn insert_before(line: usize, text: &str) -> Edit {
        Edit::Insert {
            cursor: Cursor::BeforeAnchor {
                anchor: Anchor { line },
            },
            text: text.to_string(),
            mode: InsertMode::Normal,
            block_start: None,
        }
    }

    fn replace_line(line: usize, text: &str) -> Edit {
        Edit::Insert {
            cursor: Cursor::BeforeAnchor {
                anchor: Anchor { line },
            },
            text: text.to_string(),
            mode: InsertMode::Replacement,
            block_start: None,
        }
    }

    fn delete(line: usize) -> Edit {
        Edit::Delete {
            anchor: Anchor { line },
        }
    }

    #[test]
    fn replace_one_line() {
        let edits = vec![replace_line(2, "B2"), delete(2)];
        let result = apply_edits("A\nB\nC\n", &edits).unwrap();
        assert_eq!(result.text, "A\nB2\nC\n");
        assert_eq!(result.first_changed_line, Some(2));
    }

    #[test]
    fn insert_after_preserves_following_line() {
        let edits = vec![insert_before(2, "B1")];
        let result = apply_edits("A\nB\nC\n", &edits).unwrap();
        assert_eq!(result.text, "A\nB1\nB\nC\n");
    }

    #[test]
    fn delete_line() {
        let edits = vec![delete(2)];
        let result = apply_edits("A\nB\nC\n", &edits).unwrap();
        assert_eq!(result.text, "A\nC\n");
    }

    #[test]
    fn insert_at_eof_keeps_trailing_newline() {
        let edits = vec![Edit::Insert {
            cursor: Cursor::Eof,
            text: "D".to_string(),
            mode: InsertMode::Normal,
            block_start: None,
        }];
        let result = apply_edits("A\nB\nC\n", &edits).unwrap();
        assert_eq!(result.text, "A\nB\nC\nD\n");
    }

    #[test]
    fn out_of_bounds_rejected() {
        let edits = vec![delete(99)];
        assert!(apply_edits("A\nB\n", &edits).is_err());
    }
}
