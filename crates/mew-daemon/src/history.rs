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
    /// Whether this message opens a turn, recorded during the scan so page
    /// selection can avoid splitting a turn without re-reading the log.
    starts_turn: bool,
}

/// Whether a message opens a turn. Mirrors the transcript fold clients do: a
/// prompt with something to render starts a turn, while tool results (user
/// messages whose parts clients drop) stay inside the turn that produced them.
fn starts_turn(message: &Message) -> bool {
    if message.role == mew_message::Role::Assistant {
        return false;
    }
    message.parts.iter().any(|part| match part {
        mew_message::Part::Text(text) => !text.text.trim().is_empty(),
        mew_message::Part::Reasoning(reasoning) => !reasoning.text.trim().is_empty(),
        mew_message::Part::ToolResult(_) => false,
        _ => true,
    })
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
                    starts_turn: starts_turn(&stored.message),
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
    /// Where a page that ends at `end` should begin. A page that starts inside a
    /// turn splits a fold: the row the reader already sees is re-labeled when
    /// the turn's earlier messages page in, so extend back to the turn's first
    /// message. `floor` bounds the extension, keeping a pathological turn from
    /// making a page unbounded.
    fn page_start(&self, first: usize, end: usize, floor: usize) -> usize {
        let floor = end.saturating_sub(floor);
        let mut position = first;
        loop {
            if self.starts_turn_at(position) {
                return position;
            }
            // No turn boundary inside the bound: keep the requested window
            // rather than inventing a start the transcript does not have.
            if position == floor || position == 0 {
                return first;
            }
            position -= 1;
        }
    }

    fn starts_turn_at(&self, position: usize) -> bool {
        match self.index.rows.get(position) {
            Some(row) => row.starts_turn,
            None => self
                .pending
                .get(position.saturating_sub(self.index.rows.len()))
                .is_some_and(starts_turn),
        }
    }

    pub(crate) async fn page(
        &mut self,
        before: usize,
        limit: usize,
    ) -> Result<(Vec<Message>, Option<usize>)> {
        let end = before.min(self.index.rows.len() + self.pending.len());
        let limit = limit.max(1);
        let first_candidate = self.page_start(end.saturating_sub(limit), end, limit * 4);
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
                // The byte cap is the only thing that can still cut a turn, and
                // it sits far above any page a normal turn produces.
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
