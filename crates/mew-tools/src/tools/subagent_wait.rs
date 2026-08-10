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
                },
                "task_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Wait for several tasks at once. Returns a JSON object keyed by task ID, each with a per-task status, so one failed task does not fail the batch."
                },
                "all": {
                    "type": "boolean",
                    "description": "Wait for every outstanding subagent task. Returns the same keyed JSON object as task_ids.",
                    "default": false
                }
            }
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
        "Wait for background subagents and get their results. Pass exactly one of: \
         task_id (a single task ID from subagent_start), task_ids (collect several \
         at once, results keyed by task ID), or all (collect every outstanding task). \
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
