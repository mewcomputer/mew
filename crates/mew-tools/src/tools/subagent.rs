use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct Subagent {
    defs: Arc<Vec<mew_subagents::SubagentDef>>,
    desc: String,
    schema: Value,
}

impl Subagent {
    pub fn new(defs: Arc<Vec<mew_subagents::SubagentDef>>) -> Self {
        let mut desc = String::from(
            "Launch a subagent to handle a specialized task. The subagent runs in its own \
             session with its own tools and model. Use this when you want to delegate work \
             to a specialized agent.",
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
                "model": {
                    "type": "string",
                    "description": "Optional model override. Use \"micro\" or \"deci\" to select the router's configured tier, or pass a fully-qualified \"provider/model\"."
                }
            },
            "required": ["name", "prompt"]
        });

        Self { defs, desc, schema }
    }
}

#[async_trait]
impl Tool for Subagent {
    fn name(&self) -> &str {
        "subagent"
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

    async fn execute(&self, _ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing 'name' field".into()))?;

        let prompt = input.get("prompt").and_then(|v| v.as_str()).unwrap_or("");

        let def = self
            .defs
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| ToolError::InvalidInput(format!("unknown subagent: {name}")))?;

        Err(ToolError::Execution(format!(
            "subagent '{}' execution must be handled by the agent core. \
             this tool is registered for schema purposes only. prompt length: {}",
            def.name,
            prompt.len()
        )))
    }
}
