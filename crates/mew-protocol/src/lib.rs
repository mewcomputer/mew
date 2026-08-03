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

pub mod command_registry;

pub use command_registry::{is_known, lookup, CommandDef, CommandLocus, BUILTIN_COMMANDS};

// ---------------------------------------------------------------------------
// Client → Daemon messages
// ---------------------------------------------------------------------------

/// A message from the frontend to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Authenticate an iroh connection before any daemon operation. The
    /// daemon derives the granted scope from the transport-side pairing
    /// record; the client never gets to choose its own authority.
    RemoteHello {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        device_name: String,
    },
    /// Create a new session.
    NewSession {
        /// Working directory for the session. Defaults to the daemon's cwd.
        cwd: Option<String>,
        /// What kind of client is connecting (TUI, Web, etc.).
        #[serde(default)]
        client_kind: ClientKind,
    },

    /// Create a new session and assign it to an existing group before it is
    /// attached to the client.
    NewSessionInGroup {
        /// Working directory for the session. Defaults to the daemon's cwd.
        cwd: Option<String>,
        group_id: String,
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
        request_id: String,
        decision: PermissionDecision,
    },

    /// Respond to an `AskUserRequest` from the daemon.
    AskUserResponse {
        request_id: String,
        /// One answer per question, in order.
        answers: Vec<String>,
    },

    /// Respond to a `PlanApprovalRequest` from the daemon. `approved = false`
    /// with optional `feedback` requests changes to the plan.
    PlanApprovalResponse {
        request_id: String,
        approved: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feedback: Option<String>,
    },

    /// Respond to a `GoalProposed` from the daemon. `accepted = true` activates
    /// the goal; `accepted = false` rejects it.
    GoalResponse {
        request_id: String,
        accepted: bool,
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
    /// Pass an empty string or "none" to disable thinking. Numeric token
    /// budgets ride this message as the string convention `"budget:<n>"`
    /// (e.g. `"budget:8192"`); the daemon clamps/snaps the value to the
    /// model's declared range.
    SetThinkingVariant {
        /// Variant name (e.g. "high", "max", "thinking"), `"budget:<n>"`,
        /// or empty/none to disable.
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
    /// Browse user-accessible directories before a session exists.
    ListFilesystemDir {
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

    // -- Personas --
    /// List available personas for the active session.
    ListPersonas,

    /// Switch the active session to a different persona.
    SwitchPersona {
        /// Persona name (must match one returned by `ListPersonas`).
        name: String,
    },

    // -- Projects --
    /// List known projects (recent session cwds + configured workspace.roots).
    /// Does NOT require a session — used before creating one to populate a
    /// project picker. The daemon responds with `ServerMessage::ProjectList`.
    ListProjects,

    /// Regenerate the session title from the conversation history.
    /// Useful when auto-title was disabled or the first message was a poor title source.
    /// The daemon extracts the first user message, calls the LLM for a concise title,
    /// persists it via `set_custom_title`, and broadcasts `SessionTitleChanged`.
    RegenerateTitle {
        session_id: String,
    },

    /// Ping the daemon for liveness check and version negotiation.
    /// The daemon responds with `ServerMessage::Pong { version }`.
    Ping,

    // -- Browser --
    /// Navigate the browser session to an HTTP(S) URL.
    BrowserOpen {
        url: String,
        /// Frontend-owned tab identity, echoed by browser responses.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    /// Return the active page's accessibility snapshot.
    BrowserSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    /// Capture the active page as a base64-encoded PNG.
    BrowserScreenshot {
        annotate: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    /// Click an accessibility ref or CSS selector.
    BrowserClick {
        selector: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    /// Fill an input identified by an accessibility ref or CSS selector.
    BrowserFill {
        selector: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    /// Press a keyboard key in the active page.
    BrowserPress {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    /// Close the browser session.
    BrowserClose {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },

    // -- Native terminal ----------------------------------------------------
    /// Open one interactive shell for the current daemon connection.
    /// The shell runs in the attached session's workspace.
    TerminalOpen {
        rows: u16,
        cols: u16,
    },
    /// Write raw bytes to the active terminal's stdin.
    TerminalInput {
        terminal_id: String,
        bytes: Vec<u8>,
    },
    /// Resize the active terminal grid without restarting the shell.
    TerminalResize {
        terminal_id: String,
        rows: u16,
        cols: u16,
    },
    /// Close the active terminal shell.
    TerminalClose {
        terminal_id: String,
    },
}

/// What kind of client is connected to a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// Terminal UI (`mew chat --connect`).
    Tui,
    /// Browser-based web UI.
    Web,
    /// Native desktop app with access to the in-app browser.
    Desktop,
    /// Headless CLI script.
    Cli,
    /// Mobile app (iOS / Android).
    Mobile,
    /// A client connected through the opt-in remote daemon transport.
    Remote,
    /// Unknown / unspecified.
    #[default]
    Unknown,
}

/// Authority granted to a remote client by the daemon owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteScope {
    /// Read session state and workspace previews, but do not drive a turn.
    Observe,
    /// Send prompts and answer requests, but do not change workspace files.
    Collaborate,
    /// Full daemon authority, including mutating tools subject to permissions.
    Control,
}

/// Info about a single available model, returned by `ListModels`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Numeric thinking-budget range, when the model accepts a
    /// `thinking_budget` token cap (e.g. Qwen3.8-max). `None` means the
    /// model has no configurable budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<ThinkingBudgetInfo>,
    /// Maximum context window in tokens, if known from the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
}

/// A named thinking/reasoning variant (e.g. "high", "max", "thinking").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingVariantInfo {
    /// Display name (e.g. "high", "max", "thinking").
    pub name: String,
}

/// Numeric thinking-budget range for models that accept a `thinking_budget`
/// token cap. Budget selection rides `SetThinkingVariant` as the string
/// convention `"budget:<n>"` (clamped/snapped to `min..=max` by `step` by
/// the daemon); no dedicated message type exists for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingBudgetInfo {
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub default: i64,
    /// Canonical budget (in tokens) for each named effort variant, so UIs
    /// can seed a slider position from the active effort level.
    #[serde(default)]
    pub by_effort: Vec<(String, i64)>,
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
    /// First user message text (truncated), used as a display title fallback
    /// when no AI-generated title or summary is available. Populated by the
    /// daemon from session history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_message: Option<String>,
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
    /// Confirms the scope granted to an authenticated remote connection.
    RemoteReady {
        scope: RemoteScope,
    },
    /// Sent after `NewSession` succeeds. The session is ready for prompts.
    /// `model` and `provider` are the daemon's current model, so the
    /// frontend can display it immediately without a separate ListModels round-trip.
    SessionReady {
        session_id: String,
        /// The session's workspace, when one is available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
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
        request_id: String,
        tool_name: String,
        /// The tool input as JSON.
        input: serde_json::Value,
    },

    /// Request user approval for a path outside the workspace.
    WorkspacePermissionRequest {
        request_id: String,
        path: String,
    },

    /// Ask the user one to four free-text questions.
    AskUserRequest {
        request_id: String,
        call_id: String,
        questions: Vec<Question>,
    },

    /// Present a completed plan for user approval (from `handoff_plan`). The
    /// frontend responds with `PlanApprovalResponse`.
    PlanApprovalRequest {
        request_id: String,
        call_id: String,
        plan_path: String,
        plan_markdown: String,
        persona: String,
    },

    /// Present a proposed goal for user approval (from `propose_goal`). The
    /// frontend responds with `GoalResponse`.
    GoalProposed {
        request_id: String,
        call_id: String,
        objective: String,
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
        #[serde(default)]
        manifests: Vec<mew_message::TurnManifest>,
    },

    /// A permission request from a child subagent.
    SubagentPermissionRequest {
        request_id: String,
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
        request_id: String,
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
        session_id: String,
        path: String,
        entries: Vec<DirEntry>,
    },
    FilesystemDirListing {
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
        #[serde(default)]
        archived: bool,
        #[serde(default)]
        pinned: bool,
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

    // -- Personas --
    /// Response to `ListPersonas`: the full set of personas available
    /// to the active session.
    PersonaList {
        personas: Vec<PersonaInfo>,
    },

    /// Confirmation that a persona switch succeeded. Distinct from
    /// `PersonaSwitchRequested` (which is emitted when a tool queues
    /// a persona switch during a turn).
    PersonaSwitched {
        name: String,
    },

    /// Response to `ClientMessage::Ping`. Carries the daemon's version
    /// so clients can detect version skew.
    Pong {
        /// Daemon version string (e.g. "0.2.0").
        version: String,
    },

    /// Response to `ListProjects`. Contains deduped project directories
    /// derived from session metadata.
    ProjectList {
        projects: Vec<ProjectInfo>,
    },

    // -- Browser --
    BrowserSnapshot {
        snapshot: String,
        url: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    BrowserScreenshot {
        data: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    BrowserState {
        open: bool,
        url: Option<String>,
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    /// A browser operation failed. The tab identity prevents a late error
    /// from replacing the state of whichever tab is active now.
    BrowserError {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },

    // -- Native terminal ----------------------------------------------------
    /// Confirms that a terminal shell was created for this connection.
    TerminalOpened {
        terminal_id: String,
    },
    /// Raw bytes emitted by the terminal shell. The client owns ANSI parsing
    /// and presentation so the daemon remains transport-only here.
    TerminalOutput {
        terminal_id: String,
        bytes: Vec<u8>,
    },
    /// The terminal shell exited and will not accept more input.
    TerminalExited {
        terminal_id: String,
        status: String,
    },
    /// A terminal operation failed without taking down the daemon connection.
    TerminalError {
        terminal_id: Option<String>,
        message: String,
    },
}

/// Wire-format info about a flagged file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlaggedFileWire {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A known project directory, returned by `ListProjects`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Absolute path to the project directory.
    pub path: String,
    /// Human-friendly display name (last path component).
    pub display_name: String,
    /// Number of sessions in this project.
    pub session_count: u32,
    /// Timestamp of the last activity in this project (epoch seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
}

/// Wire-format info about a persona, returned by `ListPersonas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaInfo {
    /// Persona name (unique identifier).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Optional color token for UI display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Whether this persona is currently active.
    pub active: bool,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Question {
    pub prompt: String,
    pub options: Vec<QuestionOption>,
}

/// An option within a question.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
// Extension protocol surface (additive — no changes to existing types)
// ---------------------------------------------------------------------------

/// Protocol version for extension handshake. The daemon rejects mismatches
/// with a clear message. The SDK majors in lockstep with this.
pub const EXTENSION_PROTOCOL_VERSION: u32 = 1;

/// A message from an extension process to the daemon.
///
/// The first message on any extension connection is always `ExtensionHello`.
/// After the daemon responds with `ExtensionReady`, the extension may send
/// any of the other variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtensionMessage {
    /// Handshake: sent first by the extension.
    ExtensionHello {
        name: String,
        version: String,
        protocol_version: u32,
        /// Capability IDs requested (strings from `Capability::id()`).
        requested_capabilities: Vec<String>,
        /// Hook IDs this extension subscribes to (e.g. ["on-system-prompt",
        /// "on-tool-execute-before"]).
        hook_subscriptions: Vec<String>,
        /// Event types this extension subscribes to (e.g. ["MessageEnd",
        /// "ToolEnd"]). Empty means no events.
        event_subscriptions: Vec<String>,
    },

    /// Response to a hook request from the daemon.
    HookResponse {
        request_id: String,
        /// The raw JSON result string, or "block:reason" / "suppress".
        outcome: String,
    },

    /// Call a session method (requires `sessions:*` capability).
    ExtensionListSessions,

    ExtensionAttachSession {
        session_id: String,
    },

    ExtensionNewSession {
        cwd: Option<String>,
    },

    ExtensionPrompt {
        session_id: String,
        text: String,
    },

    ExtensionCancel {
        session_id: String,
    },

    /// Subscribe to events (requires `events` capability).
    ExtensionSubscribeEvents {
        /// Event types to receive (e.g. ["MessageEnd", "ToolEnd"]).
        types: Vec<String>,
    },

    /// Resolve a permission prompt (requires `permissions:resolve`).
    ExtensionResolvePermission {
        request_id: String,
        decision: String,
    },

    /// Call a host function (notify, storage, set_ui — under `ui`/`storage`).
    ExtensionHostCall(ExtensionHostCall),

    /// Read session history (requires `sessions:read`).
    ExtensionReadSessionHistory {
        session_id: String,
    },
}

/// A message from the daemon to an extension process.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonMessage {
    /// Handshake response: grants (a subset of) requested capabilities.
    ExtensionReady {
        /// Capability IDs that were granted.
        granted: Vec<String>,
        /// Daemon's protocol version.
        protocol_version: u32,
    },

    /// Handshake rejected (version mismatch or invalid capabilities).
    ExtensionRejected { reason: String },

    /// A hook request: the extension must respond with `HookResponse`
    /// using the same `request_id`.
    HookRequest {
        request_id: String,
        /// Hook method name (e.g. "on-system-prompt", "on-permission-ask").
        hook: String,
        /// Parameters as a JSON value.
        params: serde_json::Value,
    },

    /// An event the extension subscribed to.
    ExtensionEvent(Box<ExtensionEvent>),

    /// Response to `ExtensionListSessions`.
    SessionList { sessions: Vec<SessionInfo> },

    /// Response to `ExtensionAttachSession` / `ExtensionNewSession`.
    SessionAttached { session_id: String },

    /// Response to `ExtensionReadSessionHistory`.
    SessionHistory {
        session_id: String,
        messages: Vec<mew_message::Message>,
    },

    /// Response to `ExtensionHostCall`.
    HostCallResult { result: String },

    /// A permission request surfaced to the extension (for
    /// `permissions:resolve` — the extension should respond with
    /// `ExtensionResolvePermission`).
    PermissionRequest {
        request_id: String,
        session_id: String,
        tool: String,
        input: serde_json::Value,
        current_decision: String,
    },

    /// Error response for any extension request.
    Error { message: String },
}

/// An event delivered to an extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtensionEvent {
    /// Session lifecycle: created, turn ended, tool ran — no message bodies.
    SessionCreated {
        session_id: String,
        cwd: String,
    },
    SessionDestroyed {
        session_id: String,
    },
    TurnEnded {
        session_id: String,
    },
    ToolStarted {
        session_id: String,
        tool: String,
    },
    ToolEnded {
        session_id: String,
        tool: String,
        success: bool,
    },
    /// Full message content (requires `events` with `content: full`).
    MessageEnd {
        session_id: String,
        message: Box<mew_message::Message>,
    },
    /// The extension's event queue overflowed — some events were dropped.
    Lagged {
        count: u64,
    },
}

/// A host function call from the extension to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ExtensionHostCall {
    /// Show a toast notification (requires `ui`).
    Notify { message: String },
    /// Set an input-area widget (requires `ui`).
    SetUi { key: String, value: String },
    /// Read from namespaced storage (always granted).
    StorageRead { key: String },
    /// Write to namespaced storage (always granted).
    StorageWrite { key: String, value: String },
    /// Delete from namespaced storage (always granted).
    StorageDelete { key: String },
    /// Read own config subtree (always granted).
    ConfigRead { key: String },
}

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
            manifest: None,
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

    #[test]
    fn remote_auth_messages_roundtrip() {
        let hello = ClientMessage::RemoteHello {
            token: Some("one-time-token".into()),
            device_name: "laptop".into(),
        };
        assert!(matches!(
            round_trip(&hello),
            ClientMessage::RemoteHello { token: Some(token), device_name }
                if token == "one-time-token" && device_name == "laptop"
        ));

        let ready = ServerMessage::RemoteReady {
            scope: RemoteScope::Collaborate,
        };
        assert!(matches!(
            round_trip(&ready),
            ServerMessage::RemoteReady {
                scope: RemoteScope::Collaborate
            }
        ));
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
            request_id: "uuid-42".into(),
            decision: PermissionDecision::AllowOnce,
        };
        let json = encode_json(&msg).unwrap();
        let decoded: ClientMessage = decode_json(&json).unwrap();
        match decoded {
            ClientMessage::PermissionResponse {
                request_id,
                decision,
            } => {
                assert_eq!(request_id, "uuid-42");
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

    #[test]
    fn test_list_projects_roundtrip() {
        let msg = ClientMessage::ListProjects;
        let decoded = round_trip(&msg);
        assert!(matches!(decoded, ClientMessage::ListProjects));

        let msg = ServerMessage::ProjectList {
            projects: vec![ProjectInfo {
                path: "/home/user/myproject".to_string(),
                display_name: "myproject".to_string(),
                session_count: 3,
                last_used_at: Some(1700000000),
            }],
        };
        let decoded = round_trip(&msg);
        match decoded {
            ServerMessage::ProjectList { projects } => {
                assert_eq!(projects.len(), 1);
                assert_eq!(projects[0].path, "/home/user/myproject");
                assert_eq!(projects[0].display_name, "myproject");
                assert_eq!(projects[0].session_count, 3);
                assert_eq!(projects[0].last_used_at, Some(1700000000));
            }
            _ => panic!("expected ProjectList"),
        }
    }

    #[test]
    fn test_filesystem_directory_browse_roundtrip() {
        let request = ClientMessage::ListFilesystemDir {
            path: Some("/Users/tester/projects".into()),
        };
        match round_trip(&request) {
            ClientMessage::ListFilesystemDir { path } => {
                assert_eq!(path.as_deref(), Some("/Users/tester/projects"));
            }
            _ => panic!("wrong client variant"),
        }

        let response = ServerMessage::FilesystemDirListing {
            path: "/Users/tester/projects".into(),
            entries: vec![DirEntry {
                name: "mew".into(),
                is_dir: true,
                size: None,
            }],
        };
        match round_trip(&response) {
            ServerMessage::FilesystemDirListing { path, entries } => {
                assert_eq!(path, "/Users/tester/projects");
                assert_eq!(entries[0].name, "mew");
            }
            _ => panic!("wrong server variant"),
        }
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

    #[test]
    fn test_client_kind_mobile_roundtrip() {
        let msg = ClientMessage::NewSession {
            cwd: None,
            client_kind: ClientKind::Mobile,
        };
        match round_trip(&msg) {
            ClientMessage::NewSession { client_kind, .. } => {
                assert_eq!(client_kind, ClientKind::Mobile);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_client_kind_desktop_roundtrip() {
        let message = ClientMessage::NewSession {
            cwd: None,
            client_kind: ClientKind::Desktop,
        };
        let encoded = encode_json(&message).unwrap();
        let decoded: ClientMessage = decode_json(&encoded).unwrap();
        assert!(matches!(
            decoded,
            ClientMessage::NewSession {
                client_kind: ClientKind::Desktop,
                ..
            }
        ));
    }

    #[test]
    fn new_session_in_group_roundtrip() {
        let message = ClientMessage::NewSessionInGroup {
            cwd: Some("/tmp/project".into()),
            group_id: "grp_1".into(),
            client_kind: ClientKind::Desktop,
        };
        match round_trip(&message) {
            ClientMessage::NewSessionInGroup {
                cwd,
                group_id,
                client_kind,
            } => {
                assert_eq!(cwd.as_deref(), Some("/tmp/project"));
                assert_eq!(group_id, "grp_1");
                assert_eq!(client_kind, ClientKind::Desktop);
            }
            _ => panic!("wrong variant"),
        }
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
    fn terminal_messages_roundtrip_binary_input_and_output() {
        let input = ClientMessage::TerminalInput {
            terminal_id: "term-1".into(),
            bytes: vec![0x1b, b'[', b'2', b'J'],
        };
        match round_trip(&input) {
            ClientMessage::TerminalInput { terminal_id, bytes } => {
                assert_eq!(terminal_id, "term-1");
                assert_eq!(bytes, vec![0x1b, b'[', b'2', b'J']);
            }
            _ => panic!("wrong client terminal variant"),
        }

        let output = ServerMessage::TerminalOutput {
            terminal_id: "term-1".into(),
            bytes: vec![0, 255, b'\n'],
        };
        match round_trip(&output) {
            ServerMessage::TerminalOutput { terminal_id, bytes } => {
                assert_eq!(terminal_id, "term-1");
                assert_eq!(bytes, vec![0, 255, b'\n']);
            }
            _ => panic!("wrong server terminal variant"),
        }
    }

    #[test]
    fn terminal_open_and_exit_tags_are_stable() {
        let open = encode_json(&ClientMessage::TerminalOpen { rows: 24, cols: 80 }).unwrap();
        assert!(open.contains(r#""type":"terminal_open""#));
        let exited = encode_json(&ServerMessage::TerminalExited {
            terminal_id: "term-1".into(),
            status: "exit status: 0".into(),
        })
        .unwrap();
        assert!(exited.contains(r#""type":"terminal_exited""#));
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
                request_id: "uuid-7".into(),
                decision,
            };
            match round_trip(&m) {
                ClientMessage::PermissionResponse {
                    request_id,
                    decision: d,
                } => {
                    assert_eq!(request_id, "uuid-7");
                    assert_eq!(d as u8, expected);
                }
                _ => panic!(),
            }
        }
    }

    #[test]
    fn client_message_ask_user_response_multiple_answers_roundtrip() {
        let m = ClientMessage::AskUserResponse {
            request_id: "uuid-5".into(),
            answers: vec!["alpha".into(), "beta".into(), "gamma".into()],
        };
        match round_trip(&m) {
            ClientMessage::AskUserResponse {
                request_id,
                answers,
            } => {
                assert_eq!(request_id, "uuid-5");
                assert_eq!(answers, vec!["alpha", "beta", "gamma"]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn client_message_plan_approval_response_approved_roundtrip() {
        let m = ClientMessage::PlanApprovalResponse {
            request_id: "uuid-6".into(),
            approved: true,
            feedback: None,
        };
        // Approved responses omit the feedback field on the wire.
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("feedback"));
        match round_trip(&m) {
            ClientMessage::PlanApprovalResponse {
                request_id,
                approved,
                feedback,
            } => {
                assert_eq!(request_id, "uuid-6");
                assert!(approved);
                assert_eq!(feedback, None);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn client_message_plan_approval_response_changes_roundtrip() {
        let m = ClientMessage::PlanApprovalResponse {
            request_id: "uuid-7".into(),
            approved: false,
            feedback: Some("add tests".into()),
        };
        match round_trip(&m) {
            ClientMessage::PlanApprovalResponse {
                request_id,
                approved,
                feedback,
            } => {
                assert_eq!(request_id, "uuid-7");
                assert!(!approved);
                assert_eq!(feedback.as_deref(), Some("add tests"));
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
            cwd: Some("/tmp/workspace".into()),
            model: Some("deepseek-v4-flash".into()),
            provider: Some("deepseek".into()),
            permission_mode: None,
        };
        match round_trip(&m) {
            ServerMessage::SessionReady {
                session_id,
                cwd,
                model,
                provider,
                permission_mode,
            } => {
                assert_eq!(session_id, "01H");
                assert_eq!(cwd.as_deref(), Some("/tmp/workspace"));
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
            ServerMessage::Provider {
                event:
                    mew_message::ProviderEventWire::PartDelta {
                        part_id,
                        field,
                        delta,
                    },
            } => {
                assert_eq!(part_id, pid);
                assert_eq!(field, "text");
                assert_eq!(delta, "abc");
            }
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
                manifest: None,
            },
        };
        match round_trip(&m) {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { finish, .. },
            } => {
                assert_eq!(finish, mew_message::Finish::ToolUse);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn message_end_with_manifest_roundtrip() {
        use mew_message::{Segment, SegmentKind, TurnManifest};

        let manifest = TurnManifest {
            model: "gpt-4o".into(),
            context_window: 128000,
            input_tokens: Some(5000),
            output_tokens: Some(1200),
            cache_read_tokens: Some(2000),
            cache_write_tokens: None,
            reasoning_tokens: None,
            segments: vec![Segment {
                label: "scaffold".into(),
                kind: SegmentKind::Scaffold,
                source_id: None,
                tokens: 300,
                tokens_scaled: 300,
                children: vec![],
            }],
        };

        let m = ServerMessage::Provider {
            event: mew_message::ProviderEventWire::MessageEnd {
                finish: mew_message::Finish::Stop,
                usage: mew_message::Tokens {
                    input: 5000,
                    output: 1200,
                    reasoning: 0,
                    cache_read: 2000,
                    cache_write: 0,
                },
                cost: 0.05,
                manifest: Some(manifest.clone()),
            },
        };

        match round_trip(&m) {
            ServerMessage::Provider {
                event:
                    mew_message::ProviderEventWire::MessageEnd {
                        finish,
                        usage,
                        cost,
                        manifest: roundtripped,
                    },
            } => {
                assert_eq!(finish, mew_message::Finish::Stop);
                assert_eq!(usage.input, 5000);
                assert_eq!(cost, 0.05);
                let rt = roundtripped.expect("manifest should survive round-trip");
                assert_eq!(rt.model, "gpt-4o");
                assert_eq!(rt.context_window, 128000);
                assert_eq!(rt.input_tokens, Some(5000));
                assert_eq!(rt.segments.len(), 1);
                assert_eq!(rt.segments[0].label, "scaffold");
                assert_eq!(rt.segments[0].tokens, 300);
            }
            _ => panic!("expected MessageEnd"),
        }
    }

    #[test]
    fn subagent_end_manifests_roundtrip() {
        use mew_message::{Segment, SegmentKind, TurnManifest};

        let manifest = TurnManifest {
            model: "gpt-4o".into(),
            context_window: 128000,
            input_tokens: Some(800),
            output_tokens: Some(200),
            cache_read_tokens: None,
            cache_write_tokens: Some(100),
            reasoning_tokens: None,
            segments: vec![Segment {
                label: "subagent: researcher".into(),
                kind: SegmentKind::Part,
                source_id: None,
                tokens: 800,
                tokens_scaled: 800,
                children: vec![],
            }],
        };

        let m = ServerMessage::SubagentEnd {
            parent_call_id: "call-1".into(),
            child_session_id: "01H".into(),
            outcome: SubagentOutcome::Completed,
            manifests: vec![manifest.clone()],
        };

        match round_trip(&m) {
            ServerMessage::SubagentEnd {
                parent_call_id,
                child_session_id,
                outcome,
                manifests,
            } => {
                assert_eq!(parent_call_id, "call-1");
                assert_eq!(child_session_id, "01H");
                assert!(matches!(outcome, SubagentOutcome::Completed));
                assert_eq!(manifests.len(), 1);
                assert_eq!(manifests[0].model, "gpt-4o");
                assert_eq!(manifests[0].input_tokens, Some(800));
                assert_eq!(manifests[0].segments[0].label, "subagent: researcher");
            }
            _ => panic!("expected SubagentEnd"),
        }
    }

    #[test]
    fn subagent_end_no_manifests_roundtrip() {
        // Verify backward compatibility: a SubagentEnd with empty manifests
        // round-trips correctly.
        let m = ServerMessage::SubagentEnd {
            parent_call_id: "call-2".into(),
            child_session_id: "02H".into(),
            outcome: SubagentOutcome::Cancelled,
            manifests: vec![],
        };

        match round_trip(&m) {
            ServerMessage::SubagentEnd { manifests, .. } => {
                assert!(manifests.is_empty());
            }
            _ => panic!("expected SubagentEnd"),
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
            ServerMessage::Provider {
                event:
                    mew_message::ProviderEventWire::RetryWait {
                        attempt,
                        max_attempts,
                        delay_secs,
                        reason,
                    },
            } => {
                assert_eq!(attempt, 2);
                assert_eq!(max_attempts, 5);
                assert_eq!(delay_secs, 1);
                assert_eq!(reason, "429");
            }
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
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::Error(e),
            } => {
                assert_eq!(e.message, "rate limit");
                assert_eq!(e.kind, mew_message::ErrorKind::Network);
            }
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
            request_id: "uuid-1".into(),
            tool_name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        match round_trip(&m) {
            ServerMessage::PermissionRequest {
                request_id,
                tool_name,
                input,
            } => {
                assert_eq!(request_id, "uuid-1");
                assert_eq!(tool_name, "bash");
                assert_eq!(input["command"], "ls");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_workspace_permission_request_roundtrip() {
        let m = ServerMessage::WorkspacePermissionRequest {
            request_id: "uuid-2".into(),
            path: "/etc/passwd".into(),
        };
        match round_trip(&m) {
            ServerMessage::WorkspacePermissionRequest { request_id, path } => {
                assert_eq!(request_id, "uuid-2");
                assert_eq!(path, "/etc/passwd");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_ask_user_request_multiple_questions_roundtrip() {
        let m = ServerMessage::AskUserRequest {
            request_id: "uuid-3".into(),
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
                assert_eq!(request_id, "uuid-3");
                assert_eq!(call_id, "ask_1");
                assert_eq!(questions.len(), 2);
                assert_eq!(questions[0].options.len(), 2);
                assert_eq!(questions[1].options[0].label, "yes");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_plan_approval_request_roundtrip() {
        let m = ServerMessage::PlanApprovalRequest {
            request_id: "uuid-8".into(),
            call_id: "handoff_1".into(),
            plan_path: "/repo/PLAN.md".into(),
            plan_markdown: "# Goal\n\n1. do the thing".into(),
            persona: "builder".into(),
        };
        match round_trip(&m) {
            ServerMessage::PlanApprovalRequest {
                request_id,
                call_id,
                plan_path,
                plan_markdown,
                persona,
            } => {
                assert_eq!(request_id, "uuid-8");
                assert_eq!(call_id, "handoff_1");
                assert_eq!(plan_path, "/repo/PLAN.md");
                assert!(plan_markdown.contains("do the thing"));
                assert_eq!(persona, "builder");
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
            manifests: vec![],
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
            manifests: vec![],
        };
        match round_trip(&m) {
            ServerMessage::SubagentEnd {
                outcome: SubagentOutcome::Failed { reason },
                ..
            } => assert_eq!(reason, "timed out"),
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_subagent_permission_request_roundtrip() {
        let m = ServerMessage::SubagentPermissionRequest {
            request_id: "uuid-9".into(),
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
                assert_eq!(request_id, "uuid-9");
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
    fn client_message_list_personas_roundtrip() {
        let m = ClientMessage::ListPersonas;
        assert!(matches!(round_trip(&m), ClientMessage::ListPersonas));
    }

    #[test]
    fn client_message_switch_persona_roundtrip() {
        let m = ClientMessage::SwitchPersona {
            name: "code-reviewer".into(),
        };
        match round_trip(&m) {
            ClientMessage::SwitchPersona { name } => assert_eq!(name, "code-reviewer"),
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_persona_list_roundtrip() {
        let m = ServerMessage::PersonaList {
            personas: vec![PersonaInfo {
                name: "default".into(),
                description: "Default persona".into(),
                color: Some("blue".into()),
                active: true,
            }],
        };
        match round_trip(&m) {
            ServerMessage::PersonaList { personas } => {
                assert_eq!(personas.len(), 1);
                assert_eq!(personas[0].name, "default");
                assert!(personas[0].active);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_message_persona_switched_roundtrip() {
        let m = ServerMessage::PersonaSwitched {
            name: "code-reviewer".into(),
        };
        match round_trip(&m) {
            ServerMessage::PersonaSwitched { name } => {
                assert_eq!(name, "code-reviewer")
            }
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
        // request_id is now a String (UUID), not a number.
        // A numeric request_id must be rejected.
        let bad = r#"{"type":"permission_response","request_id":42,"decision":"allow_once"}"#;
        let result: Result<ClientMessage, _> = decode_json(bad);
        assert!(result.is_err(), "number in string field must be rejected");
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
        let samples: Vec<(&'static str, ClientMessage)> = vec![
            (
                "new_session",
                ClientMessage::NewSession {
                    cwd: None,
                    client_kind: ClientKind::Unknown,
                },
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
                    request_id: "uuid-0".into(),
                    decision: PermissionDecision::AllowOnce,
                },
            ),
            (
                "ask_user_response",
                ClientMessage::AskUserResponse {
                    request_id: "uuid-0".into(),
                    answers: vec![],
                },
            ),
            (
                "slash_command",
                ClientMessage::SlashCommand {
                    command: "/help".into(),
                },
            ),
            ("ping", ClientMessage::Ping),
            ("list_projects", ClientMessage::ListProjects),
            (
                "regenerate_title",
                ClientMessage::RegenerateTitle {
                    session_id: "s1".into(),
                },
            ),
        ];
        for (expected, msg) in samples {
            let json = encode_json(&msg).unwrap();
            assert!(
                json.contains(&format!(r#""type":"{}""#, expected)),
                "tag mismatch for {}: {}",
                expected,
                json
            );
        }
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
            first_message: None,
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
            first_message: None,
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
        let m = ServerMessage::RequestResolved {
            request_id: "uuid-99".into(),
        };
        match round_trip(&m) {
            ServerMessage::RequestResolved { request_id } => assert_eq!(request_id, "uuid-99"),
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
                first_message: None,
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
                first_message: None,
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
    fn browser_messages_roundtrip_tab_identity() {
        let request = ClientMessage::BrowserOpen {
            url: "https://example.com".into(),
            tab_id: Some("browser-2".into()),
        };
        match round_trip(&request) {
            ClientMessage::BrowserOpen { url, tab_id } => {
                assert_eq!(url, "https://example.com");
                assert_eq!(tab_id.as_deref(), Some("browser-2"));
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let response = ServerMessage::BrowserError {
            message: "browser operation failed".into(),
            tab_id: Some("browser-2".into()),
        };
        match round_trip(&response) {
            ServerMessage::BrowserError { message, tab_id } => {
                assert_eq!(message, "browser operation failed");
                assert_eq!(tab_id.as_deref(), Some("browser-2"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn browser_messages_remain_compatible_without_tab_identity() {
        let request: ClientMessage = decode_json(r#"{"type":"browser_close"}"#).unwrap();
        assert!(matches!(
            request,
            ClientMessage::BrowserClose { tab_id: None }
        ));

        let response: ServerMessage =
            decode_json(r#"{"type":"browser_state","open":false,"url":null,"title":null}"#)
                .unwrap();
        assert!(matches!(
            response,
            ServerMessage::BrowserState { tab_id: None, .. }
        ));
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
    fn test_regenerate_title_roundtrip() {
        let m = ClientMessage::RegenerateTitle {
            session_id: "sess_abc".into(),
        };
        match round_trip(&m) {
            ClientMessage::RegenerateTitle { session_id } => {
                assert_eq!(session_id, "sess_abc");
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
            thinking_budget: None,
            context_window: Some(128_000),
        };
        let j = encode_json(&m).unwrap();
        assert!(j.contains(r#""thinking_variants""#));
        let parsed: ModelInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.thinking_variants.len(), 2);
        assert_eq!(parsed.thinking_variants[0].name, "high");
        assert_eq!(parsed.context_window, Some(128_000));
    }

    #[test]
    fn test_model_info_with_thinking_budget() {
        let m = ModelInfo {
            id: "qwen/qwen3.8-max".into(),
            provider: "qwen".into(),
            model: "qwen3.8-max".into(),
            description: None,
            thinking_variants: vec![
                ThinkingVariantInfo { name: "low".into() },
                ThinkingVariantInfo {
                    name: "xhigh".into(),
                },
            ],
            thinking_budget: Some(ThinkingBudgetInfo {
                min: 0,
                max: 262_144,
                step: 1024,
                default: 131_072,
                by_effort: vec![("low".to_owned(), 4096), ("xhigh".to_owned(), 262_144)],
            }),
            context_window: None,
        };
        let j = encode_json(&m).unwrap();
        assert!(j.contains(r#""thinking_budget""#));
        let parsed: ModelInfo = serde_json::from_str(&j).unwrap();
        let budget = parsed.thinking_budget.expect("budget present");
        assert_eq!(
            (budget.min, budget.max, budget.step, budget.default),
            (0, 262_144, 1024, 131_072)
        );
        assert_eq!(
            budget.by_effort,
            vec![("low".to_owned(), 4096), ("xhigh".to_owned(), 262_144)]
        );
    }

    #[test]
    fn test_model_info_without_thinking_variants_skips_field() {
        let m = ModelInfo {
            id: "test/model".into(),
            provider: "test".into(),
            model: "model".into(),
            description: None,
            thinking_variants: vec![],
            thinking_budget: None,
            context_window: None,
        };
        let j = encode_json(&m).unwrap();
        // Empty vec should be skipped in serialization.
        assert!(!j.contains(r#""thinking_variants""#));
        // thinking_budget should be skipped when None.
        assert!(!j.contains(r#""thinking_budget""#));
        // context_window should be skipped when None.
        assert!(!j.contains(r#""context_window""#));
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
            cwd: None,
            model: Some("test/model".into()),
            provider: Some("test".into()),
            permission_mode: Some("permissive".into()),
        };
        match round_trip(&m) {
            ServerMessage::SessionReady {
                session_id,
                cwd,
                model,
                provider,
                permission_mode,
            } => {
                assert_eq!(session_id, "s1");
                assert!(cwd.is_none());
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
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        };
        let j = encode_json(&m).unwrap();
        // permission_mode should be skipped from serialization when None.
        assert!(!j.contains(r#""permission_mode""#));
    }

    // -- Extension protocol tests (W3a) ------------------------------------

    #[test]
    fn test_extension_hello_roundtrip() {
        let msg = ExtensionMessage::ExtensionHello {
            name: "zedra-host".into(),
            version: "0.4.0".into(),
            protocol_version: EXTENSION_PROTOCOL_VERSION,
            requested_capabilities: vec![
                "sessions:read".into(),
                "sessions:prompt".into(),
                "events:global:meta".into(),
            ],
            hook_subscriptions: vec!["on-system-prompt".into()],
            event_subscriptions: vec!["MessageEnd".into(), "ToolEnd".into()],
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionMessage::ExtensionHello {
                name,
                version,
                protocol_version,
                requested_capabilities,
                hook_subscriptions,
                event_subscriptions,
            } => {
                assert_eq!(name, "zedra-host");
                assert_eq!(version, "0.4.0");
                assert_eq!(protocol_version, EXTENSION_PROTOCOL_VERSION);
                assert_eq!(requested_capabilities.len(), 3);
                assert_eq!(requested_capabilities[0], "sessions:read");
                assert_eq!(hook_subscriptions, vec!["on-system-prompt"]);
                assert_eq!(event_subscriptions, vec!["MessageEnd", "ToolEnd"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_ready_roundtrip() {
        let msg = DaemonMessage::ExtensionReady {
            granted: vec!["sessions:read".into(), "hooks:observe".into()],
            protocol_version: EXTENSION_PROTOCOL_VERSION,
        };
        let parsed = round_trip(&msg);
        match parsed {
            DaemonMessage::ExtensionReady {
                granted,
                protocol_version,
            } => {
                assert_eq!(granted, vec!["sessions:read", "hooks:observe"]);
                assert_eq!(protocol_version, EXTENSION_PROTOCOL_VERSION);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_rejected_roundtrip() {
        let msg = DaemonMessage::ExtensionRejected {
            reason: "protocol version mismatch: extension v2, daemon v1".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            DaemonMessage::ExtensionRejected { reason } => {
                assert!(reason.contains("protocol version mismatch"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_hook_request_roundtrip() {
        let msg = DaemonMessage::HookRequest {
            request_id: "01HXYZ123ABC".into(),
            hook: "on-system-prompt".into(),
            params: serde_json::json!({ "value": "hello world" }),
        };
        let parsed = round_trip(&msg);
        match parsed {
            DaemonMessage::HookRequest {
                request_id,
                hook,
                params,
            } => {
                assert_eq!(request_id, "01HXYZ123ABC");
                assert_eq!(hook, "on-system-prompt");
                assert_eq!(params["value"], "hello world");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_hook_response_roundtrip() {
        let msg = ExtensionMessage::HookResponse {
            request_id: "01HXYZ123ABC".into(),
            outcome: "[sample-plugin] hello world".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionMessage::HookResponse {
                request_id,
                outcome,
            } => {
                assert_eq!(request_id, "01HXYZ123ABC");
                assert_eq!(outcome, "[sample-plugin] hello world");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_hook_response_block_outcome_roundtrip() {
        let msg = ExtensionMessage::HookResponse {
            request_id: "01BLOCK456".into(),
            outcome: "block:nope".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionMessage::HookResponse { outcome, .. } => {
                assert_eq!(outcome, "block:nope");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_list_sessions_roundtrip() {
        let msg = ExtensionMessage::ExtensionListSessions;
        let parsed: ExtensionMessage = round_trip(&msg);
        assert!(matches!(parsed, ExtensionMessage::ExtensionListSessions));
    }

    #[test]
    fn test_extension_prompt_roundtrip() {
        let msg = ExtensionMessage::ExtensionPrompt {
            session_id: "s123".into(),
            text: "what files are in this dir?".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionMessage::ExtensionPrompt { session_id, text } => {
                assert_eq!(session_id, "s123");
                assert_eq!(text, "what files are in this dir?");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_resolve_permission_roundtrip() {
        let msg = ExtensionMessage::ExtensionResolvePermission {
            request_id: "perm-uuid-123".into(),
            decision: "Deny".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionMessage::ExtensionResolvePermission {
                request_id,
                decision,
            } => {
                assert_eq!(request_id, "perm-uuid-123");
                assert_eq!(decision, "Deny");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_daemon_permission_request_roundtrip() {
        let msg = DaemonMessage::PermissionRequest {
            request_id: "perm-uuid-456".into(),
            session_id: "s789".into(),
            tool: "bash".into(),
            input: serde_json::json!({ "command": "rm -rf /" }),
            current_decision: "Prompt".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            DaemonMessage::PermissionRequest {
                request_id,
                session_id,
                tool,
                input,
                current_decision,
            } => {
                assert_eq!(request_id, "perm-uuid-456");
                assert_eq!(session_id, "s789");
                assert_eq!(tool, "bash");
                assert_eq!(input["command"], "rm -rf /");
                assert_eq!(current_decision, "Prompt");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_event_lifecycle_roundtrip() {
        let msg = ExtensionEvent::SessionCreated {
            session_id: "s1".into(),
            cwd: "/home/user/project".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionEvent::SessionCreated { session_id, cwd } => {
                assert_eq!(session_id, "s1");
                assert_eq!(cwd, "/home/user/project");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_event_lagged_roundtrip() {
        let msg = ExtensionEvent::Lagged { count: 42 };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionEvent::Lagged { count } => assert_eq!(count, 42),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_event_tool_ended_roundtrip() {
        let msg = ExtensionEvent::ToolEnded {
            session_id: "s1".into(),
            tool: "bash".into(),
            success: true,
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionEvent::ToolEnded {
                session_id,
                tool,
                success,
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(tool, "bash");
                assert!(success);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_host_call_notify_roundtrip() {
        let msg = ExtensionMessage::ExtensionHostCall(ExtensionHostCall::Notify {
            message: "deployment complete".into(),
        });
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionMessage::ExtensionHostCall(ExtensionHostCall::Notify { message }) => {
                assert_eq!(message, "deployment complete");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_extension_host_call_storage_write_roundtrip() {
        let msg = ExtensionHostCall::StorageWrite {
            key: "last_run".into(),
            value: "2026-07-08".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionHostCall::StorageWrite { key, value } => {
                assert_eq!(key, "last_run");
                assert_eq!(value, "2026-07-08");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_daemon_error_roundtrip() {
        let msg = DaemonMessage::Error {
            message: "capability not granted: sessions:prompt".into(),
        };
        let parsed = round_trip(&msg);
        match parsed {
            DaemonMessage::Error { message } => {
                assert!(message.contains("capability not granted"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_protocol_version_rejection() {
        // An extension sending protocol_version=99 should be rejected by
        // the daemon. We simulate this by checking that the version
        // field survives round-trip and can be compared.
        let msg = ExtensionMessage::ExtensionHello {
            name: "bad-ext".into(),
            version: "0.1.0".into(),
            protocol_version: 99, // mismatch
            requested_capabilities: vec![],
            hook_subscriptions: vec![],
            event_subscriptions: vec![],
        };
        let parsed = round_trip(&msg);
        match parsed {
            ExtensionMessage::ExtensionHello {
                protocol_version, ..
            } => {
                assert_eq!(protocol_version, 99);
                assert_ne!(protocol_version, EXTENSION_PROTOCOL_VERSION);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_daemon_session_list_roundtrip() {
        let msg = DaemonMessage::SessionList {
            sessions: vec![SessionInfo {
                session_id: "s1".into(),
                state: SessionState::Active,
                model: None,
                provider: None,
                created_at: 1000,
                last_message_at: Some(2000),
                summary: None,
                client_count: 1,
                cwd: Some("/tmp".into()),
                last_turn_failed: false,
                archived: false,
                pinned: false,
                group_id: None,
                change_stats: None,
                usage: None,
                pending_permissions: 0,
                pending_questions: 0,
                first_message: None,
            }],
        };
        let parsed = round_trip(&msg);
        match parsed {
            DaemonMessage::SessionList { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].session_id, "s1");
                assert_eq!(sessions[0].state, SessionState::Active);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_every_extension_message_variant_has_distinct_type_tag() {
        // Ensure no two ExtensionMessage variants serialize to the same
        // "type" tag, which would cause ambiguity on the wire.
        let variants = [
            ExtensionMessage::ExtensionHello {
                name: "x".into(),
                version: "1".into(),
                protocol_version: 1,
                requested_capabilities: vec![],
                hook_subscriptions: vec![],
                event_subscriptions: vec![],
            },
            ExtensionMessage::HookResponse {
                request_id: "r".into(),
                outcome: "ok".into(),
            },
            ExtensionMessage::ExtensionListSessions,
            ExtensionMessage::ExtensionAttachSession {
                session_id: "s".into(),
            },
            ExtensionMessage::ExtensionNewSession { cwd: None },
            ExtensionMessage::ExtensionPrompt {
                session_id: "s".into(),
                text: "t".into(),
            },
            ExtensionMessage::ExtensionCancel {
                session_id: "s".into(),
            },
            ExtensionMessage::ExtensionSubscribeEvents { types: vec![] },
            ExtensionMessage::ExtensionResolvePermission {
                request_id: "r".into(),
                decision: "Deny".into(),
            },
            ExtensionMessage::ExtensionHostCall(ExtensionHostCall::Notify {
                message: "m".into(),
            }),
            ExtensionMessage::ExtensionReadSessionHistory {
                session_id: "s".into(),
            },
        ];
        let mut tags: Vec<String> = variants
            .iter()
            .map(|v| {
                let json = encode_json(v).unwrap();
                let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
                parsed["type"].as_str().unwrap().to_string()
            })
            .collect();
        tags.sort();
        tags.dedup();
        assert_eq!(
            tags.len(),
            variants.len(),
            "duplicate type tags in ExtensionMessage: {:?}",
            tags
        );
    }

    #[test]
    fn test_every_daemon_message_variant_has_distinct_type_tag() {
        let variants = [
            DaemonMessage::ExtensionReady {
                granted: vec![],
                protocol_version: 1,
            },
            DaemonMessage::ExtensionRejected { reason: "x".into() },
            DaemonMessage::HookRequest {
                request_id: "r".into(),
                hook: "h".into(),
                params: serde_json::json!({}),
            },
            DaemonMessage::ExtensionEvent(Box::new(ExtensionEvent::TurnEnded {
                session_id: "s".into(),
            })),
            DaemonMessage::SessionList { sessions: vec![] },
            DaemonMessage::SessionAttached {
                session_id: "s".into(),
            },
            DaemonMessage::SessionHistory {
                session_id: "s".into(),
                messages: vec![],
            },
            DaemonMessage::HostCallResult {
                result: "ok".into(),
            },
            DaemonMessage::PermissionRequest {
                request_id: "r".into(),
                session_id: "s".into(),
                tool: "t".into(),
                input: serde_json::json!({}),
                current_decision: "Prompt".into(),
            },
            DaemonMessage::Error {
                message: "e".into(),
            },
        ];
        let mut tags: Vec<String> = variants
            .iter()
            .map(|v| {
                let json = encode_json(v).unwrap();
                let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
                parsed["type"].as_str().unwrap().to_string()
            })
            .collect();
        tags.sort();
        tags.dedup();
        assert_eq!(
            tags.len(),
            variants.len(),
            "duplicate type tags in DaemonMessage: {:?}",
            tags
        );
    }
}
