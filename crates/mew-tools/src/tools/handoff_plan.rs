use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Submit the completed plan for user approval and hand off execution.
/// Execution is intercepted by the agent core, which reads the configured
/// plan file, routes it to the frontend as an `AgentEvent::PlanApprovalRequest`,
/// and blocks the tool until the user approves or requests changes. On
/// approval the session switches to the target persona.
pub struct HandoffPlan;

#[async_trait]
impl Tool for HandoffPlan {
    fn name(&self) -> &str {
        "handoff_plan"
    }

    fn description(&self) -> &str {
        "Submit the completed plan for user approval and hand off execution. On \
         approval the session switches to the target persona (default: \
         builder). On rejection the result contains the user's feedback — \
         revise with edit_plan and submit again. Intended as the final step of \
         the planning workflow."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "persona": {
                        "type": "string",
                        "description": "The persona to switch to on approval. Defaults to \"builder\"."
                    }
                }
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "handoff_plan execution must be handled by the agent core".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata() {
        let tool = HandoffPlan;
        assert_eq!(tool.name(), "handoff_plan");
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let schema = tool.schema();
        // persona is optional — no required array.
        assert!(schema.get("required").is_none());
        let persona = schema
            .get("properties")
            .and_then(|p| p.get("persona"))
            .unwrap();
        assert_eq!(persona.get("type").and_then(|v| v.as_str()), Some("string"));
    }

    #[tokio::test]
    async fn test_execute_errors_when_not_intercepted() {
        let tool = HandoffPlan;
        let ctx = ToolCtx::test_new(std::path::PathBuf::from("."));
        let result = tool.execute(ctx, serde_json::json!({})).await;
        assert!(result.is_err());
    }
}
