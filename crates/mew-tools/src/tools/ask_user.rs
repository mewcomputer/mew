use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Ask the user one to four free-text questions. Execution is intercepted by
/// the agent core, which routes the questions to the TUI as an
/// `AgentEvent::AskUser` and blocks the tool until the user answers.
pub struct AskUser;

#[async_trait]
impl Tool for AskUser {
    fn name(&self) -> &str {
        "ask_user_question"
    }

    fn description(&self) -> &str {
        "Ask the user 1-4 questions when their answer would change your next \
         step. Every question allows a free-text answer. Use this instead of \
         guessing when only the user can decide (which branch to target, which \
         file to edit, whether an assumption is correct). Do not use it for \
         yes/no questions the conversation already settled, or for anything \
         you can find out yourself with the other tools."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 4,
                        "items": {
                            "type": "object",
                            "properties": {
                                "prompt": {
                                    "type": "string",
                                    "description": "The question to ask the user."
                                },
                                "default": {
                                    "type": "string",
                                    "description": "Optional default answer, shown as a hint and pre-filled."
                                }
                            },
                            "required": ["prompt"]
                        }
                    }
                },
                "required": ["questions"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "ask_user_question execution must be handled by the agent core".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata() {
        let tool = AskUser;
        assert_eq!(tool.name(), "ask_user_question");
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let schema = tool.schema();
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v == "questions"));
        let questions = schema
            .get("properties")
            .and_then(|p| p.get("questions"))
            .unwrap();
        assert_eq!(questions.get("minItems").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(questions.get("maxItems").and_then(|v| v.as_u64()), Some(4));
    }

    #[tokio::test]
    async fn test_execute_errors_when_not_intercepted() {
        let tool = AskUser;
        let ctx = ToolCtx::test_new(std::path::PathBuf::from("."));
        let result = tool.execute(ctx, serde_json::json!({})).await;
        assert!(result.is_err());
    }
}
