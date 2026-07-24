use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Propose a goal for the session. The objective is presented to the user
/// for approval — if accepted, the goal becomes active and the turn loop
/// will auto-continue until the agent calls `complete_goal` or `block_goal`.
/// The agent does NOT set goals directly; it proposes them and the user
/// decides.
pub struct ProposeGoal;

#[async_trait]
impl Tool for ProposeGoal {
    fn name(&self) -> &str {
        "propose_goal"
    }

    fn description(&self) -> &str {
        "Propose a goal for the session. The objective is presented to the \
         user for approval. If accepted, the goal becomes active and the \
         agent will continue working across turns until the goal is \
         complete. Use this when the user gives a task that requires \
         multiple turns of autonomous work. The agent should call \
         complete_goal when the objective is achieved, or block_goal if \
         the goal cannot proceed without user input."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "objective": {
                        "type": "string",
                        "description": "A clear, concrete statement of the goal."
                    }
                },
                "required": ["objective"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "propose_goal execution must be handled by the agent core".into(),
        ))
    }
}

/// Mark the active goal as complete. Stops the turn-loop continuation.
/// Only call this after verifying the objective has been achieved.
pub struct CompleteGoal;

#[async_trait]
impl Tool for CompleteGoal {
    fn name(&self) -> &str {
        "complete_goal"
    }

    fn description(&self) -> &str {
        "Mark the active goal as complete. Call this only after verifying \
         that the objective has been fully achieved. Stops the turn-loop \
         continuation so the agent yields control back to the user."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "terminal_reason": {
                        "type": "string",
                        "description": "A brief explanation of why the goal is complete."
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
            "complete_goal execution must be handled by the agent core".into(),
        ))
    }
}

/// Block (pause) the active goal. The agent stops auto-continuing, but the
/// goal is not cleared — the user can resume it with `/goal resume`.
pub struct BlockGoal;

#[async_trait]
impl Tool for BlockGoal {
    fn name(&self) -> &str {
        "block_goal"
    }

    fn description(&self) -> &str {
        "Block the active goal, stopping the turn-loop continuation. Use \
         this when the goal cannot proceed productively without user input. \
         The goal is not cleared — the user can resume it with /goal resume."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "terminal_reason": {
                        "type": "string",
                        "description": "A brief explanation of why the goal is blocked."
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
            "block_goal execution must be handled by the agent core".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_propose_goal_metadata() {
        let tool = ProposeGoal;
        assert_eq!(tool.name(), "propose_goal");
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let schema = tool.schema();
        let required = schema.get("required").and_then(|r| r.as_array());
        assert_eq!(
            required.and_then(|a| a.first()).and_then(|v| v.as_str()),
            Some("objective")
        );
    }

    #[test]
    fn test_complete_goal_metadata() {
        let tool = CompleteGoal;
        assert_eq!(tool.name(), "complete_goal");
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let schema = tool.schema();
        // No required fields.
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn test_block_goal_metadata() {
        let tool = BlockGoal;
        assert_eq!(tool.name(), "block_goal");
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let schema = tool.schema();
        assert!(schema.get("required").is_none());
    }

    #[tokio::test]
    async fn test_propose_goal_errors_when_not_intercepted() {
        let tool = ProposeGoal;
        let ctx = ToolCtx::test_new(std::path::PathBuf::from("."));
        let result = tool
            .execute(ctx, serde_json::json!({"objective": "test"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_complete_goal_errors_when_not_intercepted() {
        let tool = CompleteGoal;
        let ctx = ToolCtx::test_new(std::path::PathBuf::from("."));
        let result = tool.execute(ctx, serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_block_goal_errors_when_not_intercepted() {
        let tool = BlockGoal;
        let ctx = ToolCtx::test_new(std::path::PathBuf::from("."));
        let result = tool.execute(ctx, serde_json::json!({})).await;
        assert!(result.is_err());
    }
}
