use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Write;

#[async_trait]
impl Tool for Write {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it does not exist, overwrites if it does."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file, relative to the current working directory."
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file."
                    }
                },
                "required": ["path", "content"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Mutating
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing path".into()))?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing content".into()))?;

        let path = ctx.cwd.join(path);

        let old_content = tokio::fs::read_to_string(&path).await.ok();

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Execution(format!("create dirs failed: {}", e)))?;
        }

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ToolError::Execution(format!("write failed: {}", e)))?;

        let diff = if let Some(ref old) = old_content {
            let old_len = old.len();
            let mut diff_text = format!(
                "overwrote {} (was {} bytes, now {} bytes)\n",
                path.display(),
                old_len,
                content.len()
            );
            diff_text.push_str(&make_unified_diff(old, content, &path));
            Some(diff_text)
        } else {
            let preview: String = content
                .lines()
                .take(6)
                .map(|l| format!("+ {}", l))
                .collect::<Vec<_>>()
                .join("\n");
            let more = if content.lines().count() > 6 {
                format!("\n  ... ({} more lines)", content.lines().count() - 6)
            } else {
                String::new()
            };
            Some(format!("created {}\n{}{}", path.display(), preview, more))
        };

        Ok(ToolOutput {
            output: format!("wrote {} bytes to {}", content.len(), path.display()),
            error: String::new(),
            diff,
        })
    }
}

/// Build a compact unified diff of two file contents.
fn make_unified_diff(old: &str, new: &str, path: &std::path::Path) -> String {
    use similar::TextDiff;

    let diff = TextDiff::from_lines(old, new);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");

    let mut out = String::new();
    for hunk in diff
        .unified_diff()
        .context_radius(3)
        .header(&file_name, &file_name)
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
        ToolCtx {
            session_id: mew_message::SessionId::from(ulid::Ulid::new()),
            call_id: "test".to_string(),
            cancel: tokio_util::sync::CancellationToken::new(),
            progress_tx: tokio::sync::mpsc::channel(1).0,
            cwd,
            dispatcher: None,
        }
    }

    #[tokio::test]
    async fn test_write_file() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Write;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "test.txt", "content": "hello world"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("wrote 11 bytes"));

        let content = tokio::fs::read_to_string(dir.path().join("test.txt"))
            .await
            .unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_write_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Write;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input =
            serde_json::json!({"path": "subdir/nested/test.txt", "content": "nested"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("wrote 6 bytes"));

        let content = tokio::fs::read_to_string(dir.path().join("subdir/nested/test.txt"))
            .await
            .unwrap();
        assert_eq!(content, "nested");
    }

    #[tokio::test]
    async fn test_write_diff_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Write;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "new.txt", "content": "line1\nline2"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.diff.is_some());
        let diff = result.diff.unwrap();
        assert!(diff.contains("created"));
        assert!(diff.contains("+ line1"));
        assert!(diff.contains("+ line2"));
    }

    #[tokio::test]
    async fn test_write_diff_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        tokio::fs::write(&path, "old content").await.unwrap();

        let tool = Write;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "existing.txt", "content": "new content"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.diff.is_some());
        let diff = result.diff.unwrap();
        assert!(diff.contains("overwrote"));
        assert!(diff.contains("was 11 bytes"));
        assert!(diff.contains("now 11 bytes"));
    }
}
