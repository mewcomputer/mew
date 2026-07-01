pub mod secrets;
pub mod tools;

use async_trait::async_trait;
use mew_hooks::ToolOutput;
use mew_message::SessionId;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    ReadOnly,
    Mutating,
    Dangerous,
}

/// Session-shared state that every tool call sees. Built once at agent
/// startup and cloned via `Arc` for each `ToolCtx`. Grows without bloating
/// the per-call `ToolCtx` struct.
#[derive(Clone)]
pub struct ToolCtxShared {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub dispatcher: Option<Arc<dyn mew_hooks::Dispatcher>>,
    /// Secret values and file globs to redact from this tool's output.
    /// Shared via `Arc` from the agent's startup config; defaults empty in
    /// tests via `Default::default()`.
    pub secrets: Arc<SecretSet>,
    /// Optional persistent shell session shared across bash tool calls.
    /// When `Some`, the `bash` tool uses it instead of spawning a fresh
    /// process — so `cd`, `export`, and other state survive between calls.
    /// `None` (the default) means each bash call is independent.
    pub shell_session: Option<crate::tools::shell_session::SharedShellSession>,
    /// Shared snapshot store for hashline tag binding and recovery.
    pub snapshot_store: std::sync::Arc<dyn mew_hashline::SnapshotStore>,
}

impl Default for ToolCtxShared {
    fn default() -> Self {
        Self {
            session_id: SessionId::default(),
            cwd: PathBuf::from("."),
            dispatcher: None,
            secrets: Arc::new(SecretSet::default()),
            shell_session: None,
            snapshot_store: std::sync::Arc::new(mew_hashline::InMemorySnapshotStore::new()),
        }
    }
}

/// Per-call context passed to every `Tool::execute`. The per-call fields
/// (`call_id`, `cancel`, `progress_tx`) are unique to each invocation; the
/// session-shared fields live behind `Arc<ToolCtxShared>` and are reused
/// across calls.
///
/// `ToolCtx` implements `Deref<Target = ToolCtxShared>` so tools can access
/// `ctx.session_id`, `ctx.cwd`, `ctx.secrets`, `ctx.dispatcher` directly —
/// same syntax as before the refactor.
pub struct ToolCtx {
    pub call_id: String,
    pub cancel: CancellationToken,
    pub progress_tx: mpsc::Sender<ToolProgress>,
    pub shared: Arc<ToolCtxShared>,
}

impl std::ops::Deref for ToolCtx {
    type Target = ToolCtxShared;
    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

impl ToolCtx {
    /// Construct a per-call context from shared state + per-call fields.
    pub fn new(
        shared: Arc<ToolCtxShared>,
        call_id: String,
        cancel: CancellationToken,
        progress_tx: mpsc::Sender<ToolProgress>,
    ) -> Self {
        Self {
            shared,
            call_id,
            cancel,
            progress_tx,
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub fn test_new(cwd: PathBuf) -> Self {
        Self::new(
            Arc::new(ToolCtxShared {
                cwd,
                ..Default::default()
            }),
            "test".into(),
            CancellationToken::new(),
            mpsc::channel(1).0,
        )
    }

    /// Test helper: build a ToolCtx with custom secrets (for testing
    /// redaction in Read/Bash/Grep/Glob).
    #[cfg(test)]
    pub fn test_with_secrets(cwd: PathBuf, secrets: Arc<SecretSet>) -> Self {
        Self::new(
            Arc::new(ToolCtxShared {
                cwd,
                secrets,
                ..Default::default()
            }),
            "test".into(),
            CancellationToken::new(),
            mpsc::channel(1).0,
        )
    }
}

/// Secret values and file globs that must be redacted from tool output.
#[derive(Debug, Clone, Default)]
pub struct SecretSet {
    /// Substrings to redact from any tool output line that contains them.
    pub words: Vec<String>,
    /// Glob patterns for secret files; results touching these are dropped.
    pub globs: Vec<String>,
}

impl SecretSet {
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.globs.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum ToolProgress {
    OutputChunk(String),
    Metadata(Value),
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("cancelled")]
    Cancelled,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &Value;
    fn sensitivity(&self) -> Sensitivity;

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError>;
}
