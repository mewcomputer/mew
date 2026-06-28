use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Graceful exit for a subagent. Calling this tool signals "I'm done; here is
/// my final answer" and stops the subagent's run cleanly (without burning
/// remaining turn or time budget). The `final_answer` argument is what the
/// parent agent will see as the subagent's result.
///
/// This tool is a no-op as far as the model is concerned (it just echoes the
/// answer back as its output). The runner detects the call by name and
/// uses the output as the subagent's `result_text` before breaking the loop.
pub struct ExitTool;

#[async_trait]
impl Tool for ExitTool {
    fn name(&self) -> &str {
        "exit_tool"
    }

    fn description(&self) -> &str {
        "Stop the subagent run and return `final_answer` to the parent agent. \
         Use this when you have completed the task and want to short-circuit \
         any remaining turns. After this tool completes, the subagent loop \
         ends; the parent receives `final_answer` as your result."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "final_answer": {
                        "type": "string",
                        "description": "The final answer to return to the parent agent. \
                                        Should be the complete deliverable: a summary, a finding, \
                                        a code block, etc."
                    }
                },
                "required": ["final_answer"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let answer = input
            .get("final_answer")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing or non-string final_answer".into()))?;

        Ok(ToolOutput {
            output: answer.to_string(),
            error: String::new(),
            diff: None,
            metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx() -> ToolCtx {
        ToolCtx::test_new(PathBuf::from("."))
    }

    #[tokio::test]
    async fn test_exit_tool_returns_answer() {
        let tool = ExitTool;
        let input = serde_json::json!({"final_answer": "the answer is 42"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert_eq!(result.output, "the answer is 42");
        assert!(result.error.is_empty());
    }

    #[tokio::test]
    async fn test_exit_tool_missing_final_answer_errors() {
        let tool = ExitTool;
        let input = serde_json::json!({});
        let result = tool.execute(dummy_ctx(), input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_exit_tool_metadata() {
        let tool = ExitTool;
        assert_eq!(tool.name(), "exit_tool");
        assert!(tool.description().contains("final_answer"));
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let schema = tool.schema();
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v == "final_answer"));
    }
}
