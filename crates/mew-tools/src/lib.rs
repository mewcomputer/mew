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
