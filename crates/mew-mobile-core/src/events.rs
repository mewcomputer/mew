//! Event types and listener trait for the mobile core.
//!
//! `CoreEvent` is app-vocabulary (sessions, turns, requests), not
//! wire-vocabulary. Swift never sees `ClientMessage`/`ServerMessage` JSON.

/// A permission decision from the user (mirrors `mew_protocol::PermissionDecision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Decision {
    AllowOnce,
    AllowSession,
    Deny,
}

/// Events emitted by the mobile core to the Swift layer.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum CoreEvent {
    /// Daemon connection status changed.
    DaemonStatusChanged {
        daemon: String,
        status: DaemonStatus,
    },

    /// Full or updated session list for a daemon.
    SessionList {
        daemon: String,
        sessions: Vec<SessionSummary>,
    },

    /// Project list for a daemon (response to list_projects).
    ProjectList {
        daemon: String,
        projects: Vec<ProjectInfo>,
    },

    /// Directory listing (response to list_dir).
    DirListing {
        daemon: String,
        session_id: String,
        path: String,
        entries: Vec<DirEntry>,
    },

    /// A session was reloaded after reconnect. Swift should pull `snapshot()`.
    SessionReloaded { daemon: String, session_id: String },

    /// Streaming text delta for a part. Coalesced in Rust before FFI.
    TextDelta {
        daemon: String,
        session_id: String,
        part_id: String,
        delta: String,
    },

    /// A part's state was updated (tool state, reasoning, errors).
    PartUpdated {
        daemon: String,
        session_id: String,
        part_id: String,
        part_kind: String,
        state: Option<String>,
    },

    /// A turn ended (MessageEnd received).
    TurnEnded {
        daemon: String,
        session_id: String,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
        failed: bool,
    },

    /// A permission request from the agent.
    PermissionRequested {
        daemon: String,
        session_id: String,
        request_id: String,
        tool_name: String,
        input: String,
    },

    /// An ask-user request from the agent.
    AskUserRequested {
        daemon: String,
        session_id: String,
        request_id: String,
        call_id: String,
        questions: Vec<String>,
    },

    /// A request was resolved (by this device or another). Dismiss the sheet.
    RequestResolved { daemon: String, request_id: String },

    /// Cross-session alert from the daemon.
    Alert {
        daemon: String,
        session_id: String,
        kind: String,
        title: String,
        detail: Option<String>,
    },

    /// Attention count changed for a session.
    AttentionChanged {
        daemon: String,
        session_id: String,
        pending_permissions: u32,
        pending_questions: u32,
    },

    /// Todo list updated for a session.
    TodosUpdated {
        daemon: String,
        session_id: String,
        todos: Vec<TodoItem>,
    },

    /// Permission mode changed (via cross-device or local action).
    PermissionModeChanged { daemon: String, mode: String },

    /// Model was switched (via cross-device or local action).
    ModelSwitched {
        daemon: String,
        provider: String,
        model: String,
    },

    /// Thinking variant changed.
    ThinkingVariantChanged {
        daemon: String,
        variant: Option<String>,
    },

    /// Available models from the daemon.
    ModelList {
        daemon: String,
        models: Vec<ModelSummary>,
    },

    /// Slash command result text.
    SlashResult {
        daemon: String,
        session_id: String,
        text: String,
    },

    /// Daemon version from Pong.
    DaemonVersion { daemon: String, version: String },
}

/// Connection status for a daemon.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum DaemonStatus {
    Disconnected,
    Connecting,
    Connected,
    /// Backing off before reconnect attempt N.
    ///
    /// `error` carries the human-readable reason for the most recent
    /// failure so the UI can surface it instead of a generic "retrying"
    /// string. Empty when no specific reason is available.
    Backoff {
        attempt: u32,
        error: String,
    },
    /// Pairing was lost (NodeId changed or unauthorized).
    PairedLost,
}

/// Summary of a session for the session rail.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub state: String,
    pub archived: bool,
    pub pinned: bool,
    pub pending_permissions: u32,
    pub pending_questions: u32,
    pub usage_cost: f64,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub created_at: i64,
    pub last_message_at: Option<i64>,
    pub last_turn_failed: bool,
    pub group_id: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub turns: u32,
}

/// A known project directory, returned by `list_projects`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ProjectInfo {
    pub path: String,
    pub display_name: String,
    pub session_count: u32,
    pub last_used_at: Option<i64>,
}

/// One entry in a directory listing (response to list_dir).
#[derive(Debug, Clone, uniffi::Record)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

/// Summary of an available model.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ModelSummary {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub description: Option<String>,
    pub context_window: Option<i64>,
    pub thinking_variants: Vec<String>,
}

/// A todo item from the agent's todo list.
/// Uses u64 for id/depends_on to match the protocol's `usize` without narrowing.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TodoItem {
    pub id: u64,
    pub content: String,
    pub status: String,
    pub depends_on: Vec<u64>,
}

/// Callback trait implemented by the Swift layer.
#[uniffi::export(with_foreign)]
pub trait CoreListener: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}
