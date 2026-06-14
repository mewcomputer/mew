use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput, ToolProgress};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use tokio::io::AsyncBufReadExt;

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

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&ctx.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Apply shell env hook if a dispatcher is available.
        if let Some(ref dispatcher) = ctx.dispatcher {
            let current_env = std::env::vars().collect::<HashMap<String, String>>();
            let filtered = dispatcher.on_shell_env(current_env).await;
            for (k, v) in &filtered {
                cmd.env(k, v);
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::Execution(format!("spawn failed: {}", e)))?;

        let pid = child
            .id()
            .ok_or_else(|| ToolError::Execution("child process has no pid".into()))?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut stdout_reader = tokio::io::BufReader::new(stdout).lines();
        let mut stderr_reader = tokio::io::BufReader::new(stderr).lines();

        let mut full_output = String::new();
        let mut stdout_done = false;
        let mut stderr_done = false;
        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let deadline = tokio::time::Instant::now() + timeout;

        while !stdout_done || !stderr_done {
            if ctx.cancel.is_cancelled() {
                let _ = tokio::process::Command::new("kill")
                    .arg(pid.to_string())
                    .output()
                    .await;
                return Err(ToolError::Cancelled);
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let _ = tokio::process::Command::new("kill")
                    .arg(pid.to_string())
                    .output()
                    .await;
                return Err(ToolError::Execution("timeout".into()));
            }

            let line = tokio::time::timeout(remaining, async {
                tokio::select! {
                    line = stdout_reader.next_line(), if !stdout_done => {
                        match line {
                            Ok(Some(l)) => Some((l, false)),
                            Ok(None) => { stdout_done = true; None }
                            Err(_) => { stdout_done = true; None }
                        }
                    }
                    line = stderr_reader.next_line(), if !stderr_done => {
                        match line {
                            Ok(Some(l)) => Some((l, true)),
                            Ok(None) => { stderr_done = true; None }
                            Err(_) => { stderr_done = true; None }
                        }
                    }
                    else => None,
                }
            })
            .await;

            match line {
                Ok(Some((l, _is_stderr))) => {
                    if !full_output.is_empty() {
                        full_output.push('\n');
                    }
                    full_output.push_str(&l);
                    let _ = ctx.progress_tx.send(ToolProgress::OutputChunk(l)).await;
                }
                Ok(None) => {}
                Err(_) => {
                    std::mem::drop(tokio::spawn(
                        tokio::process::Command::new("kill")
                            .arg(pid.to_string())
                            .output(),
                    ));
                    return Err(ToolError::Execution("timeout".into()));
                }
            }
        }

        let status = tokio::time::timeout(timeout, child.wait())
            .await
            .map_err(|_| {
                std::mem::drop(tokio::spawn(
                    tokio::process::Command::new("kill")
                        .arg(pid.to_string())
                        .output(),
                ));
                ToolError::Execution("timeout".into())
            })?
            .map_err(|e| ToolError::Execution(format!("wait failed: {}", e)))?;

        let truncated = if full_output.len() > OUTPUT_TRUNCATE_AT {
            format!(
                "{}...[truncated {} chars]",
                &full_output[..OUTPUT_TRUNCATE_AT],
                full_output.len() - OUTPUT_TRUNCATE_AT
            )
        } else {
            full_output
        };

        if status.success() {
            Ok(ToolOutput {
                output: truncated,
                error: String::new(),
                diff: None,
            })
        } else {
            Ok(ToolOutput {
                output: truncated,
                error: format!("exit code {}", status.code().unwrap_or(-1)),
                diff: None,
            })
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
            dispatcher: None,
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
