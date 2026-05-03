use mew_message::Message;
use serde_json;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::debug;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Appends [`Message`] values to a JSONL session file.
pub struct Writer {
    file: BufWriter<tokio::fs::File>,
    path: PathBuf,
}

impl Writer {
    /// Opens (or creates) a session file at `sessions/<session_id>.jsonl`.
    pub async fn open(session_id: &str) -> Result<Self, SessionError> {
        let dir = session_dir();
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("{}.jsonl", session_id));
        debug!(?path, "opening session file");

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .write(true)
            .open(&path)
            .await?;

        Ok(Self {
            file: BufWriter::new(file),
            path,
        })
    }

    /// Appends a single message as one JSON line.
    pub async fn write_message(&mut self, msg: &Message) -> Result<(), SessionError> {
        let line = serde_json::to_vec(msg)?;
        self.file.write_all(&line).await?;
        self.file.write_all(b"\n").await?;
        self.file.flush().await?;
        Ok(())
    }

    /// Ensures all buffered writes are persisted to disk.
    pub async fn flush(&mut self) -> Result<(), SessionError> {
        self.file.flush().await?;
        Ok(())
    }

    /// Consumes the writer and flushes/ closes the file.
    pub async fn close(mut self) -> Result<(), SessionError> {
        self.flush().await?;
        // Dropping BufWriter will close the underlying file.
        Ok(())
    }

    /// Returns the path of the session file.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

pub fn session_dir() -> PathBuf {
    directories::ProjectDirs::from("ai", "mew", "mew")
        .map(|d| d.config_dir().join("sessions"))
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config").join("mew").join("sessions"))
                .unwrap_or_else(|| PathBuf::from(".").join(".config").join("mew").join("sessions"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{Message, Role, Time};
    use ulid::Ulid;

    #[tokio::test]
    async fn test_round_trip() {
        let session_id = format!("test-{}", Ulid::new());
        let mut w = Writer::open(&session_id).await.expect("open");

        let msg = Message {
            id: Ulid::new(),
            session_id: Ulid::from_string(&session_id).unwrap_or_else(|_| Ulid::new()),
            role: Role::User,
            parts: vec![],
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };

        w.write_message(&msg).await.expect("write");
        let path = w.path().clone();
        w.close().await.expect("close");

        let data = tokio::fs::read_to_string(&path).await.expect("read");
        let got: Message = serde_json::from_str(data.trim()).expect("parse");
        assert_eq!(got.role, Role::User);

        // cleanup
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn test_multiple_messages() {
        let session_id = format!("test-{}", Ulid::new());
        let mut w = Writer::open(&session_id).await.expect("open");

        let msgs = vec![
            Message {
                id: Ulid::new(),
                session_id: Ulid::from_string(&session_id).unwrap_or_else(|_| Ulid::new()),
                role: Role::User,
                parts: vec![],
                time: Time { created: 0, completed: None },
                assistant: None,
            },
            Message {
                id: Ulid::new(),
                session_id: Ulid::from_string(&session_id).unwrap_or_else(|_| Ulid::new()),
                role: Role::Assistant,
                parts: vec![],
                time: Time { created: 1, completed: Some(2) },
                assistant: None,
            },
        ];

        for msg in &msgs {
            w.write_message(msg).await.expect("write");
        }

        let path = w.path().clone();
        w.close().await.expect("close");

        let data = tokio::fs::read_to_string(&path).await.expect("read");
        let lines: Vec<&str> = data.lines().collect();
        assert_eq!(lines.len(), 2);

        let got0: Message = serde_json::from_str(lines[0]).expect("parse");
        let got1: Message = serde_json::from_str(lines[1]).expect("parse");
        assert_eq!(got0.role, Role::User);
        assert_eq!(got1.role, Role::Assistant);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn test_concurrent_writes() {
        let session_id = format!("test-{}", Ulid::new());
        let mut writer = Writer::open(&session_id).await.expect("open");
        let path = writer.path().clone();

        let mut handles = Vec::new();
        for i in 0..10 {
            let msg = Message {
                id: Ulid::new(),
                session_id: Ulid::from_string(&session_id).unwrap_or_else(|_| Ulid::new()),
                role: Role::User,
                parts: vec![],
                time: Time { created: i, completed: None },
                assistant: None,
            };
            let sid = session_id.clone();
            handles.push(tokio::spawn(async move {
                let mut w = Writer::open(&sid).await.expect("open");
                w.write_message(&msg).await.expect("write");
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        writer.close().await.expect("close");

        let data = tokio::fs::read_to_string(&path).await.expect("read");
        let lines: Vec<&str> = data.lines().collect();
        assert_eq!(lines.len(), 10);

        let _ = tokio::fs::remove_file(&path).await;
    }
}
