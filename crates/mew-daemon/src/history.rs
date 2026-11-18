//! Byte ranges into the append-only session log. The latest revision keeps
//! the original message's position, so a cursor stays valid across appends.

use std::collections::{HashMap, HashSet};
use std::fs::Metadata;
use std::io::SeekFrom;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{ensure, Context, Result};
use mew_message::{Message, MessageId};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::sync::Mutex;

#[derive(Clone, Copy)]
struct Record {
    id: MessageId,
    offset: u64,
    len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileVersion {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl FileVersion {
    fn new(meta: &Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(unix)]
            device: meta.dev(),
            #[cfg(unix)]
            inode: meta.ino(),
        }
    }

    fn can_extend(&self, previous: &Self) -> bool {
        #[cfg(unix)]
        if self.device != previous.device || self.inode != previous.inode {
            return false;
        }

        self.len > previous.len
    }
}

#[derive(Clone, Default)]
pub(crate) struct HistoryIndex {
    version: Option<FileVersion>,
    offset: u64,
    rows: Vec<Record>,
    positions: HashMap<MessageId, usize>,
}

impl HistoryIndex {
    pub(crate) async fn load(file: &mut File) -> Result<(Self, Vec<Message>, HashSet<String>)> {
        let metadata = file.metadata().await?;
        let mut index = Self::default();
        let mut messages = Vec::new();
        let mut accepted_requests = HashSet::new();
        index
            .scan(file, &metadata, |position, stored| {
                if let Some(request_id) = stored.prompt_request_id {
                    accepted_requests.insert(request_id);
                }
                if position == messages.len() {
                    messages.push(stored.message);
                } else {
                    messages[position] = stored.message;
                }
            })
            .await?;
        Ok((index, messages, accepted_requests))
    }

    pub(crate) async fn refresh(
        index: &mut Arc<Self>,
        file: &mut File,
        metadata: &Metadata,
    ) -> Result<()> {
        let version = FileVersion::new(metadata);
        if index.version.as_ref() == Some(&version) {
            return Ok(());
        }
        if !index
            .version
            .as_ref()
            .is_some_and(|previous| version.can_extend(previous))
        {
            *index = Arc::new(Self::default());
        }
        Arc::make_mut(index).scan(file, metadata, |_, _| {}).await
    }

    async fn scan(
        &mut self,
        file: &mut File,
        metadata: &Metadata,
        mut on_record: impl FnMut(usize, mew_session::StoredMessage),
    ) -> Result<()> {
        let version = FileVersion::new(metadata);
        file.seek(SeekFrom::Start(self.offset)).await?;
        let mut input = BufReader::new(file.take(version.len - self.offset));
        let mut line = Vec::new();
        loop {
            line.clear();
            let len = input.read_until(b'\n', &mut line).await?;
            if len == 0 {
                ensure!(
                    self.offset == version.len,
                    "session transcript was truncated"
                );
                break;
            }
            let complete = line.last() == Some(&b'\n');
            let text = match std::str::from_utf8(&line) {
                Ok(text) => text,
                Err(error) if !complete && error.error_len().is_none() => break,
                Err(error) => return Err(error.into()),
            };
            if !text.trim().is_empty() {
                let stored = match serde_json::from_str::<mew_session::StoredMessage>(text) {
                    Ok(stored) => stored,
                    Err(error) if !complete && error.is_eof() => break,
                    Err(error) => return Err(error.into()),
                };
                let id = stored.message.id;
                let record = Record {
                    id,
                    offset: self.offset,
                    len,
                };
                let position = *self.positions.entry(id).or_insert(self.rows.len());
                if position == self.rows.len() {
                    self.rows.push(record);
                } else {
                    self.rows[position] = record;
                }
                on_record(position, stored);
            }
            self.offset += len as u64;
        }
        self.version = Some(version);
        Ok(())
    }
}

/// A fixed index and open log descriptor for one attachment or history request.
/// Revisions from the live model context are resolved by ID before disk reads.
pub(crate) struct HistorySnapshot {
    file: Option<BufReader<File>>,
    offset: Option<u64>,
    index: Arc<HistoryIndex>,
    live: Arc<Mutex<Vec<Message>>>,
    live_positions: HashMap<MessageId, usize>,
    pending: Vec<Message>,
}

impl HistorySnapshot {
    pub(crate) async fn new(
        file: Option<File>,
        index: Arc<HistoryIndex>,
        live: Arc<Mutex<Vec<Message>>>,
    ) -> Self {
        let mut live_positions = HashMap::new();
        let mut pending = Vec::new();
        for (position, message) in live.lock().await.iter().enumerate() {
            if index.positions.contains_key(&message.id) {
                live_positions.insert(message.id, position);
            } else {
                pending.push(message.clone());
            }
        }
        Self {
            file: file.map(BufReader::new),
            offset: None,
            index,
            live,
            live_positions,
            pending,
        }
    }

    pub(crate) async fn page(
        &mut self,
        before: usize,
        limit: usize,
    ) -> Result<(Vec<Message>, Option<usize>)> {
        let end = before.min(self.index.rows.len() + self.pending.len());
        let first_candidate = end.saturating_sub(limit.max(1));
        let mut selected = Vec::with_capacity(end - first_candidate);
        let mut bytes: usize = 0;
        {
            let live = self.live.lock().await;
            for position in (first_candidate..end).rev() {
                let row = self.index.rows.get(position).copied();
                let revision = match row {
                    Some(row) => self
                        .live_positions
                        .get(&row.id)
                        .and_then(|&position| live.get(position))
                        .filter(|message| message.id == row.id),
                    None => Some(&self.pending[position - self.index.rows.len()]),
                };
                let len = match revision {
                    Some(message) => serde_json::to_vec(message)?.len(),
                    None => row.expect("missing disk row").len,
                };
                if !selected.is_empty() && bytes.saturating_add(len) > crate::HISTORY_PAGE_MAX_BYTES
                {
                    break;
                }
                bytes = bytes.saturating_add(len);
                selected.push((row, revision.cloned()));
            }
        }

        let start = end - selected.len();
        let mut messages = Vec::with_capacity(selected.len());
        let mut buffer = Vec::new();
        for (row, revision) in selected.into_iter().rev() {
            if let Some(message) = revision {
                messages.push(message);
                continue;
            }
            let row = row.expect("selected disk row");
            let file = self.file.as_mut().context("session transcript not open")?;
            if self.offset != Some(row.offset) {
                file.seek(SeekFrom::Start(row.offset)).await?;
            }
            buffer.resize(row.len, 0);
            file.read_exact(&mut buffer).await?;
            self.offset = Some(row.offset + row.len as u64);
            let message: Message = serde_json::from_slice(&buffer)?;
            ensure!(
                message.id == row.id,
                "session transcript changed while reading a page"
            );
            messages.push(message);
        }
        Ok((messages, (start > 0).then_some(start)))
    }
}
