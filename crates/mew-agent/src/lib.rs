use std::sync::Arc;

use tokio::sync::{oneshot, Mutex};

use mew_hooks::{PermissionDecision, ToolCall as HookToolCall};
use mew_message::{Part, PartId, TurnManifest};
use mew_provider::ProviderEvent;
use mew_session::Writer as SessionWriterInner;

/// Alias for the interior-mutability wrapper required by the async agent loop.
pub type SessionWriter = Arc<Mutex<SessionWriterInner>>;

/// One option within an `ask_user_question` question. The TUI renders each
/// option as a numbered row, with `description` shown beneath as a hint.
#[derive(Debug, Clone)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// The user's decision on a `handoff_plan` request. Sent back through the
/// `oneshot` channel carried by `AgentEvent::PlanApprovalRequest`. On
/// `Approved` the agent queues the target persona switch; on
/// `ChangesRequested` the feedback becomes the tool's (successful) result so
/// the planner can revise and resubmit.
#[derive(Debug, Clone)]
pub enum PlanDecision {
    Approved,
    ChangesRequested(String),
}

/// The user's decision on a `propose_goal` request. Sent back through the
/// `oneshot` channel carried by `AgentEvent::GoalProposed`. On `Accepted`
/// the goal becomes active and the turn loop will auto-continue; on
/// `Rejected` the agent receives a tool result indicating the user declined.
#[derive(Debug, Clone)]
pub enum GoalDecision {
    Accepted,
    Rejected,
}

/// Status of an active session goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStatus {
    /// The goal is active — the turn loop will inject continuation prompts.
    Active,
    /// The goal is paused — the turn loop will not continue.
    Paused,
    /// The goal is complete — the turn loop will not continue.
    Complete,
}

/// A persistent session goal that drives turn-loop continuation.
#[derive(Debug, Clone)]
pub struct GoalState {
    pub objective: String,
    pub status: GoalStatus,
    /// How many continuation prompts have been injected for this goal.
    pub continuation_count: u32,
    /// Wall-clock timestamp (millis) when the goal was created.
    pub started_at: i64,
}

/// One question in an `ask_user_question` request. Carried from the tool
/// through `AgentEvent::AskUser` to the TUI, which renders the options as
/// a numbered list and returns the selected answer (label or freeform text).
#[derive(Debug, Clone)]
pub struct AskUserQuestion {
    pub prompt: String,
    pub options: Vec<QuestionOption>,
}

/// Events emitted by the agent core to the TUI.
pub enum AgentEvent {
    /// A raw provider event.
    Provider(ProviderEvent),
    /// Request user approval for a tool call.
    PermissionRequest {
        call: HookToolCall,
        tx: oneshot::Sender<PermissionDecision>,
    },
    /// A tool execution has started.
    ToolStart { call_id: String },
    /// A tool execution has finished.
    ToolEnd { call_id: String, success: bool },
    /// A part's content or state has changed (e.g. tool-call state transition).
    PartUpdated { part_id: PartId, part: Part },
    /// A tool produced intermediate output while running.
    ToolProgress { call_id: String, chunk: String },
    /// A terminal error occurred.
    Error(String),
    /// Request user approval for a path outside the workspace.
    WorkspacePermissionRequest {
        path: std::path::PathBuf,
        tx: oneshot::Sender<PermissionDecision>,
    },
    /// A subagent has started executing.
    SubagentStart {
        parent_call_id: String,
        name: String,
        child_session_id: String,
        /// Human-friendly per-run name picked by the runner. Optional
        /// for backwards compatibility (older callers may not set it).
        display_name: Option<String>,
    },
    /// An event from within a running subagent.
    SubagentProgress {
        parent_call_id: String,
        child_event: Box<AgentEvent>,
    },
    /// A status update from a running subagent (e.g. it called
    /// `progress_update`). Distinct from `SubagentProgress { child_event }`
    /// which wraps a stream event — this is a purpose-built channel for
    /// "what is the subagent working on right now" messages.
    SubagentStatus {
        parent_call_id: String,
        tool_name: String,
        message: String,
    },
    /// A subagent has finished executing.
    SubagentEnd {
        parent_call_id: String,
        child_session_id: String,
        outcome: mew_subagents::SubagentOutcome,
        /// Per-turn manifests from the child agent's turns. Carried on the
        /// event path for the UI/wire. May be partial for cancelled/errored runs.
        manifests: Vec<TurnManifest>,
    },
    /// A permission request from a child subagent.
    SubagentPermissionRequest {
        parent_call_id: String,
        call: HookToolCall,
        tx: oneshot::Sender<PermissionDecision>,
    },
    /// Ask the user one to four free-text questions. The tool blocks until the
    /// user answers; the answers become the tool's result text.
    AskUser {
        call_id: String,
        questions: Vec<AskUserQuestion>,
        tx: oneshot::Sender<Vec<String>>,
    },
    /// The model called `handoff_plan`: present the completed plan to the user
    /// for approval and block the tool until they respond. On `Approved` the
    /// session switches to `persona`; on `ChangesRequested` the feedback flows
    /// back as the tool result so the planner can revise and resubmit.
    PlanApprovalRequest {
        call_id: String,
        /// Resolved plan path, for display.
        plan_path: String,
        /// Plan file content captured at submit time.
        plan_markdown: String,
        /// Target persona to switch to on approval.
        persona: String,
        tx: oneshot::Sender<PlanDecision>,
    },
    /// The session's todo list changed (or is being reported). Carries the
    /// full current snapshot so the TUI can render the sidebar pane without
    /// reaching into agent state.
    TodosUpdated { todos: Vec<Todo> },
    /// A `switch_persona` tool call was queued and the current turn has
    /// ended. The caller (main loop) is responsible for looking up the
    /// persona, applying the agent-state change, and rebuilding the
    /// provider if the persona pins a model. Emitted at end of turn only
    /// — never mid-turn — so the user sees the full response before the
    /// model swap happens.
    PersonaSwitchRequested { name: String },
    /// A background shell job started, completed, failed, or was cancelled.
    /// The TUI uses this to keep the sidebar's background-jobs section
    /// in sync without polling the agent's registry.
    JobUpdate {
        job_id: String,
        command: String,
        /// One of "running", "completed", "failed", "cancelled".
        state: String,
    },
    /// A file-writing tool produced a diff delta. The daemon accumulates
    /// these into per-session change stats and broadcasts them to frontends.
    FileDelta {
        path: String,
        added: u64,
        removed: u64,
    },
    /// The flagged-files set changed (file flagged or unflagged).
    FlaggedFilesChanged { files: Vec<FlaggedFileInfo> },
    /// The model called `propose_goal`: present the objective to the user
    /// for approval and block the tool until they respond. On `Accepted`
    /// the goal becomes active; on `Rejected` the tool returns an error.
    GoalProposed {
        call_id: String,
        objective: String,
        tx: oneshot::Sender<GoalDecision>,
    },
}

/// Info about a flagged file, for the wire protocol.
#[derive(Debug, Clone)]
pub struct FlaggedFileInfo {
    pub path: String,
    pub reason: Option<String>,
}

impl std::fmt::Debug for AgentEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentEvent::Provider(ev) => f.debug_tuple("Provider").field(ev).finish(),
            AgentEvent::PermissionRequest { call, .. } => f
                .debug_struct("PermissionRequest")
                .field("call", call)
                .finish(),
            AgentEvent::ToolStart { call_id } => f
                .debug_struct("ToolStart")
                .field("call_id", call_id)
                .finish(),
            AgentEvent::ToolEnd { call_id, success } => f
                .debug_struct("ToolEnd")
                .field("call_id", call_id)
                .field("success", success)
                .finish(),
            AgentEvent::PartUpdated { part_id, part } => f
                .debug_struct("PartUpdated")
                .field("part_id", part_id)
                .field("part", part)
                .finish(),
            AgentEvent::ToolProgress { call_id, chunk } => f
                .debug_struct("ToolProgress")
                .field("call_id", call_id)
                .field("chunk", chunk)
                .finish(),
            AgentEvent::Error(msg) => f.debug_tuple("Error").field(msg).finish(),
            AgentEvent::WorkspacePermissionRequest { path, .. } => f
                .debug_struct("WorkspacePermissionRequest")
                .field("path", path)
                .finish(),
            AgentEvent::SubagentStart {
                parent_call_id,
                name,
                child_session_id,
                display_name,
            } => f
                .debug_struct("SubagentStart")
                .field("parent_call_id", parent_call_id)
                .field("name", name)
                .field("child_session_id", child_session_id)
                .field("display_name", display_name)
                .finish(),
            AgentEvent::SubagentProgress {
                parent_call_id,
                child_event,
            } => f
                .debug_struct("SubagentProgress")
                .field("parent_call_id", parent_call_id)
                .field("child_event", child_event)
                .finish(),
            AgentEvent::SubagentEnd {
                parent_call_id,
                child_session_id,
                outcome,
                ..
            } => f
                .debug_struct("SubagentEnd")
                .field("parent_call_id", parent_call_id)
                .field("child_session_id", child_session_id)
                .field("outcome", outcome)
                .finish(),
            AgentEvent::SubagentPermissionRequest {
                parent_call_id,
                call,
                ..
            } => f
                .debug_struct("SubagentPermissionRequest")
                .field("parent_call_id", parent_call_id)
                .field("call", call)
                .finish(),
            AgentEvent::SubagentStatus {
                parent_call_id,
                tool_name,
                message,
            } => f
                .debug_struct("SubagentStatus")
                .field("parent_call_id", parent_call_id)
                .field("tool_name", tool_name)
                .field("message", message)
                .finish(),
            AgentEvent::AskUser {
                call_id, questions, ..
            } => f
                .debug_struct("AskUser")
                .field("call_id", call_id)
                .field("questions", questions)
                .finish(),
            AgentEvent::PlanApprovalRequest {
                call_id,
                plan_path,
                plan_markdown,
                persona,
                ..
            } => f
                .debug_struct("PlanApprovalRequest")
                .field("call_id", call_id)
                .field("plan_path", plan_path)
                .field("plan_markdown_len", &plan_markdown.len())
                .field("persona", persona)
                .finish(),
            AgentEvent::TodosUpdated { todos } => f
                .debug_struct("TodosUpdated")
                .field("count", &todos.len())
                .finish(),
            AgentEvent::PersonaSwitchRequested { name } => f
                .debug_struct("PersonaSwitchRequested")
                .field("name", name)
                .finish(),
            AgentEvent::JobUpdate { job_id, state, .. } => f
                .debug_struct("JobUpdate")
                .field("job_id", job_id)
                .field("state", state)
                .finish(),
            AgentEvent::FileDelta {
                path,
                added,
                removed,
            } => f
                .debug_struct("FileDelta")
                .field("path", path)
                .field("added", added)
                .field("removed", removed)
                .finish(),
            AgentEvent::FlaggedFilesChanged { files } => f
                .debug_struct("FlaggedFilesChanged")
                .field("count", &files.len())
                .finish(),
            AgentEvent::GoalProposed {
                call_id,
                objective,
                ..
            } => f
                .debug_struct("GoalProposed")
                .field("call_id", call_id)
                .field("objective", objective)
                .finish(),
        }
    }
}

mod agent;
mod events;
pub mod manifest;
mod prompt_cache;
mod reasoning_truncator;
pub mod runner;
mod todos;
mod tools;
mod turn;
mod workspace;

pub use agent::Agent;
pub use mew_subagents::SubagentOutcome;
pub use prompt_cache::PromptCacheRetention;
pub use reasoning_truncator::{
    ReasoningTruncator, DEFAULT_REASONING_TRUNCATION_THRESHOLD, TRUNCATION_ACK_TEXT,
};
pub use todos::{Todo, TodoList, TodoStatus};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hooks_tests;
