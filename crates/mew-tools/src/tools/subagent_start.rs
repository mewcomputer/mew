use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct SubagentStart {
    #[allow(dead_code)]
    defs: Arc<Vec<mew_subagents::SubagentDef>>,
    desc: String,
    schema: Value,
}

impl SubagentStart {
    pub fn new(defs: Arc<Vec<mew_subagents::SubagentDef>>) -> Self {
        let mut desc = String::from(
            "Invoke a subagent. By default this blocks until the subagent finishes and returns \
             its result directly, so you can use the result in your next step. The subagent has \
             its own isolated session — its tool calls and intermediate steps do not appear in \
             your history.\n\
             \n\
             Pass `async: true` to start the subagent in the background and return a task ID \
             immediately. Use subagent_wait with the task ID to collect the result later, which \
             is useful for running multiple subagents in parallel before combining their results.",
        );
        if !defs.is_empty() {
            desc.push_str("\n\nAvailable subagents:");
            for def in defs.iter() {
                desc.push_str(&format!("\n- {}: {}", def.name, def.description));
            }
        }

        let mut names: Vec<String> = defs.iter().map(|d| d.name.clone()).collect();
        names.sort();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The subagent name to invoke.",
                    "enum": names
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to send to the subagent."
                },
                "async": {
                    "type": "boolean",
                    "description": "If true, return a task ID immediately and run in the background; collect with subagent_wait. Defaults to false (block and return the result inline).",
                    "default": false
                }
            },
            "required": ["name", "prompt"]
        });

        Self { defs, desc, schema }
    }
}

#[async_trait]
impl Tool for SubagentStart {
    fn name(&self) -> &str {
        "subagent_start"
    }

    fn description(&self) -> &str {
        &self.desc
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
            "subagent_start execution must be handled by the agent core".into(),
        ))
    }
}
