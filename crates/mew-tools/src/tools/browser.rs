//! Explicit browser tools available only to the native desktop host.

use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::process::Command;

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
        if !ctx.browser_enabled {
            return Err(ToolError::Execution(
                "in-app browser is unavailable; attach this session in the mew desktop app".into(),
            ));
        }

        let mut args = vec!["--session".to_string(), format!("mew-{}", ctx.session_id)];
        if let Ok(port) = std::env::var("MEW_BROWSER_CDP_PORT") {
            args = vec!["--cdp".into(), port];
        }
        let mut screenshot_path: Option<std::path::PathBuf> = None;
        match self.name {
            "browser_open" => args.extend(["open".into(), string_arg(&input, "url")?]),
            "browser_snapshot" => args.extend(["snapshot".into(), "--json".into()]),
            "browser_screenshot" => {
                let path = std::env::temp_dir().join(format!("mew-tool-{}.png", ulid::Ulid::new()));
                screenshot_path = Some(path.clone());
                args.extend(["screenshot".into(), path.to_string_lossy().into_owned()]);
                if input
                    .get("annotate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    args.push("--annotate".into());
                }
            }
            "browser_click" => args.extend(["click".into(), string_arg(&input, "selector")?]),
            "browser_fill" => args.extend([
                "fill".into(),
                string_arg(&input, "selector")?,
                string_arg(&input, "text")?,
            ]),
            "browser_press" => args.extend(["press".into(), string_arg(&input, "key")?]),
            "browser_close" => args.push("close".into()),
            _ => return Err(ToolError::InvalidInput("unknown browser tool".into())),
        }

        let output = Command::new("agent-browser")
            .args(&args)
            .output()
            .await
            .map_err(|e| ToolError::Execution(format!("run agent-browser: {e}")))?;
        if !output.status.success() {
            return Err(ToolError::Execution(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Some(path) = screenshot_path {
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| ToolError::Execution(format!("read browser screenshot: {e}")))?;
            let _ = tokio::fs::remove_file(path).await;
            return Ok(ToolOutput {
                output: "untrusted browser screenshot".into(),
                metadata: Some(serde_json::json!({
                    "mime": "image/png",
                    "data": base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        bytes,
                    ),
                })),
                ..Default::default()
            });
        }
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
