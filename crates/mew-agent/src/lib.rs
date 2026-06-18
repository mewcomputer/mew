use std::sync::Arc;

use tokio::sync::{oneshot, Mutex};

use mew_hooks::{PermissionDecision, ToolCall as HookToolCall};
use mew_message::{Part, PartId};
use mew_provider::ProviderEvent;
use mew_session::Writer as SessionWriterInner;

/// Alias for the interior-mutability wrapper required by the async agent loop.
pub type SessionWriter = Arc<Mutex<SessionWriterInner>>;

/// One question in an `ask_user_question` request. Carried from the tool
/// through `AgentEvent::AskUser` to the TUI, which renders it as a free-text
/// input and returns the answer.
#[derive(Debug, Clone)]
pub struct AskUserQuestion {
    pub prompt: String,
    pub default: Option<String>,
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
        }
    }
}

mod agent;
mod events;
pub mod runner;
mod tools;
mod turn;
mod workspace;

pub use agent::Agent;
pub use mew_subagents::SubagentOutcome;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hooks_tests;
