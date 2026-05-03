use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use mew_message::{ErrorKind, Finish, Message, Part, PartId, Tokens};

pub type EventStream = Pin<Box<dyn Stream<Item = ProviderEvent> + Send>>;

/// Basic info about a model returned by a provider.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub owned_by: String,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError>;
    /// List available models from the provider API.
    /// Default implementation returns an empty list.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub system: String,
}

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum ProviderEvent {
    PartStart {
        part: Part,
    },
    PartDelta {
        part_id: PartId,
        field: &'static str,
        delta: String,
    },
    PartEnd {
        part_id: PartId,
    },
    MessageEnd {
        finish: Finish,
        usage: Tokens,
        cost: f64,
    },
    /// The provider is waiting before retrying (rate limit / server error).
    RetryWait {
        attempt: u32,
        max_attempts: u32,
        delay_secs: u64,
        reason: String,
    },
    Error(mew_message::MessageError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("provider error: {0}")]
    Message(String),
    #[error("{kind:?}: {message}")]
    Classified { kind: ErrorKind, message: String },
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub retry_5xx: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 4,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            retry_5xx: true,
        }
    }
}

impl RetryPolicy {
    pub fn should_retry(&self, status_code: u16, attempt: usize) -> (Duration, bool) {
        match status_code {
            429 => {
                if attempt >= self.max_retries {
                    return (Duration::ZERO, false);
                }
                let backoff = self.initial_backoff * 2_u32.pow(attempt as u32);
                let backoff = backoff.min(self.max_backoff);
                (backoff, true)
            }
            500..=599 => {
                if !self.retry_5xx || attempt >= 1 {
                    return (Duration::ZERO, false);
                }
                (self.initial_backoff, true)
            }
            _ => (Duration::ZERO, false),
        }
    }
}

pub fn classify_error(status_code: u16, body: &str) -> (ErrorKind, String) {
    match status_code {
        401 | 403 => (
            ErrorKind::ProviderAuth,
            format!("authentication failed: {body}"),
        ),
        429 => (
            ErrorKind::ProviderRateLimit,
            format!("rate limited: {body}"),
        ),
        500..=599 => (
            ErrorKind::ProviderOverload,
            format!("server error ({status_code}): {body}"),
        ),
        400..=499 => (
            ErrorKind::ProviderApi,
            format!("client error ({status_code}): {body}"),
        ),
        _ => (ErrorKind::Unknown, format!("http {status_code}: {body}")),
    }
}

/// Map an HTTP status code to a human-readable retry reason.
pub fn classify_reason(status_code: u16) -> String {
    match status_code {
        429 => "rate limited".into(),
        500..=599 => "server overloaded".into(),
        _ => format!("http {status_code}"),
    }
}

pub mod imageutil {
    use base64::Engine;
    use std::path::Path;

    pub async fn resolve(url: &str) -> anyhow::Result<(String, String)> {
        if let Some(rest) = url.strip_prefix("data:") {
            parse_data_url(rest)
        } else if let Some(path) = url.strip_prefix("file://") {
            let bytes = tokio::fs::read(path).await?;
            let mime = guess_mime(path);
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok((mime, b64))
        } else if url.starts_with("http://") || url.starts_with("https://") {
            let client = reqwest::Client::new();
            let resp = client.get(url).send().await?;
            let mime = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(';').next().unwrap_or(s).to_string())
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let bytes = resp.bytes().await?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok((mime, b64))
        } else {
            anyhow::bail!("unsupported image url scheme: {}", url)
        }
    }

    fn parse_data_url(rest: &str) -> anyhow::Result<(String, String)> {
        let rest = rest.trim_start_matches("data:");
        let Some((meta, data)) = rest.split_once(',') else {
            anyhow::bail!("invalid data url");
        };
        let mime = if let Some((m, _)) = meta.split_once(";base64") {
            if m.is_empty() {
                "text/plain".to_string()
            } else {
                m.to_string()
            }
        } else {
            "text/plain".to_string()
        };
        Ok((mime, data.to_string()))
    }

    fn guess_mime(path: &str) -> String {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        match ext.to_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => "application/octet-stream",
        }
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_policy_429_backoff() {
        let policy = RetryPolicy::default();

        // 429: exponential backoff starting at 1s
        let (backoff, retry) = policy.should_retry(429, 0);
        assert!(retry);
        assert_eq!(backoff, Duration::from_secs(1));

        let (backoff, retry) = policy.should_retry(429, 1);
        assert!(retry);
        assert_eq!(backoff, Duration::from_secs(2));

        let (backoff, retry) = policy.should_retry(429, 2);
        assert!(retry);
        assert_eq!(backoff, Duration::from_secs(4));

        let (backoff, retry) = policy.should_retry(429, 3);
        assert!(retry);
        assert_eq!(backoff, Duration::from_secs(8));

        // 4th retry (attempt 4) exceeds max_retries=4
        let (backoff, retry) = policy.should_retry(429, 4);
        assert!(!retry);
        assert_eq!(backoff, Duration::ZERO);
    }

    #[test]
    fn test_retry_policy_429_capped() {
        let policy = RetryPolicy {
            initial_backoff: Duration::from_secs(10),
            max_backoff: Duration::from_secs(30),
            max_retries: 10,
            retry_5xx: true,
        };

        // 10s * 2^2 = 40s, but capped at 30s
        let (backoff, retry) = policy.should_retry(429, 2);
        assert!(retry);
        assert_eq!(backoff, Duration::from_secs(30));
    }

    #[test]
    fn test_retry_policy_5xx_single_retry() {
        let policy = RetryPolicy::default();

        // First 5xx gets one retry
        let (backoff, retry) = policy.should_retry(500, 0);
        assert!(retry);
        assert_eq!(backoff, Duration::from_secs(1));

        // Second 5xx attempt fails
        let (backoff, retry) = policy.should_retry(500, 1);
        assert!(!retry);
        assert_eq!(backoff, Duration::ZERO);
    }

    #[test]
    fn test_retry_policy_5xx_disabled() {
        let policy = RetryPolicy {
            retry_5xx: false,
            ..Default::default()
        };

        let (_, retry) = policy.should_retry(500, 0);
        assert!(!retry);
    }

    #[test]
    fn test_retry_policy_4xx_no_retry() {
        let policy = RetryPolicy::default();

        let (_, retry) = policy.should_retry(400, 0);
        assert!(!retry);

        let (_, retry) = policy.should_retry(404, 0);
        assert!(!retry);
    }

    #[test]
    fn test_classify_error() {
        assert_eq!(
            classify_error(401, "unauthorized"),
            (
                ErrorKind::ProviderAuth,
                "authentication failed: unauthorized".to_string()
            )
        );
        assert_eq!(
            classify_error(429, "too many requests"),
            (
                ErrorKind::ProviderRateLimit,
                "rate limited: too many requests".to_string()
            )
        );
        assert_eq!(
            classify_error(500, "internal error"),
            (
                ErrorKind::ProviderOverload,
                "server error (500): internal error".to_string()
            )
        );
        assert_eq!(
            classify_error(400, "bad request"),
            (
                ErrorKind::ProviderApi,
                "client error (400): bad request".to_string()
            )
        );
    }
}
