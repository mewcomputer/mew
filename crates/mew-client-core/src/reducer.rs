//! Deterministic daemon-message reducer used by platform clients.

use mew_message::{AssistantMeta, Message, Part, PartId, ProviderEventWire, Role, Time, ToolState};
use mew_protocol::{
    ClientKind, ClientMessage, DirEntry, FlaggedFileWire, GroupInfo, ModelInfo, PermissionDecision,
    PersonaInfo, ProjectInfo, Question, RemoteScope, ServerMessage, SessionInfo, SessionUsageWire,
    Todo,
};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Backoff { attempt: u32, error: String },
}

#[derive(Debug, Clone)]
pub struct ClientSession {
    pub session_id: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub permission_mode: Option<String>,
    pub messages: Vec<Message>,
    pub running: bool,
    pub usage: SessionUsageWire,
    /// Current context occupancy (latest request's prompt size), if known.
    pub context_tokens: Option<u64>,
    pub subagents: Vec<SubagentEntry>,
    pub todos: Vec<Todo>,
    pub flagged_files: Vec<FlaggedFileWire>,
    pub pending_actions: Vec<PendingAction>,
    pub last_sent_prompt: Option<String>,
    pub streaming_part_id: Option<PartId>,
    pub streaming_text: String,
}

impl ClientSession {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
            messages: Vec::new(),
            running: false,
            usage: SessionUsageWire::default(),
            context_tokens: None,
            subagents: Vec::new(),
            todos: Vec::new(),
            flagged_files: Vec::new(),
            pending_actions: Vec::new(),
            last_sent_prompt: None,
            streaming_part_id: None,
            streaming_text: String::new(),
        }
    }

    /// Apply provider events to one session without requiring a platform
    /// client to own the daemon-wide reducer.
    pub fn apply_provider_event(&mut self, event: ProviderEventWire) -> Vec<SessionEvent> {
        // Keep the daemon-wide reducer as the behavioral source of truth while
        // this API is introduced. The temporary projection is intentionally
        // private to the core and can be removed once all adapters consume the
        // session reducer directly.
        let session_id = self.session_id.clone();
        let mut state = ClientState {
            attached_session: Some(session_id.clone()),
            ..ClientState::default()
        };
        state.sessions.insert(session_id.clone(), self.clone());
        let events = state.apply_provider_event(event);
        *self = state
            .sessions
            .remove(&session_id)
            .expect("projected client session");
        events
            .into_iter()
            .filter_map(|event| match event {
                ClientEvent::MessageChanged { .. } => Some(SessionEvent::MessageChanged),
                ClientEvent::TextDelta { part_id, delta, .. } => {
                    Some(SessionEvent::TextDelta { part_id, delta })
                }
                ClientEvent::TurnEnded {
                    cost,
                    input_tokens,
                    output_tokens,
                    ..
                } => Some(SessionEvent::TurnEnded {
                    cost,
                    input_tokens,
                    output_tokens,
                }),
                ClientEvent::Error(message) => Some(SessionEvent::Error(message)),
                _ => None,
            })
            .collect()
    }

    pub fn record_prompt(&mut self, text: String) {
        self.last_sent_prompt = Some(text.clone());
        let session_id = Ulid::from_string(&self.session_id).unwrap_or_else(|_| Ulid::new());
        let message_id = Ulid::new();
        let part_id = Ulid::new();
        self.messages.push(Message {
            id: message_id,
            session_id,
            role: Role::User,
            parts: vec![Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: part_id,
                    message_id,
                    session_id,
                },
                text,
                synthetic: false,
            })],
            time: Time {
                created: now_millis(),
                completed: None,
            },
            assistant: None,
        });
    }
}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    MessageChanged,
    TextDelta {
        part_id: PartId,
        delta: String,
    },
    TurnEnded {
        cost: f64,
        input_tokens: u32,
        output_tokens: u32,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActionKind {
    Permission {
        tool_name: String,
        input: serde_json::Value,
    },
    WorkspacePermission {
        path: String,
    },
    AskUser {
        call_id: String,
        questions: Vec<Question>,
    },
    PlanApproval {
        call_id: String,
        plan_path: String,
        plan_markdown: String,
        persona: String,
    },
    GoalApproval {
        call_id: String,
        objective: String,
    },
    SubagentPermission {
        parent_call_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingAction {
    pub request_id: String,
    pub kind: ActionKind,
}

/// One active subagent task, keyed by the parent tool call id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentEntry {
    pub task_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub status: String,
    pub tool_name: Option<String>,
}

/// One other client attached to the same session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceEntry {
    pub client_id: u64,
    pub client_kind: ClientKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserState {
    pub open: bool,
    pub url: Option<String>,
    pub title: Option<String>,
    pub tab_id: Option<String>,
    pub snapshot: Option<String>,
    pub screenshot: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    ConnectionChanged(ConnectionStatus),
    RemoteReady(RemoteScope),
    SessionReady {
        session_id: String,
    },
    SessionListChanged,
    ProjectListChanged,
    GroupsChanged,
    SessionMetaChanged {
        session_id: String,
    },
    ModelListChanged,
    PersonaListChanged,
    SubagentsChanged {
        session_id: String,
    },
    TodosChanged {
        session_id: String,
    },
    FlaggedFilesChanged {
        session_id: String,
    },
    PresenceChanged,
    UsageChanged {
        session_id: String,
    },
    FileTreeChanged,
    SessionHistoryLoaded {
        session_id: String,
    },
    PermissionModeChanged {
        mode: String,
    },
    MessageChanged {
        session_id: String,
    },
    TextDelta {
        session_id: String,
        part_id: PartId,
        delta: String,
    },
    ToolProgress {
        session_id: String,
        call_id: String,
        chunk: String,
    },
    TurnEnded {
        session_id: String,
        cost: f64,
        input_tokens: u32,
        output_tokens: u32,
    },
    RequiredActionChanged {
        session_id: String,
        request_id: String,
    },
    RequestResolved {
        request_id: String,
    },
    TerminalOpened {
        terminal_id: String,
    },
    TerminalOutput {
        terminal_id: String,
        bytes: Vec<u8>,
    },
    TerminalExited {
        terminal_id: String,
        status: String,
    },
    TerminalError {
        terminal_id: Option<String>,
        message: String,
    },
    BrowserStateChanged {
        open: bool,
        url: Option<String>,
        title: Option<String>,
        tab_id: Option<String>,
    },
    BrowserSnapshot {
        snapshot: String,
        url: String,
        title: String,
        tab_id: Option<String>,
    },
    BrowserScreenshot {
        data: String,
        url: String,
        tab_id: Option<String>,
    },
    BrowserError {
        message: String,
        tab_id: Option<String>,
    },
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct ClientState {
    pub connection: Option<ConnectionStatus>,
    pub remote_scope: Option<RemoteScope>,
    pub daemon_version: Option<String>,
    pub attached_session: Option<String>,
    pub sessions: BTreeMap<String, ClientSession>,
    pub session_list: Vec<SessionInfo>,
    pub session_titles: BTreeMap<String, String>,
    pub groups: Vec<GroupInfo>,
    pub projects: Vec<ProjectInfo>,
    pub models: Vec<ModelInfo>,
    pub current_model: Option<String>,
    pub current_provider: Option<String>,
    pub thinking_variant: Option<String>,
    pub permission_mode: Option<String>,
    pub current_persona: Option<String>,
    pub personas: Vec<PersonaInfo>,
    pub browser: BrowserState,
    pub presence: Vec<PresenceEntry>,
    pub control_yielded_by: Option<u64>,
    pub dir_listing_session_id: Option<String>,
    pub dir_listing_path: Option<String>,
    pub dir_listing: Vec<DirEntry>,
}

impl ClientState {
    /// Build the state needed by a presentation client without cloning idle
    /// sessions that are not visible in the current session.
    pub fn ui_snapshot(&self) -> Self {
        let mut snapshot = self.ui_metadata_snapshot();
        if let Some(session_id) = &self.attached_session {
            if let Some(session) = self.sessions.get(session_id) {
                snapshot
                    .sessions
                    .insert(session_id.clone(), session.clone());
            }
        }
        snapshot
    }

    /// Build the presentation state that excludes message history. Streaming
    /// deltas are applied incrementally by the presentation layer.
    pub fn ui_metadata_snapshot(&self) -> Self {
        Self {
            connection: self.connection.clone(),
            remote_scope: self.remote_scope,
            daemon_version: self.daemon_version.clone(),
            attached_session: self.attached_session.clone(),
            sessions: BTreeMap::new(),
            session_list: self.session_list.clone(),
            session_titles: self.session_titles.clone(),
            groups: self.groups.clone(),
            projects: self.projects.clone(),
            models: self.models.clone(),
            current_model: self.current_model.clone(),
            current_provider: self.current_provider.clone(),
            thinking_variant: self.thinking_variant.clone(),
            permission_mode: self.permission_mode.clone(),
            current_persona: self.current_persona.clone(),
            personas: self.personas.clone(),
            browser: self.browser.clone(),
            presence: self.presence.clone(),
            control_yielded_by: self.control_yielded_by,
            dir_listing_session_id: self.dir_listing_session_id.clone(),
            dir_listing_path: self.dir_listing_path.clone(),
            dir_listing: self.dir_listing.clone(),
        }
    }

    pub fn set_connection_status(&mut self, status: ConnectionStatus) -> ClientEvent {
        self.connection = Some(status.clone());
        ClientEvent::ConnectionChanged(status)
    }

    pub fn session(&self, session_id: &str) -> Option<&ClientSession> {
        self.sessions.get(session_id)
    }

    pub fn usage(&self, session_id: &str) -> Option<&SessionUsageWire> {
        self.sessions.get(session_id).map(|session| &session.usage)
    }

    pub fn session_mut(&mut self, session_id: &str) -> &mut ClientSession {
        self.sessions
            .entry(session_id.to_string())
            .or_insert_with(|| ClientSession::new(session_id.to_string()))
    }

    /// Record a prompt before sending it. A later UserMessage echo is matched
    /// and consumed instead of duplicating the message in the transcript.
    pub fn record_prompt(&mut self, session_id: &str, text: String) {
        self.session_mut(session_id).record_prompt(text);
    }

    pub fn apply_server_message(&mut self, message: ServerMessage) -> Vec<ClientEvent> {
        match message {
            ServerMessage::RemoteReady { scope } => {
                self.remote_scope = Some(scope);
                vec![ClientEvent::RemoteReady(scope)]
            }
            ServerMessage::Pong { version } => {
                self.daemon_version = Some(version);
                Vec::new()
            }
            ServerMessage::SessionReady {
                session_id,
                cwd,
                model,
                provider,
                permission_mode,
            } => {
                self.attached_session = Some(session_id.clone());
                let session = self.session_mut(&session_id);
                session.cwd = cwd;
                session.model = model.clone();
                session.provider = provider.clone();
                session.permission_mode = permission_mode.clone();
                self.current_model = model;
                self.current_provider = provider;
                self.permission_mode = permission_mode;
                self.presence.clear();
                self.control_yielded_by = None;
                vec![ClientEvent::SessionReady { session_id }]
            }
            ServerMessage::SessionList { sessions } => {
                self.session_list = sessions;
                vec![ClientEvent::SessionListChanged]
            }
            ServerMessage::GroupList { groups } | ServerMessage::GroupsChanged { groups } => {
                self.groups = groups;
                vec![ClientEvent::GroupsChanged]
            }
            ServerMessage::SessionTitleChanged { session_id, title } => {
                self.session_titles.insert(session_id, title);
                vec![ClientEvent::SessionListChanged]
            }
            ServerMessage::SessionSummaryChanged {
                session_id,
                summary,
            } => {
                if let Some(session) = self
                    .session_list
                    .iter_mut()
                    .find(|session| session.session_id == session_id)
                {
                    session.summary = Some(summary);
                }
                vec![ClientEvent::SessionListChanged]
            }
            ServerMessage::SessionActivityChanged {
                session_id,
                activity,
            } => {
                if let Some(session) = self
                    .session_list
                    .iter_mut()
                    .find(|session| session.session_id == session_id)
                {
                    session.state = activity;
                }
                vec![ClientEvent::SessionListChanged]
            }
            ServerMessage::SessionStatsChanged {
                session_id,
                added,
                removed,
                ..
            } => {
                if let Some(session) = self
                    .session_list
                    .iter_mut()
                    .find(|session| session.session_id == session_id)
                {
                    let stats = session
                        .change_stats
                        .get_or_insert_with(mew_session::ChangeStats::default);
                    stats.added = added;
                    stats.removed = removed;
                }
                vec![ClientEvent::SessionListChanged]
            }
            ServerMessage::SessionAttentionChanged {
                session_id,
                pending_permissions,
                pending_questions,
            } => {
                if let Some(session) = self
                    .session_list
                    .iter_mut()
                    .find(|session| session.session_id == session_id)
                {
                    session.pending_permissions = pending_permissions;
                    session.pending_questions = pending_questions;
                }
                vec![ClientEvent::SessionListChanged]
            }
            ServerMessage::FlaggedFilesChanged { session_id, files } => {
                self.session_mut(&session_id).flagged_files = files;
                vec![ClientEvent::FlaggedFilesChanged { session_id }]
            }
            ServerMessage::SessionMetaChanged {
                session_id,
                archived,
                pinned,
                group_id,
            } => {
                if let Some(session) = self
                    .session_list
                    .iter_mut()
                    .find(|session| session.session_id == session_id)
                {
                    session.archived = archived;
                    session.pinned = pinned;
                    session.group_id = group_id;
                }
                vec![ClientEvent::SessionMetaChanged { session_id }]
            }
            ServerMessage::SessionHistory { messages } => {
                let Some(session_id) = self.attached_session.clone().or_else(|| {
                    messages
                        .first()
                        .map(|message| message.session_id.to_string())
                }) else {
                    return Vec::new();
                };
                self.attached_session = Some(session_id.clone());
                self.session_mut(&session_id).messages = messages;
                vec![ClientEvent::SessionHistoryLoaded { session_id }]
            }
            ServerMessage::UserMessage { text } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                if session.last_sent_prompt.as_deref() == Some(text.as_str()) {
                    session.last_sent_prompt = None;
                    return Vec::new();
                }
                session.last_sent_prompt = None;
                self.record_prompt(&session_id, text);
                vec![ClientEvent::MessageChanged { session_id }]
            }
            ServerMessage::Provider { event } => self.apply_provider_event(event),
            ServerMessage::PartUpdated { part_id, part } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                for message in &mut session.messages {
                    if let Some(existing) =
                        message.parts.iter_mut().find(|item| item.id() == part_id)
                    {
                        *existing = part;
                        return vec![ClientEvent::MessageChanged { session_id }];
                    }
                }
                Vec::new()
            }
            ServerMessage::ToolProgress { call_id, chunk } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                for message in &mut session.messages {
                    if let Some(Part::ToolCall(part)) = message.parts.iter_mut().find(
                        |part| matches!(part, Part::ToolCall(tool) if tool.call_id == call_id),
                    ) {
                        if let ToolState::Running(state) = &mut part.state {
                            state.output.push_str(&chunk);
                            return vec![ClientEvent::ToolProgress {
                                session_id,
                                call_id,
                                chunk,
                            }];
                        }
                    }
                }
                Vec::new()
            }
            ServerMessage::ToolStart { .. } | ServerMessage::ToolEnd { .. } => Vec::new(),
            ServerMessage::PermissionRequest {
                request_id,
                tool_name,
                input,
            } => self.add_action(ActionKind::Permission { tool_name, input }, request_id),
            ServerMessage::WorkspacePermissionRequest { request_id, path } => {
                self.add_action(ActionKind::WorkspacePermission { path }, request_id)
            }
            ServerMessage::AskUserRequest {
                request_id,
                call_id,
                questions,
            } => self.add_action(ActionKind::AskUser { call_id, questions }, request_id),
            ServerMessage::PlanApprovalRequest {
                request_id,
                call_id,
                plan_path,
                plan_markdown,
                persona,
            } => self.add_action(
                ActionKind::PlanApproval {
                    call_id,
                    plan_path,
                    plan_markdown,
                    persona,
                },
                request_id,
            ),
            ServerMessage::GoalProposed {
                request_id,
                call_id,
                objective,
            } => self.add_action(ActionKind::GoalApproval { call_id, objective }, request_id),
            ServerMessage::SubagentPermissionRequest {
                request_id,
                parent_call_id,
                tool_name,
                input,
            } => self.add_action(
                ActionKind::SubagentPermission {
                    parent_call_id,
                    tool_name,
                    input,
                },
                request_id,
            ),
            ServerMessage::SubagentStart {
                parent_call_id,
                name,
                display_name,
                ..
            } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                session
                    .subagents
                    .retain(|entry| entry.task_id != parent_call_id);
                session.subagents.push(SubagentEntry {
                    task_id: parent_call_id,
                    name,
                    display_name,
                    status: "running".into(),
                    tool_name: None,
                });
                vec![ClientEvent::SubagentsChanged { session_id }]
            }
            ServerMessage::SubagentStatus {
                parent_call_id,
                tool_name,
                message,
            } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                match session
                    .subagents
                    .iter_mut()
                    .find(|entry| entry.task_id == parent_call_id)
                {
                    Some(entry) => {
                        entry.status = message;
                        entry.tool_name = Some(tool_name);
                    }
                    None => session.subagents.push(SubagentEntry {
                        task_id: parent_call_id,
                        name: String::new(),
                        display_name: None,
                        status: message,
                        tool_name: Some(tool_name),
                    }),
                }
                vec![ClientEvent::SubagentsChanged { session_id }]
            }
            ServerMessage::SubagentEnd { parent_call_id, .. } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                let before = session.subagents.len();
                session
                    .subagents
                    .retain(|entry| entry.task_id != parent_call_id);
                if session.subagents.len() == before {
                    return Vec::new();
                }
                vec![ClientEvent::SubagentsChanged { session_id }]
            }
            ServerMessage::TodosUpdated { todos } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                self.session_mut(&session_id).todos = todos;
                vec![ClientEvent::TodosChanged { session_id }]
            }
            ServerMessage::ClientAttached {
                client_id,
                client_kind,
            } => {
                self.presence.retain(|entry| entry.client_id != client_id);
                self.presence.push(PresenceEntry {
                    client_id,
                    client_kind,
                });
                vec![ClientEvent::PresenceChanged]
            }
            ServerMessage::ClientDetached { client_id } => {
                self.presence.retain(|entry| entry.client_id != client_id);
                vec![ClientEvent::PresenceChanged]
            }
            ServerMessage::ControlYielded { client_id } => {
                self.control_yielded_by = Some(client_id);
                vec![ClientEvent::PresenceChanged]
            }
            ServerMessage::RequestResolved { request_id } => {
                for session in self.sessions.values_mut() {
                    session
                        .pending_actions
                        .retain(|action| action.request_id != request_id);
                }
                vec![ClientEvent::RequestResolved { request_id }]
            }
            ServerMessage::SessionCleared => {
                if let Some(session_id) = self.attached_session.clone() {
                    self.session_mut(&session_id).messages.clear();
                    return vec![ClientEvent::MessageChanged { session_id }];
                }
                Vec::new()
            }
            ServerMessage::SessionUsageChanged {
                session_id,
                usage,
                context_tokens,
            } => {
                let session = self.session_mut(&session_id);
                session.usage = usage;
                session.context_tokens = context_tokens;
                vec![ClientEvent::UsageChanged { session_id }]
            }
            ServerMessage::ModelList { models } => {
                self.models = models;
                vec![ClientEvent::ModelListChanged]
            }
            ServerMessage::ModelSwitched { provider, model } => {
                self.current_provider = Some(provider);
                self.current_model = Some(model);
                Vec::new()
            }
            ServerMessage::ThinkingVariantChanged { variant } => {
                self.thinking_variant = variant;
                Vec::new()
            }
            ServerMessage::PermissionModeChanged { mode } => {
                self.permission_mode = Some(mode.clone());
                vec![ClientEvent::PermissionModeChanged { mode }]
            }
            ServerMessage::PersonaSwitched { name } => {
                self.current_persona = Some(name);
                Vec::new()
            }
            ServerMessage::PersonaList { personas } => {
                self.current_persona = personas
                    .iter()
                    .find(|persona| persona.active)
                    .map(|persona| persona.name.clone());
                self.personas = personas;
                vec![ClientEvent::PersonaListChanged]
            }
            ServerMessage::ProjectList { projects } => {
                self.projects = projects;
                vec![ClientEvent::ProjectListChanged]
            }
            ServerMessage::SlashResult { text } => {
                let message_id = Ulid::new();
                let session_id = Ulid::new();
                let part_id = Ulid::new();
                let part = Part::Text(mew_message::TextPart {
                    base: mew_message::PartBase {
                        id: part_id,
                        message_id,
                        session_id,
                    },
                    text,
                    synthetic: true,
                });
                let mut events = self.apply_provider_event(ProviderEventWire::PartStart { part });
                events.extend(self.apply_provider_event(ProviderEventWire::PartEnd { part_id }));
                events.extend(self.apply_provider_event(ProviderEventWire::MessageEnd {
                    finish: mew_message::Finish::Stop,
                    usage: mew_message::Tokens::default(),
                    cost: 0.0,
                    manifest: None,
                }));
                events
            }
            ServerMessage::BrowserState {
                open,
                url,
                title,
                tab_id,
            } => {
                self.browser = BrowserState {
                    open,
                    url: url.clone(),
                    title: title.clone(),
                    tab_id: tab_id.clone(),
                    snapshot: self.browser.snapshot.take(),
                    screenshot: self.browser.screenshot.take(),
                    error: None,
                };
                vec![ClientEvent::BrowserStateChanged {
                    open,
                    url,
                    title,
                    tab_id,
                }]
            }
            ServerMessage::BrowserSnapshot {
                snapshot,
                url,
                title,
                tab_id,
            } => {
                self.browser.open = true;
                self.browser.url = Some(url.clone());
                self.browser.title = Some(title.clone());
                self.browser.tab_id = tab_id.clone();
                self.browser.snapshot = Some(snapshot.clone());
                self.browser.error = None;
                vec![ClientEvent::BrowserSnapshot {
                    snapshot,
                    url,
                    title,
                    tab_id,
                }]
            }
            ServerMessage::BrowserScreenshot { data, url, tab_id } => {
                self.browser.open = true;
                self.browser.url = Some(url.clone());
                self.browser.tab_id = tab_id.clone();
                self.browser.screenshot = Some(data.clone());
                self.browser.error = None;
                vec![ClientEvent::BrowserScreenshot { data, url, tab_id }]
            }
            ServerMessage::BrowserError { message, tab_id } => {
                self.browser.error = Some(message.clone());
                vec![ClientEvent::BrowserError { message, tab_id }]
            }
            ServerMessage::TerminalOpened { terminal_id } => {
                vec![ClientEvent::TerminalOpened { terminal_id }]
            }
            ServerMessage::TerminalOutput { terminal_id, bytes } => {
                vec![ClientEvent::TerminalOutput { terminal_id, bytes }]
            }
            ServerMessage::TerminalExited {
                terminal_id,
                status,
            } => vec![ClientEvent::TerminalExited {
                terminal_id,
                status,
            }],
            ServerMessage::TerminalError {
                terminal_id,
                message,
            } => vec![ClientEvent::TerminalError {
                terminal_id,
                message,
            }],
            ServerMessage::DirListing {
                session_id,
                path,
                entries,
            } => {
                self.dir_listing_session_id = Some(session_id);
                self.dir_listing_path = Some(path);
                self.dir_listing = entries;
                vec![ClientEvent::FileTreeChanged]
            }
            // The daemon pushes this after a WatchWorkspace subscription; the
            // client is expected to re-request a `ListDir`, like the web UI.
            ServerMessage::FsChanged { .. } => vec![ClientEvent::FileTreeChanged],
            ServerMessage::Error { message } | ServerMessage::ErrorEvent { message } => {
                vec![ClientEvent::Error(message)]
            }
            _ => Vec::new(),
        }
    }

    fn add_action(&mut self, kind: ActionKind, request_id: String) -> Vec<ClientEvent> {
        let Some(session_id) = self.attached_session.clone() else {
            return Vec::new();
        };
        self.session_mut(&session_id)
            .pending_actions
            .push(PendingAction {
                request_id: request_id.clone(),
                kind,
            });
        vec![ClientEvent::RequiredActionChanged {
            session_id,
            request_id,
        }]
    }

    fn apply_provider_event(&mut self, event: ProviderEventWire) -> Vec<ClientEvent> {
        let Some(session_id) = self.attached_session.clone() else {
            return Vec::new();
        };
        let session = self.session_mut(&session_id);
        match event {
            ProviderEventWire::PartStart { part } => {
                session.running = true;
                let part_id = part.id();
                match &part {
                    Part::Text(part) => {
                        session.streaming_part_id = Some(part_id);
                        session.streaming_text = part.text.clone();
                    }
                    Part::Reasoning(part) => {
                        session.streaming_part_id = Some(part_id);
                        session.streaming_text = part.text.clone();
                    }
                    _ => {}
                }
                let message_id = match &part {
                    Part::Text(part) => part.base.message_id,
                    Part::Reasoning(part) => part.base.message_id,
                    Part::File(part) => part.base.message_id,
                    Part::ToolCall(part) => part.base.message_id,
                    Part::ToolResult(part) => part.base.message_id,
                    Part::Compaction(part) => part.base.message_id,
                };
                let has_message = session
                    .messages
                    .last()
                    .map(|message| message.id == message_id && message.role == Role::Assistant)
                    .unwrap_or(false);
                if !has_message {
                    session.messages.push(Message {
                        id: message_id,
                        session_id: Ulid::from_string(&session.session_id)
                            .unwrap_or_else(|_| Ulid::new()),
                        role: Role::Assistant,
                        parts: Vec::new(),
                        time: Time {
                            created: now_millis(),
                            completed: None,
                        },
                        assistant: None,
                    });
                }
                session
                    .messages
                    .last_mut()
                    .expect("assistant message")
                    .parts
                    .push(part);
                vec![ClientEvent::MessageChanged { session_id }]
            }
            ProviderEventWire::PartDelta { part_id, delta, .. } => {
                if session.streaming_part_id != Some(part_id) {
                    return Vec::new();
                }
                session.streaming_text.push_str(&delta);
                for message in session.messages.iter_mut().rev() {
                    if let Some(part) = message.parts.iter_mut().find(|part| part.id() == part_id) {
                        match part {
                            Part::Text(part) => part.text = session.streaming_text.clone(),
                            Part::Reasoning(part) => part.text = session.streaming_text.clone(),
                            _ => {}
                        }
                        break;
                    }
                }
                vec![ClientEvent::TextDelta {
                    session_id,
                    part_id,
                    delta,
                }]
            }
            ProviderEventWire::PartEnd { part_id } => {
                if session.streaming_part_id == Some(part_id) {
                    session.streaming_part_id = None;
                    session.streaming_text.clear();
                }
                Vec::new()
            }
            ProviderEventWire::MessageEnd {
                usage,
                cost,
                finish,
                manifest,
            } => {
                session.running = false;
                session.streaming_part_id = None;
                session.streaming_text.clear();
                session.usage.input_tokens += usage.input as u64;
                session.usage.output_tokens += usage.output as u64;
                session.usage.cost += cost;
                session.usage.turns += 1;
                if let Some(message) = session
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                {
                    let assistant = message.assistant.get_or_insert(AssistantMeta {
                        provider_id: session.provider.clone().unwrap_or_default(),
                        model_id: session.model.clone().unwrap_or_default(),
                        cost: 0.0,
                        tokens: usage,
                        finish: None,
                        error: None,
                        manifest: None,
                    });
                    assistant.cost += cost;
                    assistant.tokens = usage;
                    assistant.finish = Some(finish);
                    assistant.manifest = manifest;
                }
                vec![ClientEvent::TurnEnded {
                    session_id,
                    cost,
                    input_tokens: usage.input,
                    output_tokens: usage.output,
                }]
            }
            ProviderEventWire::Error(error) => vec![ClientEvent::Error(error.message)],
            ProviderEventWire::RetryWait { reason, .. } => vec![ClientEvent::Error(reason)],
        }
    }

    pub fn permission_response(request_id: String, decision: PermissionDecision) -> ClientMessage {
        ClientMessage::PermissionResponse {
            request_id,
            decision,
        }
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{
        Finish, PartBase, TextPart, Tokens, ToolCallPart, ToolState, ToolStateRunning, ToolTime,
    };

    fn session_ready(state: &mut ClientState) {
        state.apply_server_message(ServerMessage::SessionReady {
            session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            cwd: Some("/tmp/project".into()),
            model: Some("gpt-test".into()),
            provider: Some("fake".into()),
            permission_mode: Some("standard".into()),
        });
    }

    #[test]
    fn prompt_echo_is_not_added_twice() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let id = state.attached_session.clone().unwrap();
        state.record_prompt(&id, "hello".into());
        let before = state.session(&id).unwrap().messages.len();

        let events = state.apply_server_message(ServerMessage::UserMessage {
            text: "hello".into(),
        });
        assert!(events.is_empty());
        assert_eq!(state.session(&id).unwrap().messages.len(), before);
    }

    #[test]
    fn session_history_keeps_the_session_ready_identifier() {
        let mut state = ClientState::default();
        state.apply_server_message(ServerMessage::SessionReady {
            session_id: "sess_01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            cwd: Some("/tmp/project".into()),
            model: Some("gpt-test".into()),
            provider: Some("fake".into()),
            permission_mode: Some("standard".into()),
        });
        let message_id = Ulid::new();
        let session_ulid = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let events = state.apply_server_message(ServerMessage::SessionHistory {
            messages: vec![Message {
                id: message_id,
                session_id: session_ulid,
                role: Role::User,
                parts: Vec::new(),
                time: Time {
                    created: 0,
                    completed: None,
                },
                assistant: None,
            }],
        });

        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SessionHistoryLoaded { session_id }]
                if session_id == "sess_01ARZ3NDEKTSV4RRFFQ69G5FAV"
        ));
        assert_eq!(
            state.attached_session.as_deref(),
            Some("sess_01ARZ3NDEKTSV4RRFFQ69G5FAV")
        );
        assert_eq!(
            state
                .session("sess_01ARZ3NDEKTSV4RRFFQ69G5FAV")
                .unwrap()
                .messages
                .len(),
            1
        );
        assert!(state.session("01ARZ3NDEKTSV4RRFFQ69G5FAV").is_none());
    }

    #[test]
    fn ui_snapshot_keeps_only_the_attached_session() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        state
            .session_mut("01ARZ3NDEKTSV4RRFFQ69G5FAV-other")
            .record_prompt("background history".into());

        let snapshot = state.ui_snapshot();

        assert_eq!(snapshot.sessions.len(), 1);
        assert!(snapshot
            .session(state.attached_session.as_deref().unwrap())
            .is_some());
        assert!(snapshot
            .session("01ARZ3NDEKTSV4RRFFQ69G5FAV-other")
            .is_none());
    }

    #[test]
    fn provider_stream_updates_message_and_usage() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();
        let session_ulid = Ulid::from_string(&session_id).unwrap();
        let message_id = Ulid::new();
        let part_id = Ulid::new();
        let part = Part::Text(TextPart {
            base: PartBase {
                id: part_id,
                message_id,
                session_id: session_ulid,
            },
            text: "hello".into(),
            synthetic: false,
        });

        state.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::PartStart { part },
        });
        let events = state.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::PartDelta {
                part_id,
                field: "text".into(),
                delta: " world".into(),
            },
        });
        assert!(
            matches!(events.as_slice(), [ClientEvent::TextDelta { delta, .. }] if delta == " world")
        );
        state.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::MessageEnd {
                finish: Finish::Stop,
                usage: Tokens {
                    input: 10,
                    output: 4,
                    ..Tokens::default()
                },
                cost: 0.25,
                manifest: None,
            },
        });

        let session = state.session(&session_id).unwrap();
        assert_eq!(session.messages.len(), 1);
        assert!(
            matches!(&session.messages[0].parts[0], Part::Text(part) if part.text == "hello world")
        );
        assert_eq!(session.usage.input_tokens, 10);
        assert_eq!(session.usage.output_tokens, 4);
        assert_eq!(session.usage.turns, 1);
        assert!(!session.running);
    }

    #[test]
    fn tool_progress_updates_the_running_tool_part() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();
        let session_ulid = Ulid::from_string(&session_id).unwrap();
        let message_id = Ulid::new();
        let part_id = Ulid::new();
        state.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::PartStart {
                part: Part::ToolCall(ToolCallPart {
                    base: PartBase {
                        id: part_id,
                        message_id,
                        session_id: session_ulid,
                    },
                    tool_name: "shell".into(),
                    call_id: "call-1".into(),
                    state: ToolState::Running(ToolStateRunning {
                        input: serde_json::json!({"command": "pwd"}),
                        output: String::new(),
                        time: ToolTime {
                            start: 0,
                            end: None,
                        },
                    }),
                    raw_input: String::new(),
                }),
            },
        });

        let events = state.apply_server_message(ServerMessage::ToolProgress {
            call_id: "call-1".into(),
            chunk: "/tmp/mew\n".into(),
        });
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::ToolProgress { session_id: changed, call_id, chunk }]
                if changed == &session_id && call_id == "call-1" && chunk == "/tmp/mew\n"
        ));
        assert!(matches!(
            &state.session(&session_id).unwrap().messages[0].parts[0],
            Part::ToolCall(part) if part.state.output() == Some("/tmp/mew\n")
        ));
    }

    #[test]
    fn required_actions_are_stored_and_resolved() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();
        let events = state.apply_server_message(ServerMessage::PermissionRequest {
            request_id: "request-1".into(),
            tool_name: "shell".into(),
            input: serde_json::json!({"command": "pwd"}),
        });
        assert!(
            matches!(events.as_slice(), [ClientEvent::RequiredActionChanged { request_id, .. }] if request_id == "request-1")
        );
        assert_eq!(state.session(&session_id).unwrap().pending_actions.len(), 1);

        state.apply_server_message(ServerMessage::RequestResolved {
            request_id: "request-1".into(),
        });
        assert!(state
            .session(&session_id)
            .unwrap()
            .pending_actions
            .is_empty());
    }

    #[test]
    fn terminal_messages_become_client_events_without_touching_transcript() {
        let mut state = ClientState::default();
        let opened = state.apply_server_message(ServerMessage::TerminalOpened {
            terminal_id: "term-1".into(),
        });
        assert!(matches!(
            opened.as_slice(),
            [ClientEvent::TerminalOpened { terminal_id }] if terminal_id == "term-1"
        ));

        let output = state.apply_server_message(ServerMessage::TerminalOutput {
            terminal_id: "term-1".into(),
            bytes: b"$ pwd\n".to_vec(),
        });
        assert!(matches!(
            output.as_slice(),
            [ClientEvent::TerminalOutput { terminal_id, bytes }]
                if terminal_id == "term-1" && bytes == b"$ pwd\n"
        ));
        assert!(state.sessions.is_empty());
    }

    #[test]
    fn persona_list_keeps_active_persona_in_shared_state() {
        let mut state = ClientState::default();
        let events = state.apply_server_message(ServerMessage::PersonaList {
            personas: vec![
                PersonaInfo {
                    name: "default".into(),
                    description: "Default".into(),
                    color: None,
                    active: false,
                },
                PersonaInfo {
                    name: "reviewer".into(),
                    description: "Reviews code".into(),
                    color: Some("blue".into()),
                    active: true,
                },
            ],
        });

        assert!(matches!(
            events.as_slice(),
            [ClientEvent::PersonaListChanged]
        ));
        assert_eq!(state.personas.len(), 2);
        assert_eq!(state.current_persona.as_deref(), Some("reviewer"));
    }

    #[test]
    fn model_list_updates_shared_state_and_emits_a_catalog_event() {
        let mut state = ClientState::default();
        let events = state.apply_server_message(ServerMessage::ModelList {
            models: vec![ModelInfo {
                id: "fake/fake".into(),
                provider: "fake".into(),
                model: "fake".into(),
                description: Some("local test model".into()),
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            }],
        });

        assert!(matches!(events.as_slice(), [ClientEvent::ModelListChanged]));
        assert_eq!(state.models[0].id, "fake/fake");
    }

    #[test]
    fn permission_mode_change_emits_a_targeted_event() {
        let mut state = ClientState::default();
        let events = state.apply_server_message(ServerMessage::PermissionModeChanged {
            mode: "permissive".into(),
        });

        assert!(matches!(
            events.as_slice(),
            [ClientEvent::PermissionModeChanged { mode }] if mode == "permissive"
        ));
        assert_eq!(state.permission_mode.as_deref(), Some("permissive"));
    }

    #[test]
    fn groups_and_session_metadata_are_reduced_for_sidebar_clients() {
        let mut state = ClientState::default();
        let group = mew_protocol::GroupInfo {
            id: "grp_1".into(),
            name: "Project work".into(),
            color: Some("blue".into()),
            order: 0,
        };
        let events = state.apply_server_message(ServerMessage::GroupList {
            groups: vec![group.clone()],
        });
        assert!(matches!(events.as_slice(), [ClientEvent::GroupsChanged]));
        assert_eq!(state.groups, vec![group]);

        state.session_list.push(SessionInfo {
            session_id: "session-1".into(),
            state: mew_protocol::SessionState::Idle,
            model: None,
            provider: None,
            created_at: 0,
            last_message_at: None,
            summary: Some("A session".into()),
            client_count: 0,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            pinned: false,
            group_id: None,
            change_stats: None,
            usage: None,
            context_tokens: None,
            pending_permissions: 0,
            pending_questions: 0,
            first_message: None,
        });
        let events = state.apply_server_message(ServerMessage::SessionMetaChanged {
            session_id: "session-1".into(),
            archived: false,
            pinned: true,
            group_id: Some("grp_1".into()),
        });
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SessionMetaChanged { session_id }] if session_id == "session-1"
        ));
        assert_eq!(state.session_list[0].group_id.as_deref(), Some("grp_1"));
        assert!(state.session_list[0].pinned);
    }

    #[test]
    fn browser_state_and_events_share_tab_identity() {
        let mut state = ClientState::default();
        let events = state.apply_server_message(ServerMessage::BrowserState {
            open: true,
            url: Some("https://example.com".into()),
            title: Some("Example".into()),
            tab_id: Some("browser-1".into()),
        });

        assert_eq!(state.browser.url.as_deref(), Some("https://example.com"));
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::BrowserStateChanged { tab_id, url, .. }]
                if tab_id.as_deref() == Some("browser-1")
                    && url.as_deref() == Some("https://example.com")
        ));

        let events = state.apply_server_message(ServerMessage::BrowserError {
            message: "navigation failed".into(),
            tab_id: Some("browser-1".into()),
        });
        assert_eq!(state.browser.error.as_deref(), Some("navigation failed"));
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::BrowserError { tab_id, .. }]
                if tab_id.as_deref() == Some("browser-1")
        ));
    }

    #[test]
    fn subagent_lifecycle_is_tracked_per_session() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();

        let events = state.apply_server_message(ServerMessage::SubagentStart {
            parent_call_id: "call-1".into(),
            name: "researcher".into(),
            child_session_id: "01H".into(),
            display_name: Some("Curie".into()),
        });
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SubagentsChanged { session_id: changed }] if changed == &session_id
        ));
        assert_eq!(state.session(&session_id).unwrap().subagents.len(), 1);

        let events = state.apply_server_message(ServerMessage::SubagentStatus {
            parent_call_id: "call-1".into(),
            tool_name: "shell".into(),
            message: "scanning".into(),
        });
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SubagentsChanged { .. }]
        ));
        let entry = &state.session(&session_id).unwrap().subagents[0];
        assert_eq!(entry.task_id, "call-1");
        assert_eq!(entry.status, "scanning");
        assert_eq!(entry.tool_name.as_deref(), Some("shell"));
        assert_eq!(entry.display_name.as_deref(), Some("Curie"));

        let events = state.apply_server_message(ServerMessage::SubagentEnd {
            parent_call_id: "call-1".into(),
            child_session_id: "01H".into(),
            outcome: mew_protocol::SubagentOutcome::Completed,
            manifests: Vec::new(),
        });
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SubagentsChanged { .. }]
        ));
        assert!(state.session(&session_id).unwrap().subagents.is_empty());
    }

    #[test]
    fn subagent_status_without_a_start_creates_an_entry() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();

        let events = state.apply_server_message(ServerMessage::SubagentStatus {
            parent_call_id: "call-9".into(),
            tool_name: "read".into(),
            message: "reading files".into(),
        });

        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SubagentsChanged { .. }]
        ));
        let entry = &state.session(&session_id).unwrap().subagents[0];
        assert_eq!(entry.task_id, "call-9");
        assert_eq!(entry.status, "reading files");
    }

    #[test]
    fn todos_updated_replaces_the_session_todos() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();

        let events = state.apply_server_message(ServerMessage::TodosUpdated {
            todos: vec![
                Todo {
                    id: 1,
                    content: "first".into(),
                    status: "completed".into(),
                    depends_on: Vec::new(),
                },
                Todo {
                    id: 2,
                    content: "second".into(),
                    status: "in_progress".into(),
                    depends_on: vec![1],
                },
            ],
        });

        assert!(matches!(
            events.as_slice(),
            [ClientEvent::TodosChanged { session_id: changed }] if changed == &session_id
        ));
        let todos = &state.session(&session_id).unwrap().todos;
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[1].depends_on, vec![1]);
    }

    #[test]
    fn presence_and_control_yield_are_tracked() {
        let mut state = ClientState::default();

        let events = state.apply_server_message(ServerMessage::ClientAttached {
            client_id: 7,
            client_kind: ClientKind::Web,
        });
        assert!(matches!(events.as_slice(), [ClientEvent::PresenceChanged]));
        assert_eq!(state.presence.len(), 1);
        assert_eq!(state.presence[0].client_kind, ClientKind::Web);

        let events = state.apply_server_message(ServerMessage::ControlYielded { client_id: 7 });
        assert!(matches!(events.as_slice(), [ClientEvent::PresenceChanged]));
        assert_eq!(state.control_yielded_by, Some(7));

        let events = state.apply_server_message(ServerMessage::ClientDetached { client_id: 7 });
        assert!(matches!(events.as_slice(), [ClientEvent::PresenceChanged]));
        assert!(state.presence.is_empty());
    }

    #[test]
    fn session_ready_clears_session_scoped_presence() {
        let mut state = ClientState::default();
        state.presence.push(PresenceEntry {
            client_id: 7,
            client_kind: ClientKind::Web,
        });
        state.control_yielded_by = Some(7);

        state.apply_server_message(ServerMessage::SessionReady {
            session_id: "session-2".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });

        assert!(state.presence.is_empty());
        assert_eq!(state.control_yielded_by, None);
    }

    #[test]
    fn session_metadata_notifications_reduce_into_shared_state() {
        let mut state = ClientState::default();
        state.session_list.push(SessionInfo {
            session_id: "session-1".into(),
            state: mew_protocol::SessionState::Idle,
            model: None,
            provider: None,
            created_at: 0,
            last_message_at: None,
            summary: None,
            client_count: 0,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            pinned: false,
            group_id: None,
            change_stats: None,
            usage: None,
            context_tokens: None,
            pending_permissions: 0,
            pending_questions: 0,
            first_message: Some("first prompt".into()),
        });

        let events = state.apply_server_message(ServerMessage::SessionTitleChanged {
            session_id: "session-1".into(),
            title: "Renamed session".into(),
        });
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SessionListChanged]
        ));
        assert_eq!(state.session_titles["session-1"], "Renamed session");

        state.apply_server_message(ServerMessage::SessionSummaryChanged {
            session_id: "session-1".into(),
            summary: "A concise summary".into(),
        });
        state.apply_server_message(ServerMessage::SessionActivityChanged {
            session_id: "session-1".into(),
            activity: mew_protocol::SessionState::Running,
        });
        state.apply_server_message(ServerMessage::SessionStatsChanged {
            session_id: "session-1".into(),
            added: 12,
            removed: 3,
            files_changed: 2,
        });
        state.apply_server_message(ServerMessage::SessionAttentionChanged {
            session_id: "session-1".into(),
            pending_permissions: 1,
            pending_questions: 2,
        });
        state.apply_server_message(ServerMessage::FlaggedFilesChanged {
            session_id: "session-1".into(),
            files: vec![FlaggedFileWire {
                path: "src/main.rs".into(),
                reason: Some("important".into()),
            }],
        });

        let info = &state.session_list[0];
        assert_eq!(info.summary.as_deref(), Some("A concise summary"));
        assert_eq!(info.state, mew_protocol::SessionState::Running);
        assert_eq!(
            info.change_stats.as_ref().map(|stats| stats.added),
            Some(12)
        );
        assert_eq!(
            info.change_stats.as_ref().map(|stats| stats.removed),
            Some(3)
        );
        assert_eq!(info.pending_permissions, 1);
        assert_eq!(info.pending_questions, 2);
        assert_eq!(state.session("session-1").unwrap().flagged_files.len(), 1);

        let projects = vec![ProjectInfo {
            path: "/tmp/project".into(),
            display_name: "project".into(),
            session_count: 1,
            last_used_at: Some(1),
        }];
        let events = state.apply_server_message(ServerMessage::ProjectList { projects });
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::ProjectListChanged]
        ));
        assert_eq!(state.projects[0].path, "/tmp/project");
    }

    #[test]
    fn slash_results_are_rendered_as_synthetic_assistant_messages() {
        let mut state = ClientState::default();
        session_ready(&mut state);

        let events = state.apply_server_message(ServerMessage::SlashResult {
            text: "compacted".into(),
        });

        assert!(matches!(
            events.as_slice(),
            [ClientEvent::MessageChanged { .. }, ..]
        ));
        let message = state
            .session(state.attached_session.as_deref().unwrap())
            .unwrap()
            .messages
            .last()
            .unwrap();
        assert_eq!(message.role, Role::Assistant);
        assert!(
            matches!(message.parts[0], Part::Text(ref part) if part.text == "compacted" && part.synthetic)
        );
    }

    #[test]
    fn session_usage_changed_updates_state_and_emits_an_event() {
        let mut state = ClientState::default();
        let events = state.apply_server_message(ServerMessage::SessionUsageChanged {
            session_id: "sess-1".into(),
            usage: SessionUsageWire {
                input_tokens: 10,
                output_tokens: 4,
                cache_read_tokens: 2,
                cache_write_tokens: 0,
                cost: 0.5,
                turns: 1,
            },
            context_tokens: Some(9),
        });

        assert!(matches!(
            events.as_slice(),
            [ClientEvent::UsageChanged { session_id }] if session_id == "sess-1"
        ));
        let usage = state.usage("sess-1").unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 4);
        assert_eq!(usage.cost, 0.5);
        assert_eq!(usage.turns, 1);
        assert_eq!(
            state.session("sess-1").unwrap().context_tokens,
            Some(9),
            "usage broadcast must carry the current context reading"
        );
    }

    #[test]
    fn dir_listing_and_fs_changed_emit_file_tree_events() {
        let mut state = ClientState::default();

        let events = state.apply_server_message(ServerMessage::DirListing {
            session_id: "sess-1".into(),
            path: "src".into(),
            entries: vec![DirEntry {
                name: "main.rs".into(),
                is_dir: false,
                size: Some(12),
            }],
        });
        assert!(matches!(events.as_slice(), [ClientEvent::FileTreeChanged]));
        assert_eq!(state.dir_listing_path.as_deref(), Some("src"));
        assert_eq!(state.dir_listing_session_id.as_deref(), Some("sess-1"));
        assert_eq!(state.dir_listing.len(), 1);
        assert_eq!(state.dir_listing[0].name, "main.rs");

        let events = state.apply_server_message(ServerMessage::FsChanged {
            paths: vec!["src/main.rs".into()],
        });
        assert!(matches!(events.as_slice(), [ClientEvent::FileTreeChanged]));
    }
}
