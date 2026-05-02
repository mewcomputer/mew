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

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Execution(format!("create dirs failed: {}", e)))?;
        }

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ToolError::Execution(format!("write failed: {}", e)))?;

        Ok(ToolOutput {
            output: format!("wrote {} bytes to {}", content.len(), path.display()),
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
}
