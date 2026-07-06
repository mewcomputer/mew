use std::sync::Arc;

use async_trait::async_trait;
use mew_message::Part;
use mew_provider::{EventStream, ModelInfo, Provider, ProviderError, Request};

/// Routes requests between `nano` (cheapest), `micro` (medium), and `deci`
/// (most capable) providers based on conversation complexity.
pub struct Router {
    nano: Option<Arc<dyn Provider>>,
    micro: Arc<dyn Provider>,
    deci: Arc<dyn Provider>,
}

impl Router {
    pub fn new(
        nano: Option<Arc<dyn Provider>>,
        micro: Arc<dyn Provider>,
        deci: Arc<dyn Provider>,
    ) -> Self {
        Self { nano, micro, deci }
    }

    /// Decide which provider to use based on conversation complexity.
    fn select(&self, req: &Request) -> &Arc<dyn Provider> {
        // If there are tool results in any message, use the most capable model.
        let has_tool_results = req
            .messages
            .iter()
            .any(|m| m.parts.iter().any(|p| matches!(p, Part::ToolResult(_))));

        if has_tool_results {
            return &self.deci;
        }

        // First turn uses nano if configured; otherwise micro. All later turns
        // without tool results stay on micro.
        if req.messages.len() <= 1 {
            if let Some(ref nano) = self.nano {
                return nano;
            }
        }

        &self.micro
    }
}

#[async_trait]
impl Provider for Router {
    fn name(&self) -> &str {
        "router"
    }

    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError> {
        let provider = self.select(&req);
        tracing::debug!(
            provider = provider.name(),
            messages = req.messages.len(),
            "router selected provider"
        );
        provider.stream(req).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut models = self.deci.list_models().await?;
        models.extend(self.micro.list_models().await?);
        if let Some(ref nano) = self.nano {
            models.extend(nano.list_models().await?);
        }
        Ok(models)
    }
}

/// Wraps a Router with a display model name for the TUI status line.
pub struct Routed {
    inner: Router,
    /// The model ID to show in the status line (typically the deci model).
    pub display_model: String,
    /// The provider ID to show in the status line.
    pub display_provider: String,
}

impl Routed {
    pub fn new(router: Router, display_provider: String, display_model: String) -> Self {
        Self {
            inner: router,
            display_model,
            display_provider,
        }
    }
}

#[async_trait]
impl Provider for Routed {
    fn name(&self) -> &str {
        "router"
    }

    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError> {
        self.inner.stream(req).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.inner.list_models().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc;
    use futures::SinkExt;
    use mew_message::{Message, Part, PartBase, Role, Time, ToolResultPart};
    use mew_provider::ProviderError;

    struct TaggedProvider {
        tag: &'static str,
    }

    #[async_trait]
    impl Provider for TaggedProvider {
        fn name(&self) -> &str {
            self.tag
        }
        async fn stream(&self, _req: Request) -> Result<EventStream, ProviderError> {
            let (mut tx, rx) = mpsc::channel(1);
            tokio::spawn(async move {
                let _ = tx
                    .send(mew_provider::ProviderEvent::MessageEnd {
                        finish: mew_message::Finish::Stop,
                        usage: mew_message::Tokens::default(),
                        cost: 0.0,
                    })
                    .await;
            });
            Ok(Box::pin(rx))
        }
    }

    fn empty_request(messages: Vec<Message>) -> Request {
        Request {
            model: String::new(),
            messages,
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: Default::default(),
        }
    }

    fn empty_message() -> Message {
        Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::User,
            parts: vec![],
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        }
    }

    #[tokio::test]
    async fn test_router_uses_nano_for_first_turn() {
        let nano = Arc::new(TaggedProvider { tag: "nano" });
        let micro = Arc::new(TaggedProvider { tag: "micro" });
        let deci = Arc::new(TaggedProvider { tag: "deci" });
        let router = Router::new(Some(nano), micro, deci);

        let req = empty_request(vec![empty_message()]);
        let provider = router.select(&req);
        assert_eq!(provider.name(), "nano");
    }

    #[tokio::test]
    async fn test_router_uses_micro_for_simple_turn_without_nano() {
        let micro = Arc::new(TaggedProvider { tag: "micro" });
        let deci = Arc::new(TaggedProvider { tag: "deci" });
        let router = Router::new(None, micro, deci);

        let req = empty_request(vec![empty_message()]);
        let provider = router.select(&req);
        assert_eq!(provider.name(), "micro");
    }

    #[tokio::test]
    async fn test_router_uses_micro_for_short_conversation() {
        let nano = Arc::new(TaggedProvider { tag: "nano" });
        let micro = Arc::new(TaggedProvider { tag: "micro" });
        let deci = Arc::new(TaggedProvider { tag: "deci" });
        let router = Router::new(Some(nano), micro, deci);

        let req = empty_request(vec![empty_message(), empty_message()]);
        let provider = router.select(&req);
        assert_eq!(provider.name(), "micro");
    }

    #[tokio::test]
    async fn test_router_uses_deci_with_tool_results() {
        let nano = Arc::new(TaggedProvider { tag: "nano" });
        let micro = Arc::new(TaggedProvider { tag: "micro" });
        let deci = Arc::new(TaggedProvider { tag: "deci" });
        let router = Router::new(Some(nano), micro, deci);

        let msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::User,
            parts: vec![Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                call_id: "c1".into(),
            })],
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };

        let req = empty_request(vec![msg]);
        let provider = router.select(&req);
        assert_eq!(provider.name(), "deci");
    }

    #[tokio::test]
    async fn test_router_uses_micro_for_long_text_conversation() {
        let nano = Arc::new(TaggedProvider { tag: "nano" });
        let micro = Arc::new(TaggedProvider { tag: "micro" });
        let deci = Arc::new(TaggedProvider { tag: "deci" });
        let router = Router::new(Some(nano), micro, deci);

        let mut messages = Vec::new();
        for _ in 0..5 {
            messages.push(empty_message());
        }

        let req = empty_request(messages);
        let provider = router.select(&req);
        assert_eq!(provider.name(), "micro");
    }
}
