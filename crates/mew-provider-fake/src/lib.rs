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
                sensitivity: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use mew_provider::Provider;

    /// Tiny helper: drain a stream into a Vec with a generous timeout so a
    /// deadlock or hang surfaces as a test failure rather than a CI stall.
    async fn drain(stream: EventStream) -> Vec<ProviderEvent> {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.collect::<Vec<_>>(),
        )
        .await
        .expect("fake provider stream did not terminate within 5s")
    }

    fn empty_request() -> Request {
        Request::default()
    }

    #[tokio::test]
    async fn text_response_produces_expected_event_sequence() {
        let script = FakeProvider::text_response("hi");
        assert!(matches!(script[0], ProviderEvent::PartStart { .. }));
        assert!(matches!(
            script[script.len() - 2],
            ProviderEvent::PartEnd { .. }
        ));
        assert!(matches!(
            script.last().unwrap(),
            ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn text_response_reassembles_to_original_text() {
        let script = FakeProvider::text_response("hello world");
        let provider = FakeProvider::new(script.clone());

        let events = drain(provider.stream(empty_request()).await.unwrap()).await;

        let mut text = String::new();
        for e in &events {
            if let ProviderEvent::PartDelta { field, delta, .. } = e {
                assert_eq!(*field, "text", "non-text delta in text_response script");
                text.push_str(delta);
            }
        }
        assert_eq!(text, "hello world");
    }

    #[tokio::test]
    async fn text_response_empty_string_yields_no_deltas() {
        let script = FakeProvider::text_response("");
        assert_eq!(script.len(), 3, "expected PartStart + PartEnd + MessageEnd");
        assert!(script
            .iter()
            .all(|e| !matches!(e, ProviderEvent::PartDelta { .. })));

        let provider = FakeProvider::new(script);
        let events = drain(provider.stream(empty_request()).await.unwrap()).await;
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn text_response_unicode_round_trips() {
        // Multi-byte chars must not get split mid-codepoint. The script
        // uses chars().chunks(4) so this verifies that path holds.
        let original = "héllo 🦀 世界";
        let script = FakeProvider::text_response(original);
        let provider = FakeProvider::new(script);

        let events = drain(provider.stream(empty_request()).await.unwrap()).await;
        let text: String = events
            .iter()
            .filter_map(|e| {
                if let ProviderEvent::PartDelta { delta, .. } = e {
                    Some(delta.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(text, original);
    }

    #[tokio::test]
    async fn text_response_delta_count_matches_chunked_text() {
        // Chunks are 4 chars wide; expect ceil(len / 4) PartDelta events.
        for n in [1usize, 4, 5, 12, 100] {
            let text: String = "x".repeat(n);
            let script = FakeProvider::text_response(&text);
            let deltas = script
                .iter()
                .filter(|e| matches!(e, ProviderEvent::PartDelta { .. }))
                .count();
            assert_eq!(
                deltas,
                n.div_ceil(4),
                "expected {} deltas for len={}",
                n.div_ceil(4),
                n
            );
        }
    }

    #[tokio::test]
    async fn text_response_part_id_is_consistent_across_deltas() {
        let script = FakeProvider::text_response("abcdefghij");
        let start_id = match &script[0] {
            ProviderEvent::PartStart { part } => part.id(),
            _ => panic!("expected PartStart first"),
        };
        for e in &script[1..] {
            match e {
                ProviderEvent::PartDelta { part_id, .. } | ProviderEvent::PartEnd { part_id } => {
                    assert_eq!(*part_id, start_id, "part_id drift across events");
                }
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn tool_call_emits_pending_state_and_tool_use_finish() {
        let script =
            FakeProvider::tool_call("bash", "call_1", serde_json::json!({"command": "ls"}));
        assert_eq!(script.len(), 3, "PartStart + PartEnd + MessageEnd");
        assert!(matches!(script[0], ProviderEvent::PartStart { .. }));
        assert!(matches!(script[1], ProviderEvent::PartEnd { .. }));
        assert!(matches!(
            script[2],
            ProviderEvent::MessageEnd {
                finish: Finish::ToolUse,
                ..
            }
        ));

        // The PartStart must carry the tool name and call id, with Pending state.
        if let ProviderEvent::PartStart { part } = &script[0] {
            if let Part::ToolCall(tc) = part {
                assert_eq!(tc.tool_name, "bash");
                assert_eq!(tc.call_id, "call_1");
                assert!(matches!(tc.state, ToolState::Pending(_)));
            } else {
                panic!("expected Part::ToolCall at PartStart, got {:?}", part);
            }
        }
    }

    #[tokio::test]
    async fn tool_call_input_is_preserved_in_pending_state() {
        let input = serde_json::json!({"path": "/tmp/x", "mode": "included"});
        let script = FakeProvider::tool_call("flag_important", "c2", input.clone());
        let provider = FakeProvider::new(script);
        let events = drain(provider.stream(empty_request()).await.unwrap()).await;

        if let Some(ProviderEvent::PartStart { part }) = events.first() {
            if let Part::ToolCall(tc) = part {
                if let ToolState::Pending(p) = &tc.state {
                    assert_eq!(p.input, input);
                } else {
                    panic!("expected Pending state");
                }
            } else {
                panic!("expected ToolCall part");
            }
        }
    }

    #[tokio::test]
    async fn tool_call_and_text_response_get_distinct_part_ids() {
        let t1 = FakeProvider::text_response("a");
        let t2 = FakeProvider::tool_call("bash", "x", serde_json::json!({}));
        let t3 = FakeProvider::text_response("b");
        let id1 = match &t1[0] {
            ProviderEvent::PartStart { part } => part.id(),
            _ => unreachable!(),
        };
        let id2 = match &t2[0] {
            ProviderEvent::PartStart { part } => part.id(),
            _ => unreachable!(),
        };
        let id3 = match &t3[0] {
            ProviderEvent::PartStart { part } => part.id(),
            _ => unreachable!(),
        };
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[tokio::test]
    async fn stream_replays_script_in_order() {
        let script = FakeProvider::text_response("ordered");
        let provider = FakeProvider::new(script.clone());
        let events = drain(provider.stream(empty_request()).await.unwrap()).await;

        // Compare structurally: same variants in same order. (We can't
        // compare part_ids / message_ids since those are regenerated on
        // each stream() call.)
        let shape: Vec<&'static str> = events
            .iter()
            .map(|e| match e {
                ProviderEvent::PartStart { .. } => "PartStart",
                ProviderEvent::PartDelta { .. } => "PartDelta",
                ProviderEvent::PartEnd { .. } => "PartEnd",
                ProviderEvent::MessageEnd { .. } => "MessageEnd",
                ProviderEvent::RetryWait { .. } => "RetryWait",
                ProviderEvent::Error(_) => "Error",
            })
            .collect();
        let expected: Vec<&'static str> = script
            .iter()
            .map(|e| match e {
                ProviderEvent::PartStart { .. } => "PartStart",
                ProviderEvent::PartDelta { .. } => "PartDelta",
                ProviderEvent::PartEnd { .. } => "PartEnd",
                ProviderEvent::MessageEnd { .. } => "MessageEnd",
                ProviderEvent::RetryWait { .. } => "RetryWait",
                ProviderEvent::Error(_) => "Error",
            })
            .collect();
        assert_eq!(shape, expected);
    }

    #[tokio::test]
    async fn stream_terminates_after_last_event() {
        // Two short scripts back-to-back: the stream from the first call
        // must close, then a second stream() call must produce fresh
        // events. This catches futures that hold onto a leaked iterator.
        let provider = FakeProvider::new(FakeProvider::text_response(""));
        let first = drain(provider.stream(empty_request()).await.unwrap()).await;
        assert_eq!(first.len(), 3); // PartStart + PartEnd + MessageEnd

        let provider = FakeProvider::new(FakeProvider::text_response(""));
        let second = drain(provider.stream(empty_request()).await.unwrap()).await;
        assert_eq!(second.len(), 3);
    }

    #[tokio::test]
    async fn stream_with_empty_script_yields_no_events() {
        let provider = FakeProvider::new(vec![]);
        let events = drain(provider.stream(empty_request()).await.unwrap()).await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn name_is_fake() {
        let provider = FakeProvider::new(vec![]);
        assert_eq!(provider.name(), "fake");
    }
}
