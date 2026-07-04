//! Wire protocol for mew daemon ↔ frontend communication.
//!
//! The daemon owns the agent loop and streams events to connected frontends.
//! Frontends send commands (prompts, cancellations, permission decisions).
//!
//! Wire format: JSON over WebSocket (text frames). MessagePack will be added
//! later as binary frames — same schema, different codec.
//!
//! The protocol mirrors `AgentEvent` but replaces the `oneshot::Sender`
//! channels with request/response pairs keyed by a `request_id`. The daemon
//! sends a `PermissionRequest` or `AskUserRequest` with an ID; the frontend
//! responds with `PermissionResponse` or `AskUserResponse` using the same ID.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Client → Daemon messages
// ---------------------------------------------------------------------------

/// A message from the frontend to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Create a new session.
    NewSession {
        /// Working directory for the session. Defaults to the daemon's cwd.
        cwd: Option<String>,
        /// What kind of client is connecting (TUI, Web, etc.).
        #[serde(default)]
        client_kind: ClientKind,
    },

    /// Attach to an existing session (active or idle). If the session is idle,
    /// the daemon loads its persisted history from disk.
    AttachSession {
        /// Session ID to attach to.
        session_id: String,
        /// What kind of client is connecting (TUI, Web, etc.).
        #[serde(default)]
        client_kind: ClientKind,
    },

    /// List all sessions known to the daemon (active + persisted idle).
    ListSessions,

    /// Delete a session from disk and remove it from the active list.
    DeleteSession {
        session_id: String,
    },

    /// Rename a session (set a custom title).
    RenameSession {
        session_id: String,
        title: String,
    },

    /// Enable or disable auto-generated session titles.
    SetAutoTitle {
        enabled: bool,
    },

    /// Enable or disable idle session summaries.
    SetAutoSummary {
        enabled: bool,
    },

    /// Send a prompt to the active session. The daemon streams events back.
    Prompt {
        text: String,
        /// Optional file attachments (e.g. image paths).
        #[serde(default)]
        attachments: Vec<Attachment>,
    },

    /// Cancel the current turn.
    Cancel,

    /// Respond to a `PermissionRequest` from the daemon.
    PermissionResponse {
        request_id: u64,
        decision: PermissionDecision,
    },

    /// Respond to an `AskUserRequest` from the daemon.
    AskUserResponse {
        request_id: u64,
        /// One answer per question, in order.
        answers: Vec<String>,
    },

    /// Run a slash command on the daemon (the ones that mutate agent state).
    SlashCommand {
        command: String,
    },

    /// List available models from all configured providers.
    ListModels,

    /// Switch the active session to a different model.
    SwitchModel {
        /// Provider ID (e.g. "deepseek", "anthropic").
        provider: String,
        /// Model ID within that provider.
        model: String,
    },

    /// Set or clear the thinking/reasoning variant for the active session.
    /// Pass an empty string or "none" to disable thinking.
    SetThinkingVariant {
        /// Variant name (e.g. "high", "max", "thinking") or empty/none to disable.
        variant: String,
    },

    /// Set the permission mode for the active session.
    /// Mode is one of: "standard", "permissive", "auto", "auto_plus", "dangerous".
    SetPermissionMode {
        /// Lowercase mode id (see `mew_hooks::PermissionMode::id`).
        mode: String,
    },

    /// Yield control of the session. Advisory — other clients can use this
    /// to update their UI (e.g. switch from observer to active input).
    YieldControl {},

    // -- Phase 2: groups & archive --
    CreateGroup {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    UpdateGroup {
        group_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order: Option<u32>,
    },
    DeleteGroup {
        group_id: String,
    },
    AssignSessionGroup {
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<u32>,
    },
    ArchiveSession {
        session_id: String,
        archived: bool,
    },
    PinSession {
        session_id: String,
        pinned: bool,
    },

    // -- Phase 3: File service --
    ListDir {
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    ReadFilePreview {
        session_id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bytes: Option<u64>,
    },
    GitStatus {
        session_id: String,
    },
    WatchWorkspace {
        session_id: String,
        enabled: bool,
    },
    OpenPath {
        session_id: String,
        path: String,
    },

    // -- Flagged files --
    /// Unflag a file (remove from the session's flagged-files set).
    UnflagFile {
        session_id: String,
        path: String,
    },

    /// Ping the daemon for liveness check and version negotiation.
    /// The daemon responds with `ServerMessage::Pong { version }`.
    Ping,
}

/// What kind of client is connected to a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// Terminal UI (`mew chat --connect`).
    Tui,
    /// Browser-based web UI.
    Web,
    /// Headless CLI script.
    Cli,
    /// Mobile app (iOS / Android).
    Mobile,
    /// Unknown / unspecified.
    #[default]
    Unknown,
}

/// Info about a single available model, returned by `ListModels`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Fully-qualified ID: "provider/model" (e.g. "deepseek/deepseek-v4-flash").
    pub id: String,
    /// Provider ID (e.g. "deepseek").
    pub provider: String,
    /// Model ID within the provider (e.g. "deepseek-v4-flash").
    pub model: String,
    /// Human-readable description for the picker UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Available thinking/reasoning variants for this model. Empty if the
    /// model doesn't support configurable thinking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_variants: Vec<ThinkingVariantInfo>,
    /// Maximum context window in tokens, if known from the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
}

/// A named thinking/reasoning variant (e.g. "high", "max", "thinking").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingVariantInfo {
    /// Display name (e.g. "high", "max", "thinking").
    pub name: String,
}

/// Session lifecycle state as exposed by the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Idle,
    /// A turn is currently in progress (provider streaming or tool execution).
    Running,
}

/// Metadata returned by `ListSessions` for one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub state: SessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<i64>,
    /// AI-generated summary of the conversation (if enabled and generated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub client_count: usize,
    /// Working directory for the session, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// True if the last turn ended with an error.
    #[serde(default)]
    pub last_turn_failed: bool,
    /// True if this session has been archived.
    #[serde(default)]
    pub archived: bool,
    /// True if this session is pinned (exempt from auto-archive).
    #[serde(default)]
    pub pinned: bool,
    /// ID of the group this session belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// Cumulative diff stats, if any file changes have been recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_stats: Option<mew_session::ChangeStats>,
    /// Cumulative token usage and cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsageWire>,
    /// Count of pending permission requests (needs human approval).
    #[serde(default)]
    pub pending_permissions: u32,
    /// Count of pending questions (ask_user awaiting response).
    #[serde(default)]
    pub pending_questions: u32,
}

/// Wire-format usage stats for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionUsageWire {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: f64,
    pub turns: u32,
}

impl From<&mew_session::SessionUsage> for SessionUsageWire {
    fn from(u: &mew_session::SessionUsage) -> Self {
        Self {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_write_tokens: u.cache_write_tokens,
            cost: u.cost,
            turns: u.turns,
        }
    }
}

impl From<SessionUsageWire> for mew_session::SessionUsage {
    fn from(u: SessionUsageWire) -> Self {
        Self {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_write_tokens: u.cache_write_tokens,
            cost: u.cost,
            turns: u.turns,
        }
    }
}

/// A file attachment for a prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub path: String,
    /// MIME type if known (e.g. "image/png").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

/// The outcome of a permission check. Mirrors `mew_hooks::PermissionDecision`
/// but is serializable and owned by this crate to avoid a cross-crate coupling
/// for frontends that don't depend on `mew-hooks`.
/// Note that snake case is used for the wire format, so e.g. `AllowOnce` becomes `"allow_once"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
}

impl From<mew_hooks::PermissionDecision> for PermissionDecision {
    fn from(d: mew_hooks::PermissionDecision) -> Self {
        match d {
            mew_hooks::PermissionDecision::AllowOnce => Self::AllowOnce,
            mew_hooks::PermissionDecision::AllowSession => Self::AllowSession,
            mew_hooks::PermissionDecision::Deny => Self::Deny,
            // Prompt is a daemon-internal decision; frontends never send it.
            mew_hooks::PermissionDecision::Prompt => Self::Deny,
        }
    }
}

impl From<PermissionDecision> for mew_hooks::PermissionDecision {
    fn from(d: PermissionDecision) -> Self {
        match d {
            PermissionDecision::AllowOnce => Self::AllowOnce,
            PermissionDecision::AllowSession => Self::AllowSession,
            PermissionDecision::Deny => Self::Deny,
        }
    }
}

// ---------------------------------------------------------------------------
// Daemon → Frontend messages
// ---------------------------------------------------------------------------

/// A message from the daemon to the frontend.
///
/// This is the wire representation of `AgentEvent`. The variants that carry
/// `oneshot::Sender` in `AgentEvent` become ID-paired requests here; the
/// frontend responds with a `ClientMessage` using the same `request_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Sent after `NewSession` succeeds. The session is ready for prompts.
    /// `model` and `provider` are the daemon's current model, so the
    /// frontend can display it immediately without a separate ListModels round-trip.
    SessionReady {
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        /// Current permission mode (lowercase id). Absent means "standard".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_mode: Option<String>,
    },

    /// An error before or outside a session turn.
    Error {
        message: String,
    },

    // -- Streaming events (map 1:1 to AgentEvent variants without channels) --
    /// A raw provider streaming event (text chunks, tool-call parts, etc.).
    Provider {
        event: mew_message::ProviderEventWire,
    },

    /// A user message was sent to this session. Broadcast to all
    /// attached clients so multi-device clients see the prompt.
    /// The sending client can deduplicate by matching text content.
    UserMessage {
        text: String,
    },

    /// A tool execution has started.
    ToolStart {
        call_id: String,
    },

    /// A tool execution has finished.
    ToolEnd {
        call_id: String,
        success: bool,
    },

    /// A part's content or state has changed.
    PartUpdated {
        part_id: mew_message::PartId,
        part: mew_message::Part,
    },

    /// A tool produced intermediate output while running.
    ToolProgress {
        call_id: String,
        chunk: String,
    },

    /// A terminal error occurred.
    ErrorEvent {
        message: String,
    },

    // -- Request/response pairs (replaces oneshot::Sender variants) --
    /// Request user approval for a tool call.
    PermissionRequest {
        request_id: u64,
        tool_name: String,
        /// The tool input as JSON.
        input: serde_json::Value,
    },

    /// Request user approval for a path outside the workspace.
    WorkspacePermissionRequest {
        request_id: u64,
        path: String,
    },

    /// Ask the user one to four free-text questions.
    AskUserRequest {
        request_id: u64,
        call_id: String,
        questions: Vec<Question>,
    },

    // -- Subagent events --
    SubagentStart {
        parent_call_id: String,
        name: String,
        child_session_id: String,
        display_name: Option<String>,
    },

    SubagentStatus {
        parent_call_id: String,
        tool_name: String,
        message: String,
    },

    SubagentEnd {
        parent_call_id: String,
        child_session_id: String,
        outcome: SubagentOutcome,
    },

    /// A permission request from a child subagent.
    SubagentPermissionRequest {
        request_id: u64,
        parent_call_id: String,
        tool_name: String,
        input: serde_json::Value,
    },

    // -- Session-level events --
    /// The session's todo list changed.
    TodosUpdated {
        todos: Vec<Todo>,
    },

    /// A `switch_persona` tool call was queued and the turn ended.
    PersonaSwitchRequested {
        name: String,
    },

    /// A background shell job state changed.
    JobUpdate {
        job_id: String,
        command: String,
        state: String,
    },

    /// A slash command produced a text result.
    SlashResult {
        text: String,
    },

    /// Broadcast when a pending permission / ask-user / subagent-permission
    /// request has been resolved by any attached client. All frontends should
    /// dismiss the matching modal.
    RequestResolved {
        request_id: u64,
    },

    /// Broadcast when the session context has been cleared (e.g. `/clear`).
    /// All attached clients should wipe their message list.
    SessionCleared,

    // -- Session management --
    /// Response to `ListSessions`.
    SessionList {
        sessions: Vec<SessionInfo>,
    },

    /// Full message history replay for a resumed session. Only sent to the
    /// client that triggered the resume.
    SessionHistory {
        messages: Vec<mew_message::Message>,
    },

    // -- Model management --
    /// Response to `ListModels`: the full set of models the daemon can build.
    ModelList {
        models: Vec<ModelInfo>,
    },

    /// Response to `SwitchModel`: confirms the switch succeeded.
    ModelSwitched {
        provider: String,
        model: String,
    },

    /// Response to `SetThinkingVariant`: confirms the variant was applied.
    /// `variant` is `None` when thinking was disabled.
    ThinkingVariantChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
    },

    /// Broadcast when the permission mode changes. Sent to all attached
    /// clients so multi-device stays in sync.
    PermissionModeChanged {
        /// Lowercase mode id (e.g. "standard", "dangerous").
        mode: String,
    },

    /// A new client attached to the session. Broadcast to all other clients.
    ClientAttached {
        client_id: u64,
        client_kind: ClientKind,
    },

    /// A client detached from the session. Broadcast to remaining clients.
    ClientDetached {
        client_id: u64,
    },

    /// Control was yielded. Advisory — other clients can become active.
    ControlYielded {
        /// The client that yielded.
        client_id: u64,
    },

    /// The daemon generated a title for the session. Frontends should update
    /// their session title display.
    SessionTitleChanged {
        session_id: String,
        title: String,
    },

    /// The daemon generated a summary for an idle session. Frontends
    /// should display this in the session list / detail view.
    SessionSummaryChanged {
        session_id: String,
        summary: String,
    },

    // -- Phase 1: session activity & stats --
    SessionActivityChanged {
        session_id: String,
        activity: SessionState,
    },
    SessionStatsChanged {
        session_id: String,
        added: u64,
        removed: u64,
        files_changed: u64,
    },

    // -- Phase 2: groups --
    GroupList {
        groups: Vec<GroupInfo>,
    },
    GroupsChanged {
        groups: Vec<GroupInfo>,
    },

    // -- Phase 3: File service responses --
    DirListing {
        path: String,
        entries: Vec<DirEntry>,
    },
    FilePreview {
        path: String,
        content: String,
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    GitStatusResult {
        entries: Vec<GitEntry>,
    },
    FsChanged {
        paths: Vec<String>,
    },

    // -- Cost & usage --
    /// Broadcast at turn end with updated cumulative usage.
    SessionUsageChanged {
        session_id: String,
        usage: SessionUsageWire,
    },

    // -- Notifications --
    /// Cross-session alert (permission needed, turn complete, etc.).
    /// Sent to ALL clients regardless of session attachment.
    SessionAlert {
        session_id: String,
        title: String,
        kind: AlertKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },

    // -- Flagged files visibility --
    /// Broadcast when the flagged-files set changes.
    FlaggedFilesChanged {
        session_id: String,
        files: Vec<FlaggedFileWire>,
    },

    // -- Session meta changes (archive/pin/group) --
    /// Broadcast when a session's archived/pinned/group_id changes,
    /// so all clients can update their session rail.
    SessionMetaChanged {
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        archived: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pinned: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group_id: Option<String>,
    },

    // -- "Needs you" attention --
    /// Broadcast when a session's pending permission/question count changes.
    SessionAttentionChanged {
        session_id: String,
        pending_permissions: u32,
        pending_questions: u32,
    },

    /// Response to `ClientMessage::Ping`. Carries the daemon's version
    /// so clients can detect version skew.
    Pong {
        /// Daemon version string (e.g. "0.2.0").
        version: String,
    },
}

/// Wire-format info about a flagged file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlaggedFileWire {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Alert kind for cross-session notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    TurnComplete,
    TurnFailed,
    PermissionNeeded,
    InputNeeded,
}

/// A question for `AskUserRequest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub prompt: String,
    pub options: Vec<QuestionOption>,
}

/// An option within a question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// The outcome of a subagent run.
///
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentOutcome {
    Completed,
    Cancelled,
    Failed { reason: String },
}

/// A todo item, mirroring `mew_agent::Todo` but serializable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: usize,
    pub content: String,
    pub status: String,
    pub depends_on: Vec<usize>,
}

/// Metadata for a session group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub order: u32,
}

/// One entry in a directory listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// One entry in a git status result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GitEntry {
    pub path: String,
    pub status: GitFileStatus,
}

/// Git file status, simplified from porcelain output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

// ---------------------------------------------------------------------------
// Codec helpers
// ---------------------------------------------------------------------------

/// Encode a message as a JSON string (WebSocket text frame).
pub fn encode_json<T: Serialize>(msg: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(msg)
}

/// Decode a message from a JSON string (WebSocket text frame).
pub fn decode_json<'a, T: Deserialize<'a>>(text: &'a str) -> Result<T, serde_json::Error> {
    serde_json::from_str(text)
}

// ---------------------------------------------------------------------------
// Conversions from agent types to wire types
// ---------------------------------------------------------------------------

/// Convert a `ProviderEvent` to its wire representation.
/// (Free function because orphan rules prevent a `From` impl in this crate.)
pub fn provider_event_to_wire(e: &mew_provider::ProviderEvent) -> mew_message::ProviderEventWire {
    match e {
        mew_provider::ProviderEvent::PartStart { part } => {
            mew_message::ProviderEventWire::PartStart { part: part.clone() }
        }
        mew_provider::ProviderEvent::PartDelta {
            part_id,
            field,
            delta,
        } => mew_message::ProviderEventWire::PartDelta {
            part_id: *part_id,
            field: field.to_string(),
            delta: delta.clone(),
        },
        mew_provider::ProviderEvent::PartEnd { part_id } => {
            mew_message::ProviderEventWire::PartEnd { part_id: *part_id }
        }
        mew_provider::ProviderEvent::MessageEnd {
            finish,
            usage,
            cost,
        } => mew_message::ProviderEventWire::MessageEnd {
            finish: *finish,
            usage: *usage,
            cost: *cost,
        },
        mew_provider::ProviderEvent::RetryWait {
            attempt,
            max_attempts,
            delay_secs,
            reason,
        } => mew_message::ProviderEventWire::RetryWait {
            attempt: *attempt,
            max_attempts: *max_attempts,
            delay_secs: *delay_secs,
            reason: reason.clone(),
        },
        mew_provider::ProviderEvent::Error(e) => mew_message::ProviderEventWire::Error(e.clone()),
    }
}

/// Convert a `SubagentOutcome` to its wire representation.
pub fn subagent_outcome_to_wire(o: &mew_subagents::SubagentOutcome) -> SubagentOutcome {
    match o {
        mew_subagents::SubagentOutcome::Completed => SubagentOutcome::Completed,
        mew_subagents::SubagentOutcome::Cancelled => SubagentOutcome::Cancelled,
        mew_subagents::SubagentOutcome::Failed { reason } => SubagentOutcome::Failed {
            reason: reason.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{Part, PartBase, TextPart};

    // -- Helpers -------------------------------------------------------------

    fn round_trip<T: Serialize + serde::de::DeserializeOwned>(msg: &T) -> T {
        let json = encode_json(msg).unwrap();
        decode_json(&json).unwrap()
    }

    fn sample_text_part() -> Part {
        Part::Text(TextPart {
            base: PartBase {
                id: mew_message::PartId::new(),
                message_id: mew_message::MessageId::new(),
                session_id: mew_message::SessionId::new(),
            },
            text: "hello".into(),
            synthetic: false,
        })
    }

    // -- Existing tests (kept verbatim) --------------------------------------

    #[test]
    fn test_roundtrip_prompt() {
        let msg = ClientMessage::Prompt {
            text: "hello world".into(),
            attachments: vec![],
        };
        let json = encode_json(&msg).unwrap();
        let decoded: ClientMessage = decode_json(&json).unwrap();
        match decoded {
            ClientMessage::Prompt { text, .. } => assert_eq!(text, "hello world"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_roundtrip_permission() {
        let msg = ClientMessage::PermissionResponse {
            request_id: 42,
            decision: PermissionDecision::AllowOnce,
        };
        let json = encode_json(&msg).unwrap();
        let decoded: ClientMessage = decode_json(&json).unwrap();
        match decoded {
            ClientMessage::PermissionResponse {
                request_id,
                decision,
            } => {
                assert_eq!(request_id, 42);
                assert_eq!(decision, PermissionDecision::AllowOnce);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_roundtrip_server_event() {
        let msg = ServerMessage::ToolStart {
            call_id: "call_123".into(),
        };
        let json = encode_json(&msg).unwrap();
        let decoded: ServerMessage = decode_json(&json).unwrap();
        match decoded {
            ServerMessage::ToolStart { call_id } => assert_eq!(call_id, "call_123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_tagged_enums() {
        // Verify the serde tag produces {"type": "..."} shape.
        let msg = ClientMessage::Cancel;
        let json = encode_json(&msg).unwrap();
        assert!(json.contains(r#""type":"cancel""#));
    }

    // -- ClientMessage: exhaustive variant coverage --------------------------

    #[test]
    fn client_message_new_session_cwd_none_roundtrip() {
        let m = ClientMessage::NewSession {
            cwd: None,
            client_kind: ClientKind::Unknown,
        };
        match round_trip(&m) {
            ClientMessage::NewSession { cwd, .. } => assert!(cwd.is_none()),
            _ => panic!(),
        }
    }

    #[test]
    fn client_message_new_session_cwd_some_roundtrip() {
        let m = ClientMessage::NewSession {
            cwd: Some("/tmp/work".into()),
            client_kind: ClientKind::Unknown,
        };
        match round_trip(&m) {
            ClientMessage::NewSession { cwd, .. } => assert_eq!(cwd.as_deref(), Some("/tmp/work")),
            _ => panic!(),
        }
    }

    #[test]
    fn client_message_prompt_with_attachments_roundtrip() {
        let m = ClientMessage::Prompt {
            text: "look at this".into(),
            attachments: vec![
                Attachment {
                    path: "/x.png".into(),
                    mime: Some("image/png".into()),
                },
                Attachment {
                    path: "/y.jpg".into(),
                    mime: None,
                },
            ],
        };
        let j = encode_json(&m).unwrap();
        // Omitting mime should not serialize (skip_serializing_if).
        assert!(!j.contains(r#""mime":null"#));
        match round_trip(&m) {
            ClientMessage::Prompt { text, attachments } => {
                assert_eq!(text, "look at this");
                assert_eq!(attachments.len(), 2);
                assert_eq!(attachments[0].path, "/x.png");
                assert_eq!(attachments[0].mime.as_deref(), Some("image/png"));
                assert_eq!(attachments[1].mime, None);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn client_message_cancel_roundtrip() {
        let m = ClientMessage::Cancel;
        assert!(matches!(round_trip(&m), ClientMessage::Cancel));
    }

    #[test]
    fn client_message_permission_response_all_decisions_roundtrip() {
        for (decision, expected) in [
            (PermissionDecision::AllowOnce, 0u8),
            (PermissionDecision::AllowSession, 1u8),
            (PermissionDecision::Deny, 2u8),
        ] {
            let m = ClientMessage::PermissionResponse {
                request_id: 7,
                decision,
            };
            match round_trip(&m) {
                ClientMessage::PermissionResponse {
                    request_id,
                    decision: d,
                } => {
                    assert_eq!(request_id, 7);
                    assert_eq!(d as u8, expected);
                }
                _ => panic!(),
            }
        }
    }

    #[test]
    fn client_message_ask_user_response_multiple_answers_roundtrip() {
        let m = ClientMessage::AskUserResponse {
            request_id: 5,
            answers: vec!["alpha".into(), "beta".into(), "gamma".into()],
        };
        match round_trip(&m) {
            ClientMessage::AskUserResponse {
                request_id,
                answers,
            } => {
                assert_eq!(request_id, 5);
                assert_eq!(answers, vec!["alpha", "beta", "gamma"]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn client_message_slash_command_roundtrip() {
        let m = ClientMessage::SlashCommand {
            command: "/clear".into(),
        };
        match round_trip(&m) {
            ClientMessage::SlashCommand { command } => assert_eq!(command, "/clear"),
            _ => panic!(),
        }
    }

    // -- ServerMessage: every variant round-trips ----------------------------

    #[test]
    fn server_message_session_ready_roundtrip() {
        let m = ServerMessage::SessionReady {
            session_id: "01H".into(),
            model: Some("deepseek-v4-flash".into()),
            provider: Some("deepseek".into()),
            permission_mode: None,
        };
        match round_trip(&m) {
            ServerMessage::SessionReady {
                session_id,
                model,
                provider,
                permission_mode,
            } => {
                assert_eq!(session_id, "01H");
                assert_eq!(model.as_deref(), Some("deepseek-v4-flash"));
                assert_eq!(provider.as_deref(), Some("deepseek"));
                assert!(permission_mode.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_error_roundtrip() {
        let m = ServerMessage::Error {
            message: "boom".into(),
        };
        match round_trip(&m) {
            ServerMessage::Error { message } => assert_eq!(message, "boom"),
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_provider_part_start_text_roundtrip() {
        let part = sample_text_part();
        let m = ServerMessage::Provider {
            event: mew_message::ProviderEventWire::PartStart { part: part.clone() },
        };
        match round_trip(&m) {
            ServerMessage::Provider { event } => match event {
                mew_message::ProviderEventWire::PartStart { part: p } => {
                    assert_eq!(p.id(), part.id());
                }
                _ => panic!("wrong wire variant"),
            },
            _ => panic!("wrong server variant"),
        }
    }

    #[test]
    fn server_message_provider_part_delta_field_becomes_string() {
        // The wire form replaces &'static str field with String. Verify that
        // a delta with field="text" round-trips correctly.
        let pid = mew_message::PartId::new();
        let m = ServerMessage::Provider {
            event: mew_message::ProviderEventWire::PartDelta {
                part_id: pid,
                field: "text".into(),
                delta: "abc".into(),
            },
        };
        match round_trip(&m) {
            ServerMessage::Provider { event } => match event {
                mew_message::ProviderEventWire::PartDelta {
                    part_id,
                    field,
                    delta,
                } => {
                    assert_eq!(part_id, pid);
                    assert_eq!(field, "text");
                    assert_eq!(delta, "abc");
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_provider_message_end_tool_use_roundtrip() {
        let m = ServerMessage::Provider {
            event: mew_message::ProviderEventWire::MessageEnd {
                finish: mew_message::Finish::ToolUse,
                usage: mew_message::Tokens::default(),
                cost: 0.0,
            },
        };
        match round_trip(&m) {
            ServerMessage::Provider { event } => match event {
                mew_message::ProviderEventWire::MessageEnd { finish, .. } => {
                    assert_eq!(finish, mew_message::Finish::ToolUse);
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_provider_retry_wait_roundtrip() {
        let m = ServerMessage::Provider {
            event: mew_message::ProviderEventWire::RetryWait {
                attempt: 2,
                max_attempts: 5,
                delay_secs: 1,
                reason: "429".into(),
            },
        };
        match round_trip(&m) {
            ServerMessage::Provider { event } => match event {
                mew_message::ProviderEventWire::RetryWait {
                    attempt,
                    max_attempts,
                    delay_secs,
                    reason,
                } => {
                    assert_eq!(attempt, 2);
                    assert_eq!(max_attempts, 5);
                    assert_eq!(delay_secs, 1);
                    assert_eq!(reason, "429");
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_provider_error_roundtrip() {
        let err = mew_message::MessageError {
            kind: mew_message::ErrorKind::Network,
            message: "rate limit".into(),
        };
        let m = ServerMessage::Provider {
            event: mew_message::ProviderEventWire::Error(err.clone()),
        };
        match round_trip(&m) {
            ServerMessage::Provider { event } => match event {
                mew_message::ProviderEventWire::Error(e) => {
                    assert_eq!(e.message, "rate limit");
                    assert_eq!(e.kind, mew_message::ErrorKind::Network);
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_tool_start_end_roundtrip() {
        let start = ServerMessage::ToolStart {
            call_id: "c1".into(),
        };
        let end = ServerMessage::ToolEnd {
            call_id: "c1".into(),
            success: false,
        };
        match round_trip(&start) {
            ServerMessage::ToolStart { call_id } => assert_eq!(call_id, "c1"),
            _ => panic!(),
        }
        match round_trip(&end) {
            ServerMessage::ToolEnd { call_id, success } => {
                assert_eq!(call_id, "c1");
                assert!(!success);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_part_updated_text_roundtrip() {
        let part = sample_text_part();
        let part_id = part.id();
        let m = ServerMessage::PartUpdated {
            part_id,
            part: part.clone(),
        };
        match round_trip(&m) {
            ServerMessage::PartUpdated { part_id, part: p } => {
                assert_eq!(part_id, part_id);
                assert_eq!(p.id(), part.id());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_tool_progress_roundtrip() {
        let m = ServerMessage::ToolProgress {
            call_id: "c1".into(),
            chunk: "out".into(),
        };
        match round_trip(&m) {
            ServerMessage::ToolProgress { call_id, chunk } => {
                assert_eq!(call_id, "c1");
                assert_eq!(chunk, "out");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_error_event_roundtrip() {
        let m = ServerMessage::ErrorEvent {
            message: "fail".into(),
        };
        match round_trip(&m) {
            ServerMessage::ErrorEvent { message } => assert_eq!(message, "fail"),
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_permission_request_roundtrip() {
        let m = ServerMessage::PermissionRequest {
            request_id: 1,
            tool_name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        match round_trip(&m) {
            ServerMessage::PermissionRequest {
                request_id,
                tool_name,
                input,
            } => {
                assert_eq!(request_id, 1);
                assert_eq!(tool_name, "bash");
                assert_eq!(input["command"], "ls");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_workspace_permission_request_roundtrip() {
        let m = ServerMessage::WorkspacePermissionRequest {
            request_id: 2,
            path: "/etc/passwd".into(),
        };
        match round_trip(&m) {
            ServerMessage::WorkspacePermissionRequest { request_id, path } => {
                assert_eq!(request_id, 2);
                assert_eq!(path, "/etc/passwd");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_ask_user_request_multiple_questions_roundtrip() {
        let m = ServerMessage::AskUserRequest {
            request_id: 3,
            call_id: "ask_1".into(),
            questions: vec![
                Question {
                    prompt: "Pick one".into(),
                    options: vec![
                        QuestionOption {
                            label: "a".into(),
                            description: "first".into(),
                        },
                        QuestionOption {
                            label: "b".into(),
                            description: "second".into(),
                        },
                    ],
                },
                Question {
                    prompt: "Or two?".into(),
                    options: vec![QuestionOption {
                        label: "yes".into(),
                        description: "ok".into(),
                    }],
                },
            ],
        };
        match round_trip(&m) {
            ServerMessage::AskUserRequest {
                request_id,
                call_id,
                questions,
            } => {
                assert_eq!(request_id, 3);
                assert_eq!(call_id, "ask_1");
                assert_eq!(questions.len(), 2);
                assert_eq!(questions[0].options.len(), 2);
                assert_eq!(questions[1].options[0].label, "yes");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_subagent_events_roundtrip() {
        let start = ServerMessage::SubagentStart {
            parent_call_id: "p1".into(),
            name: "researcher".into(),
            child_session_id: "01H".into(),
            display_name: Some("Curie".into()),
        };
        let status = ServerMessage::SubagentStatus {
            parent_call_id: "p1".into(),
            tool_name: "bash".into(),
            message: "scanning".into(),
        };
        let end = ServerMessage::SubagentEnd {
            parent_call_id: "p1".into(),
            child_session_id: "01H".into(),
            outcome: SubagentOutcome::Completed,
        };

        match round_trip(&start) {
            ServerMessage::SubagentStart { display_name, .. } => {
                assert_eq!(display_name.as_deref(), Some("Curie"))
            }
            _ => panic!(),
        }
        match round_trip(&status) {
            ServerMessage::SubagentStatus { message, .. } => assert_eq!(message, "scanning"),
            _ => panic!(),
        }
        match round_trip(&end) {
            ServerMessage::SubagentEnd { outcome, .. } => {
                assert!(matches!(outcome, SubagentOutcome::Completed))
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_subagent_outcome_failed_carries_reason() {
        let m = ServerMessage::SubagentEnd {
            parent_call_id: "p".into(),
            child_session_id: "01H".into(),
            outcome: SubagentOutcome::Failed {
                reason: "timed out".into(),
            },
        };
        match round_trip(&m) {
            ServerMessage::SubagentEnd { outcome, .. } => match outcome {
                SubagentOutcome::Failed { reason } => assert_eq!(reason, "timed out"),
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_subagent_permission_request_roundtrip() {
        let m = ServerMessage::SubagentPermissionRequest {
            request_id: 9,
            parent_call_id: "p1".into(),
            tool_name: "write".into(),
            input: serde_json::json!({"path": "/x"}),
        };
        match round_trip(&m) {
            ServerMessage::SubagentPermissionRequest {
                request_id,
                parent_call_id,
                tool_name,
                ..
            } => {
                assert_eq!(request_id, 9);
                assert_eq!(parent_call_id, "p1");
                assert_eq!(tool_name, "write");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_todos_updated_with_deps_roundtrip() {
        let m = ServerMessage::TodosUpdated {
            todos: vec![
                Todo {
                    id: 1,
                    content: "first".into(),
                    status: "pending".into(),
                    depends_on: vec![],
                },
                Todo {
                    id: 2,
                    content: "second".into(),
                    status: "pending".into(),
                    depends_on: vec![1],
                },
                Todo {
                    id: 3,
                    content: "third".into(),
                    status: "in_progress".into(),
                    depends_on: vec![1, 2],
                },
            ],
        };
        match round_trip(&m) {
            ServerMessage::TodosUpdated { todos } => {
                assert_eq!(todos.len(), 3);
                assert_eq!(todos[1].depends_on, vec![1]);
                assert_eq!(todos[2].depends_on, vec![1, 2]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_persona_switch_roundtrip() {
        let m = ServerMessage::PersonaSwitchRequested {
            name: "explorer".into(),
        };
        match round_trip(&m) {
            ServerMessage::PersonaSwitchRequested { name } => assert_eq!(name, "explorer"),
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_job_update_roundtrip() {
        let m = ServerMessage::JobUpdate {
            job_id: "j1".into(),
            command: "sleep 10".into(),
            state: "running".into(),
        };
        match round_trip(&m) {
            ServerMessage::JobUpdate {
                job_id,
                command,
                state,
            } => {
                assert_eq!(job_id, "j1");
                assert_eq!(command, "sleep 10");
                assert_eq!(state, "running");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_slash_result_roundtrip() {
        let m = ServerMessage::SlashResult {
            text: "compacted".into(),
        };
        match round_trip(&m) {
            ServerMessage::SlashResult { text } => assert_eq!(text, "compacted"),
            _ => panic!(),
        }
    }

    // -- Negative tests: malformed / missing / wrong-type --------------------

    #[test]
    fn malformed_json_is_rejected() {
        let bad = r#"{"type":"prompt","text":#}"#;
        let result: Result<ClientMessage, _> = decode_json(bad);
        assert!(result.is_err(), "malformed JSON must not decode");
    }

    #[test]
    fn missing_required_field_is_rejected() {
        // Prompt requires `text`. Drop it.
        let bad = r#"{"type":"prompt","attachments":[]}"#;
        let result: Result<ClientMessage, _> = decode_json(bad);
        assert!(result.is_err(), "Prompt without text must fail to decode");
    }

    #[test]
    fn wrong_type_for_known_field_is_rejected() {
        // request_id must be a number, not a string.
        let bad = r#"{"type":"permission_response","request_id":"oops","decision":"allow_once"}"#;
        let result: Result<ClientMessage, _> = decode_json(bad);
        assert!(result.is_err(), "string in number field must be rejected");
    }

    #[test]
    fn unknown_variant_tag_is_rejected() {
        let bad = r#"{"type":"NoSuchMessage"}"#;
        let result: Result<ClientMessage, _> = decode_json(bad);
        assert!(result.is_err(), "unknown tag must be rejected");
    }

    #[test]
    fn unknown_server_variant_tag_is_rejected() {
        let bad = r#"{"type":"NoSuchServerEvent"}"#;
        let result: Result<ServerMessage, _> = decode_json(bad);
        assert!(result.is_err());
    }

    #[test]
    fn missing_type_tag_is_rejected() {
        // The serde `tag = "type"` attribute requires every payload to have
        // a type discriminator.
        let bad = r#"{"text":"hi"}"#;
        let result: Result<ClientMessage, _> = decode_json(bad);
        assert!(result.is_err());
    }

    // -- Wire-tag shape for every variant ------------------------------------

    #[test]
    fn every_client_variant_has_distinct_type_tag() {
        // Exhaustive: every variant of `ClientMessage` must serialize with the
        // snake_case `type` tag derived from its Rust name, and every tag must
        // be distinct. Catches typos in `#[serde(rename)]`, camelCase leaks,
        // and accidental tag collisions.
        let samples: Vec<(&'static str, ClientMessage)> = vec![
            (
                "new_session",
                ClientMessage::NewSession {
                    cwd: None,
                    client_kind: ClientKind::Unknown,
                },
            ),
            (
                "attach_session",
                ClientMessage::AttachSession {
                    session_id: "s".into(),
                    client_kind: ClientKind::Unknown,
                },
            ),
            ("list_sessions", ClientMessage::ListSessions),
            (
                "delete_session",
                ClientMessage::DeleteSession {
                    session_id: "s".into(),
                },
            ),
            (
                "rename_session",
                ClientMessage::RenameSession {
                    session_id: "s".into(),
                    title: "t".into(),
                },
            ),
            (
                "set_auto_title",
                ClientMessage::SetAutoTitle { enabled: true },
            ),
            (
                "set_auto_summary",
                ClientMessage::SetAutoSummary { enabled: true },
            ),
            (
                "prompt",
                ClientMessage::Prompt {
                    text: "x".into(),
                    attachments: vec![],
                },
            ),
            ("cancel", ClientMessage::Cancel),
            (
                "permission_response",
                ClientMessage::PermissionResponse {
                    request_id: 0,
                    decision: PermissionDecision::AllowOnce,
                },
            ),
            (
                "ask_user_response",
                ClientMessage::AskUserResponse {
                    request_id: 0,
                    answers: vec![],
                },
            ),
            (
                "slash_command",
                ClientMessage::SlashCommand {
                    command: "/help".into(),
                },
            ),
            ("list_models", ClientMessage::ListModels),
            (
                "switch_model",
                ClientMessage::SwitchModel {
                    provider: "p".into(),
                    model: "m".into(),
                },
            ),
            (
                "set_thinking_variant",
                ClientMessage::SetThinkingVariant {
                    variant: "high".into(),
                },
            ),
            (
                "set_permission_mode",
                ClientMessage::SetPermissionMode {
                    mode: "standard".into(),
                },
            ),
            ("yield_control", ClientMessage::YieldControl {}),
            (
                "create_group",
                ClientMessage::CreateGroup {
                    name: "g".into(),
                    color: None,
                },
            ),
            (
                "update_group",
                ClientMessage::UpdateGroup {
                    group_id: "g".into(),
                    name: None,
                    color: None,
                    order: None,
                },
            ),
            (
                "delete_group",
                ClientMessage::DeleteGroup {
                    group_id: "g".into(),
                },
            ),
            (
                "assign_session_group",
                ClientMessage::AssignSessionGroup {
                    session_id: "s".into(),
                    group_id: None,
                    position: None,
                },
            ),
            (
                "archive_session",
                ClientMessage::ArchiveSession {
                    session_id: "s".into(),
                    archived: true,
                },
            ),
            (
                "pin_session",
                ClientMessage::PinSession {
                    session_id: "s".into(),
                    pinned: true,
                },
            ),
            (
                "list_dir",
                ClientMessage::ListDir {
                    session_id: "s".into(),
                    path: None,
                },
            ),
            (
                "read_file_preview",
                ClientMessage::ReadFilePreview {
                    session_id: "s".into(),
                    path: "/x".into(),
                    max_bytes: None,
                },
            ),
            (
                "git_status",
                ClientMessage::GitStatus {
                    session_id: "s".into(),
                },
            ),
            (
                "watch_workspace",
                ClientMessage::WatchWorkspace {
                    session_id: "s".into(),
                    enabled: true,
                },
            ),
            (
                "open_path",
                ClientMessage::OpenPath {
                    session_id: "s".into(),
                    path: "/x".into(),
                },
            ),
            (
                "unflag_file",
                ClientMessage::UnflagFile {
                    session_id: "s".into(),
                    path: "/x".into(),
                },
            ),
            ("ping", ClientMessage::Ping),
        ];

        let mut seen: Vec<&'static str> = Vec::with_capacity(samples.len());
        for (expected, msg) in &samples {
            let json = encode_json(msg).unwrap();
            assert!(
                json.contains(&format!(r#""type":"{}""#, expected)),
                "tag mismatch for {}: expected {:?}, got {}",
                expected,
                expected,
                json
            );
            assert!(
                !seen.contains(expected),
                "duplicate expected tag {:?} in client samples",
                expected
            );
            seen.push(*expected);
        }
        assert_eq!(
            seen.len(),
            samples.len(),
            "client tag list should have no duplicates"
        );
    }

    #[test]
    fn every_server_variant_has_distinct_type_tag() {
        // Exhaustive: every variant of `ServerMessage` must serialize with the
        // snake_case `type` tag derived from its Rust name, and every tag must
        // be distinct. Catches typos in `#[serde(rename)]`, camelCase leaks,
        // and accidental tag collisions.
        let part = sample_text_part();
        let samples: Vec<(&'static str, ServerMessage)> = vec![
            (
                "session_ready",
                ServerMessage::SessionReady {
                    session_id: "s".into(),
                    model: None,
                    provider: None,
                    permission_mode: None,
                },
            ),
            (
                "error",
                ServerMessage::Error {
                    message: "boom".into(),
                },
            ),
            (
                "provider",
                ServerMessage::Provider {
                    event: mew_message::ProviderEventWire::PartStart { part: part.clone() },
                },
            ),
            (
                "user_message",
                ServerMessage::UserMessage {
                    text: "hi".into(),
                },
            ),
            (
                "tool_start",
                ServerMessage::ToolStart {
                    call_id: "c".into(),
                },
            ),
            (
                "tool_end",
                ServerMessage::ToolEnd {
                    call_id: "c".into(),
                    success: true,
                },
            ),
            (
                "part_updated",
                ServerMessage::PartUpdated {
                    part_id: mew_message::PartId::new(),
                    part: part.clone(),
                },
            ),
            (
                "tool_progress",
                ServerMessage::ToolProgress {
                    call_id: "c".into(),
                    chunk: "x".into(),
                },
            ),
            (
                "error_event",
                ServerMessage::ErrorEvent {
                    message: "boom".into(),
                },
            ),
            (
                "permission_request",
                ServerMessage::PermissionRequest {
                    request_id: 0,
                    tool_name: "bash".into(),
                    input: serde_json::Value::Null,
                },
            ),
            (
                "workspace_permission_request",
                ServerMessage::WorkspacePermissionRequest {
                    request_id: 0,
                    path: "/x".into(),
                },
            ),
            (
                "ask_user_request",
                ServerMessage::AskUserRequest {
                    request_id: 0,
                    call_id: "c".into(),
                    questions: vec![],
                },
            ),
            (
                "subagent_start",
                ServerMessage::SubagentStart {
                    parent_call_id: "p".into(),
                    name: "researcher".into(),
                    child_session_id: "c".into(),
                    display_name: None,
                },
            ),
            (
                "subagent_status",
                ServerMessage::SubagentStatus {
                    parent_call_id: "p".into(),
                    tool_name: "bash".into(),
                    message: "scanning".into(),
                },
            ),
            (
                "subagent_end",
                ServerMessage::SubagentEnd {
                    parent_call_id: "p".into(),
                    child_session_id: "c".into(),
                    outcome: SubagentOutcome::Completed,
                },
            ),
            (
                "subagent_permission_request",
                ServerMessage::SubagentPermissionRequest {
                    request_id: 0,
                    parent_call_id: "p".into(),
                    tool_name: "bash".into(),
                    input: serde_json::Value::Null,
                },
            ),
            (
                "todos_updated",
                ServerMessage::TodosUpdated { todos: vec![] },
            ),
            (
                "persona_switch_requested",
                ServerMessage::PersonaSwitchRequested {
                    name: "n".into(),
                },
            ),
            (
                "job_update",
                ServerMessage::JobUpdate {
                    job_id: "j".into(),
                    command: "ls".into(),
                    state: "running".into(),
                },
            ),
            (
                "slash_result",
                ServerMessage::SlashResult {
                    text: "ok".into(),
                },
            ),
            (
                "request_resolved",
                ServerMessage::RequestResolved { request_id: 0 },
            ),
            ("session_cleared", ServerMessage::SessionCleared),
            (
                "session_list",
                ServerMessage::SessionList { sessions: vec![] },
            ),
            (
                "session_history",
                ServerMessage::SessionHistory { messages: vec![] },
            ),
            (
                "model_list",
                ServerMessage::ModelList { models: vec![] },
            ),
            (
                "model_switched",
                ServerMessage::ModelSwitched {
                    provider: "p".into(),
                    model: "m".into(),
                },
            ),
            (
                "thinking_variant_changed",
                ServerMessage::ThinkingVariantChanged { variant: None },
            ),
            (
                "permission_mode_changed",
                ServerMessage::PermissionModeChanged {
                    mode: "standard".into(),
                },
            ),
            (
                "client_attached",
                ServerMessage::ClientAttached {
                    client_id: 0,
                    client_kind: ClientKind::Unknown,
                },
            ),
            (
                "client_detached",
                ServerMessage::ClientDetached { client_id: 0 },
            ),
            (
                "control_yielded",
                ServerMessage::ControlYielded { client_id: 0 },
            ),
            (
                "session_title_changed",
                ServerMessage::SessionTitleChanged {
                    session_id: "s".into(),
                    title: "t".into(),
                },
            ),
            (
                "session_summary_changed",
                ServerMessage::SessionSummaryChanged {
                    session_id: "s".into(),
                    summary: "sm".into(),
                },
            ),
            (
                "session_activity_changed",
                ServerMessage::SessionActivityChanged {
                    session_id: "s".into(),
                    activity: SessionState::Active,
                },
            ),
            (
                "session_stats_changed",
                ServerMessage::SessionStatsChanged {
                    session_id: "s".into(),
                    added: 0,
                    removed: 0,
                    files_changed: 0,
                },
            ),
            (
                "group_list",
                ServerMessage::GroupList { groups: vec![] },
            ),
            (
                "groups_changed",
                ServerMessage::GroupsChanged { groups: vec![] },
            ),
            (
                "dir_listing",
                ServerMessage::DirListing {
                    path: "/".into(),
                    entries: vec![],
                },
            ),
            (
                "file_preview",
                ServerMessage::FilePreview {
                    path: "/x".into(),
                    content: "c".into(),
                    truncated: false,
                    language: None,
                },
            ),
            (
                "git_status_result",
                ServerMessage::GitStatusResult { entries: vec![] },
            ),
            (
                "fs_changed",
                ServerMessage::FsChanged { paths: vec![] },
            ),
            (
                "session_usage_changed",
                ServerMessage::SessionUsageChanged {
                    session_id: "s".into(),
                    usage: SessionUsageWire::from(&mew_session::SessionUsage::default()),
                },
            ),
            (
                "session_alert",
                ServerMessage::SessionAlert {
                    session_id: "s".into(),
                    title: "t".into(),
                    kind: AlertKind::TurnComplete,
                    detail: None,
                },
            ),
            (
                "flagged_files_changed",
                ServerMessage::FlaggedFilesChanged {
                    session_id: "s".into(),
                    files: vec![],
                },
            ),
            (
                "session_meta_changed",
                ServerMessage::SessionMetaChanged {
                    session_id: "s".into(),
                    archived: None,
                    pinned: None,
                    group_id: None,
                },
            ),
            (
                "session_attention_changed",
                ServerMessage::SessionAttentionChanged {
                    session_id: "s".into(),
                    pending_permissions: 0,
                    pending_questions: 0,
                },
            ),
            (
                "pong",
                ServerMessage::Pong {
                    version: "0.0.0".into(),
                },
            ),
        ];

        let mut seen: Vec<&'static str> = Vec::with_capacity(samples.len());
        for (expected, msg) in &samples {
            let json = encode_json(msg).unwrap();
            assert!(
                json.contains(&format!(r#""type":"{}""#, expected)),
                "tag mismatch for {}: expected {:?}, got {}",
                expected,
                expected,
                json
            );
            assert!(
                !seen.contains(expected),
                "duplicate expected tag {:?} in server samples",
                expected
            );
            seen.push(*expected);
        }
        assert_eq!(
            seen.len(),
            samples.len(),
            "server tag list should have no duplicates"
        );
    }

    // -- Converters ----------------------------------------------------------

    #[test]
    fn permission_decision_from_hooks_prompt_collapses_to_deny() {
        // Prompt is a daemon-internal decision; frontends never send it.
        // The conversion collapses it to Deny so we don't expose a value
        // the wire can't represent.
        let pd: PermissionDecision = mew_hooks::PermissionDecision::Prompt.into();
        assert_eq!(pd, PermissionDecision::Deny);
    }

    #[test]
    fn permission_decision_from_hooks_allow_once_roundtrip() {
        let pd: PermissionDecision = mew_hooks::PermissionDecision::AllowOnce.into();
        assert_eq!(pd, PermissionDecision::AllowOnce);
        let back: mew_hooks::PermissionDecision = pd.into();
        assert_eq!(back, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[test]
    fn permission_decision_from_hooks_allow_session_roundtrip() {
        let pd: PermissionDecision = mew_hooks::PermissionDecision::AllowSession.into();
        assert_eq!(pd, PermissionDecision::AllowSession);
        let back: mew_hooks::PermissionDecision = pd.into();
        assert_eq!(back, mew_hooks::PermissionDecision::AllowSession);
    }

    #[test]
    fn permission_decision_from_hooks_deny_roundtrip() {
        let pd: PermissionDecision = mew_hooks::PermissionDecision::Deny.into();
        assert_eq!(pd, PermissionDecision::Deny);
        let back: mew_hooks::PermissionDecision = pd.into();
        assert_eq!(back, mew_hooks::PermissionDecision::Deny);
    }

    #[test]
    fn provider_event_to_wire_part_delta_carries_string_field() {
        use mew_provider::ProviderEvent;
        let pid = mew_message::PartId::new();
        let ev = ProviderEvent::PartDelta {
            part_id: pid,
            field: "text",
            delta: "x".into(),
        };
        let wire = provider_event_to_wire(&ev);
        match wire {
            mew_message::ProviderEventWire::PartDelta {
                part_id,
                field,
                delta,
            } => {
                assert_eq!(part_id, pid);
                assert_eq!(field, "text"); // &'static str -> String
                assert_eq!(delta, "x");
            }
            _ => panic!("wrong wire variant"),
        }
    }

    #[test]
    fn provider_event_to_wire_message_end_preserves_finish_and_cost() {
        use mew_provider::ProviderEvent;
        let ev = ProviderEvent::MessageEnd {
            finish: mew_message::Finish::Stop,
            usage: mew_message::Tokens::default(),
            cost: 0.0042,
        };
        let wire = provider_event_to_wire(&ev);
        match wire {
            mew_message::ProviderEventWire::MessageEnd { finish, cost, .. } => {
                assert_eq!(finish, mew_message::Finish::Stop);
                assert!((cost - 0.0042).abs() < f64::EPSILON);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn subagent_outcome_to_wire_preserves_failed_reason() {
        let outcome = mew_subagents::SubagentOutcome::Failed {
            reason: "boom".into(),
        };
        let wire = subagent_outcome_to_wire(&outcome);
        match wire {
            SubagentOutcome::Failed { reason } => assert_eq!(reason, "boom"),
            _ => panic!(),
        }
    }

    #[test]
    fn subagent_outcome_to_wire_preserves_completed() {
        let outcome = mew_subagents::SubagentOutcome::Completed;
        let wire = subagent_outcome_to_wire(&outcome);
        assert!(matches!(wire, SubagentOutcome::Completed));
    }

    // -- Shared-sessions wire variants ---------------------------------------

    #[test]
    fn client_message_attach_session_roundtrip() {
        let m = ClientMessage::AttachSession {
            session_id: "sess_01H8XKJ9ABCDEFGH0123456789".into(),
            client_kind: ClientKind::Unknown,
        };
        match round_trip(&m) {
            ClientMessage::AttachSession { session_id, .. } => {
                assert_eq!(session_id, "sess_01H8XKJ9ABCDEFGH0123456789");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn client_message_list_sessions_roundtrip() {
        let m = ClientMessage::ListSessions;
        assert!(matches!(round_trip(&m), ClientMessage::ListSessions));
    }

    #[test]
    fn session_state_serializes_lowercase() {
        // serde(rename_all = "lowercase") should produce "active"/"idle".
        let active = serde_json::to_string(&SessionState::Active).unwrap();
        let idle = serde_json::to_string(&SessionState::Idle).unwrap();
        assert_eq!(active, r#""active""#);
        assert_eq!(idle, r#""idle""#);
    }

    #[test]
    fn session_info_full_roundtrip() {
        let info = SessionInfo {
            session_id: "sess_abc".into(),
            state: SessionState::Active,
            model: Some("deepseek-v4-flash".into()),
            provider: Some("deepseek".into()),
            created_at: 1_700_000_000,
            last_message_at: Some(1_700_000_123),
            summary: None,
            client_count: 2,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            pinned: false,
            group_id: None,
            change_stats: None,
            usage: None,
            pending_permissions: 0,
            pending_questions: 0,
        };
        let decoded = round_trip(&info);
        assert_eq!(decoded.session_id, "sess_abc");
        assert_eq!(decoded.state, SessionState::Active);
        assert_eq!(decoded.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(decoded.provider.as_deref(), Some("deepseek"));
        assert_eq!(decoded.created_at, 1_700_000_000);
        assert_eq!(decoded.last_message_at, Some(1_700_000_123));
        assert_eq!(decoded.client_count, 2);
    }

    #[test]
    fn session_info_optional_fields_skip_when_none() {
        // model/provider/last_message_at use skip_serializing_if = "Option::is_none".
        let info = SessionInfo {
            session_id: "sess_xyz".into(),
            state: SessionState::Idle,
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
            pending_permissions: 0,
            pending_questions: 0,
        };
        let json = encode_json(&info).unwrap();
        assert!(
            !json.contains(r#""model""#),
            "none model should be skipped: {json}"
        );
        assert!(
            !json.contains(r#""provider""#),
            "none provider should be skipped: {json}"
        );
        assert!(
            !json.contains(r#""last_message_at""#),
            "none last_message_at should be skipped: {json}"
        );
        // Round-trip still decodes to None.
        let decoded = round_trip(&info);
        assert!(decoded.model.is_none());
        assert!(decoded.provider.is_none());
        assert!(decoded.last_message_at.is_none());
    }

    #[test]
    fn server_message_request_resolved_roundtrip() {
        let m = ServerMessage::RequestResolved { request_id: 99 };
        match round_trip(&m) {
            ServerMessage::RequestResolved { request_id } => assert_eq!(request_id, 99),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_message_session_cleared_roundtrip() {
        let m = ServerMessage::SessionCleared;
        assert!(matches!(round_trip(&m), ServerMessage::SessionCleared));
    }

    #[test]
    fn server_message_session_list_roundtrip() {
        let sessions = vec![
            SessionInfo {
                session_id: "sess_a".into(),
                state: SessionState::Active,
                model: Some("m1".into()),
                provider: Some("p1".into()),
                created_at: 100,
                last_message_at: None,
                summary: None,
                client_count: 1,
                cwd: None,
                last_turn_failed: false,
                archived: false,
                pinned: false,
                group_id: None,
                change_stats: None,
                usage: None,
                pending_permissions: 0,
                pending_questions: 0,
            },
            SessionInfo {
                session_id: "sess_b".into(),
                state: SessionState::Idle,
                model: None,
                provider: None,
                created_at: 200,
                last_message_at: Some(250),
                summary: None,
                client_count: 0,
                cwd: None,
                last_turn_failed: false,
                archived: false,
                pinned: false,
                group_id: None,
                change_stats: None,
                usage: None,
                pending_permissions: 0,
                pending_questions: 0,
            },
        ];
        let m = ServerMessage::SessionList { sessions };
        match round_trip(&m) {
            ServerMessage::SessionList { sessions } => {
                assert_eq!(sessions.len(), 2);
                assert_eq!(sessions[0].session_id, "sess_a");
                assert_eq!(sessions[0].state, SessionState::Active);
                assert_eq!(sessions[0].client_count, 1);
                assert_eq!(sessions[1].session_id, "sess_b");
                assert_eq!(sessions[1].state, SessionState::Idle);
                assert_eq!(sessions[1].last_message_at, Some(250));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_message_session_history_roundtrip() {
        // A SessionHistory carrying one user text message. This proves the
        // mew_message::Message shape survives the wire intact.
        let sid = mew_message::SessionId::new();
        let mid = mew_message::MessageId::new();
        let msg = mew_message::Message {
            id: mid,
            session_id: sid,
            role: mew_message::Role::User,
            parts: vec![mew_message::Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: mew_message::PartId::new(),
                    message_id: mid,
                    session_id: sid,
                },
                text: "hello history".into(),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: 1_700_000_000,
                completed: None,
            },
            assistant: None,
        };
        let m = ServerMessage::SessionHistory {
            messages: vec![msg],
        };
        match round_trip(&m) {
            ServerMessage::SessionHistory { messages } => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].role, mew_message::Role::User);
                match &messages[0].parts[0] {
                    mew_message::Part::Text(tp) => assert_eq!(tp.text, "hello history"),
                    other => panic!("expected Text part, got {other:?}"),
                }
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_message_session_history_empty_roundtrip() {
        // An idle session with no history sends an empty message list.
        let m = ServerMessage::SessionHistory { messages: vec![] };
        match round_trip(&m) {
            ServerMessage::SessionHistory { messages } => {
                assert!(messages.is_empty());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_roundtrip_set_thinking_variant() {
        let m = ClientMessage::SetThinkingVariant {
            variant: "high".into(),
        };
        match round_trip(&m) {
            ClientMessage::SetThinkingVariant { variant } => {
                assert_eq!(variant, "high");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_roundtrip_thinking_variant_changed() {
        let m = ServerMessage::ThinkingVariantChanged {
            variant: Some("max".into()),
        };
        match round_trip(&m) {
            ServerMessage::ThinkingVariantChanged { variant } => {
                assert_eq!(variant.as_deref(), Some("max"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_roundtrip_thinking_variant_changed_none() {
        let m = ServerMessage::ThinkingVariantChanged { variant: None };
        let j = encode_json(&m).unwrap();
        // variant=None should be skipped in serialization.
        assert!(!j.contains(r#""variant":null"#));
        match round_trip(&m) {
            ServerMessage::ThinkingVariantChanged { variant } => {
                assert!(variant.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_roundtrip_session_title_changed() {
        let m = ServerMessage::SessionTitleChanged {
            session_id: "sess_123".into(),
            title: "hello world".into(),
        };
        match round_trip(&m) {
            ServerMessage::SessionTitleChanged { session_id, title } => {
                assert_eq!(session_id, "sess_123");
                assert_eq!(title, "hello world");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_model_info_with_thinking_variants() {
        let m = ModelInfo {
            id: "deepseek/deepseek-v4-flash".into(),
            provider: "deepseek".into(),
            model: "deepseek-v4-flash".into(),
            description: Some("Fast model".into()),
            thinking_variants: vec![
                ThinkingVariantInfo {
                    name: "high".into(),
                },
                ThinkingVariantInfo { name: "max".into() },
            ],
            context_window: Some(128_000),
        };
        let j = encode_json(&m).unwrap();
        assert!(j.contains(r#""thinking_variants""#));
        let parsed: ModelInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.thinking_variants.len(), 2);
        assert_eq!(parsed.thinking_variants[0].name, "high");
    }

    #[test]
    fn test_model_info_without_thinking_variants_skips_field() {
        let m = ModelInfo {
            id: "test/model".into(),
            provider: "test".into(),
            model: "model".into(),
            description: None,
            thinking_variants: vec![],
            context_window: None,
        };
        let j = encode_json(&m).unwrap();
        // Empty vec should be skipped in serialization.
        assert!(!j.contains(r#""thinking_variants""#));
    }

    #[test]
    fn test_roundtrip_set_permission_mode() {
        let m = ClientMessage::SetPermissionMode {
            mode: "dangerous".into(),
        };
        match round_trip(&m) {
            ClientMessage::SetPermissionMode { mode } => {
                assert_eq!(mode, "dangerous");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_roundtrip_permission_mode_changed() {
        let m = ServerMessage::PermissionModeChanged {
            mode: "auto_plus".into(),
        };
        match round_trip(&m) {
            ServerMessage::PermissionModeChanged { mode } => {
                assert_eq!(mode, "auto_plus");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_session_ready_includes_permission_mode() {
        let m = ServerMessage::SessionReady {
            session_id: "s1".into(),
            model: Some("test/model".into()),
            provider: Some("test".into()),
            permission_mode: Some("permissive".into()),
        };
        match round_trip(&m) {
            ServerMessage::SessionReady {
                session_id,
                model,
                provider,
                permission_mode,
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(model.as_deref(), Some("test/model"));
                assert_eq!(provider.as_deref(), Some("test"));
                assert_eq!(permission_mode.as_deref(), Some("permissive"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_session_ready_permission_mode_skipped_when_absent() {
        let m = ServerMessage::SessionReady {
            session_id: "s1".into(),
            model: None,
            provider: None,
            permission_mode: None,
        };
        let j = encode_json(&m).unwrap();
        // permission_mode should be skipped from serialization when None.
        assert!(!j.contains(r#""permission_mode""#));
    }

    #[test]
    fn test_roundtrip_ping_pong() {
        let ping = ClientMessage::Ping;
        let j = encode_json(&ping).unwrap();
        assert!(j.contains(r#""type":"ping""#));
        let decoded: ClientMessage = decode_json(&j).unwrap();
        assert!(matches!(decoded, ClientMessage::Ping));

        let pong = ServerMessage::Pong {
            version: "0.2.0".into(),
        };
        let j = encode_json(&pong).unwrap();
        assert!(j.contains(r#""type":"pong""#));
        assert!(j.contains(r#""version":"0.2.0""#));
        let decoded: ServerMessage = decode_json(&j).unwrap();
        match decoded {
            ServerMessage::Pong { version } => assert_eq!(version, "0.2.0"),
            _ => panic!("expected Pong"),
        }
    }
}
