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

pub struct ToolCtx {
    pub session_id: SessionId,
    pub call_id: String,
    pub cancel: CancellationToken,
    pub progress_tx: mpsc::Sender<ToolProgress>,
    pub cwd: PathBuf,
    pub dispatcher: Option<Arc<dyn mew_hooks::Dispatcher>>,
    /// Secret values and file globs to redact from this tool's output.
    /// Shared via `Arc` from the agent's startup config; defaults empty in
    /// tests via `Default::default()`.
    pub secrets: Arc<SecretSet>,
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
