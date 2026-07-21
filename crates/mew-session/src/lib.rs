use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::debug;

use mew_message::Message;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Metadata for a session, persisted as `<session_dir>/<id>/meta.json`.
///
/// Top-level user sessions have `parent_session_id = None` and `depth = 0`.
/// Subagent sessions link back to their parent via `parent_session_id` and
/// live one level deeper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub children_session_ids: Vec<String>,
    #[serde(default)]
    pub depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_name: Option<String>,
    #[serde(default = "default_created_at")]
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    /// AI-generated summary of the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Working directory for the session. Set at creation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// True if the last turn ended with an error (provider error or
    /// tool hard-failure). Cleared on the next successful turn.
    #[serde(default)]
    pub last_turn_failed: bool,
    /// True if this session has been archived. Archived sessions are
    /// filtered into a separate view in the UI.
    #[serde(default)]
    pub archived: bool,
    /// ID of the group this session belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// Pinned sessions are exempt from auto-archive.
    #[serde(default)]
    pub pinned: bool,
    /// Cumulative line-level diff stats for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_stats: Option<ChangeStats>,
    /// Cumulative token usage and cost for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
}

/// Cumulative token usage and cost accumulated over a session's lifetime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: f64,
    pub turns: u32,
}

impl SessionUsage {
    /// Add a single message's usage delta.
    pub fn add_message(
        &mut self,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        cost: f64,
    ) {
        self.input_tokens += input;
        self.output_tokens += output;
        self.cache_read_tokens += cache_read;
        self.cache_write_tokens += cache_write;
        self.cost += cost;
    }

    /// Increment the turn counter (called once per completed turn).
    pub fn add_turn(&mut self) {
        self.turns += 1;
    }
}

/// Cumulative diff stats accumulated over a session's lifetime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeStats {
    /// Total lines added across all file writes/edits.
    pub added: u64,
    /// Total lines removed across all file writes/edits.
    pub removed: u64,
    /// Distinct file paths that were modified.
    #[serde(default)]
    pub files: Vec<String>,
}

impl ChangeStats {
    /// Merge a single file delta into the cumulative stats.
    pub fn apply_delta(&mut self, path: &str, added: u64, removed: u64) {
        self.added += added;
        self.removed += removed;
        if !self.files.iter().any(|f| f == path) {
            self.files.push(path.to_string());
        }
    }
}

fn default_created_at() -> i64 {
    Utc::now().timestamp_millis()
}

impl Meta {
    /// Build a fresh meta for a new top-level session.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parent_session_id: None,
            children_session_ids: Vec::new(),
            depth: 0,
            model: None,
            subagent_name: None,
            created_at: default_created_at(),
            last_message_at: None,
            custom_title: None,
            summary: None,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            group_id: None,
            pinned: false,
            change_stats: None,
            usage: None,
        }
    }

    /// Build a meta for a subagent session under `parent`.
    pub fn for_subagent(
        child_id: impl Into<String>,
        parent: &Meta,
        subagent_name: impl Into<String>,
    ) -> Self {
        Self {
            id: child_id.into(),
            parent_session_id: Some(parent.id.clone()),
            children_session_ids: Vec::new(),
            depth: parent.depth + 1,
            model: None,
            subagent_name: Some(subagent_name.into()),
            created_at: default_created_at(),
            last_message_at: None,
            custom_title: None,
            summary: None,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            group_id: None,
            pinned: false,
            change_stats: None,
            usage: None,
        }
    }

    /// Update last_message_at to now and persist to disk.
    pub async fn touch_last_message(&mut self, dir: &Path) -> Result<(), SessionError> {
        self.last_message_at = Some(Utc::now().timestamp_millis());
        self.write(dir).await
    }

    /// Set a custom title and persist to disk.
    pub async fn set_custom_title(
        &mut self,
        dir: &Path,
        title: impl Into<String>,
    ) -> Result<(), SessionError> {
        self.custom_title = Some(title.into());
        self.write(dir).await
    }

    /// Set an AI-generated summary and persist to disk.
    pub async fn set_summary(
        &mut self,
        dir: &Path,
        summary: impl Into<String>,
    ) -> Result<(), SessionError> {
        self.summary = Some(summary.into());
        self.write(dir).await
    }

    /// Set the cwd and persist to disk.
    pub async fn set_cwd(
        &mut self,
        dir: &Path,
        cwd: impl Into<String>,
    ) -> Result<(), SessionError> {
        self.cwd = Some(cwd.into());
        self.write(dir).await
    }

    /// Set the last_turn_failed flag and persist to disk.
    pub async fn set_last_turn_failed(
        &mut self,
        dir: &Path,
        failed: bool,
    ) -> Result<(), SessionError> {
        self.last_turn_failed = failed;
        self.write(dir).await
    }

    /// Set the archived flag and persist to disk.
    pub async fn set_archived(&mut self, dir: &Path, archived: bool) -> Result<(), SessionError> {
        self.archived = archived;
        self.write(dir).await
    }

    /// Set the group_id and persist to disk.
    pub async fn set_group_id(
        &mut self,
        dir: &Path,
        group_id: Option<String>,
    ) -> Result<(), SessionError> {
        self.group_id = group_id;
        self.write(dir).await
    }

    /// Set the pinned flag and persist to disk.
    pub async fn set_pinned(&mut self, dir: &Path, pinned: bool) -> Result<(), SessionError> {
        self.pinned = pinned;
        self.write(dir).await
    }

    /// Apply a file delta to cumulative change stats and persist to disk.
    pub async fn apply_file_delta(
        &mut self,
        dir: &Path,
        path: &str,
        added: u64,
        removed: u64,
    ) -> Result<(), SessionError> {
        let stats = self.change_stats.get_or_insert_with(ChangeStats::default);
        stats.apply_delta(path, added, removed);
        self.write(dir).await
    }

    /// Set the session usage and persist to disk.
    pub async fn set_usage(&mut self, dir: &Path, usage: SessionUsage) -> Result<(), SessionError> {
        self.usage = Some(usage);
        self.write(dir).await
    }

    /// Read meta.json from `<dir>/<id>/meta.json`. Returns None if not present.
    pub async fn read(dir: &Path, id: &str) -> Result<Option<Self>, SessionError> {
        let path = dir.join(id).join("meta.json");
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let meta: Meta = serde_json::from_slice(&bytes)?;
                Ok(Some(meta))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write meta.json to `<dir>/<id>/meta.json` (creates parent dir if needed).
    pub async fn write(&self, dir: &Path) -> Result<(), SessionError> {
        let session_dir = dir.join(&self.id);
        tokio::fs::create_dir_all(&session_dir).await?;
        let path = session_dir.join("meta.json");
        let bytes = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(&path, bytes).await?;
        Ok(())
    }
}

/// Appends [`Message`] values to a JSONL session file.
pub struct Writer {
    file: BufWriter<tokio::fs::File>,
    path: PathBuf,
    meta: Meta,
}

impl Writer {
    /// Opens (or creates) a top-level session at `sessions/<id>/session.jsonl`.
    /// If a legacy flat `<id>.jsonl` exists, it is migrated to the new layout.
    pub async fn open(session_id: &str) -> Result<Self, SessionError> {
        Self::open_at(&session_dir(), session_id).await
    }

    /// Opens (or creates) a top-level session under an explicit root dir.
    /// Useful for tests that need isolation from the global `session_dir()`.
    pub async fn open_at(root: &Path, session_id: &str) -> Result<Self, SessionError> {
        Self::open_at_with_meta(root, session_id, Meta::new(session_id)).await
    }

    /// Opens (or creates) a session, persisting `meta` as its `meta.json`.
    /// If a legacy flat file is found, it is migrated and the supplied meta
    /// is written (overwriting the placeholder from migration).
    pub async fn open_with_meta(session_id: &str, meta: Meta) -> Result<Self, SessionError> {
        Self::open_at_with_meta(&session_dir(), session_id, meta).await
    }

    /// Opens a session under an explicit root dir with the given meta.
    pub async fn open_at_with_meta(
        root: &Path,
        session_id: &str,
        meta: Meta,
    ) -> Result<Self, SessionError> {
        Self::migrate_if_needed(root, session_id).await?;

        let session_dir = root.join(session_id);
        tokio::fs::create_dir_all(&session_dir).await?;

        // Only write meta if it doesn't already exist; migration + reopen
        // would otherwise clobber any updates (e.g. children added later).
        let meta_path = session_dir.join("meta.json");
        if !meta_path.exists() {
            meta.write(root).await?;
        }

        let path = session_dir.join("session.jsonl");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .write(true)
            .open(&path)
            .await?;
        debug!(?path, "opening session file");

        let on_disk_meta = Meta::read(root, session_id)
            .await?
            .unwrap_or_else(|| meta.clone());

        Ok(Self {
            file: BufWriter::new(file),
            path,
            meta: on_disk_meta,
        })
    }

    /// Opens a subagent session at
    /// `sessions/<parent_id>/subagents/<child_id>/session.jsonl` and registers
    /// the child on the parent's `meta.json` (creating it if missing).
    pub async fn open_subagent(
        parent_id: &str,
        child_id: &str,
        subagent_name: &str,
    ) -> Result<Self, SessionError> {
        Self::open_subagent_at(&session_dir(), parent_id, child_id, subagent_name).await
    }

    /// Opens a subagent session under an explicit root dir.
    pub async fn open_subagent_at(
        root: &Path,
        parent_id: &str,
        child_id: &str,
        subagent_name: &str,
    ) -> Result<Self, SessionError> {
        let parent_dir = root.join(parent_id);
        tokio::fs::create_dir_all(&parent_dir).await?;

        let parent_meta = Meta::read(root, parent_id).await?.unwrap_or_else(|| {
            // Parent was never opened as a Writer (e.g. resume-only) — synthesize
            // a meta so the child link is real. depth defaults to 0.
            Meta::new(parent_id)
        });

        let child_meta = Meta::for_subagent(child_id, &parent_meta, subagent_name);
        let subagents_root = parent_dir.join("subagents");
        tokio::fs::create_dir_all(&subagents_root).await?;

        if !subagents_root.join(child_id).join("meta.json").exists() {
            child_meta.write(&subagents_root).await?;
        }

        // Add child to parent's children list (idempotent).
        let mut updated_parent = parent_meta;
        if !updated_parent
            .children_session_ids
            .iter()
            .any(|c| c == child_id)
        {
            updated_parent
                .children_session_ids
                .push(child_id.to_string());
            updated_parent.write(root).await?;
        }

        let path = subagents_root.join(child_id).join("session.jsonl");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .write(true)
            .open(&path)
            .await?;
        debug!(?path, "opening subagent session file");

        let on_disk_meta = Meta::read(&subagents_root, child_id)
            .await?
            .unwrap_or(child_meta);

        Ok(Self {
            file: BufWriter::new(file),
            path,
            meta: on_disk_meta,
        })
    }

    /// If a flat `<id>.jsonl` exists and the new folder layout does not, move
    /// the flat file into the new layout and write a placeholder meta.json.
    async fn migrate_if_needed(dir: &Path, session_id: &str) -> Result<(), SessionError> {
        let legacy = dir.join(format!("{}.jsonl", session_id));
        let new_path = dir.join(session_id).join("session.jsonl");

        if legacy.exists() && !new_path.exists() {
            let new_dir = dir.join(session_id);
            tokio::fs::create_dir_all(&new_dir).await?;
            tokio::fs::rename(&legacy, &new_path).await?;

            let meta = Meta::new(session_id);
            meta.write(dir).await?;
            debug!(id = %session_id, "migrated legacy session to folder layout");
        }

        Ok(())
    }

    /// Appends a single message as one JSON line.
    /// Also updates `last_message_at` in the session's meta.json.
    pub async fn write_message(&mut self, msg: &Message) -> Result<(), SessionError> {
        let line = serde_json::to_vec(msg)?;
        self.file.write_all(&line).await?;
        self.file.write_all(b"\n").await?;
        self.file.flush().await?;

        // Update last_message_at in meta.json.
        let meta_path = self
            .path
            .parent()
            .ok_or_else(|| SessionError::Io(std::io::Error::other("no parent dir")))?
            .join("meta.json");
        self.meta.last_message_at = Some(Utc::now().timestamp_millis());
        let meta_bytes = serde_json::to_vec_pretty(&self.meta)?;
        tokio::fs::write(&meta_path, meta_bytes).await?;

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

    /// Returns the session's metadata.
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// Set and persist the session workspace while keeping the writer's
    /// in-memory metadata in sync with the on-disk record.
    pub async fn set_cwd(
        &mut self,
        dir: &Path,
        cwd: impl Into<String>,
    ) -> Result<(), SessionError> {
        self.meta.set_cwd(dir, cwd).await
    }
}

impl Meta {
    /// Returns the path to this session's folder, given the sessions root dir.
    pub fn dir(&self, root: &Path) -> PathBuf {
        root.join(&self.id)
    }
}

pub fn session_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("MEW_SESSION_DIR") {
        return PathBuf::from(p);
    }
    // Sessions are persistent user data — use the data directory.
    mew_config::data_dir().join("sessions")
}

/// Reads messages and metadata from session files.
pub struct Reader;

impl Reader {
    /// Loads all messages from the session file for the given ID.
    /// Migrates a legacy flat file to the new layout if needed.
    pub async fn load(session_id: &str) -> Result<Vec<Message>, SessionError> {
        Self::load_from(&session_dir(), session_id).await
    }

    /// Loads all messages from the session file under an explicit root dir.
    pub async fn load_from(root: &Path, session_id: &str) -> Result<Vec<Message>, SessionError> {
        Writer::migrate_if_needed(root, session_id).await?;

        let path = root.join(session_id).join("session.jsonl");
        let data = tokio::fs::read_to_string(&path).await?;
        let messages: Vec<Message> = data
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(SessionError::Serialize))
            .collect::<Result<Vec<_>, _>>()?;
        debug!(session = %session_id, count = messages.len(), "loaded session");
        Ok(messages)
    }

    /// Reads the `meta.json` for a session.
    pub async fn load_meta(session_id: &str) -> Result<Option<Meta>, SessionError> {
        Self::load_meta_from(&session_dir(), session_id).await
    }

    /// Reads the `meta.json` for a session under an explicit root dir.
    pub async fn load_meta_from(
        root: &Path,
        session_id: &str,
    ) -> Result<Option<Meta>, SessionError> {
        Writer::migrate_if_needed(root, session_id).await?;
        Meta::read(root, session_id).await
    }

    /// Loads messages from a subagent session.
    pub async fn load_subagent(
        parent_id: &str,
        child_id: &str,
    ) -> Result<Vec<Message>, SessionError> {
        Self::load_subagent_from(&session_dir(), parent_id, child_id).await
    }

    /// Loads messages from a subagent session under an explicit root dir.
    pub async fn load_subagent_from(
        root: &Path,
        parent_id: &str,
        child_id: &str,
    ) -> Result<Vec<Message>, SessionError> {
        let path = root
            .join(parent_id)
            .join("subagents")
            .join(child_id)
            .join("session.jsonl");
        let data = tokio::fs::read_to_string(&path).await?;
        let messages: Vec<Message> = data
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(SessionError::Serialize))
            .collect::<Result<Vec<_>, _>>()?;
        debug!(parent = %parent_id, child = %child_id, count = messages.len(), "loaded subagent session");
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{Message, Role, Time};
    use ulid::Ulid;

    fn tmp_root() -> PathBuf {
        let tmp = tempfile::tempdir().expect("tempdir");
        tmp.keep()
    }

    fn make_message(role: Role) -> Message {
        Message {
            id: Ulid::new(),
            session_id: Ulid::new(),
            role,
            parts: vec![],
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        }
    }

    #[tokio::test]
    async fn test_round_trip() {
        let root = tmp_root();
        let session_id = format!("test-{}", Ulid::new());
        let mut w = Writer::open_at(&root, &session_id).await.expect("open");

        let msg = make_message(Role::User);
        w.write_message(&msg).await.expect("write");
        let path = w.path().clone();
        w.close().await.expect("close");

        let data = tokio::fs::read_to_string(&path).await.expect("read");
        let got: Message = serde_json::from_str(data.trim()).expect("parse");
        assert_eq!(got.role, Role::User);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_meta_written_on_open() {
        let root = tmp_root();
        let session_id = format!("test-{}", Ulid::new());
        let w = Writer::open_at(&root, &session_id).await.expect("open");
        w.close().await.expect("close");

        let meta = Reader::load_meta_from(&root, &session_id)
            .await
            .expect("meta")
            .expect("present");
        assert_eq!(meta.id, session_id);
        assert!(meta.parent_session_id.is_none());
        assert_eq!(meta.depth, 0);
        assert!(meta.children_session_ids.is_empty());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_writer_set_cwd_updates_memory_and_disk() {
        let root = tmp_root();
        let session_id = format!("test-{}", Ulid::new());
        let mut writer = Writer::open_at(&root, &session_id).await.expect("open");

        writer
            .set_cwd(&root, "/tmp/project")
            .await
            .expect("set cwd");

        assert_eq!(writer.meta().cwd.as_deref(), Some("/tmp/project"));
        let meta = Reader::load_meta_from(&root, &session_id)
            .await
            .expect("meta")
            .expect("present");
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/project"));

        writer.close().await.expect("close");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_legacy_flat_file_migrates() {
        let root = tmp_root();
        let session_id = format!("test-{}", Ulid::new());
        let legacy_path = root.join(format!("{}.jsonl", session_id));

        tokio::fs::create_dir_all(&root).await.unwrap();
        let msg = make_message(Role::User);
        let line = serde_json::to_string(&msg).unwrap();
        tokio::fs::write(&legacy_path, format!("{line}\n"))
            .await
            .unwrap();

        let mut w = Writer::open_at(&root, &session_id).await.expect("open");
        w.write_message(&msg).await.expect("write");
        w.close().await.expect("close");

        assert!(!legacy_path.exists(), "legacy file should have been moved");
        let new_path = root.join(&session_id).join("session.jsonl");
        assert!(new_path.exists(), "new layout file should exist");

        let meta_path = root.join(&session_id).join("meta.json");
        assert!(meta_path.exists(), "meta.json should be created");

        let msgs = Reader::load_from(&root, &session_id).await.expect("load");
        assert_eq!(msgs.len(), 2);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_subagent_creates_nested_session() {
        let root = tmp_root();
        let parent_id = format!("test-{}", Ulid::new());
        let child_id = format!("test-{}", Ulid::new());

        let parent = Writer::open_at(&root, &parent_id)
            .await
            .expect("open parent");
        let child = Writer::open_subagent_at(&root, &parent_id, &child_id, "researcher")
            .await
            .expect("open subagent");
        let child_path = child.path().clone();
        drop(child);

        parent.close().await.expect("close parent");
        let parent_meta = Reader::load_meta_from(&root, &parent_id)
            .await
            .expect("meta")
            .expect("present");
        assert!(parent_meta.children_session_ids.contains(&child_id));

        let expected = root
            .join(&parent_id)
            .join("subagents")
            .join(&child_id)
            .join("session.jsonl");
        assert_eq!(child_path, expected);

        let child_meta_path = root
            .join(&parent_id)
            .join("subagents")
            .join(&child_id)
            .join("meta.json");
        let bytes = tokio::fs::read(&child_meta_path)
            .await
            .expect("read child meta");
        let cm: Meta = serde_json::from_slice(&bytes).expect("parse child meta");
        assert_eq!(cm.parent_session_id.as_deref(), Some(parent_id.as_str()));
        assert_eq!(cm.depth, 1);
        assert_eq!(cm.subagent_name.as_deref(), Some("researcher"));

        let mut child_w = Writer::open_subagent_at(&root, &parent_id, &child_id, "researcher")
            .await
            .expect("reopen child");
        child_w
            .write_message(&make_message(Role::Assistant))
            .await
            .unwrap();
        child_w.close().await.unwrap();
        let msgs = Reader::load_subagent_from(&root, &parent_id, &child_id)
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_subagent_parent_meta_survives_reopen() {
        let root = tmp_root();
        let parent_id = format!("test-{}", Ulid::new());
        let child_id = format!("test-{}", Ulid::new());

        {
            let p = Writer::open_at(&root, &parent_id)
                .await
                .expect("open parent");
            p.close().await.expect("close");
            let c = Writer::open_subagent_at(&root, &parent_id, &child_id, "researcher")
                .await
                .expect("open child");
            c.close().await.expect("close child");
        }

        let p2 = Writer::open_at(&root, &parent_id)
            .await
            .expect("reopen parent");
        let m = p2.meta().clone();
        p2.close().await.expect("close");

        assert!(m.children_session_ids.contains(&child_id));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
