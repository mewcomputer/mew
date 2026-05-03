use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Edit;

#[async_trait]
impl Tool for Edit {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replace old_string with new_string in a file. Exact match required; fails if ambiguous."
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
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to replace."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text."
                    }
                },
                "required": ["path", "old_string", "new_string"]
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
        let old = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing old_string".into()))?;
        let new = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing new_string".into()))?;

        let path = ctx.cwd.join(path);
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::Execution(format!("read failed: {}", e)))?;

        let count = content.matches(old).count();
        if count == 0 {
            return Err(ToolError::Execution("old_string not found".into()));
        }
        if count > 1 {
            return Err(ToolError::Execution(format!(
                "old_string matched {} times; ambiguous",
                count
            )));
        }

        let new_content = content.replacen(old, new, 1);
        tokio::fs::write(&path, &new_content)
            .await
            .map_err(|e| ToolError::Execution(format!("write failed: {}", e)))?;

        let diff = make_unified_diff(&content, &new_content, &path);

        Ok(ToolOutput {
            output: "replaced 1 occurrence".to_string(),
            error: String::new(),
            diff: Some(diff),
        })
    }
}

/// Build a compact unified diff of two file contents.
fn make_unified_diff(old: &str, new: &str, path: &std::path::Path) -> String {
    use similar::TextDiff;

    let diff = TextDiff::from_lines(old, new);
    let file_name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");

    let mut out = String::new();
    for hunk in diff.unified_diff().context_radius(3).header(&file_name, &file_name).iter_hunks() {
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
    async fn test_edit_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "world",
            "new_string": "mew"
        });
        let result = tool.execute(ctx, input).await.unwrap();
        assert_eq!(result.output, "replaced 1 occurrence");

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "hello mew");
    }

    #[tokio::test]
    async fn test_edit_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "missing",
            "new_string": "mew"
        });
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_ambiguous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "hello",
            "new_string": "hi"
        });
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ambiguous"));
    }

    #[tokio::test]
    async fn test_edit_diff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "world",
            "new_string": "mew"
        });
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.diff.is_some());
        let diff = result.diff.unwrap();
        assert!(diff.contains("-hello world"));
        assert!(diff.contains("+hello mew"));
    }
}
