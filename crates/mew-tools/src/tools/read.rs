use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Read;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

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

        // Check file size before reading.
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ToolError::Execution(format!("stat failed: {}", e)))?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(ToolError::Execution(format!(
                "file too large ({} bytes, max {})",
                metadata.len(),
                MAX_FILE_SIZE
            )));
        }

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::Execution(format!("read failed: {}", e)))?;

        // Detect binary files via null byte.
        if content.contains('\0') {
            return Err(ToolError::Execution("cannot read binary file".into()));
        }

        let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

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

        // Strip configured secret words from the returned content. This is
        // a second line of defense behind the permission-engine pre-check:
        // even when the user approves reading a file, individual secret
        // values (API keys, tokens) are still scrubbed so they do not land
        // in the model's context. The structure (variable names, line
        // shape) is preserved so the model can still reason about the file.
        let (content, redacted) = crate::secrets::redact_secret_words(&content, &ctx.secrets);
        let content = crate::secrets::annotate_redaction(content, redacted);

        Ok(ToolOutput {
            output: content,
            error: String::new(),
            diff: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    fn ctx_with_secret_words(cwd: PathBuf, words: Vec<&str>) -> ToolCtx {
        ToolCtx {
            session_id: mew_message::SessionId::from(ulid::Ulid::new()),
            call_id: "test".to_string(),
            cancel: tokio_util::sync::CancellationToken::new(),
            progress_tx: tokio::sync::mpsc::channel(1).0,
            cwd,
            dispatcher: None,
            secrets: std::sync::Arc::new(crate::SecretSet {
                words: words.iter().map(|s| s.to_string()).collect(),
                globs: vec![],
            }),
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
        tokio::fs::write(&path, "line1\nline2\nline3\nline4")
            .await
            .unwrap();

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

    #[tokio::test]
    async fn test_read_redacts_secret_words() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.txt");
        tokio::fs::write(&path, "API_KEY=AKIAIOSFODNN7EXAMPLE\nPORT=3000")
            .await
            .unwrap();

        let tool = Read;
        let ctx = ctx_with_secret_words(dir.path().to_path_buf(), vec!["AKIAIOSFODNN7EXAMPLE"]);
        let input = serde_json::json!({"path": "config.txt"});
        let result = tool.execute(ctx, input).await.unwrap();
        // The secret value is gone; the variable name and surrounding
        // structure survive so the model can still reason about the file.
        assert!(!result.output.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(result.output.contains("API_KEY=[REDACTED]"));
        assert!(result.output.contains("PORT=3000"));
        assert!(result.output.contains("redacted"));
    }

    #[tokio::test]
    async fn test_read_no_redaction_without_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.txt");
        tokio::fs::write(&path, "just text").await.unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "plain.txt"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert_eq!(result.output, "just text");
        assert!(!result.output.contains("redacted"));
    }
}
