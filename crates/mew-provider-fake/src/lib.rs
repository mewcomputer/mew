use async_trait::async_trait;
use mew_message::{
    Finish, MessageId, Part, PartBase, PartId, SessionId, TextPart, Tokens, ToolCallPart,
    ToolState, ToolStatePending, ToolTime,
};
use mew_provider::{EventStream, Provider, ProviderError, ProviderEvent, Request};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

pub struct FakeProvider {
    script: Vec<ProviderEvent>,
}

impl FakeProvider {
    pub fn new(script: Vec<ProviderEvent>) -> Self {
        Self { script }
    }

    pub fn text_response(text: &str) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        let part_id = PartId::new();
        let message_id = MessageId::new();
        let session_id = SessionId::new();

        events.push(ProviderEvent::PartStart {
            part: Part::Text(TextPart {
                base: PartBase {
                    id: part_id,
                    message_id,
                    session_id,
                },
                text: String::new(),
                synthetic: false,
            }),
        });

        for chunk in text.chars().collect::<Vec<_>>().chunks(4) {
            let delta: String = chunk.iter().collect();
            events.push(ProviderEvent::PartDelta {
                part_id,
                field: "text",
                delta,
            });
        }

        events.push(ProviderEvent::PartEnd { part_id });
        events.push(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            usage: Tokens::default(),
            cost: 0.0,
        });

        events
    }

    pub fn tool_call(name: &str, id: &str, input: serde_json::Value) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        let part_id = PartId::new();
        let message_id = MessageId::new();
        let session_id = SessionId::new();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        events.push(ProviderEvent::PartStart {
            part: Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: part_id,
                    message_id,
                    session_id,
                },
                tool_name: name.to_string(),
                call_id: id.to_string(),
                state: ToolState::Pending(ToolStatePending {
                    input,
                    time: ToolTime {
                        start: now,
                        end: None,
                    },
                }),
                raw_input: String::new(),
            }),
        });
        events.push(ProviderEvent::PartEnd { part_id });
        events.push(ProviderEvent::MessageEnd {
            finish: Finish::ToolUse,
            usage: Tokens::default(),
            cost: 0.0,
        });

        events
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn name(&self) -> &str {
        "fake"
    }

    async fn stream(&self, _req: Request) -> Result<EventStream, ProviderError> {
        let script = self.script.clone();
        let stream = futures::stream::unfold(script.into_iter(), |mut iter| async move {
            if let Some(event) = iter.next() {
                sleep(Duration::from_millis(10)).await;
                Some((event, iter))
            } else {
                None
            }
        });

        Ok(Box::pin(stream) as EventStream)
    }
}
