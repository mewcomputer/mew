//! Framework-independent interactive PTY ownership for native clients.

pub mod grid;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;

const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

/// A live interactive shell attached to a pseudo-terminal.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl PtySession {
    /// Start the user's shell in `cwd`, inheriting the current environment.
    pub fn spawn(cwd: &Path) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open pseudo-terminal")?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
        let mut command = CommandBuilder::new(shell);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        let child = pair
            .slave
            .spawn_command(command)
            .context("spawn interactive shell")?;
        drop(pair.slave);

        Ok(Self {
            master: pair.master,
            child,
        })
    }

    /// Create an owned reader for bytes emitted by the shell.
    pub fn reader(&self) -> Result<Box<dyn Read + Send>> {
        self.master
            .try_clone_reader()
            .context("clone pseudo-terminal reader")
    }

    /// Take the terminal writer. A session has one writer for shell input.
    pub fn writer(&self) -> Result<Box<dyn Write + Send>> {
        self.master
            .take_writer()
            .context("take pseudo-terminal writer")
    }

    /// Resize the terminal grid, preserving the PTY rather than restarting it.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("resize pseudo-terminal")
    }

    /// Ask the child process to terminate.
    pub fn kill(&mut self) -> Result<()> {
        self.child.kill().context("kill pseudo-terminal child")
    }

    /// Wait for the child process and return its exit status.
    pub fn wait(&mut self) -> Result<portable_pty::ExitStatus> {
        self.child.wait().context("wait for pseudo-terminal child")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use tempfile::tempdir;

    #[test]
    fn spawns_shell_reads_output_and_accepts_input() {
        let directory = tempdir().unwrap();
        let mut session = PtySession::spawn(directory.path()).unwrap();
        let mut reader = session.reader().unwrap();
        let mut writer = session.writer().unwrap();
        writer.write_all(b"printf 'mew-pty\\n'; exit\n").unwrap();
        writer.flush().unwrap();

        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();
        let status = session.wait().unwrap();
        assert!(status.success());
        assert!(output.contains("mew-pty"), "output was {output:?}");
    }

    #[test]
    fn resize_clamps_zero_dimensions() {
        let directory = tempdir().unwrap();
        let mut session = PtySession::spawn(directory.path()).unwrap();
        session.resize(0, 0).unwrap();
        session.kill().unwrap();
        let _ = session.wait();
    }
}
