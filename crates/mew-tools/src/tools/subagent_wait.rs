use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct SubagentWait {
    schema: Value,
}

impl Default for SubagentWait {
    fn default() -> Self {
        Self::new()
    }
}

impl SubagentWait {
    pub fn new() -> Self {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID returned by subagent_start."
                }
            },
            "required": ["task_id"]
        });
        Self { schema }
    }
}

#[async_trait]
impl Tool for SubagentWait {
    fn name(&self) -> &str {
        "subagent_wait"
    }

    fn description(&self) -> &str {
        "Wait for a background subagent to complete and get its result. \
         Use the task ID returned by subagent_start when you passed `async: true`. \
         Most of the time you should just use subagent_start without `async` and \
         it will return the result directly without needing this tool."
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        // Execution is intercepted by the agent core.
        Err(ToolError::Execution(
            "subagent_wait execution must be handled by the agent core".into(),
        ))
    }
}
