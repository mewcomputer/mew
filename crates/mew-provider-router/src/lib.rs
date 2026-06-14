use std::sync::Arc;

use async_trait::async_trait;
use mew_message::Part;
use mew_provider::{EventStream, ModelInfo, Provider, ProviderError, Request};

/// Routes requests between a small (cheap) and big (capable) provider based on
/// conversation complexity.
pub struct Router {
    small: Arc<dyn Provider>,
    big: Arc<dyn Provider>,
    /// Number of turns before switching to the big model. Default: 3.
    turn_threshold: usize,
}

impl Router {
    pub fn new(small: Arc<dyn Provider>, big: Arc<dyn Provider>) -> Self {
        Self {
            small,
            big,
            turn_threshold: 3,
        }
    }

    /// Set the turn threshold. After this many messages, the big model is used.
    pub fn set_turn_threshold(&mut self, n: usize) {
        self.turn_threshold = n;
    }

    /// Decide which provider to use based on conversation complexity.
    fn select(&self, req: &Request) -> &Arc<dyn Provider> {
        // If there are tool results in any assistant message, use big.
        let has_tool_results = req
            .messages
            .iter()
            .any(|m| m.parts.iter().any(|p| matches!(p, Part::ToolResult(_))));

        let is_long = req.messages.len() > self.turn_threshold;

        if has_tool_results || is_long {
            &self.big
        } else {
            &self.small
        }
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
        let mut models = self.big.list_models().await?;
        models.extend(self.small.list_models().await?);
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc;
    use futures::SinkExt;
    use mew_message::{Message, Part, PartBase, Role, TextPart, Time, ToolResultPart};
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
            let tag = self.tag;
            tokio::spawn(async move {
                let msg = mew_message::Message {
                    id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                    role: Role::Assistant,
                    parts: vec![Part::Text(TextPart {
                        base: PartBase {
                            id: ulid::Ulid::new(),
                            message_id: ulid::Ulid::new(),
                            session_id: ulid::Ulid::new(),
                        },
                        text: format!("from {}", tag),
                        synthetic: false,
                    })],
                    time: Time {
                        created: 0,
                        completed: None,
                    },
                    assistant: None,
                };
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

    #[tokio::test]
    async fn test_router_uses_small_for_simple_turn() {
        let small = Arc::new(TaggedProvider { tag: "small" });
        let big = Arc::new(TaggedProvider { tag: "big" });
        let router = Router::new(small, big);

        let req = Request {
            model: String::new(),
            messages: vec![],
            tools: vec![],
            system: String::new(),
            reasoning: None,
        };

        // For an empty conversation, the router should select small.
        let provider = router.select(&req);
        assert_eq!(provider.name(), "small");
    }

    #[tokio::test]
    async fn test_router_uses_big_with_tool_results() {
        let small = Arc::new(TaggedProvider { tag: "small" });
        let big = Arc::new(TaggedProvider { tag: "big" });
        let router = Router::new(small, big);

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

        let req = Request {
            model: String::new(),
            messages: vec![msg],
            tools: vec![],
            system: String::new(),
            reasoning: None,
        };

        let provider = router.select(&req);
        assert_eq!(provider.name(), "big");
    }

    #[tokio::test]
    async fn test_router_uses_big_after_threshold() {
        let small = Arc::new(TaggedProvider { tag: "small" });
        let big = Arc::new(TaggedProvider { tag: "big" });
        let router = Router::new(small, big);

        let mut messages = Vec::new();
        for _ in 0..5 {
            messages.push(Message {
                id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
                role: Role::User,
                parts: vec![],
                time: Time {
                    created: 0,
                    completed: None,
                },
                assistant: None,
            });
        }

        let req = Request {
            model: String::new(),
            messages,
            tools: vec![],
            system: String::new(),
            reasoning: None,
        };

        let provider = router.select(&req);
        assert_eq!(provider.name(), "big");
    }
}

/// Wraps a Router with a display model name for the TUI status line.
pub struct Routed {
    inner: Router,
    /// The model ID to show in the status line (typically the big model).
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
