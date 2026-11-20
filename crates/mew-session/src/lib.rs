use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::debug;

use mew_message::Message;

mod policy;
pub use policy::SessionPolicy;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("could not confirm the transcript after a write failure: {source}; cleanup failed: {rollback}")]
    TranscriptCommitUnknown {
        source: io::Error,
        rollback: io::Error,
    },
}

/// A transcript message and the submission ID, if it was admitted with a receipt.
#[derive(Deserialize)]
pub struct StoredMessage {
    #[serde(flatten)]
    pub message: Message,
    #[serde(default)]
    pub prompt_request_id: Option<String>,
}

#[derive(Serialize)]
struct StoredPrompt<'a> {
    #[serde(flatten)]
    message: &'a Message,
    prompt_request_id: &'a str,
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
    /// Current context occupancy: the latest provider request's prompt size
    /// in tokens. A snapshot (not cumulative), refreshed on every request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
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
            context_tokens: None,
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
            context_tokens: None,
        }
    }

    /// Update one session's metadata against the latest persisted version.
    pub async fn update(
        root: &Path,
        id: &str,
        change: impl FnOnce(&mut Self) + Send + 'static,
    ) -> Result<Option<Self>, SessionError> {
        Self::update_in(root.join(id), change).await
    }

    async fn update_in(
        directory: PathBuf,
        change: impl FnOnce(&mut Self) + Send + 'static,
    ) -> Result<Option<Self>, SessionError> {
        edit_metadata(directory, false, move |path| {
            let Some(mut meta) = read_metadata(std::fs::read(path))? else {
                return Ok(None);
            };
            change(&mut meta);
            replace_metadata(path, &meta)?;
            Ok(Some(meta))
        })
        .await
    }

    async fn initialize(self, root: &Path) -> Result<Self, SessionError> {
        let directory = self.dir(root);
        edit_metadata(directory, true, move |path| {
            if let Some(existing) = read_metadata(std::fs::read(path))? {
                return Ok(Some(existing));
            }
            replace_metadata(path, &self)?;
            Ok(Some(self))
        })
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound).into())
    }

    /// Read meta.json from `<dir>/<id>/meta.json`. Returns None if not present.
    pub async fn read(dir: &Path, id: &str) -> Result<Option<Self>, SessionError> {
        read_metadata(tokio::fs::read(dir.join(id).join("meta.json")).await)
    }

    /// Write a complete metadata record, creating the session directory if needed.
    pub async fn write(&self, dir: &Path) -> Result<(), SessionError> {
        let directory = self.dir(dir);
        let meta = self.clone();
        edit_metadata(directory, true, move |path| {
            replace_metadata(path, &meta)?;
            Ok(Some(meta))
        })
        .await?;
        Ok(())
    }
}

fn read_metadata(bytes: io::Result<Vec<u8>>) -> Result<Option<Meta>, SessionError> {
    match bytes {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn edit_metadata(
    directory: PathBuf,
    create_directory: bool,
    edit: impl FnOnce(&Path) -> Result<Option<Meta>, SessionError> + Send + 'static,
) -> Result<Option<Meta>, SessionError> {
    tokio::task::spawn_blocking(move || {
        if create_directory {
            std::fs::create_dir_all(&directory)?;
        }
        // A separate file keeps the lock valid when meta.json is replaced.
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("meta.lock"));
        let lock = match lock {
            Ok(file) => file,
            Err(error) if !create_directory && error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        lock.lock()?;
        edit(&directory.join("meta.json"))
    })
    .await
    .map_err(io::Error::other)?
}

fn replace_metadata(path: &Path, meta: &Meta) -> Result<(), SessionError> {
    let directory = path
        .parent()
        .expect("metadata path has a session directory");
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&serde_json::to_vec_pretty(meta)?)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Appends [`Message`] values to a JSONL session file.
pub struct Writer {
    file: BufWriter<tokio::fs::File>,
    path: PathBuf,
    depth: u32,
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

    /// Opens (or creates) a session with the given initial metadata.
    pub async fn open_with_meta(session_id: &str, meta: Meta) -> Result<Self, SessionError> {
        Self::open_at_with_meta(&session_dir(), session_id, meta).await
    }

    /// Opens a session under an explicit root dir with the given initial metadata.
    pub async fn open_at_with_meta(
        root: &Path,
        session_id: &str,
        meta: Meta,
    ) -> Result<Self, SessionError> {
        Self::migrate_if_needed(root, session_id).await?;
        let meta = meta.initialize(root).await?;
        Self::open_log(root.join(session_id).join("session.jsonl"), meta.depth).await
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
        let parent_meta = Meta::new(parent_id).initialize(root).await?;
        let subagents_root = root.join(parent_id).join("subagents");
        let child_meta = Meta::for_subagent(child_id, &parent_meta, subagent_name)
            .initialize(&subagents_root)
            .await?;
        let path = subagents_root.join(child_id).join("session.jsonl");
        let writer = Self::open_log(path, child_meta.depth).await?;

        let child_id = child_id.to_owned();
        Meta::update(root, parent_id, move |meta| {
            if !meta.children_session_ids.contains(&child_id) {
                meta.children_session_ids.push(child_id);
            }
        })
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Ok(writer)
    }

    async fn open_log(path: PathBuf, depth: u32) -> Result<Self, SessionError> {
        let log_path = path.clone();
        let file = tokio::task::spawn_blocking(move || {
            let mut file = OpenOptions::new()
                .read(true)
                .create(true)
                .append(true)
                .open(log_path)?;
            file.lock()?;
            repair_transcript_tail(&mut file)?;
            file.unlock()?;
            Ok::<_, SessionError>(file)
        })
        .await
        .map_err(io::Error::other)??;
        debug!(?path, "opening session file");
        Ok(Self {
            file: BufWriter::new(tokio::fs::File::from_std(file)),
            path,
            depth,
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
            meta.initialize(dir).await?;
            debug!(id = %session_id, "migrated legacy session to folder layout");
        }

        Ok(())
    }

    /// Appends a single message as one JSON line.
    /// Also updates `last_message_at` in the session's meta.json.
    pub async fn write_message(&mut self, msg: &Message) -> Result<(), SessionError> {
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        self.file.write_all(&line).await?;
        self.file.flush().await?;

        self.touch_last_message().await
    }

    /// Commit the user message and its receipt ID together before acknowledging it.
    pub async fn write_prompt(
        &mut self,
        message: &Message,
        request_id: &str,
    ) -> Result<(), SessionError> {
        let record = serde_json::to_vec(&StoredPrompt {
            message,
            prompt_request_id: request_id,
        })?;
        self.commit_record(record).await
    }

    /// Commit a context-reset marker before the agent drops its live context.
    pub async fn write_context_marker(&mut self, marker: &Message) -> Result<(), SessionError> {
        self.commit_record(serde_json::to_vec(marker)?).await
    }

    async fn commit_record(&mut self, mut record: Vec<u8>) -> Result<(), SessionError> {
        record.push(b'\n');
        self.file.flush().await?;

        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let mut transcript = OpenOptions::new().read(true).append(true).open(path)?;
            transcript.lock()?;
            let start = repair_transcript_tail(&mut transcript)?;
            if let Err(source) = transcript
                .write_all(&record)
                .and_then(|()| transcript.sync_data())
            {
                if let Err(rollback) = transcript
                    .set_len(start)
                    .and_then(|()| transcript.sync_data())
                {
                    return Err(SessionError::TranscriptCommitUnknown { source, rollback });
                }
                return Err(source.into());
            }
            Ok::<_, SessionError>(())
        })
        .await
        .map_err(io::Error::other)??;

        if let Err(error) = self.touch_last_message().await {
            tracing::warn!(%error, "could not update metadata after a transcript commit");
        }
        Ok(())
    }

    async fn touch_last_message(&self) -> Result<(), SessionError> {
        let directory = self
            .path
            .parent()
            .expect("session log has a parent directory");
        Meta::update_in(directory.to_path_buf(), |meta| {
            meta.last_message_at = Some(Utc::now().timestamp_millis());
        })
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
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

    /// Reads the latest session metadata.
    pub async fn meta(&self) -> Result<Option<Meta>, SessionError> {
        read_metadata(tokio::fs::read(self.path.with_file_name("meta.json")).await)
    }

    pub fn depth(&self) -> u32 {
        self.depth
    }
}

fn transcript_tail(data: &[u8]) -> &[u8] {
    data.rsplit(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default()
}

fn incomplete_record(record: &[u8]) -> Result<bool, SessionError> {
    let text = match std::str::from_utf8(record) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            let prefix = &record[..error.valid_up_to()];
            if serde_json::from_slice::<Message>(prefix).is_err_and(|error| error.is_eof()) {
                return Ok(true);
            }
            return Err(io::Error::new(io::ErrorKind::InvalidData, error).into());
        }
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error).into()),
    };
    if text.trim().is_empty() {
        return Ok(false);
    }
    match serde_json::from_str::<Message>(text) {
        Ok(_) => Ok(false),
        Err(error) if error.is_eof() => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn repair_transcript_tail(file: &mut File) -> Result<u64, SessionError> {
    let length = file.seek(SeekFrom::End(0))?;
    if length == 0 {
        return Ok(0);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0];
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(length);
    }

    // Only the final record can be torn. Leave the complete prefix unread.
    let mut start = length;
    let mut buffer = [0; 4096];
    while start > 0 {
        let bytes = start.min(buffer.len() as u64) as usize;
        start -= bytes as u64;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..bytes])?;
        if let Some(newline) = buffer[..bytes].iter().rposition(|byte| *byte == b'\n') {
            start += newline as u64 + 1;
            break;
        }
    }

    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    if incomplete_record(&tail)? {
        file.set_len(start)?;
        Ok(start)
    } else {
        file.write_all(b"\n")?;
        Ok(length + 1)
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

/// Exclusive ownership of a top-level session writer.
pub struct WriterLease(File);

impl Drop for WriterLease {
    fn drop(&mut self) {
        // A forked subprocess may briefly hold an inherited copy of the descriptor.
        if let Err(error) = self.0.unlock() {
            tracing::warn!(%error, "could not release session writer lease");
        }
    }
}

/// Try to reserve a session writer until the lease is dropped.
pub async fn try_writer_lease(
    root: &Path,
    session_id: &str,
) -> Result<Option<WriterLease>, SessionError> {
    if !Path::new(session_id)
        .file_name()
        .is_some_and(|name| name == session_id)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid session id").into());
    }

    // A separate directory keeps the lease valid if the transcript is deleted.
    let directory = root.join(".writer-leases");
    let session_id = session_id.to_owned();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&directory)?;
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(session_id))?;
        match lease.try_lock() {
            Ok(()) => Ok(Some(WriterLease(lease))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    })
    .await
    .map_err(io::Error::other)?
}

/// Reads messages and metadata from session files.
pub struct Reader;

/// Decode complete JSONL records, keeping the latest revision of each message
/// at its first chronological position. Any malformed record returns an error.
pub fn decode_messages(data: &str) -> Result<Vec<Message>, SessionError> {
    let mut messages = Vec::new();
    let mut positions = std::collections::HashMap::new();
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let message: Message = serde_json::from_str(line)?;
        if let Some(&position) = positions.get(&message.id) {
            messages[position] = message;
        } else {
            positions.insert(message.id, messages.len());
            messages.push(message);
        }
    }
    Ok(messages)
}

fn decode_transcript(data: &[u8]) -> Result<Vec<Message>, SessionError> {
    let tail = transcript_tail(data);
    let data = if !tail.is_empty() && incomplete_record(tail)? {
        &data[..data.len() - tail.len()]
    } else {
        data
    };
    let text = std::str::from_utf8(data)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    decode_messages(text)
}

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
        let data = tokio::fs::read(&path).await?;
        let messages = decode_transcript(&data)?;
        debug!(session = %session_id, count = messages.len(), "loaded session");
        Ok(messages)
    }

    /// Look up a committed prompt on disk without loading or running its session.
    pub async fn has_prompt_receipt(
        root: &Path,
        session_id: &str,
        request_id: &str,
    ) -> Result<bool, SessionError> {
        let path = root.join(session_id).join("session.jsonl");
        let request_id = request_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let file = File::open(path)?;
            // Read under the same lock the writer uses for append, sync and
            // rollback. A complete line is visible before it is committed.
            file.lock_shared()?;
            let mut reader = BufReader::new(file);
            let mut line = Vec::new();
            while reader.read_until(b'\n', &mut line)? != 0 {
                if std::str::from_utf8(&line).is_ok_and(|text| text.trim().is_empty()) {
                    line.clear();
                    continue;
                }
                if !line.ends_with(b"\n") && incomplete_record(&line)? {
                    break;
                }
                let record = serde_json::from_slice::<StoredMessage>(&line)?;
                if record.prompt_request_id.as_deref() == Some(request_id.as_str())
                    && record.message.role == mew_message::Role::User
                {
                    return Ok(true);
                }
                line.clear();
            }
            Ok(false)
        })
        .await
        .map_err(io::Error::other)?
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
        let data = tokio::fs::read(&path).await?;
        let messages = decode_transcript(&data)?;
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

    #[cfg(unix)]
    #[tokio::test]
    async fn writer_lease_is_released_even_when_its_descriptor_is_duplicated() {
        let root = tempfile::tempdir().unwrap();
        let lease = try_writer_lease(root.path(), "session")
            .await
            .unwrap()
            .unwrap();
        let inherited = lease.0.try_clone().unwrap();
        assert!(try_writer_lease(root.path(), "session")
            .await
            .unwrap()
            .is_none());

        drop(lease);
        assert!(try_writer_lease(root.path(), "session")
            .await
            .unwrap()
            .is_some());
        drop(inherited);
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
    async fn readers_keep_the_latest_message_revision_in_its_original_position() {
        let tmp = tempfile::TempDir::new().unwrap();
        for child in [false, true] {
            let id = Ulid::new().to_string();
            let parent = Ulid::new().to_string();
            let mut writer = if child {
                Writer::open_subagent_at(tmp.path(), &parent, &id, "worker")
                    .await
                    .unwrap()
            } else {
                Writer::open_at(tmp.path(), &id).await.unwrap()
            };
            let original = make_message(Role::Assistant);
            let next = make_message(Role::User);
            let mut latest = original.clone();
            latest.time.completed = Some(42);
            writer.write_message(&original).await.unwrap();
            writer.write_message(&next).await.unwrap();
            writer.write_message(&latest).await.unwrap();
            writer.close().await.unwrap();
            let loaded = if child {
                Reader::load_subagent_from(tmp.path(), &parent, &id)
                    .await
                    .unwrap()
            } else {
                Reader::load_from(tmp.path(), &id).await.unwrap()
            };
            assert_eq!(loaded, vec![latest, next]);
        }
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
    async fn test_metadata_update_reads_latest_workspace_from_open_writer() {
        let root = tmp_root();
        let session_id = format!("test-{}", Ulid::new());
        let writer = Writer::open_at(&root, &session_id).await.expect("open");

        Meta::update(&root, &session_id, |meta| {
            meta.cwd = Some("/tmp/project".into())
        })
        .await
        .expect("set cwd");

        assert_eq!(
            writer.meta().await.unwrap().unwrap().cwd.as_deref(),
            Some("/tmp/project")
        );
        let meta = Reader::load_meta_from(&root, &session_id)
            .await
            .expect("meta")
            .expect("present");
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/project"));

        writer.close().await.expect("close");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_metadata_update_reads_latest_group_from_open_writer() {
        let root = tmp_root();
        let session_id = format!("test-{}", Ulid::new());
        let writer = Writer::open_at(&root, &session_id).await.expect("open");

        Meta::update(&root, &session_id, |meta| {
            meta.group_id = Some("grp_1".into())
        })
        .await
        .expect("set group");

        assert_eq!(
            writer.meta().await.unwrap().unwrap().group_id.as_deref(),
            Some("grp_1")
        );
        let meta = Reader::load_meta_from(&root, &session_id)
            .await
            .expect("meta")
            .expect("present");
        assert_eq!(meta.group_id.as_deref(), Some("grp_1"));

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
        w.write_message(&make_message(Role::User))
            .await
            .expect("write");
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
        let m = p2.meta().await.unwrap().unwrap();
        p2.close().await.expect("close");

        assert!(m.children_session_ids.contains(&child_id));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
