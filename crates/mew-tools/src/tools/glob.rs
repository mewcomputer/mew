use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Glob;

#[async_trait]
impl Tool for Glob {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match (e.g. '**/*.rs')."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (default: current directory)."
                    }
                },
                "required": ["pattern"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing pattern".into()))?;
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let base = ctx.cwd.join(path);

        let glob = globset::Glob::new(pattern)
            .map_err(|e| ToolError::InvalidInput(format!("invalid pattern: {}", e)))?;
        let matcher = glob.compile_matcher();

        let mut files = Vec::new();
        let walker = ignore::WalkBuilder::new(&base)
            .hidden(false)
            .build();

        for result in walker {
            let entry = result.map_err(|e| ToolError::Execution(format!("walk error: {}", e)))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let rel = path.strip_prefix(&base).unwrap_or(path);
            let rel_str = rel.to_string_lossy();
            if matcher.is_match(&*rel_str) {
                files.push(rel_str.to_string());
            }
        }

        files.sort();
        Ok(ToolOutput {
            output: files.join("\n"),
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
    async fn test_glob() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("foo.rs"), "").await.unwrap();
        tokio::fs::write(dir.path().join("bar.rs"), "").await.unwrap();
        tokio::fs::write(dir.path().join("baz.txt"), "").await.unwrap();

        let tool = Glob;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "*.rs"});
        let result = tool.execute(ctx, input).await.unwrap();
        let files: Vec<&str> = result.output.lines().collect();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"bar.rs"));
        assert!(files.contains(&"foo.rs"));
    }

    #[tokio::test]
    async fn test_glob_recursive() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(dir.path().join("src")).await.unwrap();
        tokio::fs::write(dir.path().join("src/lib.rs"), "").await.unwrap();
        tokio::fs::write(dir.path().join("main.rs"), "").await.unwrap();

        let tool = Glob;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "**/*.rs"});
        let result = tool.execute(ctx, input).await.unwrap();
        let files: Vec<&str> = result.output.lines().collect();
        assert_eq!(files.len(), 2);
    }
}
