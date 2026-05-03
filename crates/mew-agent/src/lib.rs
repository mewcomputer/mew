use std::sync::Arc;

use tokio::sync::{oneshot, Mutex};

use mew_hooks::{PermissionDecision, ToolCall as HookToolCall};
use mew_message::{Part, PartId};
use mew_provider::ProviderEvent;
use mew_session::Writer as SessionWriterInner;

/// Alias for the interior-mutability wrapper required by the async agent loop.
pub type SessionWriter = Arc<Mutex<SessionWriterInner>>;

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
}

impl std::fmt::Debug for AgentEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentEvent::Provider(ev) => f.debug_tuple("Provider").field(ev).finish(),
            AgentEvent::PermissionRequest { call, .. } => f
                .debug_struct("PermissionRequest")
                .field("call", call)
                .finish(),
            AgentEvent::ToolStart { call_id } => {
                f.debug_struct("ToolStart").field("call_id", call_id).finish()
            }
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
        }
    }
}

mod agent;
mod turn;
mod events;
mod tools;

pub use agent::Agent;

#[cfg(test)]
mod tests;
