use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// A tool that returns skill bodies on demand.
pub struct Skill {
    skills: Arc<Vec<mew_skills::Skill>>,
    /// Active persona's skill allow-list. `None` = all discovered skills
    /// (default); `Some(set)` = only skills whose name is in the set.
    /// Shared with the owning `Agent` via `Arc` so the agent can update it on
    /// `apply_persona`.
    filter: Arc<tokio::sync::RwLock<Option<HashSet<String>>>>,
}

impl Skill {
    pub fn new(
        skills: Arc<Vec<mew_skills::Skill>>,
        filter: Arc<tokio::sync::RwLock<Option<HashSet<String>>>>,
    ) -> Self {
        Self { skills, filter }
    }
}

#[async_trait]
impl Tool for Skill {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        "Load a skill's full instructions. Use this when a task matches a skill's description."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The skill name to load."
                    }
                },
                "required": ["name"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing 'name' field".into()))?;

        // Check the persona's skill allow-list. An empty set is a valid
        // "no skills" filter, not a "no filter" signal.
        let filter = self.filter.read().await;
        if let Some(ref allowed) = *filter {
            if !allowed.contains(name) {
                return Err(ToolError::InvalidInput(format!(
                    "skill '{name}' is not available in the current persona"
                )));
            }
        }

        let skill = self
            .skills
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| ToolError::InvalidInput(format!("unknown skill: {name}")))?;

        Ok(ToolOutput {
            output: skill.body.clone(),
            error: String::new(),
            diff: None,
            metadata: None,
        })
    }
}
