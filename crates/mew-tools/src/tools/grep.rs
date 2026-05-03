use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Grep;

#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents for a pattern. Prefers ripgrep if available."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Pattern to search for."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (default: current directory)."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob filter for files to search (e.g. '*.rs')."
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
        let glob = input.get("glob").and_then(|v| v.as_str());
        let base = ctx.cwd.join(path);

        // Try ripgrep first
        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("--line-number")
            .arg("--with-filename")
            .arg("-H") // always show filename
            .arg(pattern)
            .current_dir(&base);

        if let Some(g) = glob {
            cmd.arg("--glob").arg(g);
        }

        let output = cmd.output().await;

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // rg exits 1 when no matches found, which is not an error
                if output.status.success() || output.status.code() == Some(1) {
                    Ok(ToolOutput {
                        output: stdout.to_string(),
                        error: String::new(),
                        diff: None,
                    })
                } else {
                    Err(ToolError::Execution(format!("rg failed: {}", stderr)))
                }
            }
            Err(_) => {
                // Fallback to grep -r
                let mut cmd = tokio::process::Command::new("grep");
                cmd.arg("-r")
                    .arg("-n")
                    .arg("-H")
                    .arg(pattern)
                    .current_dir(&base);

                if let Some(g) = glob {
                    cmd.arg("--include").arg(g);
                }

                let output = cmd
                    .output()
                    .await
                    .map_err(|e| ToolError::Execution(format!("grep failed: {}", e)))?;

                Ok(ToolOutput {
                    output: String::from_utf8_lossy(&output.stdout).to_string(),
                    error: String::new(),
                    diff: None,
                })
            }
        }
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
    async fn test_grep() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}\nfn foo() {}").await.unwrap();
        tokio::fs::write(dir.path().join("b.rs"), "fn bar() {}").await.unwrap();
        tokio::fs::write(dir.path().join("c.txt"), "fn baz() {}").await.unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "fn foo"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("a.rs"));
        assert!(result.output.contains("fn foo"));
    }

    #[tokio::test]
    async fn test_grep_glob_filter() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}").await.unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "fn main() {}").await.unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "fn main", "glob": "*.rs"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("a.rs"));
        assert!(!result.output.contains("b.txt"));
    }
}
