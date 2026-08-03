//! Daemon-owned interactive terminal sessions.
//!
//! The PTY itself is synchronous, so this module keeps process ownership on a
//! worker thread and exposes a small command/event boundary to the async
//! WebSocket handler. The protocol carries raw bytes; clients own terminal
//! emulation and presentation.

use anyhow::{Context, Result};
use mew_pty::PtySession;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

const READ_POLL: Duration = Duration::from_millis(25);

#[derive(Debug)]
pub enum TerminalEvent {
    Output(Vec<u8>),
    Exited(String),
    Failed(String),
}

enum TerminalCommand {
    Input(Vec<u8>),
    Resize { rows: u16, cols: u16 },
    Close,
}

/// A handle to one daemon-owned shell process.
pub struct TerminalHandle {
    pub id: String,
    command_tx: Sender<TerminalCommand>,
}

impl TerminalHandle {
    pub fn send_input(&self, bytes: Vec<u8>) -> Result<()> {
        self.command_tx
            .send(TerminalCommand::Input(bytes))
            .context("terminal worker is closed")
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.command_tx
            .send(TerminalCommand::Resize { rows, cols })
            .context("terminal worker is closed")
    }

    pub fn close(&self) {
        let _ = self.command_tx.send(TerminalCommand::Close);
    }
}

/// Spawn a shell in `cwd`, returning its command handle and raw output events.
pub fn spawn(
    cwd: &Path,
    rows: u16,
    cols: u16,
) -> Result<(TerminalHandle, Receiver<TerminalEvent>)> {
    let id = ulid::Ulid::new().to_string();
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    let cwd = cwd.to_owned();

    thread::Builder::new()
        .name("mew-daemon-terminal".into())
        .spawn(move || run_worker(cwd, rows, cols, command_rx, event_tx))
        .context("spawn terminal worker")?;

    Ok((TerminalHandle { id, command_tx }, event_rx))
}

fn run_worker(
    cwd: impl AsRef<Path>,
    rows: u16,
    cols: u16,
    command_rx: Receiver<TerminalCommand>,
    event_tx: Sender<TerminalEvent>,
) {
    let mut session = match PtySession::spawn(cwd.as_ref()) {
        Ok(session) => session,
        Err(error) => {
            let _ = event_tx.send(TerminalEvent::Failed(error.to_string()));
            return;
        }
    };
    if let Err(error) = session.resize(rows, cols) {
        let _ = event_tx.send(TerminalEvent::Failed(error.to_string()));
        let _ = session.kill();
        let _ = session.wait();
        return;
    }

    let mut writer = match session.writer() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = event_tx.send(TerminalEvent::Failed(error.to_string()));
            let _ = session.kill();
            let _ = session.wait();
            return;
        }
    };
    let mut reader = match session.reader() {
        Ok(reader) => reader,
        Err(error) => {
            let _ = event_tx.send(TerminalEvent::Failed(error.to_string()));
            let _ = session.kill();
            let _ = session.wait();
            return;
        }
    };

    let (read_tx, read_rx) = mpsc::channel();
    if thread::Builder::new()
        .name("mew-daemon-terminal-reader".into())
        .spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => {
                        let _ = read_tx.send(None);
                        break;
                    }
                    Ok(count) => {
                        if read_tx.send(Some(buffer[..count].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        })
        .is_err()
    {
        let _ = event_tx.send(TerminalEvent::Failed("spawn terminal reader".into()));
        let _ = session.kill();
        let status = session
            .wait()
            .map(|status| status.to_string())
            .unwrap_or_else(|_| "exited".into());
        let _ = event_tx.send(TerminalEvent::Exited(status));
        return;
    }

    let mut exited = false;
    let mut should_kill = false;
    while !exited {
        match command_rx.recv_timeout(READ_POLL) {
            Ok(TerminalCommand::Input(bytes)) => {
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    let _ =
                        event_tx.send(TerminalEvent::Failed("terminal input stream closed".into()));
                    should_kill = true;
                    break;
                }
            }
            Ok(TerminalCommand::Resize { rows, cols }) => {
                if let Err(error) = session.resize(rows, cols) {
                    let _ = event_tx.send(TerminalEvent::Failed(error.to_string()));
                }
            }
            Ok(TerminalCommand::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                should_kill = true;
                exited = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        loop {
            match read_rx.try_recv() {
                Ok(Some(bytes)) => {
                    if event_tx.send(TerminalEvent::Output(bytes)).is_err() {
                        should_kill = true;
                        exited = true;
                        break;
                    }
                }
                Ok(None) => {
                    exited = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    should_kill = true;
                    exited = true;
                    break;
                }
            }
        }
    }

    if should_kill {
        let _ = session.kill();
    }
    let status = session
        .wait()
        .map(|status| status.to_string())
        .unwrap_or_else(|_| "exited".into());
    let _ = event_tx.send(TerminalEvent::Exited(status));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn daemon_terminal_round_trips_shell_bytes() {
        let directory = tempdir().unwrap();
        let (terminal, events) = spawn(directory.path(), 24, 80).unwrap();
        terminal
            .send_input(b"printf 'daemon-pty\\n'; exit\n".to_vec())
            .unwrap();

        let mut output = Vec::new();
        loop {
            match events.recv_timeout(Duration::from_secs(3)).unwrap() {
                TerminalEvent::Output(bytes) => output.extend(bytes),
                TerminalEvent::Exited(_) => break,
                TerminalEvent::Failed(error) => panic!("terminal failed: {error}"),
            }
        }
        assert!(String::from_utf8_lossy(&output).contains("daemon-pty"));
    }

    #[test]
    fn daemon_terminal_resize_and_close_are_safe() {
        let directory = tempdir().unwrap();
        let (terminal, events) = spawn(directory.path(), 0, 0).unwrap();
        terminal.resize(0, 0).unwrap();
        terminal.close();

        loop {
            match events.recv_timeout(Duration::from_secs(3)).unwrap() {
                TerminalEvent::Exited(_) => break,
                TerminalEvent::Output(_) => {}
                TerminalEvent::Failed(error) => panic!("terminal failed: {error}"),
            }
        }
    }
}
