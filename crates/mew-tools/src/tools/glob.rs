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
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let base = ctx.cwd.join(path);

        let cancel = ctx.cancel.clone();
        let bg_base = base.clone();
        let glob_clone = globset::Glob::new(pattern)
            .map_err(|e| ToolError::InvalidInput(format!("invalid pattern: {}", e)))?
            .compile_matcher();

        let files = tokio::task::spawn_blocking(move || {
            let mut files = Vec::new();
            let walker = ignore::WalkBuilder::new(&bg_base).hidden(false).build();

            for result in walker {
                if cancel.is_cancelled() {
                    return Err(ToolError::Cancelled);
                }
                let entry =
                    result.map_err(|e| ToolError::Execution(format!("walk error: {}", e)))?;
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let rel = path.strip_prefix(&bg_base).unwrap_or(path);
                let rel_str = rel.to_string_lossy();
                if glob_clone.is_match(&*rel_str) {
                    files.push(rel_str.to_string());
                }
            }
            Ok(files)
        })
        .await
        .map_err(|e| ToolError::Execution(format!("glob join error: {}", e)))?;
        let mut files = files?;

        // Drop results touching secret files.
        if !ctx.secrets.globs.is_empty() {
            let secret_matchers: Vec<globset::GlobMatcher> = ctx
                .secrets
                .globs
                .iter()
                .filter_map(|g| globset::Glob::new(g).ok().map(|g| g.compile_matcher()))
                .collect();
            files.retain(|f| !secret_matchers.iter().any(|m| m.is_match(f)));
        }

        files.sort();
        Ok(ToolOutput {
            output: files.join("\n"),
            error: String::new(),
            diff: None,
            metadata: None,
        file_delta: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    fn ctx_with_secret_globs(cwd: PathBuf, globs: Vec<&str>) -> ToolCtx {
        let secrets = Arc::new(SecretSet {
            globs: globs.iter().map(|s| s.to_string()).collect(),
            words: vec![],
        });
        ToolCtx::test_with_secrets(cwd, secrets)
    }

    #[tokio::test]
    async fn test_glob() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("foo.rs"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("bar.rs"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("baz.txt"), "")
            .await
            .unwrap();

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
        tokio::fs::write(dir.path().join("src/lib.rs"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("main.rs"), "")
            .await
            .unwrap();

        let tool = Glob;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "**/*.rs"});
        let result = tool.execute(ctx, input).await.unwrap();
        let files: Vec<&str> = result.output.lines().collect();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn test_glob_drops_secret_files() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("main.rs"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("secrets.toml"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("creds.pem"), "")
            .await
            .unwrap();

        let tool = Glob;
        // Match everything, then confirm secret globs filter them out.
        let ctx = ctx_with_secret_globs(dir.path().to_path_buf(), vec!["secrets.toml", "*.pem"]);
        let input = serde_json::json!({"pattern": "*"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(
            result.output.contains("main.rs"),
            "non-secret file passes through"
        );
        assert!(
            !result.output.contains("secrets.toml"),
            "literal secret file dropped"
        );
        assert!(
            !result.output.contains("creds.pem"),
            "glob-matched secret file dropped"
        );
    }
}
