use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Report a status update from the subagent to the parent.
///
/// The subagent can call this tool mid-run to tell the parent "I am working
/// on X." The message is recorded in the subagent's transcript and the
/// tool-call event is forwarded to the parent (so a UI can show "subagent
/// is now working on..."). The subagent's run continues; this does not
/// terminate the loop.
pub struct ProgressUpdate;

#[async_trait]
impl Tool for ProgressUpdate {
    fn name(&self) -> &str {
        "progress_update"
    }

    fn description(&self) -> &str {
        "Report a status update to the parent agent. Use this mid-run to \
         communicate what you are currently working on, so the parent (and \
         any user watching the UI) can see progress. The subagent run \
         continues after this call; it is purely informational."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "A short status message describing what the \
                                        subagent is currently doing. Keep it to one sentence; \
                                        it is meant to be skimmed."
                    }
                },
                "required": ["message"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let msg = input
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing or non-string message".into()))?;

        Ok(ToolOutput {
            output: format!("ok: noted \"{}\"", msg),
            error: String::new(),
            diff: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx() -> ToolCtx {
        ToolCtx {
            session_id: mew_message::SessionId::from(ulid::Ulid::new()),
            call_id: "test".to_string(),
            cancel: tokio_util::sync::CancellationToken::new(),
            progress_tx: tokio::sync::mpsc::channel(1).0,
            cwd: PathBuf::from("."),
            dispatcher: None,
            secrets: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_progress_update_acknowledges() {
        let tool = ProgressUpdate;
        let input = serde_json::json!({"message": "starting work"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert!(result.output.contains("starting work"));
        assert!(result.error.is_empty());
    }

    #[tokio::test]
    async fn test_progress_update_missing_message_errors() {
        let tool = ProgressUpdate;
        let input = serde_json::json!({});
        let result = tool.execute(dummy_ctx(), input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_progress_update_metadata() {
        let tool = ProgressUpdate;
        assert_eq!(tool.name(), "progress_update");
        assert!(tool.description().contains("status"));
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let schema = tool.schema();
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v == "message"));
    }
}
