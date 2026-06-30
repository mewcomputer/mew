//! A persistent bash shell session that survives across tool invocations.
//!
//! Instead of spawning a fresh `bash -c` process for every command, the
//! `ShellSession` keeps a single long-running `bash` process alive. Commands
//! are written to its stdin; stdout/stderr are read back line-by-line until
//! a sentinel marker is seen. This means `cd`, `export`, shell variables,
//! and background jobs survive between calls — the same way an interactive
//! terminal works.
//!
//! The session is lazy: it spawns the bash process on first use. If the
//! process dies (e.g. the user runs `exit`), the next call re-spawns it.
//!
//! Thread-safety: the session is wrapped in `Arc<Mutex<...>>` so multiple
//! tool calls can share it safely. Commands are serialized — only one
//! command executes at a time.

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// Type alias for the line-by-line reader.
type Lines = tokio::io::Lines<BufReader<ChildStdout>>;
type ErrLines = tokio::io::Lines<BufReader<ChildStderr>>;

/// A unique sentinel marker written after each command so we can detect
/// when the command's output has finished. The marker includes the exit
/// code so the caller can report success/failure.
const SENTINEL: &str = "__MEW_SHELL_SENTINEL_EXIT_CODE_";

/// A persistent bash shell session.
pub struct ShellSession {
    /// The bash child process. `None` when not yet started or when the
    /// process has died and needs re-spawning.
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<Lines>,
    stderr: Option<ErrLines>,
    /// Monotonic counter for unique sentinel markers.
    counter: u64,
    /// The working directory the session was started in.
    cwd: std::path::PathBuf,
}

impl ShellSession {
    /// Create a new shell session. The bash process is not spawned until
    /// the first command is executed.
    pub fn new(cwd: std::path::PathBuf) -> Self {
        Self {
            child: None,
            stdin: None,
            stdout: None,
            stderr: None,
            counter: 0,
            cwd,
        }
    }

    /// Ensure the bash process is running. If it's not started or has
    /// died, spawn a new one.
    async fn ensure_started(&mut self) -> Result<(), ShellError> {
        // Check if the existing child is still alive.
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(None) => {
                    // Still running. Drain any stale output from a previous
                    // timed-out command so it doesn't pollute the next result.
                    if let Some(ref mut stdout) = self.stdout {
                        loop {
                            let drain = tokio::time::timeout(
                                std::time::Duration::from_millis(50),
                                stdout.next_line(),
                            )
                            .await;
                            match drain {
                                Ok(Ok(Some(_))) => continue,
                                _ => break,
                            }
                        }
                    }
                    return Ok(());
                }
                Ok(Some(_)) => {} // Exited; fall through to respawn.
                Err(_) => {}
            }
        }
        // Spawn a new bash process.
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("--norc")
            .arg("-i")
            .current_dir(&self.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Suppress bash's job control messages in interactive mode.
        cmd.env("PS1", "");
        cmd.env("BASH_ENV", "");

        let mut child = cmd.spawn().map_err(ShellError::Spawn)?;
        let stdin = child.stdin.take().ok_or(ShellError::NoStdin)?;
        let stdout = child.stdout.take().ok_or(ShellError::NoStdout)?;
        let stderr = child.stderr.take().ok_or(ShellError::NoStderr)?;

        // Write an initial setup command to suppress prompt output.
        let mut stdin = stdin;
        // Disable job control messages and set a minimal prompt.
        let _ = stdin.write_all(b"set +m\nexport PS1=''\n").await;
        let _ = stdin.flush().await;

        self.child = Some(child);
        self.stdin = Some(stdin);
        self.stdout = Some(BufReader::new(stdout).lines());
        self.stderr = Some(BufReader::new(stderr).lines());
        Ok(())
    }

    /// Execute a command in the persistent shell. Returns the combined
    /// stdout, stderr (under a separator), and exit code.
    ///
    /// The command is written to the shell's stdin followed by a sentinel
    /// echo. We read stdout/stderr lines until we see the sentinel, which
    /// carries the exit code.
    pub async fn execute(
        &mut self,
        command: &str,
        timeout_secs: u64,
    ) -> Result<ShellResult, ShellError> {
        let _ = self.ensure_started().await;

        let marker_id = self.counter;
        self.counter += 1;
        let sentinel = format!("{SENTINEL}{marker_id}");

        let stdin = self.stdin.as_mut().ok_or(ShellError::NoStdin)?;

        // Write the command, then echo the sentinel with the exit code.
        // The sentinel is always printed because `echo` runs after the
        // command completes (even on failure). If the command calls
        // `exit`, the shell process dies and we detect that in the read
        // loop below (stdout closes without a sentinel).
        let full_command = format!("{command}\n__mew_exit=$?\necho '{sentinel}'\"$__mew_exit\"\n");
        stdin
            .write_all(full_command.as_bytes())
            .await
            .map_err(ShellError::Write)?;
        stdin.flush().await.map_err(ShellError::Write)?;

        // Read stdout and stderr until we see the sentinel.
        let stdout = self.stdout.as_mut().ok_or(ShellError::NoStdout)?;
        let stderr = self.stderr.as_mut().ok_or(ShellError::NoStderr)?;

        let mut stdout_lines: Vec<String> = Vec::new();
        let mut stderr_lines: Vec<String> = Vec::new();
        let mut exit_code: Option<i32> = None;
        let mut stdout_done = false;

        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let deadline = tokio::time::Instant::now() + timeout;

        while !stdout_done {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                // Timeout. Kill the shell process entirely.
                if let Some(ref mut child) = self.child {
                    let _ = child.start_kill();
                }
                self.child = None;
                self.stdin = None;
                self.stdout = None;
                self.stderr = None;
                return Ok(ShellResult {
                    stdout: stdout_lines.join("\n"),
                    stderr: stderr_lines.join("\n"),
                    exit_code: -1,
                    timed_out: true,
                });
            }

            let line = tokio::time::timeout(remaining, stdout.next_line()).await;
            match line {
                Ok(Ok(Some(l))) => {
                    // Check for sentinel.
                    if l.starts_with(&sentinel) {
                        let code_str = &l[sentinel.len()..];
                        exit_code = code_str.trim().parse::<i32>().ok().or(Some(-1));
                        stdout_done = true;
                    } else {
                        stdout_lines.push(l);
                    }
                }
                Ok(Ok(None)) => {
                    // stdout closed — the shell process died (e.g. `exit`
                    // was called). Re-spawn on next call.
                    stdout_done = true;
                    // Kill the child if it's somehow still around.
                    if let Some(ref mut child) = self.child {
                        let _ = child.start_kill();
                    }
                }
                Ok(Err(_)) => {
                    stdout_done = true;
                }
                Err(_) => {
                    // Timeout. Kill the shell process entirely — the
                    // running command's state is uncertain and its
                    // sentinel would pollute the next call's output.
                    // `ensure_started` will re-spawn on the next call.
                    if let Some(ref mut child) = self.child {
                        let _ = child.start_kill();
                    }
                    self.child = None;
                    self.stdin = None;
                    self.stdout = None;
                    self.stderr = None;
                    return Ok(ShellResult {
                        stdout: stdout_lines.join("\n"),
                        stderr: stderr_lines.join("\n"),
                        exit_code: -1,
                        timed_out: true,
                    });
                }
            }
        }

        // Drain any stderr that's available (non-blocking, short timeout).
        loop {
            let drain_result =
                tokio::time::timeout(std::time::Duration::from_millis(50), stderr.next_line())
                    .await;
            match drain_result {
                Ok(Ok(Some(l))) => stderr_lines.push(l),
                _ => break,
            }
        }

        Ok(ShellResult {
            stdout: stdout_lines.join("\n"),
            stderr: stderr_lines.join("\n"),
            exit_code: exit_code.unwrap_or(-1),
            timed_out: false,
        })
    }

    /// Kill the shell process if it's running.
    pub async fn kill(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        self.child = None;
        self.stdin = None;
        self.stdout = None;
        self.stderr = None;
    }
}

impl Drop for ShellSession {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            // Best-effort kill; can't await in Drop.
            let _ = child.start_kill();
        }
    }
}

/// Result of a shell command execution.
#[derive(Debug, Clone)]
pub struct ShellResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

/// Errors from the shell session.
#[derive(Debug)]
pub enum ShellError {
    Spawn(std::io::Error),
    Write(std::io::Error),
    NoStdin,
    NoStdout,
    NoStderr,
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellError::Spawn(e) => write!(f, "failed to spawn shell: {e}"),
            ShellError::Write(e) => write!(f, "failed to write to shell: {e}"),
            ShellError::NoStdin => write!(f, "shell has no stdin"),
            ShellError::NoStdout => write!(f, "shell has no stdout"),
            ShellError::NoStderr => write!(f, "shell has no stderr"),
        }
    }
}

impl std::error::Error for ShellError {}

/// A shared, thread-safe handle to a persistent shell session.
pub type SharedShellSession = Arc<Mutex<ShellSession>>;

/// Create a shared shell session for the given working directory.
pub fn shared_session(cwd: std::path::PathBuf) -> SharedShellSession {
    Arc::new(Mutex::new(ShellSession::new(cwd)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shell_session_echo() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        let result = session.execute("echo hello", 10).await.unwrap();
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.exit_code, 0);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn test_shell_session_exit_code() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        // Use `false` (always returns exit code 1) instead of `exit`,
        // which would kill the shell process.
        let result = session.execute("false", 10).await.unwrap();
        assert_eq!(result.exit_code, 1);
    }

    #[tokio::test]
    async fn test_shell_session_persists_cwd() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        // cd to / and verify pwd persists across calls.
        session.execute("cd /", 10).await.unwrap();
        let result = session.execute("pwd", 10).await.unwrap();
        assert_eq!(result.stdout.trim(), "/");
    }

    #[tokio::test]
    async fn test_shell_session_persists_env() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        session
            .execute("export MEW_TEST_VAR=hello123", 10)
            .await
            .unwrap();
        let result = session.execute("echo $MEW_TEST_VAR", 10).await.unwrap();
        assert_eq!(result.stdout.trim(), "hello123");
    }

    #[tokio::test]
    async fn test_shell_session_stderr() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        let result = session.execute("echo err >&2", 10).await.unwrap();
        assert!(result.stderr.contains("err"));
    }

    #[tokio::test]
    async fn test_shell_session_multi_line_command() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        let result = session
            .execute("echo line1\necho line2\necho line3", 10)
            .await
            .unwrap();
        assert!(result.stdout.contains("line1"));
        assert!(result.stdout.contains("line2"));
        assert!(result.stdout.contains("line3"));
    }

    #[tokio::test]
    async fn test_shell_session_timeout() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        let result = session.execute("echo start; sleep 10", 1).await.unwrap();
        assert!(result.timed_out);
        assert!(result.stdout.contains("start"));
    }

    #[tokio::test]
    async fn test_shell_session_recovers_after_timeout() {
        let mut session = ShellSession::new(std::path::PathBuf::from("/tmp"));
        // First command times out.
        let _ = session.execute("sleep 10", 1).await;
        // The session should re-spawn and work for the next command.
        let result = session.execute("echo recovered", 10).await.unwrap();
        assert_eq!(result.stdout.trim(), "recovered");
    }
}
