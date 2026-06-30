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

        // If a persistent shell session is available, use it instead of
        // spawning a fresh process. This lets `cd`, `export`, and other
        // state survive across calls.
        if let Some(ref session) = ctx.shell_session {
            return execute_in_session(session, command, &ctx, timeout_secs).await;
        }

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

        // Collect stdout and stderr separately so the final output can
        // distinguish them. The model sees stdout first, then a `--- stderr
        // ---` separator, then stderr lines — making it easy to tell error
        // output apart from normal output.
        let mut stdout_lines: Vec<String> = Vec::new();
        let mut stderr_lines: Vec<String> = Vec::new();
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
                // Return partial output on timeout instead of an opaque error.
                // The model can see what the command produced before it was
                // killed, which is critical for debugging long-running builds
                // or test suites.
                return Ok(finalize_output(
                    stdout_lines,
                    stderr_lines,
                    &ctx.secrets,
                    Some(format!(
                        "timeout after {}s (partial output shown)",
                        timeout_secs
                    )),
                ));
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
                Ok(Some((l, is_stderr))) => {
                    if is_stderr {
                        stderr_lines.push(l.clone());
                    } else {
                        stdout_lines.push(l.clone());
                    }
                    let _ = ctx.progress_tx.send(ToolProgress::OutputChunk(l)).await;
                }
                Ok(None) => {}
                Err(_) => {
                    std::mem::drop(tokio::spawn(
                        tokio::process::Command::new("kill")
                            .arg(pid.to_string())
                            .output(),
                    ));
                    // Return partial output on timeout instead of an opaque
                    // error. See the comment on the `remaining.is_zero()`
                    // branch above.
                    return Ok(finalize_output(
                        stdout_lines,
                        stderr_lines,
                        &ctx.secrets,
                        Some(format!(
                            "timeout after {}s (partial output shown)",
                            timeout_secs
                        )),
                    ));
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

        let error_msg = if status.success() {
            None
        } else {
            Some(format!("exit code {}", status.code().unwrap_or(-1)))
        };

        Ok(finalize_output(
            stdout_lines,
            stderr_lines,
            &ctx.secrets,
            error_msg,
        ))
    }
}

/// Execute a command via the persistent shell session.
async fn execute_in_session(
    session: &crate::tools::shell_session::SharedShellSession,
    command: &str,
    ctx: &ToolCtx,
    timeout_secs: u64,
) -> Result<ToolOutput, ToolError> {
    let mut session = session.lock().await;

    // Check for cancellation before acquiring the session.
    if ctx.cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }

    let result = session
        .execute(command, timeout_secs)
        .await
        .map_err(|e| ToolError::Execution(format!("shell session: {}", e)))?;

    // Build combined output: stdout first, then stderr under a separator.
    let mut combined = result.stdout.clone();
    if !result.stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("--- stderr ---\n");
        combined.push_str(&result.stderr);
    }

    // Redact secrets before truncation.
    let (combined, redacted) = crate::secrets::redact_secret_words(&combined, &ctx.secrets);

    let truncated = if combined.len() > OUTPUT_TRUNCATE_AT {
        format!(
            "{}...[truncated {} chars]",
            &combined[..OUTPUT_TRUNCATE_AT],
            combined.len() - OUTPUT_TRUNCATE_AT
        )
    } else {
        combined
    };
    let truncated = crate::secrets::annotate_redaction(truncated, redacted);

    let error_msg = if result.timed_out {
        Some(format!(
            "timeout after {}s (partial output shown)",
            timeout_secs
        ))
    } else if result.exit_code != 0 {
        Some(format!("exit code {}", result.exit_code))
    } else {
        None
    };

    Ok(ToolOutput {
        output: truncated,
        error: error_msg.unwrap_or_default(),
        diff: None,
        metadata: None,
    })
}

/// Merge stdout and stderr lines into a single redacted/truncated output
/// string. Stdout comes first; if there are any stderr lines, they appear
/// after a `--- stderr ---` separator so the model can tell them apart.
///
/// `error_msg` — if `Some`, set as the `ToolOutput.error` field (non-empty
/// means the command didn't complete cleanly: exit code, timeout, etc.).
fn finalize_output(
    stdout_lines: Vec<String>,
    stderr_lines: Vec<String>,
    secrets: &crate::SecretSet,
    error_msg: Option<String>,
) -> ToolOutput {
    let mut combined = stdout_lines.join("\n");
    if !stderr_lines.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("--- stderr ---\n");
        combined.push_str(&stderr_lines.join("\n"));
    }

    // Redact configured secret words before truncation so a value
    // straddling the truncation boundary cannot leak its tail. The
    // truncation count then refers to the redacted length.
    let (combined, redacted) = crate::secrets::redact_secret_words(&combined, secrets);

    let truncated = if combined.len() > OUTPUT_TRUNCATE_AT {
        format!(
            "{}...[truncated {} chars]",
            &combined[..OUTPUT_TRUNCATE_AT],
            combined.len() - OUTPUT_TRUNCATE_AT
        )
    } else {
        combined
    };
    let truncated = crate::secrets::annotate_redaction(truncated, redacted);

    ToolOutput {
        output: truncated,
        error: error_msg.unwrap_or_default(),
        diff: None,
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
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
    async fn test_bash_timeout_preserves_partial_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Bash;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        // Print a line, then sleep past the timeout. The partial line
        // must appear in the output even though the command timed out.
        let input = serde_json::json!({
            "command": "echo partial-output; sleep 10",
            "timeout": 1
        });
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.error.contains("timeout"));
        assert!(result.error.contains("partial output shown"));
        assert!(result.output.contains("partial-output"));
    }

    #[tokio::test]
    async fn test_bash_stderr_separated() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Bash;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"command": "echo err >&2; echo out"});
        let result = tool.execute(ctx, input).await.unwrap();
        // stdout comes first, stderr under the separator.
        assert!(result.output.contains("out"));
        assert!(result.output.contains("err"));
        assert!(result.output.contains("--- stderr ---"));
        // stdout line should appear before the separator.
        let out_pos = result.output.find("out").unwrap();
        let sep_pos = result.output.find("--- stderr ---").unwrap();
        assert!(out_pos < sep_pos);
    }
}
