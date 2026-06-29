//! Web fetch tool: download a URL and return its content as markdown.

use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Maximum content length to return (128 KB). Prevents flooding the
/// context window with a huge page.
const MAX_CONTENT_CHARS: usize = 128_000;

pub struct WebFetch;

#[async_trait]
impl Tool for WebFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and return its content as markdown. \
         Useful for reading documentation pages, API references, and articles. \
         HTML pages are converted to readable markdown."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The HTTP or HTTPS URL to fetch."
                    }
                },
                "required": ["url"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing 'url' field".into()))?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::InvalidInput(
                "url must start with http:// or https://".into(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| ToolError::Execution(format!("failed to build HTTP client: {e}")))?;

        let response = client
            .get(url)
            .header(
                "User-Agent",
                "mew/0.1 (terminal coding assistant; +https://github.com/natalie/mew)",
            )
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/json,text/plain,*/*;q=0.8",
            )
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(ToolError::Execution(format!(
                "HTTP {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )));
        }

        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|ct| ct.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = response
            .text()
            .await
            .map_err(|e| ToolError::Execution(format!("failed to read response body: {e}")))?;

        // Convert based on content type. HTML gets markdown conversion;
        // everything else (JSON, plain text) is returned as-is.
        let content = if content_type.contains("text/html") || content_type.contains("xhtml") {
            match htmd::convert(&body) {
                Ok(md) => md,
                Err(_) => body,
            }
        } else {
            body
        };

        // Truncate to prevent flooding the context window.
        let (content, truncated) = if content.chars().count() > MAX_CONTENT_CHARS {
            let truncated: String = content.chars().take(MAX_CONTENT_CHARS).collect();
            (
                format!(
                    "{}\n\n[... content truncated at {} chars ...]",
                    truncated, MAX_CONTENT_CHARS
                ),
                true,
            )
        } else {
            (content, false)
        };

        let _ = truncated; // available for future metadata

        Ok(ToolOutput {
            output: format!(
                "URL: {}\nContent-Type: {}\n\n{}",
                final_url, content_type, content
            ),
            error: String::new(),
            diff: None,
            ..Default::default()
        })
    }
}
