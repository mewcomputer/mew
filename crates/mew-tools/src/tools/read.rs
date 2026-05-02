use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Read;

#[async_trait]
impl Tool for Read {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file."
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
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (0-indexed)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read."
                    }
                },
                "required": ["path"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing path".into()))?;
        let path = ctx.cwd.join(path);

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::Execution(format!("read failed: {}", e)))?;

        let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = input.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);

        let content = if offset > 0 || limit.is_some() {
            let lines: Vec<&str> = content.lines().collect();
            let start = offset.min(lines.len());
            let end = limit
                .map(|l| (start + l).min(lines.len()))
                .unwrap_or(lines.len());
            lines[start..end].join("\n")
        } else {
            content
        };

        Ok(ToolOutput {
            output: content,
            error: String::new(),
        })
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
        }
    }

    #[tokio::test]
    async fn test_read_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "test.txt"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert_eq!(result.output, "hello world");
        assert!(result.error.is_empty());
    }

    #[tokio::test]
    async fn test_read_offset_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "line1\nline2\nline3\nline4").await.unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "test.txt", "offset": 1, "limit": 2});
        let result = tool.execute(ctx, input).await.unwrap();
        assert_eq!(result.output, "line2\nline3");
    }

    #[tokio::test]
    async fn test_read_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "missing.txt"});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
    }
}
