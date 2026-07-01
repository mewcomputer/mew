//! Recovery from a stale hashline tag.
//!
//! When the live file has drifted from the snapshot the section tag names, we
//! try to replay the would-be edit safely rather than failing immediately.
//!
//! The recovery uses a **3-way merge** approach:
//!
//! 1. If `current` exactly matches `previous`, apply the edits directly.
//! 2. Otherwise, compute a line-level diff between `previous` (snapshot) and
//!    `current` (live file) using the `similar` crate.
//! 3. Build a line-mapping that says, for each line number in `previous`,
//!    where that line now lives in `current` (or that it was deleted).
//! 4. Remap every edit anchor from `previous` line numbers to `current` line
//!    numbers.
//! 5. Apply the remapped edits onto `current`.
//!
//! This is more robust than the previous approach (which only worked when the
//! file had the same number of lines and anchor content was unchanged). It
//! handles insertions, deletions, and modifications anywhere in the file.

use crate::apply::apply_edits;
use crate::types::{Cursor, Edit};
use similar::{Algorithm, TextDiff};

pub struct RecoveryResult {
    pub text: String,
    pub first_changed_line: Option<usize>,
    pub warnings: Vec<String>,
}

/// Try to recover from a stale tag.
///
/// 1. If `current` exactly matches `previous`, apply the edits directly.
/// 2. Otherwise, compute a line-level diff, build a line-mapping, remap
///    anchors, and apply.
pub fn try_recover(current: &str, previous: &str, edits: &[Edit]) -> Option<RecoveryResult> {
    if current == previous {
        let result = apply_edits(current, edits).ok()?;
        return Some(RecoveryResult {
            text: result.text,
            first_changed_line: result.first_changed_line,
            warnings: result.warnings,
        });
    }

    let mapping = build_line_mapping(previous, current);
    let remapped = remap_edits(edits, &mapping)?;

    // If remapping produced no effective changes (all anchors mapped to
    // deleted regions), bail out.
    if remapped.edits.is_empty() && !edits.is_empty() {
        return None;
    }

    let result = apply_edits(current, &remapped.edits).ok()?;
    if result.text == current {
        return None;
    }

    let mut warnings = result.warnings;
    let mut notes = vec![format!(
        "Recovered from a stale tag by 3-way merge. {} anchors remapped, {} unmapped ({}), {} lines shifted.",
        remapped.remapped_count,
        remapped.unmapped_count,
        if remapped.unmapped_count > 0 {
            "applied at original position"
        } else {
            "none"
        },
        mapping.shifted_count(),
    )];
    if remapped.unmapped_count > 0 {
        notes.push(format!(
            "{} edit anchor(s) could not be located in the current file and were applied at their original line numbers. Verify the result.",
            remapped.unmapped_count
        ));
    }
    warnings.extend(notes);

    Some(RecoveryResult {
        text: result.text,
        first_changed_line: result.first_changed_line,
        warnings,
    })
}

// ── Line mapping ──────────────────────────────────────────────────────────

/// A mapping from `previous` line numbers (1-indexed) to `current` line
/// numbers (1-indexed). Lines that were deleted map to `None`.
struct LineMapping {
    /// Index by (previous_line - 1). Value is Some(current_line) or None.
    map: Vec<Option<usize>>,
    prev_len: usize,
}

impl LineMapping {
    /// Look up where a `previous` line lives in `current`.
    fn lookup(&self, prev_line: usize) -> LineTarget {
        if prev_line == 0 {
            return LineTarget::Bof;
        }
        let idx = prev_line.checked_sub(1);
        match idx {
            Some(i) if i < self.map.len() => match self.map[i] {
                Some(curr) => LineTarget::Found(curr),
                None => LineTarget::Deleted,
            },
            // Beyond the end of the previous file: treat as EOF.
            _ => LineTarget::Eof,
        }
    }

    /// Count how many lines shifted position (not at the same line number and
    /// not deleted).
    fn shifted_count(&self) -> usize {
        self.map
            .iter()
            .enumerate()
            .filter(|(i, opt)| {
                opt.is_some_and(|curr| curr != i + 1)
            })
            .count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineTarget {
    Found(usize),
    Deleted,
    Bof,
    Eof,
}

/// Build a line-level mapping between two versions of a file.
///
/// Uses `similar`'s LCS-based diff to match lines. Equal (matched) lines get
/// a mapping from their old line number to their new line number. Deleted
/// lines map to `None`. Inserted lines don't appear in the mapping (they only
/// shift subsequent lines).
fn build_line_mapping(previous: &str, current: &str) -> LineMapping {
    let prev_lines: Vec<&str> = previous.split('\n').collect();

    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_lines(previous, current);
    let ops = diff.ops();

    let mut map: Vec<Option<usize>> = vec![None; prev_lines.len()];

    for op in ops {
        // Only Equal ops give us a reliable old→new line mapping. Replace
        // ops mean the line content changed, so we must NOT map them.
        let (old_start, old_end, new_start, new_end) = match op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => (*old_index, old_index + len, *new_index, new_index + len),
            _ => continue,
        };

        for (i, j) in (old_start..old_end).zip(new_start..new_end) {
            if i < map.len() {
                map[i] = Some(j + 1); // 1-indexed
            }
        }
    }

    LineMapping {
        map,
        prev_len: prev_lines.len(),
    }
}

// ── Edit remapping ────────────────────────────────────────────────────────

struct RemappedEdits {
    edits: Vec<Edit>,
    remapped_count: usize,
    unmapped_count: usize,
}

/// Remap all anchor line numbers in `edits` from `previous` coordinates to
/// `current` coordinates using the line mapping.
///
/// For inserts at Bof/Eof, the cursor stays as-is. For inserts anchored on a
/// specific line, we remap the anchor. For deletes, we remap the anchor. If
/// an anchor maps to `Deleted`, we try to find the nearest surviving line
/// (look forward, then backward) so the edit still lands near where the model
/// intended.
fn remap_edits(edits: &[Edit], mapping: &LineMapping) -> Option<RemappedEdits> {
    let mut out = Vec::with_capacity(edits.len());
    let mut remapped = 0;
    let mut unmapped = 0;

    for edit in edits {
        match edit {
            Edit::Insert {
                cursor,
                text,
                mode,
                block_start,
            } => {
                let new_cursor = remap_cursor(cursor, mapping, &mut remapped, &mut unmapped);
                out.push(Edit::Insert {
                    cursor: new_cursor,
                    text: text.clone(),
                    mode: *mode,
                    block_start: *block_start,
                });
            }
            Edit::Delete { anchor } => {
                match mapping.lookup(anchor.line) {
                    LineTarget::Found(curr) => {
                        remapped += 1;
                        out.push(Edit::Delete {
                            anchor: crate::types::Anchor { line: curr },
                        });
                    }
                    LineTarget::Deleted => {
                        // The line was deleted from the current file. Skip
                        // this delete — it's already gone.
                        unmapped += 1;
                    }
                    LineTarget::Eof => {
                        // Can't delete past EOF; skip.
                        unmapped += 1;
                    }
                    LineTarget::Bof => {
                        unmapped += 1;
                    }
                }
            }
            Edit::Block { anchor, payloads, mode } => {
                match mapping.lookup(anchor.line) {
                    LineTarget::Found(curr) => {
                        remapped += 1;
                        out.push(Edit::Block {
                            anchor: crate::types::Anchor { line: curr },
                            payloads: payloads.clone(),
                            mode: *mode,
                        });
                    }
                    _ => {
                        // Block anchor unmapped: keep original line as
                        // fallback (the block resolver may still find it).
                        unmapped += 1;
                        out.push(edit.clone());
                    }
                }
            }
        }
    }

    Some(RemappedEdits {
        edits: out,
        remapped_count: remapped,
        unmapped_count: unmapped,
    })
}

fn remap_cursor(
    cursor: &Cursor,
    mapping: &LineMapping,
    remapped: &mut usize,
    unmapped: &mut usize,
) -> Cursor {
    match cursor {
        Cursor::BeforeAnchor { anchor } | Cursor::AfterAnchor { anchor } => {
            match mapping.lookup(anchor.line) {
                LineTarget::Found(curr) => {
                    *remapped += 1;
                    let new_anchor = crate::types::Anchor { line: curr };
                    match cursor {
                        Cursor::BeforeAnchor { .. } => Cursor::BeforeAnchor {
                            anchor: new_anchor,
                        },
                        Cursor::AfterAnchor { .. } => Cursor::AfterAnchor {
                            anchor: new_anchor,
                        },
                        _ => unreachable!(),
                    }
                }
                LineTarget::Deleted => {
                    // Anchor line was deleted. Try to find the nearest
                    // surviving line so the edit lands nearby.
                    if let Some(fallback) = find_nearest_surviving(anchor.line, mapping) {
                        *remapped += 1;
                        let new_anchor = crate::types::Anchor { line: fallback };
                        match cursor {
                            Cursor::BeforeAnchor { .. } => Cursor::BeforeAnchor {
                                anchor: new_anchor,
                            },
                            Cursor::AfterAnchor { .. } => Cursor::AfterAnchor {
                                anchor: new_anchor,
                            },
                            _ => unreachable!(),
                        }
                    } else {
                        *unmapped += 1;
                        *cursor
                    }
                }
                LineTarget::Eof => {
                    // Anchor was past the end of the file; insert at EOF.
                    *remapped += 1;
                    Cursor::Eof
                }
                LineTarget::Bof => {
                    *remapped += 1;
                    Cursor::Bof
                }
            }
        }
        Cursor::Bof | Cursor::Eof => *cursor,
    }
}

/// Find the nearest line in `current` that still exists, starting from
/// `prev_line` and looking both forward and backward.
fn find_nearest_surviving(prev_line: usize, mapping: &LineMapping) -> Option<usize> {
    // Search outward from prev_line.
    for distance in 1..mapping.map.len() {
        // Try forward first.
        let forward = prev_line.saturating_add(distance);
        if forward <= mapping.prev_len {
            if let LineTarget::Found(curr) = mapping.lookup(forward) {
                return Some(curr);
            }
        }
        // Then backward.
        let backward = prev_line.saturating_sub(distance);
        if backward >= 1 {
            if let LineTarget::Found(curr) = mapping.lookup(backward) {
                return Some(curr);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Anchor, Cursor, Edit, InsertMode};

    fn delete(line: usize) -> Edit {
        Edit::Delete {
            anchor: Anchor { line },
        }
    }

    fn insert_after(line: usize, text: &str) -> Edit {
        Edit::Insert {
            cursor: Cursor::AfterAnchor {
                anchor: Anchor { line },
            },
            text: text.to_string(),
            mode: InsertMode::Normal,
            block_start: None,
        }
    }

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

    // ── Original recovery tests (now using 3-way merge) ──────────────────

    #[test]
    fn exact_match_recovery() {
        let edits = vec![insert_after(1, "b")];
        let r = try_recover("a\n", "a\n", &edits).unwrap();
        assert_eq!(r.text, "a\nb\n");
    }

    #[test]
    fn replay_when_anchors_unchanged() {
        let previous = "a\nb\nc\n";
        let current = "x\nb\nc\n";
        let edits = vec![insert_after(2, "b2")];
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "x\nb\nb2\nc\n");
        assert!(r.warnings.iter().any(|w| w.contains("Recovered")));
    }

    #[test]
    fn no_recovery_when_anchor_changed() {
        let previous = "a\nb\nc\n";
        let current = "a\nB\nc\n";
        let edits = vec![delete(2)];
        // Line 2 changed from "b" to "B" — it's not a match. The mapping
        // marks it as Deleted (no equal match). The delete is skipped, so
        // the text doesn't change, and recovery returns None.
        assert!(try_recover(current, previous, &edits).is_none());
    }

    // ── 3-way merge: insertion drift ──────────────────────────────────────

    #[test]
    fn insert_before_drifted_anchor() {
        // Previous: line 3 was "c". Now it's line 4 because a line was
        // inserted before it.
        let previous = "a\nb\nc\nd\n";
        let current = "a\nb\nNEW\nc\nd\n";
        let edits = vec![insert_before(3, "// before c")];
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "a\nb\nNEW\n// before c\nc\nd\n");
    }

    #[test]
    fn insert_after_drifted_anchor() {
        let previous = "a\nb\nc\nd\n";
        let current = "NEW\na\nb\nc\nd\n";
        let edits = vec![insert_after(1, "a2")];
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "NEW\na\na2\nb\nc\nd\n");
    }

    #[test]
    fn delete_drifted_anchor() {
        let previous = "a\nb\nc\nd\n";
        let current = "x\na\nb\nc\nd\n";
        let edits = vec![delete(3)]; // delete "c" in previous
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "x\na\nb\nd\n");
    }

    // ── 3-way merge: deletion drift ────────────────────────────────────────

    #[test]
    fn anchor_survives_line_deletion_above() {
        let previous = "a\nb\nc\nd\n";
        let current = "a\nc\nd\n"; // "b" was deleted
        let edits = vec![insert_after(3, "d2")]; // after "c" in previous
        let r = try_recover(current, previous, &edits).unwrap();
        // "c" moved from line 3 to line 2; the insert follows it.
        assert_eq!(r.text, "a\nc\nd2\nd\n");
    }

    #[test]
    fn delete_anchor_line_itself_deleted() {
        // If the line we wanted to delete is already gone, the delete is
        // a no-op and recovery produces no change → returns None.
        let previous = "a\nb\nc\n";
        let current = "a\nc\n"; // "b" was deleted
        let edits = vec![delete(2)]; // delete "b"
        assert!(try_recover(current, previous, &edits).is_none());
    }

    // ── 3-way merge: replacement ──────────────────────────────────────────

    #[test]
    fn replace_line_after_drift() {
        let previous = "a\nb\nc\nd\n";
        let current = "x\na\nb\nc\nd\n";
        let edits = vec![replace_line(3, "C"), delete(3)];
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "x\na\nb\nC\nd\n");
    }

    // ── 3-way merge: multi-edit ───────────────────────────────────────────

    #[test]
    fn multiple_edits_with_drift() {
        let previous = "line1\nline2\nline3\nline4\nline5\n";
        let current = "line0\nline1\nline2\nline3\nline4\nline5\n";
        let edits = vec![
            replace_line(2, "TWO"), // replace line2
            delete(2),
            insert_after(4, "AFTER_FOUR"),
        ];
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "line0\nline1\nTWO\nline3\nline4\nAFTER_FOUR\nline5\n");
    }

    // ── 3-way merge: edge cases ───────────────────────────────────────────

    #[test]
    fn eof_insert_survives_drift() {
        let previous = "a\nb\n";
        let current = "x\na\nb\n";
        let edits = vec![Edit::Insert {
            cursor: Cursor::Eof,
            text: "c".to_string(),
            mode: InsertMode::Normal,
            block_start: None,
        }];
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "x\na\nb\nc\n");
    }

    #[test]
    fn bof_insert_survives_drift() {
        let previous = "a\nb\n";
        let current = "a\nb\nx\n";
        let edits = vec![Edit::Insert {
            cursor: Cursor::Bof,
            text: "Z".to_string(),
            mode: InsertMode::Normal,
            block_start: None,
        }];
        let r = try_recover(current, previous, &edits).unwrap();
        assert_eq!(r.text, "Z\na\nb\nx\n");
    }

    #[test]
    fn no_recovery_when_all_anchors_deleted() {
        // All lines changed → no anchors survive → edits produce no changes.
        let previous = "a\nb\nc\n";
        let current = "x\ny\nz\n";
        let edits = vec![delete(1), delete(2), delete(3)];
        assert!(try_recover(current, previous, &edits).is_none());
    }

    #[test]
    fn recovery_includes_anchor_stats() {
        let previous = "a\nb\nc\n";
        let current = "x\nb\nc\n";
        let edits = vec![insert_after(2, "b2")];
        let r = try_recover(current, previous, &edits).unwrap();
        // Should mention remapped anchors.
        assert!(r.warnings.iter().any(|w| w.contains("anchors remapped")));
    }

    // ── Line mapping unit tests ──────────────────────────────────────────

    #[test]
    fn mapping_no_changes() {
        let m = build_line_mapping("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(m.lookup(1), LineTarget::Found(1));
        assert_eq!(m.lookup(2), LineTarget::Found(2));
        assert_eq!(m.lookup(3), LineTarget::Found(3));
    }

    #[test]
    fn mapping_line_inserted_above() {
        let m = build_line_mapping("a\nb\nc\n", "NEW\na\nb\nc\n");
        assert_eq!(m.lookup(1), LineTarget::Found(2)); // "a" moved from 1→2
        assert_eq!(m.lookup(2), LineTarget::Found(3)); // "b" moved from 2→3
        assert_eq!(m.lookup(3), LineTarget::Found(4)); // "c" moved from 3→4
    }

    #[test]
    fn mapping_line_deleted() {
        let m = build_line_mapping("a\nb\nc\n", "a\nc\n");
        assert_eq!(m.lookup(1), LineTarget::Found(1));
        assert_eq!(m.lookup(2), LineTarget::Deleted); // "b" was deleted
        assert_eq!(m.lookup(3), LineTarget::Found(2)); // "c" moved from 3→2
    }

    #[test]
    fn mapping_all_changed() {
        let m = build_line_mapping("a\nb\n", "x\ny\n");
        assert_eq!(m.lookup(1), LineTarget::Deleted);
        assert_eq!(m.lookup(2), LineTarget::Deleted);
    }
}
