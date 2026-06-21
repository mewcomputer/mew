//! Wraps a `ToolRegistration` from a plugin into a `Tool` impl.

use async_trait::async_trait;
use serde_json::Value;

use mew_hooks::ToolRegistration;

/// A `Tool` implementation wrapping a plugin-registered tool.
pub struct DynamicTool {
    reg: ToolRegistration,
    schema: Value,
}

impl DynamicTool {
    pub fn new(reg: ToolRegistration) -> Self {
        let schema = reg.input_schema.clone();
        Self { reg, schema }
    }
}

#[async_trait]
impl mew_tools::Tool for DynamicTool {
    fn name(&self) -> &str {
        &self.reg.name
    }

    fn description(&self) -> &str {
        &self.reg.description
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    fn sensitivity(&self) -> mew_tools::Sensitivity {
        mew_tools::Sensitivity::Mutating
    }

    async fn execute(
        &self,
        _ctx: mew_tools::ToolCtx,
        input: Value,
    ) -> Result<mew_hooks::ToolOutput, mew_tools::ToolError> {
        let result = (self.reg.execute)(input).await;
        Ok(mew_hooks::ToolOutput {
            output: result,
            error: String::new(),
            diff: None,
        })
    }
}
