//! Explicit browser tools available only to the native desktop host.

use crate::browser::BrowserAction;
use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

const NAMES: &[&str] = &[
    "browser_open",
    "browser_snapshot",
    "browser_screenshot",
    "browser_click",
    "browser_fill",
    "browser_press",
    "browser_close",
];

pub fn tools() -> Vec<Arc<dyn Tool>> {
    NAMES
        .iter()
        .map(|name| Arc::new(BrowserTool { name }) as Arc<dyn Tool>)
        .collect()
}

struct BrowserTool {
    name: &'static str,
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        match self.name {
            "browser_open" => "Open an HTTP(S) URL in the user's in-app browser.",
            "browser_snapshot" => {
                "Read the current in-app browser page as an untrusted semantic snapshot."
            }
            "browser_screenshot" => "Capture the current in-app browser page as an image.",
            "browser_click" => "Click an element in the in-app browser by its snapshot selector.",
            "browser_fill" => "Fill an input in the in-app browser by its snapshot selector.",
            "browser_press" => "Press a key in the in-app browser.",
            "browser_close" => "Close the current in-app browser page.",
            _ => "Use the in-app browser.",
        }
    }

    fn schema(&self) -> &Value {
        static SCHEMAS: std::sync::OnceLock<std::collections::HashMap<&'static str, Value>> =
            std::sync::OnceLock::new();
        SCHEMAS
            .get_or_init(|| {
                let mut schemas = std::collections::HashMap::new();
                schemas.insert("browser_open", serde_json::json!({
                    "type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]
                }));
                schemas.insert("browser_snapshot", serde_json::json!({"type": "object", "properties": {}}));
                schemas.insert("browser_screenshot", serde_json::json!({"type": "object", "properties": {"annotate": {"type": "boolean"}}}));
                schemas.insert("browser_click", serde_json::json!({
                    "type": "object", "properties": {"selector": {"type": "string"}}, "required": ["selector"]
                }));
                schemas.insert("browser_fill", serde_json::json!({
                    "type": "object", "properties": {"selector": {"type": "string"}, "text": {"type": "string"}}, "required": ["selector", "text"]
                }));
                schemas.insert("browser_press", serde_json::json!({
                    "type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]
                }));
                schemas.insert("browser_close", serde_json::json!({"type": "object", "properties": {}}));
                schemas
            })
            .get(self.name)
            .expect("browser tool schema exists")
    }

    fn sensitivity(&self) -> Sensitivity {
        match self.name {
            "browser_snapshot" | "browser_screenshot" => Sensitivity::ReadOnly,
            "browser_open" | "browser_click" | "browser_fill" | "browser_press"
            | "browser_close" => Sensitivity::Mutating,
            _ => Sensitivity::Mutating,
        }
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        if !ctx.browser_access.is_available() {
            return Err(ToolError::Execution(
                "in-app browser is unavailable; attach this session in the mew desktop app".into(),
            ));
        }

        let action = match self.name {
            "browser_open" => BrowserAction::Open(string_arg(&input, "url")?),
            "browser_snapshot" => BrowserAction::Snapshot,
            "browser_screenshot" => BrowserAction::Screenshot {
                annotate: input
                    .get("annotate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            "browser_click" => BrowserAction::Click(string_arg(&input, "selector")?),
            "browser_fill" => BrowserAction::Fill {
                selector: string_arg(&input, "selector")?,
                text: string_arg(&input, "text")?,
            },
            "browser_press" => BrowserAction::Press(string_arg(&input, "key")?),
            "browser_close" => BrowserAction::Close,
            _ => return Err(ToolError::InvalidInput("unknown browser tool".into())),
        };
        let result = ctx
            .browser_access
            .execute(action, None, &ctx.cancel)
            .await
            .map_err(ToolError::Execution)?;
        if let Some(bytes) = result.screenshot {
            return Ok(ToolOutput {
                output: "untrusted browser screenshot".into(),
                metadata: Some(serde_json::json!({
                    "mime": "image/png",
                    "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
                })),
                ..Default::default()
            });
        }
        let text = result.output;
        Ok(ToolOutput {
            output: format!("untrusted browser content:\n{text}"),
            ..Default::default()
        })
    }
}

fn string_arg(input: &Value, name: &str) -> Result<String, ToolError> {
    input
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ToolError::InvalidInput(format!("missing '{name}' field")))
}

#[cfg(test)]
mod tests {
    use super::tools;

    #[test]
    fn registers_explicit_browser_tools() {
        let names: Vec<_> = tools().iter().map(|tool| tool.name().to_string()).collect();
        assert_eq!(
            names,
            vec![
                "browser_open",
                "browser_snapshot",
                "browser_screenshot",
                "browser_click",
                "browser_fill",
                "browser_press",
                "browser_close",
            ]
        );
    }
}
