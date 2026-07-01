//! Hashline edit format: line-anchored edits with file-hash staleness detection.
//!
//! This tool delegates to the `mew-hashline` crate, which supports the full
//! oh-my-pi/hashline feature set: multi-section patches, block-aware ops,
//! snapshot-store recovery, seen-line validation, REM/MV, and CRLF/BOM
//! preservation.

use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct EditHashline;

#[async_trait]
impl Tool for EditHashline {
    fn name(&self) -> &str {
        "edit_hashline"
    }

    fn description(&self) -> &str {
        "Edit files using the hashline format: line-numbered operations with \
         file-hash staleness detection. Supports SWAP/DEL/INS, SWAP.BLK/\
         DEL.BLK/INS.BLK.POST, REM, and MV across one or more [path#hash] \
         sections. The file hash comes from the most recent `read` output."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Hashline patch text. One or more [path#hash] sections followed by SWAP, DEL, INS, REM, or MV ops."
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

        let fs = TokioHashlineFs {
            cwd: ctx.cwd.clone(),
        };
        let patcher = mew_hashline::Patcher::new(mew_hashline::PatcherOptions {
            snapshots: ctx.snapshot_store.clone(),
            block_resolver: Some(mew_hashline::block::default_block_resolver()),
        });

        let results = patcher
            .apply(patch_text, &fs)
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?;

        let mut output = String::new();
        let mut diff = String::new();
        for result in results {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!(
                "[{}#{}] {}",
                result.path,
                result.file_hash,
                op_name(result.op)
            ));
            if let Some(dest) = &result.move_dest {
                output.push_str(&format!(" -> {dest}"));
            }
            if let Some(line) = result.first_changed_line {
                output.push_str(&format!(" (first changed line: {line})"));
            }
            for w in &result.warnings {
                output.push_str(&format!("\n  warning: {w}"));
            }

            let section_diff = make_unified_diff(&result.before, &result.after, &result.path);
            if !section_diff.trim().is_empty() {
                if !diff.is_empty() {
                    diff.push('\n');
                }
                diff.push_str(&section_diff);
            }
        }

        Ok(ToolOutput {
            output,
            error: String::new(),
            diff: if diff.is_empty() { None } else { Some(diff) },
            metadata: None,
        })
    }
}

fn op_name(op: mew_hashline::patcher::PatchOp) -> &'static str {
    use mew_hashline::patcher::PatchOp;
    match op {
        PatchOp::Create => "created",
        PatchOp::Update => "updated",
        PatchOp::Delete => "deleted",
        PatchOp::Noop => "unchanged",
    }
}

fn make_unified_diff(old: &str, new: &str, path: &str) -> String {
    use similar::TextDiff;
    let diff = TextDiff::from_lines(old, new);
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
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

struct TokioHashlineFs {
    cwd: std::path::PathBuf,
}

#[async_trait]
impl mew_hashline::fs::HashlineFs for TokioHashlineFs {
    async fn read_text(&self, path: &str) -> std::io::Result<String> {
        let full = self.cwd.join(path);
        tokio::fs::read_to_string(&full).await
    }

    async fn write_text(&self, path: &str, content: &str) -> std::io::Result<()> {
        let full = self.cwd.join(path);
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full, content).await
    }

    async fn delete(&self, path: &str) -> std::io::Result<()> {
        let full = self.cwd.join(path);
        tokio::fs::remove_file(&full).await
    }

    async fn rename(&self, from: &str, to: &str) -> std::io::Result<()> {
        let from_full = self.cwd.join(from);
        let to_full = self.cwd.join(to);
        if let Some(parent) = to_full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&from_full, &to_full).await
    }

    fn canonical_path(&self, path: &str) -> String {
        self.cwd.join(path).to_string_lossy().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    #[tokio::test]
    async fn test_swap_replaces_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\nfn b() {}\nfn c() {}\n";
        tokio::fs::write(&path, content).await.unwrap();

        // Seed the snapshot store as if `read` had been called.
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let hash = mew_hashline::format::compute_file_hash(content);
        ctx.snapshot_store.record(
            &path.to_string_lossy(),
            &mew_hashline::format::normalize_to_lf(content),
            Some(&[1, 2, 3]),
        );

        let tool = EditHashline;
        let patch = format!("[test.rs#{hash}]\nSWAP 2.=2:\n+fn b_modified() {{}}");
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("updated"));

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(new_content.contains("fn b_modified()"));
        assert!(new_content.contains("fn a()"));
        assert!(new_content.contains("fn c()"));
    }

    #[tokio::test]
    async fn test_stale_file_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\n";
        tokio::fs::write(&path, content).await.unwrap();

        let ctx = dummy_ctx(dir.path().to_path_buf());
        ctx.snapshot_store.record(
            &path.to_string_lossy(),
            &mew_hashline::format::normalize_to_lf(content),
            Some(&[1]),
        );

        let tool = EditHashline;
        let input = serde_json::json!({"patch": "[test.rs#0000]\nDEL 1"});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("hash mismatch"),
            "expected hash mismatch: {err}"
        );

        let unchanged = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(unchanged, content);
    }

    #[tokio::test]
    async fn test_multiple_operations_in_one_patch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        let content = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n";
        tokio::fs::write(&path, content).await.unwrap();

        let ctx = dummy_ctx(dir.path().to_path_buf());
        let hash = mew_hashline::format::compute_file_hash(content);
        ctx.snapshot_store.record(
            &path.to_string_lossy(),
            &mew_hashline::format::normalize_to_lf(content),
            Some(&[1, 2, 3, 4]),
        );

        let tool = EditHashline;
        let patch = format!(
            "[test.rs#{hash}]\nDEL 1\nSWAP 2.=2:\n+fn b_modified() {{}}\nINS.TAIL:\n+fn e() {{}}"
        );
        let input = serde_json::json!({"patch": patch});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("updated"));

        let new_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!new_content.contains("fn a()"));
        assert!(new_content.contains("fn b_modified()"));
        assert!(new_content.contains("fn c()"));
        assert!(new_content.contains("fn d()"));
        assert!(new_content.contains("fn e()"));
    }
}
