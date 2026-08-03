//! The `CommandTarget` trait — the backend abstraction for dispatch.
//!
//! `LocalTarget` implements it directly against the `Agent`.
//! `DaemonTarget` (Phase 2) implements it against the `DaemonClient`.
//!
//! `Unsupported` is the only sanctioned way to not implement an operation.
//! It produces a visible alert, never a swallowed keypress.

use mew_agent::AgentEvent;
use mew_hooks::PermissionMode;
use mew_message::Part;
use mew_personas::Persona;
use tokio::sync::mpsc::Receiver;

/// Returned by `CommandTarget` methods that are not meaningful for a given
/// backend. Rendered as a visible alert — never silently dropped.
#[derive(Debug)]
pub struct Unsupported(pub &'static str);

/// Result of applying a permission mode change.
#[derive(Debug, Clone)]
pub struct SwitchedModel {
    pub provider_id: String,
    pub model_id: String,
    pub display: String,
}

/// Result of applying a persona switch.
#[derive(Debug, Clone)]
pub struct PersonaApplied {
    pub pinned_model: Option<String>,
    pub display: String,
}

/// The backend abstraction. Each method corresponds to an `Action` or
/// `SlashResult` that requires backend-specific execution.
///
/// Methods that return `Result<_, Unsupported>` may not be meaningful for
/// all backends. `Err(Unsupported(reason))` renders a visible alert.
#[async_trait::async_trait]
pub trait CommandTarget: Send {
    /// Submit a prompt. Returns the event receiver for streaming.
    fn prompt(&mut self, enriched: String, parts: Vec<Part>) -> Receiver<AgentEvent>;

    /// Intercept user input before it reaches the model (e.g. for plugin hooks).
    /// Default implementation is a passthrough.
    async fn intercept_user_input(&mut self, text: String) -> String {
        text
    }

    /// Cancel the current streaming turn.
    async fn cancel(&mut self);

    /// Clear the conversation context.
    async fn clear(&mut self) -> Result<(), Unsupported>;

    /// Force context compaction.
    async fn compact(&mut self) -> Result<(), Unsupported>;

    /// Get the current todo list as a rendered string.
    async fn todos(&mut self) -> Result<String, Unsupported>;

    /// Switch to a different model. Returns the new provider/model IDs.
    async fn switch_model(&mut self, spec: &str) -> Result<SwitchedModel, Unsupported>;

    /// Set the permission mode.
    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), Unsupported>;

    /// Set the thinking/reasoning variant. Empty string or "off"/"none" disables.
    async fn set_thinking(&mut self, variant: &str) -> Result<(), Unsupported>;

    /// Attach to a different session (daemon mode).
    async fn attach_session(&mut self, id: &str) -> Result<(), Unsupported>;

    /// Resume a previous session by ID.
    async fn resume(&mut self, id: &str) -> Result<(), Unsupported>;

    /// Rewind to keep only the first N messages.
    async fn rewind(&mut self, n: usize) -> Result<(), Unsupported>;

    /// Switch to a persona by name. "default"/"none" clears the persona.
    async fn switch_persona(
        &mut self,
        name: &str,
        personas: &[Persona],
    ) -> Result<PersonaApplied, Unsupported>;

    /// Called after a persona switch completes, for dispatcher hooks.
    /// Default is a no-op.
    async fn on_persona_change(&mut self, _old: Option<&str>, _new: &str) {}

    /// Execute a plugin-registered slash command.
    async fn plugin_command(&mut self, name: &str, args: &str) -> Result<String, Unsupported>;

    /// Cancel the most recently started running subagent. Returns true if
    /// a cancellation was requested.
    async fn cancel_subagent(&mut self, task_id: &str) -> Result<bool, Unsupported>;

    /// Manage the session goal. Returns a status string for display.
    async fn manage_goal(&mut self, action: GoalAction) -> Result<String, Unsupported>;

    /// Enable or disable auto-generated session titles (daemon mode).
    async fn set_auto_title(&mut self, enabled: bool) -> Result<(), Unsupported>;

    /// Enable or disable idle session summaries (daemon mode).
    async fn set_auto_summary(&mut self, enabled: bool) -> Result<(), Unsupported>;

    /// Yield control of the session to other clients (daemon mode).
    async fn yield_control(&mut self) -> Result<(), Unsupported>;

    /// Remove a file from the session's flagged-files set.
    async fn unflag_file(&mut self, path: &str) -> Result<(), Unsupported>;

    /// Request the daemon's project directory list (daemon mode). The
    /// project picker opens when `ProjectList` arrives.
    async fn list_projects(&mut self) -> Result<(), Unsupported>;

    /// Create and attach to a new session in the given project directory
    /// (daemon mode).
    async fn new_session_in(&mut self, path: &str) -> Result<(), Unsupported>;

    /// Archive or unarchive a session (daemon mode).
    async fn archive_session(&mut self, id: &str, archived: bool) -> Result<(), Unsupported>;

    /// Pin or unpin a session (daemon mode).
    async fn pin_session(&mut self, id: &str, pinned: bool) -> Result<(), Unsupported>;

    /// Rename a session — set a custom title (daemon mode).
    async fn rename_session(&mut self, id: &str, title: &str) -> Result<(), Unsupported>;
}

/// Goal management actions for `CommandTarget::manage_goal`.
#[derive(Debug, Clone)]
pub enum GoalAction {
    Set(String),
    Status,
    Pause,
    Resume,
    Clear,
    Complete,
}
