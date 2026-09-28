//! SSE stream parsing for `POST /v1/responses`.

use crate::Adapter;
use futures::{channel::mpsc, SinkExt, StreamExt};
use mew_message::{
    ErrorKind, Finish, MessageError, Part, PartBase, ReasoningPart, TextPart, Tokens, ToolCallPart,
    ToolState, ToolStatePending, ToolTime,
};
use mew_provider::ProviderEvent;
use tokio::io::AsyncBufReadExt;

/// Tracks per-item state during SSE streaming.
struct StreamState {
    /// Whether this is a responses-lite stream (affects reasoning handling).
    use_responses_lite: bool,
    /// item_id → (PartId, text buffer)
    text_parts: std::collections::HashMap<String, TextPart>,
    /// item_id → (PartId, reasoning buffer)
    reasoning_parts: std::collections::HashMap<String, ReasoningPart>,
    /// item_id → PartId for reasoning items, kept even after the part is
    /// finalized so we can attach encrypted_content from output_item.done.
    reasoning_part_ids: std::collections::HashMap<String, ulid::Ulid>,
    /// item_id → (PartId, tool_name, call_id, args buffer)
    tool_calls: std::collections::HashMap<String, ToolCallAccumulator>,
    /// Track which PartIds have been finalized (PartEnd emitted).
    finalized_parts: std::collections::HashSet<ulid::Ulid>,
    /// Whether we've emitted MessageEnd.
    message_end_emitted: bool,
}

impl StreamState {
    fn new() -> Self {
        Self {
            use_responses_lite: false,
            text_parts: std::collections::HashMap::new(),
            reasoning_parts: std::collections::HashMap::new(),
            reasoning_part_ids: std::collections::HashMap::new(),
            tool_calls: std::collections::HashMap::new(),
            finalized_parts: std::collections::HashSet::new(),
            message_end_emitted: false,
        }
    }

    fn is_part_finalized(&self, id: ulid::Ulid) -> bool {
        self.finalized_parts.contains(&id)
    }

    fn mark_finalized(&mut self, id: ulid::Ulid) {
        self.finalized_parts.insert(id);
    }
}

#[derive(Debug)]
struct ToolCallAccumulator {
    part: ToolCallPart,
    json: String,
}

impl ToolCallAccumulator {
    fn finalize(&mut self) {
        // Always preserve raw arguments for debugging, even if parse fails.
        self.part.raw_input = self.json.clone();

        if !self.json.is_empty() {
            match serde_json::from_str(&self.json) {
                Ok(input) => {
                    self.part.state = ToolState::Pending(ToolStatePending {
                        input,
                        time: ToolTime {
                            start: chrono::Utc::now().timestamp_millis(),
                            end: None,
                        },
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        "tool call arguments JSON parse failed: {e}. \
                         Raw arguments: {}",
                        self.json
                    );
                    // Set the input to the raw string so the agent can
                    // see what the model sent, rather than silently
                    // passing Null.
                    self.part.state = ToolState::Pending(ToolStatePending {
                        input: serde_json::Value::String(self.json.clone()),
                        time: ToolTime {
                            start: chrono::Utc::now().timestamp_millis(),
                            end: None,
                        },
                    });
                }
            }
        }
    }
}

impl Adapter {
    pub(crate) async fn read_stream(
        dump: bool,
        resp: reqwest::Response,
        mut tx: mpsc::Sender<ProviderEvent>,
        use_responses_lite: bool,
    ) {
        let stream = resp
            .bytes_stream()
            .map(|res| res.map_err(std::io::Error::other));
        let reader = tokio::io::BufReader::new(tokio_util::io::StreamReader::new(stream));
        let mut lines = reader.lines();

        let mut current_event = String::new();
        let mut state = StreamState::new();
        state.use_responses_lite = use_responses_lite;

        loop {
            let line: String = match lines.next_line().await {
                Ok(Some(l)) => l,
                Ok(None) => break,
                Err(e) => {
                    let _ = tx
                        .send(ProviderEvent::Error(MessageError {
                            kind: ErrorKind::Network,
                            message: format!("sse stream: {e}"),
                        }))
                        .await;
                    break;
                }
            };

            if dump {
                eprintln!("[RAW SSE] {}", line);
            }

            if let Some(ev) = line.strip_prefix("event: ") {
                current_event = ev.trim().to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                // Blank line or non-data line — skip.
                continue;
            };

            let data = data.trim();

            // "[DONE]" sentinel (some proxies emit it).
            if data == "[DONE]" {
                continue;
            }

            match current_event.as_str() {
                "response.created" | "response.in_progress" | "response.queued" => {}

                "response.output_item.added" => {
                    Self::handle_output_item_added(data, &mut tx, &mut state).await;
                }

                "response.content_part.added" => {
                    Self::handle_content_part_added(data, &mut tx, &mut state).await;
                }

                "response.output_text.delta" => {
                    Self::handle_output_text_delta(data, &mut tx, &mut state).await;
                }

                "response.output_text.done" => {
                    Self::handle_output_text_done(data, &mut tx, &mut state).await;
                }

                "response.reasoning_summary_text.delta" => {
                    Self::handle_reasoning_delta(data, &mut tx, &mut state).await;
                }

                "response.reasoning_summary_text.done" => {
                    Self::handle_reasoning_done(data, &mut tx, &mut state).await;
                }

                "response.function_call_arguments.delta" => {
                    Self::handle_function_call_delta(data, &mut tx, &mut state).await;
                }

                "response.function_call_arguments.done" => {
                    Self::handle_function_call_done(data, &mut tx, &mut state).await;
                }

                "response.output_item.done" => {
                    Self::handle_output_item_done(data, &mut tx, &mut state).await;
                }

                "response.content_part.done" | "response.reasoning_summary_part.done" => {
                    // Part already finalized by the specific .done event.
                }

                "response.completed" => {
                    Self::handle_response_completed(data, &mut tx, &mut state).await;
                }

                "response.incomplete" => {
                    Self::handle_response_incomplete(data, &mut tx, &mut state).await;
                }

                "response.failed" | "error" => {
                    Self::handle_error_event(data, &mut tx).await;
                    state.message_end_emitted = true;
                }

                _ => {
                    // Unknown event — ignore.
                }
            }

            // Reset the event type after processing data so a `data:`
            // line without a preceding `event:` line doesn't inherit
            // the previous event type.
            current_event.clear();
        }

        // Stream-end fallback: if we ended without a terminal event,
        // finalize all open parts and emit a synthetic MessageEnd.
        if !state.message_end_emitted {
            Self::finalize_open_parts(&mut tx, &mut state).await;
            let _ = tx
                .send(ProviderEvent::MessageEnd {
                    finish: Finish::Stop,
                    usage: Tokens::default(),
                    cost: 0.0,
                })
                .await;
        }
    }

    async fn handle_output_item_added(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item: ItemRef,
        }
        #[derive(serde::Deserialize)]
        struct ItemRef {
            id: String,
            #[serde(rename = "type")]
            typ: String,
            // Function call fields
            call_id: Option<String>,
            name: Option<String>,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if event.item.typ == "function_call" {
            let part = ToolCallPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: event.item.name.unwrap_or_default(),
                call_id: event.item.call_id.unwrap_or_default(),
                state: ToolState::Pending(ToolStatePending {
                    // Backends require function arguments to be a JSON object,
                    // even when no argument deltas ever arrive. Null here
                    // poisons the history and 400s on replay.
                    input: serde_json::json!({}),
                    time: ToolTime {
                        start: chrono::Utc::now().timestamp_millis(),
                        end: None,
                    },
                }),
                raw_input: String::new(),
            };
            let acc = ToolCallAccumulator {
                part: part.clone(),
                json: String::new(),
            };
            let _ = tx
                .send(ProviderEvent::PartStart {
                    part: Part::ToolCall(part),
                })
                .await;
            state.tool_calls.insert(event.item.id, acc);
        } else if event.item.typ == "reasoning" {
            // For responses-lite, create the reasoning part now so we can
            // attach encrypted_content from the later output_item.done event.
            // For non-lite, reasoning summary text arrives via
            // reasoning_summary_text events handled separately, and there's
            // no encrypted_content to capture.
            if state.use_responses_lite {
                let mut part = new_reasoning_part();
                // The API binds `encrypted_content` to the item id it issued,
                // and rejects a replay under a different id. Keep the id so it
                // can be echoed back verbatim.
                part.provider_item_id = Some(event.item.id.clone());
                state
                    .reasoning_part_ids
                    .insert(event.item.id.clone(), part.base.id);
                let _ = tx
                    .send(ProviderEvent::PartStart {
                        part: Part::Reasoning(part.clone()),
                    })
                    .await;
                state.reasoning_parts.insert(event.item.id, part);
            }
        }
        // Message items are handled when their content
        // parts arrive (content_part.added).
    }

    /// Handle `response.output_item.done` — primarily to capture
    /// `encrypted_content` on reasoning items. The Responses API delivers the
    /// full item (including encrypted reasoning) here, after the
    /// reasoning_summary_text sub-events have already finalized the PartEnd.
    /// We send the encrypted_content as a late PartDelta so the agent can
    /// store it on the ReasoningPart for round-tripping in subsequent turns.
    async fn handle_output_item_done(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item: ItemRef,
        }
        #[derive(serde::Deserialize)]
        struct ItemRef {
            id: String,
            #[serde(rename = "type")]
            typ: String,
            encrypted_content: Option<String>,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if event.item.typ == "reasoning" {
            if let (Some(encrypted), Some(&part_id)) = (
                event.item.encrypted_content,
                state.reasoning_part_ids.get(&event.item.id),
            ) {
                let _ = tx
                    .send(ProviderEvent::PartDelta {
                        part_id,
                        field: "encrypted_content",
                        delta: encrypted,
                    })
                    .await;
            }
        }
    }

    async fn handle_content_part_added(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item_id: String,
            part: PartRef,
        }
        #[derive(serde::Deserialize)]
        struct PartRef {
            #[serde(rename = "type")]
            typ: String,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        match event.part.typ.as_str() {
            "output_text" => {
                let part = new_text_part();
                let _ = tx
                    .send(ProviderEvent::PartStart {
                        part: Part::Text(part.clone()),
                    })
                    .await;
                state.text_parts.insert(event.item_id, part);
            }
            "reasoning_text" => {
                let part = new_reasoning_part();
                state
                    .reasoning_part_ids
                    .insert(event.item_id.clone(), part.base.id);
                let _ = tx
                    .send(ProviderEvent::PartStart {
                        part: Part::Reasoning(part.clone()),
                    })
                    .await;
                state.reasoning_parts.insert(event.item_id, part);
            }
            _ => {}
        }
    }

    async fn handle_output_text_delta(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item_id: String,
            delta: String,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Some(tp) = state.text_parts.get(&event.item_id) {
            let _ = tx
                .send(ProviderEvent::PartDelta {
                    part_id: tp.base.id,
                    field: "text",
                    delta: event.delta,
                })
                .await;
        }
    }

    async fn handle_output_text_done(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item_id: String,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Some(tp) = state.text_parts.remove(&event.item_id) {
            if !state.is_part_finalized(tp.base.id) {
                state.mark_finalized(tp.base.id);
                let _ = tx
                    .send(ProviderEvent::PartEnd {
                        part_id: tp.base.id,
                    })
                    .await;
            }
        }
    }

    async fn handle_reasoning_delta(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item_id: String,
            delta: String,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Some(rp) = state.reasoning_parts.get(&event.item_id) {
            let _ = tx
                .send(ProviderEvent::PartDelta {
                    part_id: rp.base.id,
                    field: "text",
                    delta: event.delta,
                })
                .await;
        }
    }

    async fn handle_reasoning_done(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item_id: String,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Some(rp) = state.reasoning_parts.remove(&event.item_id) {
            if !state.is_part_finalized(rp.base.id) {
                state.mark_finalized(rp.base.id);
                let _ = tx
                    .send(ProviderEvent::PartEnd {
                        part_id: rp.base.id,
                    })
                    .await;
            }
        }
    }

    async fn handle_function_call_delta(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item_id: String,
            delta: String,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Some(acc) = state.tool_calls.get_mut(&event.item_id) {
            acc.json.push_str(&event.delta);
            let _ = tx
                .send(ProviderEvent::PartDelta {
                    part_id: acc.part.base.id,
                    field: "arguments",
                    delta: event.delta,
                })
                .await;
        }
    }

    async fn handle_function_call_done(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        #[derive(serde::Deserialize)]
        struct Event {
            item_id: String,
            #[serde(default)]
            arguments: String,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Some(mut acc) = state.tool_calls.remove(&event.item_id) {
            // If the done event has arguments, use those; otherwise use
            // the accumulated buffer.
            if !event.arguments.is_empty() {
                acc.json = event.arguments;
            }
            acc.finalize();
            if !state.is_part_finalized(acc.part.base.id) {
                state.mark_finalized(acc.part.base.id);
                let _ = tx
                    .send(ProviderEvent::PartEnd {
                        part_id: acc.part.base.id,
                    })
                    .await;
            }
        }
    }

    async fn handle_response_completed(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        let v: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return,
        };

        let usage = &v["response"]["usage"];
        let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0) as u32;

        let _ = tx
            .send(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                usage: Tokens {
                    input: input_tokens,
                    output: output_tokens,
                    ..Default::default()
                },
                cost: 0.0,
            })
            .await;
        state.message_end_emitted = true;
    }

    async fn handle_response_incomplete(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        state: &mut StreamState,
    ) {
        let v: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return,
        };

        let reason = v["response"]["incomplete_details"]["reason"]
            .as_str()
            .unwrap_or("");
        let finish = if reason == "content_filter" {
            Finish::Error
        } else {
            Finish::Length
        };

        let usage = &v["response"]["usage"];
        let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0) as u32;

        let _ = tx
            .send(ProviderEvent::MessageEnd {
                finish,
                usage: Tokens {
                    input: input_tokens,
                    output: output_tokens,
                    ..Default::default()
                },
                cost: 0.0,
            })
            .await;
        state.message_end_emitted = true;
    }

    async fn handle_error_event(data: &str, tx: &mut mpsc::Sender<ProviderEvent>) {
        let v: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => {
                let _ = tx
                    .send(ProviderEvent::Error(MessageError {
                        kind: ErrorKind::ProviderApi,
                        message: "unknown responses API error".to_string(),
                    }))
                    .await;
                return;
            }
        };

        let message = v["error"]["message"]
            .as_str()
            .or_else(|| v["response"]["error"]["message"].as_str())
            .unwrap_or("unknown error")
            .to_string();

        let _ = tx
            .send(ProviderEvent::Error(MessageError {
                kind: ErrorKind::ProviderApi,
                message,
            }))
            .await;
    }

    async fn finalize_open_parts(tx: &mut mpsc::Sender<ProviderEvent>, state: &mut StreamState) {
        // Collect all PartIds that need finalization, then emit PartEnd.
        // We collect first to avoid borrow conflicts with the drain.
        let mut part_ids: Vec<ulid::Ulid> = Vec::new();

        // Finalize any text parts that got PartStart but no PartEnd.
        let text_parts: Vec<TextPart> = state.text_parts.drain().map(|(_, v)| v).collect();
        for tp in &text_parts {
            if !state.is_part_finalized(tp.base.id) {
                state.mark_finalized(tp.base.id);
                part_ids.push(tp.base.id);
            }
        }
        // Finalize any reasoning parts.
        let reasoning_parts: Vec<ReasoningPart> =
            state.reasoning_parts.drain().map(|(_, v)| v).collect();
        for rp in &reasoning_parts {
            if !state.is_part_finalized(rp.base.id) {
                state.mark_finalized(rp.base.id);
                part_ids.push(rp.base.id);
            }
        }
        // Finalize any tool calls.
        let mut tool_calls: Vec<ToolCallAccumulator> =
            state.tool_calls.drain().map(|(_, v)| v).collect();
        for acc in &mut tool_calls {
            acc.finalize();
            if !state.is_part_finalized(acc.part.base.id) {
                state.mark_finalized(acc.part.base.id);
                part_ids.push(acc.part.base.id);
            }
        }

        for id in part_ids {
            let _ = tx.send(ProviderEvent::PartEnd { part_id: id }).await;
        }
    }
}

fn new_text_part() -> TextPart {
    TextPart {
        base: PartBase {
            id: ulid::Ulid::new(),
            message_id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
        },
        text: String::new(),
        synthetic: false,
    }
}

fn new_reasoning_part() -> ReasoningPart {
    ReasoningPart {
        base: PartBase {
            id: ulid::Ulid::new(),
            message_id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
        },
        text: String::new(),
        signature: None,
        encrypted_content: None,
        provider_item_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use mew_message::Role;
    use mew_provider::{Provider, Request, ToolDef};
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_stream_text_only() {
        let server = MockServer::start().await;

        let sse_body = "\
event: response.created\n\
data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"status\":\"in_progress\"}}\n\
\n\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"status\":\"in_progress\"}}\n\
\n\
event: response.content_part.added\n\
data: {\"type\":\"response.content_part.added\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\"}}\n\
\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hello\"}\n\
\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"delta\":\" world\"}\n\
\n\
event: response.output_text.done\n\
data: {\"type\":\"response.output_text.done\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"text\":\"Hello world\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\
\n";

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let stream = adapter.stream(req).await.unwrap();
        let events: Vec<ProviderEvent> = stream.collect().await;

        // Expected: PartStart(Text) → PartDelta("Hello") → PartDelta(" world")
        //           → PartEnd → MessageEnd(Stop, usage)
        assert!(events.len() >= 5);
        assert!(matches!(
            events[0],
            ProviderEvent::PartStart {
                part: Part::Text(_)
            }
        ));
        assert!(matches!(
            events[1],
            ProviderEvent::PartDelta { delta: ref d, .. } if d == "Hello"
        ));
        assert!(matches!(
            events[2],
            ProviderEvent::PartDelta { delta: ref d, .. } if d == " world"
        ));
        assert!(matches!(events[3], ProviderEvent::PartEnd { .. }));
        assert!(matches!(
            events[4],
            ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ref usage,
                ..
            } if usage.input == 10 && usage.output == 5
        ));
    }

    #[tokio::test]
    async fn test_stream_responses_lite_header() {
        let server = MockServer::start().await;

        let sse_body = "\
event: response.created\n\
data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"status\":\"in_progress\"}}\n\
\n\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"status\":\"in_progress\"}}\n\
\n\
event: response.content_part.added\n\
data: {\"type\":\"response.content_part.added\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\"}}\n\
\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"ok\"}\n\
\n\
event: response.output_text.done\n\
data: {\"type\":\"response.output_text.done\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"text\":\"ok\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\
\n";

        Mock::given(method("POST"))
            .and(path("/responses"))
            .and(header("authorization", "Bearer test-access"))
            .and(header("x-openai-internal-codex-responses-lite", "true"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let provider = std::sync::Arc::new(TestOAuthProvider {
            base_url: server.uri(),
        });
        let adapter = Adapter::new_oauth(
            "test".into(),
            "gpt-5.6-luna".into(),
            mew_provider::auth::TokenSet {
                access_token: "test-access".into(),
                refresh_token: "test-refresh".into(),
                expires_at: 9_999_999_999,
            },
            vec![],
            provider,
        )
        .with_responses_lite(true);

        let req = Request {
            model: "gpt-5.6-luna".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let stream = adapter.stream(req).await.unwrap();
        let events: Vec<ProviderEvent> = stream.collect().await;
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::MessageEnd { .. })));
    }

    #[tokio::test]
    async fn test_stream_tool_call() {
        let server = MockServer::start().await;

        let sse_body = "\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"bash\",\"status\":\"in_progress\"}}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"output_index\":0,\"delta\":\"{\\\"command\\\":\\\"\"}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"output_index\":0,\"delta\":\"ls\\\"}\"}\n\
\n\
event: response.function_call_arguments.done\n\
data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_1\",\"output_index\":0,\"name\":\"bash\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_2\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\
\n";

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "run ls")],
            tools: vec![ToolDef {
                name: "bash".to_string(),
                description: "Run a command".to_string(),
                schema: json!({"type": "object"}),
            }],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),

            ..Default::default()
        };

        let stream = adapter.stream(req).await.unwrap();
        let events: Vec<ProviderEvent> = stream.collect().await;

        // Expected: PartStart(ToolCall) → PartDelta(args) → PartDelta(args)
        //           → PartEnd → MessageEnd
        assert!(events.len() >= 4);
        assert!(matches!(
            events[0],
            ProviderEvent::PartStart {
                part: Part::ToolCall(_)
            }
        ));
        assert!(matches!(events[1], ProviderEvent::PartDelta { .. }));
        assert!(matches!(events[2], ProviderEvent::PartDelta { .. }));
        assert!(matches!(events[3], ProviderEvent::PartEnd { .. }));
    }

    #[tokio::test]
    async fn test_stream_reasoning() {
        let server = MockServer::start().await;

        let sse_body = "\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_1\",\"type\":\"reasoning\",\"status\":\"in_progress\"}}\n\
\n\
event: response.reasoning_summary_part.added\n\
data: {\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_1\",\"output_index\":0,\"summary_index\":0,\"part\":{\"type\":\"summary_text\",\"text\":\"\"}}\n\
\n\
event: response.reasoning_summary_text.delta\n\
data: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_1\",\"output_index\":0,\"summary_index\":0,\"delta\":\"Thinking...\"}\n\
\n\
event: response.reasoning_summary_text.done\n\
data: {\"type\":\"response.reasoning_summary_text.done\",\"item_id\":\"rs_1\",\"output_index\":0,\"summary_index\":0,\"text\":\"Thinking...\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_3\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\
\n";

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let stream = adapter.stream(req).await.unwrap();
        let events: Vec<ProviderEvent> = stream.collect().await;

        // The reasoning summary delta is NOT handled by
        // content_part.added (that's only for output_text/reasoning_text).
        // The reasoning comes via reasoning_summary events, which the
        // adapter doesn't create a PartStart for (no content_part.added
        // with type "output_text" or "reasoning_text"). So we expect
        // only MessageEnd from response.completed.
        //
        // NOTE: This test documents the current behavior — reasoning
        // summaries are not surfaced as Part::Reasoning because they
        // arrive via reasoning_summary_text events, not content_part.added.
        // The output_item.added for type="reasoning" is a no-op in our
        // handler. This is acceptable for Phase 1 — reasoning summaries
        // are a nice-to-have display feature, not critical for the agent
        // loop.
        let has_msg_end = events.iter().any(|e| {
            matches!(
                e,
                ProviderEvent::MessageEnd {
                    finish: Finish::Stop,
                    ..
                }
            )
        });
        assert!(has_msg_end);
    }

    #[tokio::test]
    async fn test_stream_reasoning_encrypted_content_lite() {
        // Responses Lite: reasoning items arrive via output_item.added (type
        // "reasoning") followed by reasoning_summary_text deltas, then
        // output_item.done carries the encrypted_content. We should emit
        // PartStart → PartDelta(text) → PartEnd → PartDelta(encrypted_content).
        let server = MockServer::start().await;

        let sse_body = "\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_1\",\"type\":\"reasoning\",\"status\":\"in_progress\"}}\n\
\n\
event: response.reasoning_summary_text.delta\n\
data: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_1\",\"output_index\":0,\"summary_index\":0,\"delta\":\"Thinking...\"}\n\
\n\
event: response.reasoning_summary_text.done\n\
data: {\"type\":\"response.reasoning_summary_text.done\",\"item_id\":\"rs_1\",\"output_index\":0,\"summary_index\":0,\"text\":\"Thinking...\"}\n\
\n\
event: response.output_item.done\n\
data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_1\",\"type\":\"reasoning\",\"encrypted_content\":\"ENC_BLOB_123\"}}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_4\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\
\n";

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5.6-luna".to_string(),
            "test-key".to_string(),
        )
        .with_responses_lite(true);

        let req = Request {
            model: "gpt-5.6-luna".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let stream = adapter.stream(req).await.unwrap();
        let events: Vec<ProviderEvent> = stream.collect().await;

        // Should have PartStart(Reasoning)
        assert!(
            events.iter().any(|e| matches!(
                e,
                ProviderEvent::PartStart {
                    part: Part::Reasoning(_),
                }
            )),
            "expected PartStart for reasoning"
        );

        // Should have PartDelta with encrypted_content
        let has_encrypted = events.iter().any(|e| {
            matches!(
                e,
                ProviderEvent::PartDelta {
                    field: "encrypted_content",
                    delta: ref d,
                    ..
                } if d == "ENC_BLOB_123"
            )
        });
        assert!(has_encrypted, "expected PartDelta with encrypted_content");

        // Should have PartEnd for the reasoning part
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ProviderEvent::PartEnd { .. })),
            "expected PartEnd"
        );

        // Should have MessageEnd
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ProviderEvent::MessageEnd { .. })),
            "expected MessageEnd"
        );
    }

    #[tokio::test]
    async fn test_stream_error() {
        let server = MockServer::start().await;

        let sse_body = "\
event: response.failed\n\
data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_err\",\"error\":{\"code\":\"server_error\",\"message\":\"something went wrong\"}}}\n\
\n";

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let stream = adapter.stream(req).await.unwrap();
        let events: Vec<ProviderEvent> = stream.collect().await;

        assert!(events.iter().any(|e| {
            matches!(
                e,
                ProviderEvent::Error(mew_message::MessageError {
                    kind: ErrorKind::ProviderApi,
                    ..
                })
            )
        }));
    }

    #[tokio::test]
    async fn test_stream_usage() {
        let server = MockServer::start().await;

        let sse_body = "\
event: response.created\n\
data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_u\",\"status\":\"in_progress\"}}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_u\",\"status\":\"completed\",\"usage\":{\"input_tokens\":42,\"output_tokens\":17}}}\n\
\n";

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let stream = adapter.stream(req).await.unwrap();
        let events: Vec<ProviderEvent> = stream.collect().await;

        let usage_event = events
            .iter()
            .find(|e| matches!(e, ProviderEvent::MessageEnd { .. }));
        assert!(usage_event.is_some());
        if let Some(ProviderEvent::MessageEnd { usage, .. }) = usage_event {
            assert_eq!(usage.input, 42);
            assert_eq!(usage.output, 17);
        }
    }
}
