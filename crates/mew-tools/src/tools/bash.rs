use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Bash;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const OUTPUT_TRUNCATE_AT: usize = 30000;

#[async_trait]
impl Tool for Bash {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command. Use with caution."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default 120)."
                    }
                },
                "required": ["command"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Dangerous
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing command".into()))?;
        let timeout_secs = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ToolError::Execution(format!("spawn failed: {}", e)))?;

        let pid = child.id().expect("child has no pid");
        let timeout = tokio::time::Duration::from_secs(timeout_secs);

        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                let mut full_output = stdout.to_string();
                if !stderr.is_empty() {
                    if !full_output.is_empty() {
                        full_output.push('\n');
                    }
                    full_output.push_str(&stderr);
                }

                let truncated = if full_output.len() > OUTPUT_TRUNCATE_AT {
                    format!(
                        "{}...[truncated {} chars]",
                        &full_output[..OUTPUT_TRUNCATE_AT],
                        full_output.len() - OUTPUT_TRUNCATE_AT
                    )
                } else {
                    full_output
                };

                if output.status.success() {
                    Ok(ToolOutput {
                        output: truncated,
                        error: String::new(),
                    })
                } else {
                    Ok(ToolOutput {
                        output: truncated,
                        error: format!("exit code {}", exit_code),
                    })
                }
            }
            Ok(Err(e)) => Err(ToolError::Execution(format!("wait failed: {}", e))),
            Err(_) => {
                let _ = tokio::process::Command::new("kill")
                    .arg(pid.to_string())
                    .output()
                    .await;
                Err(ToolError::Execution("timeout".into()))
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
    async fn test_bash_echo() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Bash;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"command": "echo hello"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert_eq!(result.output.trim(), "hello");
        assert!(result.error.is_empty());
    }

    #[tokio::test]
    async fn test_bash_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Bash;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"command": "exit 42"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.error.contains("42"));
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Bash;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"command": "sleep 10", "timeout": 1});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }

    #[tokio::test]
    async fn test_bash_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Bash;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"command": "echo err >&2; echo out"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("out"));
        assert!(result.output.contains("err"));
    }
}
