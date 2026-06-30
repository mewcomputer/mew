//! Hashline edit format: line-anchored edits with file-hash staleness detection.
//!
//! The hashline format lets the model edit files by line number instead of
//! repeating the exact old text. A 4-hex-char content hash of the file is
//! included in the edit; if the file changed since the model last saw it,
//! the edit is rejected before any write happens.
//!
//! Format:
//! ```text
//! [path#HASH]
//! SWAP 5.=10:
//! +replacement line 1
//! +replacement line 2
//! DEL 12
//! INS.POST 15:
//! +inserted line
//! ```
//!
//! Operations:
//! - `SWAP start.=end:` — replace lines start through end with the `+` lines
//! - `DEL start` or `DEL start.=end` — delete the given line(s)
//! - `INS.PRE n:` — insert before line n
//! - `INS.POST n:` — insert after line n
//! - `INS.HEAD:` — insert at the beginning of the file
//! - `INS.TAIL:` — insert at the end of the file
//!
//! The `+` prefix marks replacement/insertion content. Lines without `+`
//! between operations are ignored (comments).
//!
//! Stale detection: the `HASH` in the header is computed from the file's
//! content (normalized for trailing whitespace and CRLF). If the current
//! file hash doesn't match, the edit is rejected with a message telling the
//! model to re-read the file.

use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Compute a 4-hex-char content hash of a file's text.
/// Trailing whitespace and CRLF are normalized so cosmetic changes don't
/// invalidate the hash.
fn compute_file_hash(text: &str) -> String {
    let normalized: String = text
        .lines()
        .map(|l| l.trim_end_matches([' ', '\t', '\r']))
        .collect::<Vec<_>>()
        .join("\n");
    let hash = std::collections::hash_map::DefaultHasher::new();
    use std::hash::Hasher;
    let mut hasher = hash;
    hasher.write(normalized.as_bytes());
    let h = hasher.finish();
    format!("{:04X}", h & 0xFFFF)
}

/// A parsed hashline patch.
struct Patch {
    file_path: String,
    file_hash: String,
    operations: Vec<HashOp>,
}

enum HashOp {
    /// Replace lines start through end (inclusive, 1-indexed).
    Swap {
        start: u32,
        end: u32,
        lines: Vec<String>,
    },
    /// Delete lines start through end (inclusive, 1-indexed).
    Delete { start: u32, end: u32 },
    /// Insert before a line (1-indexed). Bof = before line 1.
    InsertBefore { line: u32, lines: Vec<String> },
    /// Insert after a line (1-indexed). Eof = after last line.
    InsertAfter { line: u32, lines: Vec<String> },
    /// Insert at the beginning of the file.
    InsertHead { lines: Vec<String> },
    /// Insert at the end of the file.
    InsertTail { lines: Vec<String> },
}

pub struct EditHashline;

#[async_trait]
impl Tool for EditHashline {
    fn name(&self) -> &str {
        "edit_hashline"
    }

    fn description(&self) -> &str {
        "Edit a file using the hashline format: line-numbered operations with \
         file-hash staleness detection. The model provides a file hash (from the \
         last read) and line-numbered SWAP/DEL/INS operations. If the file \
         changed since the last read, the edit is rejected. This saves tokens \
         compared to repeating the old text."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Hashline patch text. Format:\n[path#HASH]\nSWAP start.=end:\n+replacement\nDEL start.=end\nINS.PRE n:\n+insertion\nINS.POST n:\nINS.HEAD:\nINS.TAIL:"
                    }
                },
                "required": ["patch"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Mutating
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let patch_text = input
            .get("patch")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing 'patch' field".into()))?;

        let patch = parse_patch(patch_text)
            .map_err(|e| ToolError::InvalidInput(format!("patch parse error: {e}")))?;

        let path = ctx.cwd.join(&patch.file_path);
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::Execution(format!("read failed: {}: {}", path.display(), e)))?;

        // Stale-file check: if the hash doesn't match, reject before writing.
        let current_hash = compute_file_hash(&content);
        if current_hash != patch.file_hash {
            return Err(ToolError::Execution(format!(
                "file hash mismatch: patch expects {} but file is {}. \
                 The file has changed since it was last read — re-read it and try again.",
                patch.file_hash, current_hash,
            )));
        }

        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let line_count = lines.len();

        // Validate all operations before applying any (atomic: all or nothing).
        for op in &patch.operations {
            match op {
                HashOp::Swap { start, end, .. } | HashOp::Delete { start, end } => {
                    if *start == 0 || (*end as usize) > line_count {
                        return Err(ToolError::Execution(format!(
                            "line range {}-{} out of bounds (file has {} lines)",
                            start, end, line_count,
                        )));
                    }
                    if start > end {
                        return Err(ToolError::Execution(format!(
                            "invalid range: start {} > end {}",
                            start, end,
                        )));
                    }
                }
                HashOp::InsertBefore { line, .. } | HashOp::InsertAfter { line, .. } => {
                    if *line as usize > line_count + 1 {
                        return Err(ToolError::Execution(format!(
                            "insert anchor line {} out of bounds (file has {} lines)",
                            line, line_count,
                        )));
                    }
                }
                _ => {}
            }
        }

        // Apply operations. We process them in order, tracking line number
        // shifts as we go.
        let mut result_lines = lines;
        let mut offset: i64 = 0; // cumulative line shift

        for op in &patch.operations {
            match op {
                HashOp::Swap {
                    start,
                    end,
                    lines: new_lines,
                } => {
                    let actual_start = (*start as i64 + offset - 1) as usize;
                    let actual_end = (*end as i64 + offset) as usize;
                    result_lines.splice(actual_start..actual_end, new_lines.clone());
                    offset += new_lines.len() as i64 - (actual_end - actual_start) as i64;
                }
                HashOp::Delete { start, end } => {
                    let actual_start = (*start as i64 + offset - 1) as usize;
                    let actual_end = (*end as i64 + offset) as usize;
                    let removed = actual_end - actual_start;
                    result_lines.drain(actual_start..actual_end);
                    offset -= removed as i64;
                }
                HashOp::InsertBefore {
                    line,
                    lines: new_lines,
                } => {
                    let actual = (*line as i64 + offset - 1).max(0) as usize;
                    for (i, l) in new_lines.iter().enumerate() {
                        result_lines.insert(actual + i, l.clone());
                    }
                    offset += new_lines.len() as i64;
                }
                HashOp::InsertAfter {
                    line,
                    lines: new_lines,
                } => {
                    let actual = (*line as i64 + offset).max(0) as usize;
                    for (i, l) in new_lines.iter().enumerate() {
                        result_lines.insert(actual + i, l.clone());
                    }
                    offset += new_lines.len() as i64;
                }
                HashOp::InsertHead { lines: new_lines } => {
                    for (i, l) in new_lines.iter().enumerate() {
                        result_lines.insert(i, l.clone());
                    }
                    offset += new_lines.len() as i64;
                }
                HashOp::InsertTail { lines: new_lines } => {
                    result_lines.extend(new_lines.clone());
                    offset += new_lines.len() as i64;
                }
            }
        }

        let new_content = result_lines.join("\n");
        let original_content = if content.ends_with('\n') && !new_content.ends_with('\n') {
            format!("{new_content}\n")
        } else {
            new_content
        };

        // Atomic write.
        let parent_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let tmp = parent_dir.join(format!(".mew-tmp-{}", ulid::Ulid::new()));
        tokio::fs::write(&tmp, &original_content)
            .await
            .map_err(|e| ToolError::Execution(format!("write failed: {}", e)))?;
        if let Err(e) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(ToolError::Execution(format!("rename failed: {}", e)));
        }

        // Build diff.
        let diff = make_unified_diff(&content, &original_content, &path);

        let op_count = patch.operations.len();
        Ok(ToolOutput {
            output: format!("applied {} operation(s)", op_count),
            error: String::new(),
            diff: Some(diff),
            ..Default::default()
        })
    }
}

/// Parse a hashline patch from text.
fn parse_patch(text: &str) -> Result<Patch, String> {
    let mut lines = text.lines().peekable();

    // Parse header: [path#HASH]
    let header = lines.next().ok_or("empty patch")?.trim();
    let header = header
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .ok_or("missing [path#HASH] header")?;

    let (file_path, file_hash) = header
        .split_once('#')
        .ok_or("header must contain # separator: [path#HASH]")?;

    let mut operations = Vec::new();
    let mut current_lines: Vec<String> = Vec::new();
    let mut current_op: Option<String> = None;

    for line in lines {
        let trimmed = line.trim();

        // Skip comments and empty lines between operations.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check if this is a new operation header.
        if is_op_header(trimmed) {
            // Flush the previous operation if there is one.
            if let Some(op_str) = current_op.take() {
                if let Some(op) = parse_op(&op_str, &std::mem::take(&mut current_lines)) {
                    operations.push(op);
                }
            }
            current_op = Some(trimmed.to_string());
        } else if let Some('+') = trimmed.chars().next() {
            // Content line for the current operation.
            current_lines.push(trimmed[1..].to_string());
        }
        // Other lines are ignored.
    }

    // Flush the last operation.
    if let Some(op_str) = current_op {
        if let Some(op) = parse_op(&op_str, &current_lines) {
            operations.push(op);
        }
    }

    if operations.is_empty() {
        return Err("no operations found in patch".into());
    }

    Ok(Patch {
        file_path: file_path.to_string(),
        file_hash: file_hash.to_string(),
        operations,
    })
}

fn is_op_header(line: &str) -> bool {
    line.starts_with("SWAP ") || line.starts_with("DEL ") || line.starts_with("INS.")
}

fn parse_op(header: &str, content: &[String]) -> Option<HashOp> {
    let header = header.trim();

    if let Some(rest) = header.strip_prefix("SWAP ") {
        let (start, end) = parse_range(rest)?;
        return Some(HashOp::Swap {
            start,
            end,
            lines: content.to_vec(),
        });
    }

    if let Some(rest) = header.strip_prefix("DEL ") {
        let (start, end) = parse_range(rest)?;
        return Some(HashOp::Delete { start, end });
    }

    if let Some(rest) = header.strip_prefix("INS.") {
        let rest = rest.trim_end_matches(':');
        let (sub, arg) = rest.split_once(' ').unwrap_or((rest, ""));
        let lines = content.to_vec();
        match sub {
            "HEAD" => return Some(HashOp::InsertHead { lines }),
            "TAIL" => return Some(HashOp::InsertTail { lines }),
            "PRE" => {
                let line: u32 = arg.trim_end_matches(':').parse().ok()?;
                return Some(HashOp::InsertBefore { line, lines });
            }
            "POST" => {
                let line: u32 = arg.trim_end_matches(':').parse().ok()?;
                return Some(HashOp::InsertAfter { line, lines });
            }
            _ => return None,
        }
    }

    None
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let s = s.trim_end_matches(':');
    if let Some((start_str, end_str)) = s.split_once(".=") {
        let start: u32 = start_str.parse().ok()?;
        let end: u32 = end_str.parse().ok()?;
        Some((start, end))
    } else {
        let line: u32 = s.parse().ok()?;
        Some((line, line))
    }
}

fn make_unified_diff(old: &str, new: &str, path: &std::path::Path) -> String {
    use similar::TextDiff;
    let diff = TextDiff::from_lines(old, new);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let mut out = String::new();
    for hunk in diff
        .unified_diff()
        .context_radius(3)
        .header(file_name, file_name)
        .iter_hunks()
    {
        out.push_str(&hunk.to_string());
    }
    if out.trim().is_empty() {
        file_name.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    #[test]
    fn test_compute_file_hash_stable() {
        let h1 = compute_file_hash("hello\nworld\n");
        let h2 = compute_file_hash("hello\nworld\n");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 4);
    }

    #[test]
    fn test_compute_file_hash_whitespace_invariant() {
        let h1 = compute_file_hash("hello\nworld\n");
        let h2 = compute_file_hash("hello  \nworld\t\r\n");
        assert_eq!(h1, h2, "trailing whitespace should not change hash");
    }

    #[tokio::test]
    async fn test_swap_replaces_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\nfn b() {}\nfn c() {}\n";
        tokio::fs::write(&path, content).await.unwrap();
        let hash = compute_file_hash(content);

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let patch = format!("[test.rs#{hash}]\nSWAP 2.=2:\n+fn b_modified() {{}}");
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("1 operation"));

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(new_content.contains("fn b_modified()"));
        assert!(new_content.contains("fn a()"));
        assert!(new_content.contains("fn c()"));
    }

    #[tokio::test]
    async fn test_delete_removes_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\nfn b() {}\nfn c() {}\n";
        tokio::fs::write(&path, content).await.unwrap();
        let hash = compute_file_hash(content);

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let patch = format!("[test.rs#{hash}]\nDEL 2");
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(new_content.contains("fn a()"));
        assert!(!new_content.contains("fn b()"));
        assert!(new_content.contains("fn c()"));
    }

    #[tokio::test]
    async fn test_insert_after_adds_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\nfn b() {}\n";
        tokio::fs::write(&path, content).await.unwrap();
        let hash = compute_file_hash(content);

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let patch = format!("[test.rs#{hash}]\nINS.POST 1:\n+fn a2() {{}}");
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = new_content.lines().collect();
        assert_eq!(lines[0], "fn a() {}");
        assert_eq!(lines[1], "fn a2() {}");
        assert_eq!(lines[2], "fn b() {}");
    }

    #[tokio::test]
    async fn test_insert_head() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn main() {}\n";
        tokio::fs::write(&path, content).await.unwrap();
        let hash = compute_file_hash(content);

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let patch = format!("[test.rs#{hash}]\nINS.HEAD:\n+// header comment");
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(new_content.starts_with("// header comment"));
        assert!(new_content.contains("fn main()"));
    }

    #[tokio::test]
    async fn test_insert_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn main() {}\n";
        tokio::fs::write(&path, content).await.unwrap();
        let hash = compute_file_hash(content);

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let patch = format!("[test.rs#{hash}]\nINS.TAIL:\n+fn extra() {{}}");
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(new_content.contains("fn extra()"));
        assert!(new_content.contains("fn main()"));
    }

    #[tokio::test]
    async fn test_stale_file_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\n";
        tokio::fs::write(&path, content).await.unwrap();

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        // Use a wrong hash to simulate a stale file.
        let patch = "[test.rs#0000]\nDEL 1".to_string();
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("hash mismatch"),
            "expected hash mismatch: {err}"
        );
        assert!(err.contains("re-read"), "expected re-read hint: {err}");

        // File should be unchanged.
        let unchanged = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(unchanged, content);
    }

    #[tokio::test]
    async fn test_multiple_operations_in_one_patch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n";
        tokio::fs::write(&path, content).await.unwrap();
        let hash = compute_file_hash(content);

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let patch = format!(
            "[test.rs#{hash}]\nDEL 1\nSWAP 2.=2:\n+fn b_modified() {{}}\nINS.TAIL:\n+fn e() {{}}"
        );
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("3 operation"));

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!new_content.contains("fn a()"));
        assert!(new_content.contains("fn b_modified()"));
        assert!(new_content.contains("fn c()"));
        assert!(new_content.contains("fn d()"));
        assert!(new_content.contains("fn e()"));
    }

    #[tokio::test]
    async fn test_out_of_bounds_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\nfn b() {}\n";
        tokio::fs::write(&path, content).await.unwrap();
        let hash = compute_file_hash(content);

        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let patch = format!("[test.rs#{hash}]\nDEL 99");
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("out of bounds"),
            "expected bounds error: {err}"
        );
    }

    #[tokio::test]
    async fn test_invalid_header_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let tool = EditHashline;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"patch": "no header here"});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_range_single() {
        assert_eq!(parse_range("5"), Some((5, 5)));
        assert_eq!(parse_range("5:"), Some((5, 5)));
    }

    #[test]
    fn test_parse_range_multi() {
        assert_eq!(parse_range("5.=10"), Some((5, 10)));
        assert_eq!(parse_range("5.=10:"), Some((5, 10)));
    }
}
