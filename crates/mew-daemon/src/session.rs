//! Session manager and shared session state for the daemon.
//!
//! A session is owned by the daemon, not by a connection. Multiple clients can
//! attach to the same session; events are broadcast to all attached clients.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use mew_agent::{Agent, GoalDecision};
use mew_hooks::PermissionDecision;
use mew_message::{Message, ProviderEventWire};
use mew_protocol::{ServerMessage, SessionInfo, SessionState};

use crate::history::{HistoryIndex, HistorySnapshot};
use crate::projects::ProjectPinsStore;

pub struct PendingRequest<T> {
    pub sender: oneshot::Sender<T>,
    pub message: ServerMessage,
}
use crate::{AgentBuildParams, AgentBuilder, ClientSender, ModelLister, ModelSwitcher};

/// Error returned when attaching to a session fails.
#[derive(Debug)]
pub enum AttachError {
    NotFound,
    NotTopLevel,
    InUse,
    BuildAgent(anyhow::Error),
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachError::NotFound => write!(f, "session not found"),
            AttachError::NotTopLevel => write!(f, "only top-level sessions can be attached"),
            AttachError::InUse => write!(f, "session is open in another daemon"),
            AttachError::BuildAgent(e) => write!(f, "failed to build agent: {e}"),
        }
    }
}

impl std::error::Error for AttachError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AttachError::BuildAgent(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for AttachError {
    fn from(e: anyhow::Error) -> Self {
        AttachError::BuildAgent(e)
    }
}

/// One active session. Shared between all attached client connections.
pub struct Session {
    pub id: String,
    /// The single agent for this session. Cloned for each turn; mutated in
    /// place for model switches.
    pub agent: Mutex<Agent>,
    history_index: Mutex<Arc<HistoryIndex>>,
    pub browser_access: mew_tools::browser::BrowserAccess,
    /// Ensures only one turn or model switch runs at a time.
    pub turn_lock: Mutex<()>,
    pub(crate) provider_replay: Mutex<ProviderReplay>,
    pub(crate) activity: Mutex<crate::activity::SessionActivity>,
    pub(crate) job_observers: Mutex<HashMap<String, tokio::task::AbortHandle>>,
    pub(crate) prompt_receipts:
        Mutex<HashMap<String, (mew_protocol::PromptStatus, Option<String>)>>,
    /// Broadcast targets: (client_id, sender, kind) per attached client.
    pub clients: Mutex<Vec<(u64, ClientSender, mew_protocol::ClientKind)>>,
    pub(crate) last_usage: Mutex<Option<mew_protocol::SessionUsageWire>>,
    pub(crate) last_context_tokens: Mutex<Option<u64>>,
    /// Pending permission / workspace-permission / subagent-permission requests.
    pub pending_permissions: Mutex<HashMap<String, PendingRequest<PermissionDecision>>>,
    /// Pending ask-user requests.
    pub pending_ask_user: Mutex<HashMap<String, PendingRequest<Vec<String>>>>,
    /// Pending goal-proposal requests (from `propose_goal`).
    pub pending_goal_proposals: Mutex<HashMap<String, PendingRequest<GoalDecision>>>,
    /// Serializes request publication, attach replay, responses and cancellation.
    pub(crate) pending_action_lock: Mutex<()>,
    /// Monotonically increasing IDs for both clients and permission requests.
    pub next_id: AtomicU64,
    /// Token for the turn currently in progress, if any.
    pub current_turn_cancel: Mutex<Option<CancellationToken>>,
    /// Current model/provider display IDs for SessionReady.
    pub model: Mutex<Option<String>>,
    pub provider: Mutex<Option<String>>,
    /// Whether a title has been generated for this session.
    pub title_generated: Mutex<bool>,
    /// True when a turn is in progress. Used for the session-rail running indicator.
    pub is_running: Mutex<bool>,
    /// Prevent duplicate background summary requests for one session.
    pub summary_running: AtomicBool,
    /// Sessions root directory (for meta persistence).
    pub session_dir: PathBuf,
    // Keep the lease until the runtime and its current operations release it.
    writer_lease: Option<mew_session::WriterLease>,
}

impl Session {
    pub(crate) async fn subagent_transcript_root(&self, child_session_id: &str) -> Result<PathBuf> {
        anyhow::ensure!(
            ulid::Ulid::from_string(child_session_id)
                .is_ok_and(|id| id.to_string() == child_session_id),
            "invalid child session ID"
        );
        let recorded_parent = self.agent.lock().await.session_id.to_string();
        let mut parent = None;
        // Both parent folder conventions require recorded child membership.
        for id in [recorded_parent, self.id.clone()] {
            let meta = mew_session::Meta::read(&self.session_dir, &id)
                .await
                .context("read parent session metadata")?;
            if meta.as_ref().is_some_and(|meta| {
                meta.id == id
                    && meta
                        .children_session_ids
                        .iter()
                        .any(|child| child == child_session_id)
            }) {
                parent = Some(id);
                break;
            }
        }
        let parent = parent.context("not a registered child of the attached parent")?;
        let children = self.session_dir.join(&parent).join("subagents");
        let meta = mew_session::Meta::read(&children, child_session_id)
            .await
            .context("read child session metadata")?
            .context("missing child session metadata")?;
        anyhow::ensure!(
            meta.id == child_session_id && meta.parent_session_id.as_deref() == Some(&parent),
            "child metadata does not match the attached parent"
        );
        Ok(children)
    }

    pub(crate) async fn subagent_history(
        &self,
        child_session_id: &str,
    ) -> Result<Vec<mew_message::Message>> {
        let children = self.subagent_transcript_root(child_session_id).await?;
        let mut transcript =
            tokio::fs::File::open(children.join(child_session_id).join("session.jsonl"))
                .await
                .context("read child transcript")?
                .take((crate::HISTORY_PAGE_MAX_BYTES + 1) as u64);
        let mut data = Vec::new();
        transcript
            .read_to_end(&mut data)
            .await
            .context("read child transcript")?;
        anyhow::ensure!(
            data.len() <= crate::HISTORY_PAGE_MAX_BYTES,
            "worker history exceeds the response size limit (8 MiB)"
        );
        // A live writer can expose an incomplete final append before its newline.
        let last_record = data
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let complete = if last_record < data.len()
            && serde_json::from_slice::<serde::de::IgnoredAny>(&data[last_record..])
                .is_err_and(|error| error.is_eof())
        {
            &data[..last_record]
        } else {
            &data
        };
        let complete = std::str::from_utf8(complete).context("invalid child transcript record")?;
        mew_session::decode_messages(complete).context("invalid child transcript record")
    }

    pub(crate) async fn update_todo(
        &self,
        edit: mew_protocol::TodoEdit,
    ) -> Result<Vec<mew_protocol::Todo>> {
        let status = mew_agent::TodoStatus::parse(&edit.status).context("invalid todo status")?;
        let expected_status = mew_agent::TodoStatus::parse(&edit.expected_status)
            .context("invalid original todo status")?;
        anyhow::ensure!(
            !edit.content.trim().is_empty(),
            "todo content cannot be empty"
        );
        let (todos, path) = {
            let agent = self.agent.lock().await;
            (agent.todos.clone(), agent.todos_path.clone())
        };
        let path = path.context("session cannot persist todo edits")?;
        let mut list = todos.lock().await;
        let original = list.get(edit.todo_id).context("todo no longer exists")?;
        anyhow::ensure!(
            original.content == edit.expected_content && original.status == expected_status,
            "todo changed while editing; reopen it before saving"
        );
        let mut updated = list.clone();
        updated
            .update(edit.todo_id, Some(edit.content), Some(status))
            .map_err(anyhow::Error::msg)?;
        updated
            .save(&path)
            .await
            .map_err(anyhow::Error::msg)
            .context("could not persist todo edit")?;
        *list = updated;
        let snapshot = wire_todos(&list.items);
        self.broadcast(ServerMessage::TodosUpdated {
            todos: snapshot.clone(),
        })
        .await;
        Ok(snapshot)
    }

    /// Delayed agent events publish the latest list, never an older tool snapshot.
    pub(crate) async fn broadcast_todos(&self) {
        let todos = self.agent.lock().await.todos.clone();
        let list = todos.lock().await;
        self.broadcast(ServerMessage::TodosUpdated {
            todos: wire_todos(&list.items),
        })
        .await;
    }

    pub(crate) async fn send_todos(&self, sender: &ClientSender) {
        let todos = self.agent.lock().await.todos.clone();
        let list = todos.lock().await;
        let _ = sender.send(ServerMessage::TodosUpdated {
            todos: wire_todos(&list.items),
        });
    }
    /// Snapshot transcript positions through the current stable log boundary.
    pub(crate) async fn history(&self) -> Result<HistorySnapshot> {
        let (writer, messages) = {
            let agent = self.agent.lock().await;
            (agent.session.clone(), agent.messages.clone())
        };
        let Some(writer) = writer else {
            return Ok(HistorySnapshot::new(None, Arc::default(), messages).await);
        };

        let mut index = self.history_index.lock().await;
        let (mut file, metadata) = {
            let writer = writer.lock().await;
            let file = tokio::fs::File::open(writer.path())
                .await
                .context("open session transcript")?;
            let metadata = file.metadata().await.context("stat session transcript")?;
            (file, metadata)
        };
        HistoryIndex::refresh(&mut index, &mut file, &metadata)
            .await
            .context("read session transcript")?;
        let snapshot = index.clone();
        drop(index);
        Ok(HistorySnapshot::new(Some(file), snapshot, messages).await)
    }

    pub(crate) async fn policy(&self, agent: &Agent) -> mew_session::SessionPolicy {
        mew_session::SessionPolicy {
            model: self.model.lock().await.clone(),
            provider: self.provider.lock().await.clone(),
            persona: agent.persona_name.clone(),
            thinking_variant: agent.active_thinking_variant().map(str::to_owned),
            permission_mode: agent.permission_mode().id().to_owned(),
        }
    }

    pub(crate) async fn save_policy(&self, policy: &mew_session::SessionPolicy) -> Result<()> {
        policy
            .write(&self.session_dir, &self.id)
            .await
            .context("save session policy")
    }

    pub fn new(
        id: String,
        agent: Agent,
        model: Option<String>,
        provider: Option<String>,
        session_dir: PathBuf,
    ) -> Self {
        Self {
            id,
            browser_access: agent.browser_access.clone(),
            agent: Mutex::new(agent),
            history_index: Mutex::default(),
            turn_lock: Mutex::new(()),
            provider_replay: Mutex::new(ProviderReplay::default()),
            activity: Mutex::new(crate::activity::SessionActivity::default()),
            job_observers: Mutex::new(HashMap::new()),
            prompt_receipts: Mutex::new(HashMap::new()),
            clients: Mutex::new(Vec::new()),
            last_usage: Mutex::new(None),
            last_context_tokens: Mutex::new(None),
            pending_permissions: Mutex::new(HashMap::new()),
            pending_ask_user: Mutex::new(HashMap::new()),
            pending_goal_proposals: Mutex::new(HashMap::new()),
            pending_action_lock: Mutex::new(()),
            next_id: AtomicU64::new(1),
            current_turn_cancel: Mutex::new(None),
            model: Mutex::new(model),
            provider: Mutex::new(provider),
            title_generated: Mutex::new(false),
            is_running: Mutex::new(false),
            summary_running: AtomicBool::new(false),
            session_dir,
            writer_lease: None,
        }
    }

    /// Get a display title for this session (custom title > summary > id).
    pub async fn display_title(&self) -> String {
        let agent = self.agent.lock().await;
        if let Some(meta) = agent.session_meta().await {
            if let Some(title) = &meta.custom_title {
                return title.clone();
            }
            if let Some(summary) = &meta.summary {
                return summary.clone();
            }
        }
        self.id.clone()
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn next_request_id(&self) -> String {
        ulid::Ulid::new().to_string()
    }

    /// Attach a client sender. Returns (client_id, was_first_client).
    pub async fn attach_client(
        &self,
        sender: ClientSender,
        client_kind: mew_protocol::ClientKind,
    ) -> (u64, bool) {
        let mut clients = self.clients.lock().await;
        let was_first = clients.is_empty();
        let client_id = self.next_id();
        clients.push((client_id, sender, client_kind));
        (client_id, was_first)
    }

    /// Confirm ownership before exposing the client to implicit session events.
    pub(crate) async fn attach_client_ready(
        &self,
        sender: ClientSender,
        client_kind: mew_protocol::ClientKind,
        ready: ServerMessage,
    ) -> (u64, bool) {
        let mut clients = self.clients.lock().await;
        let was_first = clients.is_empty();
        let client_id = self.next_id();
        let _ = sender.send(ready);
        clients.push((client_id, sender, client_kind));
        (client_id, was_first)
    }

    /// Detach a client by id. Returns true if this was the last client.
    pub async fn detach_client(&self, client_id: u64) -> bool {
        let mut clients = self.clients.lock().await;
        clients.retain(|(id, _, _)| *id != client_id);
        clients.is_empty()
    }

    pub async fn client_count(&self) -> usize {
        self.clients.lock().await.len()
    }

    async fn has_running_jobs(&self) -> bool {
        let agent = self.agent.lock().await;
        let jobs = agent.shell_jobs.lock().await;
        for job in jobs.values() {
            if !job.state.lock().await.is_terminal() {
                return true;
            }
        }
        false
    }

    /// Return the clients already attached to this session, excluding one
    /// connection that is about to receive the snapshot directly.
    pub async fn client_presence_except(
        &self,
        excluded_client_id: u64,
    ) -> Vec<(u64, mew_protocol::ClientKind)> {
        self.clients
            .lock()
            .await
            .iter()
            .filter(|(client_id, _, _)| *client_id != excluded_client_id)
            .map(|(client_id, _, client_kind)| (*client_id, *client_kind))
            .collect()
    }

    /// Notify the other attached clients that a connection joined.
    pub(crate) async fn notify_client_attached(
        &self,
        client_id: u64,
        client_kind: mew_protocol::ClientKind,
    ) {
        self.clients.lock().await.retain(|(id, sender, _)| {
            *id == client_id
                || sender
                    .send(ServerMessage::ClientAttached {
                        client_id,
                        client_kind,
                    })
                    .is_ok()
        });
    }

    /// Broadcast a message to all attached clients. Removes any sender that fails.
    pub async fn broadcast(&self, mut msg: ServerMessage) {
        if let ServerMessage::Provider {
            event: mew_message::ProviderEventWire::MessageEnd { usage, cost, .. },
        } = &msg
        {
            let mut cached = self.last_usage.lock().await;
            let totals = cached.get_or_insert_with(Default::default);
            totals.input_tokens = totals.input_tokens.saturating_add(usage.input as u64);
            totals.output_tokens = totals.output_tokens.saturating_add(usage.output as u64);
            totals.cache_read_tokens = totals
                .cache_read_tokens
                .saturating_add(usage.cache_read as u64);
            totals.cache_write_tokens = totals
                .cache_write_tokens
                .saturating_add(usage.cache_write as u64);
            totals.cost += cost;
            if usage.input > 0 {
                *self.last_context_tokens.lock().await = Some(usage.input as u64);
            }
        }
        if let ServerMessage::SessionUsageChanged {
            session_id,
            usage,
            context_tokens,
        } = &msg
        {
            if session_id == &self.id {
                *self.last_usage.lock().await = Some(usage.clone());
                *self.last_context_tokens.lock().await = *context_tokens;
            }
        }
        let _provider_publication = match &msg {
            ServerMessage::Provider { event } => {
                let mut replay = self.provider_replay.lock().await;
                replay.record(event);
                Some(replay)
            }
            ServerMessage::NestedToolStart { .. } | ServerMessage::NestedToolEnd { .. } => {
                let mut replay = self.provider_replay.lock().await;
                replay.nested.push(msg.clone());
                Some(replay)
            }
            ServerMessage::PartUpdated {
                part_id,
                part: mew_message::Part::ToolCall(_),
            } => {
                let mut replay = self.provider_replay.lock().await;
                if let Some(existing) = replay.tool_updates.iter_mut().find(|update| {
                    matches!(update, ServerMessage::PartUpdated { part_id: id, .. } if id == part_id)
                }) {
                    *existing = msg.clone();
                } else {
                    replay.tool_updates.push(msg.clone());
                }
                Some(replay)
            }
            ServerMessage::ErrorEvent { .. } => {
                let mut replay = self.provider_replay.lock().await;
                replay.clear();
                Some(replay)
            }
            _ => None,
        };
        let mut activity = if matches!(
            &msg,
            ServerMessage::SubagentStart { .. }
                | ServerMessage::SubagentStatus { .. }
                | ServerMessage::SubagentEnd { .. }
                | ServerMessage::JobUpdate { .. }
        ) {
            Some(self.activity.lock().await)
        } else {
            None
        };
        if let Some(activity) = activity.as_mut() {
            if !activity.apply(&mut msg) {
                return;
            }
        }
        let mut clients = self.clients.lock().await;
        clients.retain(|(_, sender, _)| sender.send_ref(&msg).is_ok());
    }

    /// Cancel the current turn, if any. Also drains pending requests so the
    /// agent loop unblocks if it was waiting on a permission/ask-user oneshot.
    pub async fn cancel_turn(&self) {
        let _actions = self.pending_action_lock.lock().await;
        if let Some(token) = self.current_turn_cancel.lock().await.as_ref() {
            token.cancel();
        }
        let permissions = std::mem::take(&mut *self.pending_permissions.lock().await);
        let questions = std::mem::take(&mut *self.pending_ask_user.lock().await);
        let goals = std::mem::take(&mut *self.pending_goal_proposals.lock().await);
        for request_id in permissions
            .keys()
            .chain(questions.keys())
            .chain(goals.keys())
        {
            self.broadcast(ServerMessage::RequestResolved {
                request_id: request_id.clone(),
            })
            .await;
        }
        self.broadcast(ServerMessage::SessionAttentionChanged {
            session_id: self.id.clone(),
            pending_permissions: 0,
            pending_questions: 0,
        })
        .await;
    }

    /// Count of pending requests that need a user answer: `ask_user_question`,
    /// and `propose_goal` approvals. Both surface as
    /// "questions" in the session attention badge.
    pub async fn pending_questions_count(&self) -> u32 {
        let asks = self.pending_ask_user.lock().await.len();
        let goals = self.pending_goal_proposals.lock().await.len();
        (asks + goals) as u32
    }
}

/// The events needed to render the current provider response from its start.
/// Drop the replay once that response is available through ordinary history.
#[derive(Default)]
pub(crate) struct ProviderReplay {
    provider: Vec<ProviderEventWire>,
    tool_updates: Vec<ServerMessage>,
    nested: Vec<ServerMessage>,
}

impl ProviderReplay {
    fn record(&mut self, event: &ProviderEventWire) {
        match event {
            ProviderEventWire::MessageEnd { .. } | ProviderEventWire::Error(_) => {
                self.provider.clear()
            }
            ProviderEventWire::RetryWait { .. } => {}
            ProviderEventWire::PartStart { .. } if self.provider.is_empty() => {
                // The previous tool batch is already in session history.
                self.tool_updates.clear();
                self.nested.clear();
                self.provider.push(event.clone());
            }
            ProviderEventWire::PartDelta {
                part_id,
                field,
                delta,
            } => {
                if let Some(ProviderEventWire::PartDelta {
                    part_id: previous_id,
                    field: previous_field,
                    delta: previous_delta,
                }) = self.provider.last_mut()
                {
                    if previous_id == part_id && previous_field == field {
                        previous_delta.push_str(delta);
                        return;
                    }
                }
                self.provider.push(event.clone());
            }
            _ => self.provider.push(event.clone()),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.provider.clear();
        self.tool_updates.clear();
        self.nested.clear();
    }

    pub(crate) fn present_in(&self, history: &[Message]) -> bool {
        let first_part = self.provider.iter().find_map(|event| match event {
            ProviderEventWire::PartStart { part } => Some(part.id()),
            _ => None,
        });
        // The agent may have persisted MessageEnd before the event reached the
        // daemon's publisher. The history page already contains that reply.
        first_part.is_some_and(|id| {
            history
                .iter()
                .any(|message| message.parts.iter().any(|part| part.id() == id))
        })
    }

    pub(crate) fn send(&self, client: &ClientSender) {
        for event in &self.provider {
            let _ = client.send(ServerMessage::Provider {
                event: event.clone(),
            });
        }
    }

    pub(crate) fn send_tool_activity(&self, client: &ClientSender) {
        for event in &self.tool_updates {
            let _ = client.send(event.clone());
        }
        for event in &self.nested {
            let _ = client.send(event.clone());
        }
    }
}

fn wire_todos(todos: &[mew_agent::Todo]) -> Vec<mew_protocol::Todo> {
    todos
        .iter()
        .map(|todo| mew_protocol::Todo {
            id: todo.id,
            content: todo.content.clone(),
            status: todo.status.as_str().into(),
            depends_on: todo.depends_on.clone(),
        })
        .collect()
}

/// Owns all active sessions.
pub struct SessionManager {
    pub(crate) builder: AgentBuilder,
    pub(crate) switcher: Option<ModelSwitcher>,
    pub(crate) lister: Option<ModelLister>,
    pub(crate) session_dir: PathBuf,
    pub(crate) project_pins: Arc<ProjectPinsStore>,
    pub(crate) active: Mutex<HashMap<String, Arc<Session>>>,
    python_sessions: Mutex<HashMap<String, mew_agent::PythonSession>>,
    python_cache: std::sync::OnceLock<mew_agent::PythonWorkerCache>,
    loading: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub(crate) defaults_lock: Mutex<()>,
    pub remote_devices: std::sync::OnceLock<crate::remote::RemoteDeviceManager>,
    pub(crate) settings_changed: tokio::sync::watch::Sender<()>,
    #[cfg(test)]
    pub(crate) settings_operation_started: tokio::sync::Notify,
    #[cfg(test)]
    pub(crate) revocation_commit_hook: std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl SessionManager {
    async fn restore_python_session(&self, session_id: &str, agent: &mut Agent) {
        let may_use_code_mode = agent.code_mode.mode == Some(mew_config::CodeMode::Full)
            || agent
                .code_mode
                .models
                .values()
                .any(|mode| *mode == mew_config::CodeMode::Full)
            || agent.code_mode.mode.is_none();
        if !may_use_code_mode {
            return;
        }
        let cache = self
            .python_cache
            .get_or_init(|| mew_agent::PythonWorkerCache::new(agent.code_mode.max_idle_workers));
        let session = self
            .python_sessions
            .lock()
            .await
            .entry(session_id.to_owned())
            .or_insert_with(|| cache.session())
            .clone();
        agent.set_python_session(session);
    }

    pub fn new(
        builder: AgentBuilder,
        session_dir: PathBuf,
        switcher: Option<ModelSwitcher>,
        lister: Option<ModelLister>,
        project_pins: Arc<ProjectPinsStore>,
    ) -> Self {
        Self {
            builder,
            switcher,
            lister,
            session_dir,
            project_pins,
            active: Mutex::new(HashMap::new()),
            python_sessions: Mutex::new(HashMap::new()),
            python_cache: std::sync::OnceLock::new(),
            loading: Mutex::new(HashMap::new()),
            defaults_lock: Mutex::new(()),
            remote_devices: std::sync::OnceLock::new(),
            settings_changed: tokio::sync::watch::channel(()).0,
            #[cfg(test)]
            settings_operation_started: tokio::sync::Notify::new(),
            #[cfg(test)]
            revocation_commit_hook: std::sync::Mutex::new(None),
        }
    }

    /// Create a brand-new session.
    pub async fn create(&self, cwd: Option<PathBuf>) -> Result<Arc<Session>> {
        let policy = self.read_session_defaults().await?;
        let session_id = format!("sess_{}", ulid::Ulid::new());
        let lease = mew_session::try_writer_lease(&self.session_dir, &session_id)
            .await
            .context("acquire session writer")?
            .context("session is open in another daemon")?;
        let mut meta = mew_session::Meta::new(&session_id);
        meta.cwd = cwd.as_ref().map(|path| path.to_string_lossy().into_owned());
        let writer = mew_session::Writer::open_at_with_meta(&self.session_dir, &session_id, meta)
            .await
            .context("open session writer")?;
        let (mut agent, model, provider) = (self.builder)(AgentBuildParams {
            session_id: session_id.clone(),
            writer,
            cwd,
            policy,
        })?;
        self.restore_python_session(&session_id, &mut agent).await;
        load_todos(&agent).await?;
        let mut session = Session::new(
            session_id.clone(),
            agent,
            model.clone(),
            provider.clone(),
            self.session_dir.clone(),
        );
        session.writer_lease = Some(lease);
        let session = Arc::new(session);
        session
            .save_policy(&session.policy(&*session.agent.lock().await).await)
            .await?;
        self.active.lock().await.insert(session_id, session.clone());
        Ok(session)
    }

    pub(crate) async fn update_metadata(
        &self,
        session_id: &str,
        change: impl FnOnce(&mut mew_session::Meta) + Send + 'static,
    ) -> Result<mew_session::Meta> {
        anyhow::ensure!(
            std::path::Path::new(session_id)
                .file_name()
                .is_some_and(|name| name == session_id),
            "invalid session id"
        );
        mew_session::Meta::update(&self.session_dir, session_id, change)
            .await?
            .context("session not found")
    }

    /// Persisted defaults apply only when creating a chat, never when resuming one.
    pub async fn read_session_defaults(&self) -> Result<Option<mew_session::SessionPolicy>> {
        let _guard = self.defaults_lock.lock().await;
        match tokio::fs::read(self.session_dir.join("new-chat-defaults.json")).await {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("decode new-chat defaults")?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("read new-chat defaults"),
        }
    }

    pub async fn set_session_defaults(
        &self,
        policy: Option<mew_session::SessionPolicy>,
    ) -> Result<()> {
        self.set_session_defaults_authorized(policy, None).await
    }

    pub(crate) async fn set_session_defaults_authorized(
        &self,
        policy: Option<mew_session::SessionPolicy>,
        authorization: Option<crate::RemoteAuthorization>,
    ) -> Result<()> {
        let _guard = self.defaults_lock.lock().await;
        let path = self.session_dir.join("new-chat-defaults.json");
        let settings_changed = self.settings_changed.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let _file_lock = crate::remote::acquire_file_lock(&path)?;
            if let Some(authorization) = authorization {
                authorization.require_control()?;
            }
            if let Some(policy) = policy {
                anyhow::ensure!(
                    mew_hooks::PermissionMode::from_id(&policy.permission_mode).is_some(),
                    "invalid default permission mode"
                );
                let temporary = path.with_extension("json.tmp");
                std::fs::write(&temporary, serde_json::to_vec_pretty(&policy)?)?;
                std::fs::rename(temporary, path)?;
            } else {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            settings_changed.send_replace(());
            Ok(())
        })
        .await??;
        Ok(())
    }

    /// Attach to an active session, or resume an idle session from disk.
    pub async fn attach(&self, session_id: &str) -> Result<Arc<Session>, AttachError> {
        // Fast path: already active.
        if let Some(session) = self.active.lock().await.get(session_id).cloned() {
            return Ok(session);
        }

        if !std::path::Path::new(session_id)
            .file_name()
            .is_some_and(|name| name == session_id)
        {
            return Err(AttachError::NotFound);
        }

        // Acquire a per-session load lock to prevent TOCTOU races.
        let load_lock = {
            let mut loading = self.loading.lock().await;
            loading
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = load_lock.lock().await;

        // Double-check after acquiring the lock.
        if let Some(session) = self.active.lock().await.get(session_id).cloned() {
            return Ok(session);
        }

        // Avoid creating lock files for unknown session IDs. Re-read metadata
        // under the lease before building the runtime to exclude idle deletion.
        match tokio::fs::symlink_metadata(self.session_dir.join(session_id)).await {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(AttachError::NotFound),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(AttachError::NotFound);
            }
            Err(error) => return Err(AttachError::BuildAgent(error.into())),
        }
        let lease = mew_session::try_writer_lease(&self.session_dir, session_id)
            .await
            .context("acquire session writer")?
            .ok_or(AttachError::InUse)?;

        // Read meta to check depth and created_at.
        let meta = match mew_session::Meta::read(&self.session_dir, session_id).await {
            Ok(Some(m)) => m,
            Ok(None) => return Err(AttachError::NotFound),
            Err(e) => {
                tracing::warn!(session_id, error = %e, "failed to read session meta; treating as not found");
                return Err(AttachError::NotFound);
            }
        };

        if meta.depth != 0 {
            return Err(AttachError::NotTopLevel);
        }
        if meta.id != session_id {
            return Err(AttachError::NotFound);
        }

        let initial_usage = meta.usage.as_ref().map(Into::into);
        let initial_context_tokens = meta.context_tokens;
        let cwd = meta.cwd.clone().map(PathBuf::from);
        let policy = mew_session::SessionPolicy::read(&self.session_dir, session_id)
            .await
            .context("read session policy")
            .map_err(AttachError::BuildAgent)?;
        let writer = mew_session::Writer::open_at_with_meta(&self.session_dir, session_id, meta)
            .await
            .context("open session writer for resume")
            .map_err(AttachError::BuildAgent)?;

        let (mut agent, model, provider) = (self.builder)(AgentBuildParams {
            session_id: session_id.to_string(),
            writer,
            cwd,
            policy,
        })
        .context("build agent for resume")
        .map_err(AttachError::BuildAgent)?;
        self.restore_python_session(session_id, &mut agent).await;
        load_todos(&agent).await.map_err(AttachError::BuildAgent)?;
        agent
            .restore_flagged_files()
            .await
            .context("restore flagged files")
            .map_err(AttachError::BuildAgent)?;

        // The writer has already migrated and repaired the log. Reuse the
        // same scan for model context, transcript pages, and receipt recovery.
        let path = self.session_dir.join(session_id).join("session.jsonl");
        let mut file = tokio::fs::File::open(path)
            .await
            .context("open session history")
            .map_err(AttachError::BuildAgent)?;
        let (history_index, messages, accepted_requests) = HistoryIndex::load(&mut file)
            .await
            .context("load session history")
            .map_err(AttachError::BuildAgent)?;
        agent.load_messages(messages).await;

        let mut session = Session::new(
            session_id.to_string(),
            agent,
            model.clone(),
            provider.clone(),
            self.session_dir.clone(),
        );
        *session.last_usage.get_mut() = initial_usage;
        *session.last_context_tokens.get_mut() = initial_context_tokens;
        session.writer_lease = Some(lease);
        *session.history_index.get_mut() = Arc::new(history_index);
        session.prompt_receipts.get_mut().extend(
            accepted_requests
                .into_iter()
                .map(|id| (id, (mew_protocol::PromptStatus::Accepted, None))),
        );
        let session = Arc::new(session);
        session
            .save_policy(&session.policy(&*session.agent.lock().await).await)
            .await
            .map_err(AttachError::BuildAgent)?;
        self.active
            .lock()
            .await
            .insert(session_id.to_string(), session.clone());
        Ok(session)
    }

    /// Remove a session from active memory.
    pub async fn remove(&self, session_id: &str) {
        if let Some(session) = self.active.lock().await.remove(session_id) {
            session.browser_access.clear();
        }
        self.loading.lock().await.remove(session_id);
    }

    pub(crate) async fn delete(
        &self,
        session_id: &str,
        requesting_client: Option<&Arc<Session>>,
        requesting_browser: Option<&std::sync::Weak<Session>>,
    ) -> Result<()> {
        let id = session_id.strip_prefix("sess_").unwrap_or(session_id);
        anyhow::ensure!(
            ulid::Ulid::from_string(id).is_ok_and(|value| value.to_string() == id),
            "invalid session id"
        );

        let load_lock = self
            .loading
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .clone();
        let _load = load_lock.lock().await;
        let dir = self.session_dir.join(session_id);
        match tokio::fs::symlink_metadata(&dir).await {
            Ok(metadata) => anyhow::ensure!(metadata.is_dir(), "session path is not a directory"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("inspect session directory"),
        }
        let mut active = self.active.lock().await;
        let _lease = if let Some(session) = active.get(session_id) {
            let requester_is_attached =
                requesting_client.is_some_and(|owner| Arc::ptr_eq(owner, session));
            anyhow::ensure!(
                session.client_count().await <= usize::from(requester_is_attached),
                "another client is attached to this session"
            );
            let requester_owns_browser = requesting_browser
                .is_some_and(|owner| std::ptr::eq(owner.as_ptr(), Arc::as_ptr(session)));
            // Lookups and queued work hold their own references. Keep the
            // registry locked until the directory leaves its registered path.
            let owners =
                1 + usize::from(requester_is_attached) + usize::from(requester_owns_browser);
            anyhow::ensure!(
                Arc::strong_count(session) <= owners && !session.has_running_jobs().await,
                "session still has work in progress"
            );
            None
        } else {
            drop(active);
            let lease = mew_session::try_writer_lease(&self.session_dir, session_id)
                .await
                .context("acquire session writer for deletion")?
                .context("session is open in another daemon")?;
            // The local load lock prevents another local attach until the
            // directory leaves its registered path.
            active = self.active.lock().await;
            Some(lease)
        };
        let Some(meta) = mew_session::Meta::read(&self.session_dir, session_id).await? else {
            if !tokio::fs::try_exists(&dir).await? {
                return Ok(());
            }
            return Err(anyhow::anyhow!("session metadata is missing"));
        };
        anyhow::ensure!(
            meta.depth == 0 && meta.id == session_id,
            "not a top-level session"
        );
        let tombstone = self
            .session_dir
            .join(format!(".deleted-{}", ulid::Ulid::new()));
        tokio::fs::rename(&dir, &tombstone)
            .await
            .context("move session directory out of the registry")?;
        if let Some(session) = active.remove(session_id) {
            session.browser_access.clear();
        }
        drop(active);
        self.python_sessions.lock().await.remove(session_id);
        drop(_load);
        if let Err(error) = tokio::fs::remove_dir_all(tombstone).await {
            tracing::warn!(%session_id, %error, "could not clean up deleted session files");
        }
        Ok(())
    }

    /// Unload an unused runtime, or stop reaping if it was replaced or reattached.
    /// A connection that looked up the session but has not registered yet, or a
    /// queued/running operation, owns another strong reference. Check ownership
    /// under the active-map lock so lookup and unloading cannot race.
    pub(crate) async fn unload_detached(&self, session: &Arc<Session>) -> bool {
        let mut active = self.active.lock().await;
        if !active
            .get(&session.id)
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            return true;
        }
        // Background jobs outlive their parent turn. Read their state before
        // locking clients, since completion publication takes state then clients.
        let has_running_jobs = session.has_running_jobs().await;
        let clients = session.clients.lock().await;
        if !clients.is_empty() {
            return true;
        }
        // The manager and this reaper are the only expected owners of an idle
        // runtime. Other reapers hold weak references while sleeping.
        if has_running_jobs
            || Arc::strong_count(session) > 2
            || *session.is_running.lock().await
            || session.current_turn_cancel.lock().await.is_some()
        {
            return false;
        }
        active.remove(&session.id);
        // Keep the resume lock: an attach may already be waiting on it.
        true
    }

    /// List active and idle top-level sessions.
    pub async fn list(&self) -> Vec<SessionInfo> {
        let mut infos = Vec::new();
        let workspace_roots = crate::workspace_roots();
        let active: Vec<_> = self.active.lock().await.values().cloned().collect();

        // Active sessions.
        for session in active {
            let id = &session.id;
            let agent = session.agent.lock().await;
            // Skip empty sessions (no messages yet).
            let message_count = agent.messages.try_lock().map(|m| m.len()).unwrap_or(0);
            if message_count == 0 {
                continue;
            }
            let meta = match mew_session::Meta::read(&self.session_dir, id).await {
                Ok(Some(meta)) => Some(meta),
                _ => agent.session_meta().await,
            };
            let (model, provider) = {
                (
                    session.model.lock().await.clone(),
                    session.provider.lock().await.clone(),
                )
            };
            let created_at = meta.as_ref().map(|m| m.created_at).unwrap_or(0);
            let last_message_at = meta
                .as_ref()
                .and_then(|m| m.last_message_at)
                .or_else(|| meta.as_ref().map(|m| m.created_at));
            let summary = meta.as_ref().and_then(|m| m.summary.clone());
            let first_message = first_user_message_text(&agent.messages);
            infos.push(SessionInfo {
                session_id: id.clone(),
                state: if *session.is_running.lock().await {
                    SessionState::Running
                } else {
                    SessionState::Active
                },
                model,
                provider,
                created_at,
                last_message_at,
                summary,
                client_count: session.client_count().await,
                cwd: meta.as_ref().and_then(|m| m.cwd.clone()),
                workspace_path: crate::workspace::resolve_workspace_path(
                    meta.as_ref().and_then(|m| m.cwd.as_deref()),
                    &workspace_roots,
                ),
                last_turn_failed: meta.as_ref().map(|m| m.last_turn_failed).unwrap_or(false),
                archived: meta.as_ref().map(|m| m.archived).unwrap_or(false),
                pinned: meta.as_ref().map(|m| m.pinned).unwrap_or(false),
                group_id: meta.as_ref().and_then(|m| m.group_id.clone()),
                change_stats: meta.as_ref().and_then(|m| m.change_stats.clone()),
                usage: meta.as_ref().and_then(|m| m.usage.as_ref().map(Into::into)),
                context_tokens: meta.as_ref().and_then(|m| m.context_tokens),
                pending_permissions: session.pending_permissions.lock().await.len() as u32,
                pending_questions: session.pending_questions_count().await,
                first_message,
            });
        }
        // Idle sessions from disk.
        if let Ok(entries) = tokio::fs::read_dir(&self.session_dir).await {
            let mut entries = entries;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let id = match path.file_name().and_then(|s| s.to_str()) {
                    Some(id) if !id.starts_with('.') => id.to_string(),
                    _ => continue,
                };
                if self.active.lock().await.contains_key(&id) {
                    continue;
                }
                match mew_session::Meta::read(&self.session_dir, &id).await {
                    Ok(Some(meta)) if meta.depth == 0 => {
                        // Empty sessions don't need a sidebar entry.
                        let jsonl_path = path.join("session.jsonl");
                        let has_messages = match tokio::fs::metadata(&jsonl_path).await {
                            Ok(m) => m.len() > 0,
                            Err(_) => false,
                        };
                        if !has_messages {
                            continue;
                        }
                        let policy = match mew_session::SessionPolicy::read(&self.session_dir, &id)
                            .await
                        {
                            Ok(policy) => policy,
                            Err(error) => {
                                tracing::warn!(session_id = %id, error = %error, "failed to read listed session policy");
                                None
                            }
                        };
                        infos.push(SessionInfo {
                            session_id: id.clone(),
                            state: SessionState::Idle,
                            model: policy
                                .as_ref()
                                .and_then(|policy| policy.model.clone())
                                .or_else(|| meta.model.clone()),
                            provider: policy.and_then(|policy| policy.provider),
                            created_at: meta.created_at,
                            last_message_at: meta.last_message_at.or(Some(meta.created_at)),
                            summary: meta.summary.clone(),
                            client_count: 0,
                            cwd: meta.cwd.clone(),
                            workspace_path: crate::workspace::resolve_workspace_path(
                                meta.cwd.as_deref(),
                                &workspace_roots,
                            ),
                            last_turn_failed: meta.last_turn_failed,
                            archived: meta.archived,
                            pinned: meta.pinned,
                            group_id: meta.group_id.clone(),
                            change_stats: meta.change_stats.clone(),
                            usage: meta.usage.as_ref().map(Into::into),
                            context_tokens: meta.context_tokens,
                            pending_permissions: 0,
                            pending_questions: 0,
                            first_message: first_user_message_from_disk(&path).await,
                        });
                    }
                    _ => continue,
                }
            }
        }

        infos
    }

    /// Broadcast a title change for a session to all active sessions' clients.
    /// This ensures the session rail (which may be viewing a different session)
    /// picks up the title change in real-time.
    pub async fn broadcast_title(&self, session_id: &str, title: String) {
        self.broadcast_all(ServerMessage::SessionTitleChanged {
            session_id: session_id.to_string(),
            title,
        })
        .await;
    }

    /// Broadcast a session activity change to all clients of that session
    /// and all other active sessions (for the rail).
    pub async fn broadcast_activity(&self, session_id: &str, activity: SessionState) {
        let active = self.active.lock().await;
        let msg = ServerMessage::SessionActivityChanged {
            session_id: session_id.to_string(),
            activity,
        };
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Broadcast a session stats change to all clients.
    pub async fn broadcast_stats(
        &self,
        session_id: &str,
        added: u64,
        removed: u64,
        files_changed: u64,
    ) {
        let active = self.active.lock().await;
        let msg = ServerMessage::SessionStatsChanged {
            session_id: session_id.to_string(),
            added,
            removed,
            files_changed,
        };
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Broadcast a groups-changed notification to all active sessions.
    pub async fn broadcast_groups(&self, groups: Vec<mew_protocol::GroupInfo>) {
        let active = self.active.lock().await;
        let msg = ServerMessage::GroupsChanged { groups };
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Broadcast a message to ALL active sessions' clients.
    /// Used for cross-session alerts (permission needed, turn complete, etc.)
    pub async fn broadcast_all(&self, msg: ServerMessage) {
        let active = self.active.lock().await;
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Get the cwd for a session (from active agent's meta or disk).
    pub async fn session_cwd(&self, session_id: &str) -> Option<PathBuf> {
        match mew_session::Meta::read(&self.session_dir, session_id).await {
            Ok(Some(meta)) => meta.cwd.map(PathBuf::from),
            _ => None,
        }
    }
}

async fn load_todos(agent: &Agent) -> Result<()> {
    if let Some(path) = &agent.todos_path {
        *agent.todos.lock().await = mew_agent::TodoList::load(path)
            .await
            .map_err(anyhow::Error::msg)
            .context("load session todos")?;
    }
    Ok(())
}

/// Extract the first user message text from an agent's message list.
/// Returns `None` if there are no user messages or the first user message
/// has no text content. Truncates to 60 chars (matching `derive_session_title`).
fn first_user_message_text(messages: &Mutex<Vec<mew_message::Message>>) -> Option<String> {
    let msgs = messages.try_lock().ok()?;
    for msg in msgs.iter() {
        if msg.role == mew_message::Role::User {
            for part in &msg.parts {
                if let mew_message::Part::Text(tp) = part {
                    if !tp.synthetic && !tp.text.trim().is_empty() {
                        return Some(truncate_message(&tp.text));
                    }
                }
            }
        }
    }
    None
}

/// Read the first user message text from a session's JSONL file on disk.
/// Only reads the first few lines to find it — efficient for large sessions.
async fn first_user_message_from_disk(session_dir: &std::path::Path) -> Option<String> {
    let jsonl_path = session_dir.join("session.jsonl");
    let mut reader = BufReader::new(tokio::fs::File::open(jsonl_path).await.ok()?);
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line).await.ok()? > 0 {
        if let Ok(msg) = serde_json::from_slice::<mew_message::Message>(&line) {
            if msg.role == mew_message::Role::User {
                for part in &msg.parts {
                    if let mew_message::Part::Text(tp) = part {
                        if !tp.synthetic && !tp.text.trim().is_empty() {
                            return Some(truncate_message(&tp.text));
                        }
                    }
                }
            }
        }
        line.clear();
    }
    None
}

/// Truncate to 60 chars, collapsing whitespace and stripping newlines.
fn truncate_message(text: &str) -> String {
    let cleaned: String = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
    if cleaned.chars().count() == 60 && text.len() > 60 {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for observer in self.job_observers.get_mut().values() {
            observer.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn python_session_is_shared_when_a_later_model_may_use_code_mode() {
        let directory = tempfile::tempdir().unwrap();
        let server = crate::DaemonServer::with_session_dir(
            Arc::new(|params| {
                Ok((
                    Agent::new(
                        Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                        Arc::new(mew_hooks::NopDispatcher),
                        Some(params.writer),
                        Vec::new(),
                        None,
                    ),
                    None,
                    None,
                ))
            }),
            directory.path().to_path_buf(),
        );
        let mut agent = Agent::new(
            Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
            Arc::new(mew_hooks::NopDispatcher),
            None,
            Vec::new(),
            None,
        );
        agent.set_model_info("glm-5.3-flash", "opencode-go");
        server
            .session_manager
            .restore_python_session("session", &mut agent)
            .await;
        assert!(server.session_manager.python_cache.get().is_some());
        assert!(server
            .session_manager
            .python_sessions
            .lock()
            .await
            .contains_key("session"));
    }

    #[tokio::test]
    async fn detached_runtime_keeps_running_shell_jobs_until_they_finish() {
        for cancelled in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let server = crate::DaemonServer::with_session_dir(
                Arc::new(|params| {
                    Ok((
                        Agent::new(
                            Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                            Arc::new(mew_hooks::NopDispatcher),
                            Some(params.writer),
                            Vec::new(),
                            None,
                        ),
                        None,
                        None,
                    ))
                }),
                directory.path().to_path_buf(),
            );
            let session = server.session_manager.create(None).await.unwrap();
            let (job_id, cancel, job_state) = {
                let agent = session.agent.lock().await;
                let id = agent
                    .start_shell_job(
                        "while [ ! -e finish ]; do sleep 0.01; done; printf 'done\\n'",
                        directory.path(),
                    )
                    .await;
                let jobs = agent.shell_jobs.lock().await;
                let job = &jobs[&id];
                let cancel = job.cancel.clone();
                let state = job.state.clone();
                (id, cancel, state)
            };
            session
                .publish_job_update(job_id.clone(), String::new(), "running".into())
                .await;
            assert_eq!(Arc::strong_count(&session), 2);
            let current = job_state.lock().await;
            let unload = server.session_manager.unload_detached(&session);
            tokio::pin!(unload);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(10), &mut unload)
                    .await
                    .is_err()
            );
            let clients =
                tokio::time::timeout(std::time::Duration::from_secs(1), session.clients.lock())
                    .await
                    .expect("job state must be checked before locking clients");
            drop(clients);
            drop(current);
            let unloaded = unload.await;
            if unloaded {
                cancel.cancel();
            }
            assert!(
                !unloaded,
                "reaping must retain the idle parent's running job"
            );
            let (sender, _receiver) = ClientSender::channel(CancellationToken::new());
            let (client_id, _) = session
                .attach_client(sender, mew_protocol::ClientKind::Tui)
                .await;
            let stopped = server.session_manager.unload_detached(&session).await;
            if !stopped {
                cancel.cancel();
            }
            assert!(stopped, "reattachment must stop the detached reaper");
            session.detach_client(client_id).await;

            if cancelled {
                cancel.cancel();
            } else {
                tokio::fs::write(directory.path().join("finish"), "")
                    .await
                    .unwrap();
            }
            let (state, _) = session
                .agent
                .lock()
                .await
                .shell_job_block(&job_id, 3)
                .await
                .unwrap();
            assert!(state.is_terminal());
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !server.session_manager.unload_detached(&session).await {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("a completed or cancelled job must release the detached runtime");
        }
    }
}
