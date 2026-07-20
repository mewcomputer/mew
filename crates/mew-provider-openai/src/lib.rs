use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{channel::mpsc, SinkExt, StreamExt};
use mew_message::{
    ErrorKind, Finish, Message, MessageError, Part, PartBase, ReasoningPart, Role, TextPart,
    Tokens, ToolCallPart, ToolState, ToolStatePending, ToolTime,
};
use mew_provider::{
    classify_error, classify_reason, EventStream, Provider, ProviderError, ProviderEvent, Request,
    RetryPolicy,
};
use serde_json::json;
use std::collections::HashMap;

pub struct Adapter {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    client: reqwest::Client,
    dump: bool,
}

impl Adapter {
    pub fn new(name: String, base_url: String, model: String, api_key: String) -> Self {
        Self {
            name,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            client: reqwest::Client::new(),
            dump: false,
        }
    }

    pub fn set_dump(&mut self, v: bool) {
        self.dump = v;
    }
}

#[async_trait]
impl Provider for Adapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError> {
        let body = self.build_request_body(&req).await?;

        if self.dump {
            if let Ok(pretty) = serde_json::from_slice::<serde_json::Value>(&body)
                .and_then(|v| serde_json::to_string_pretty(&v))
            {
                eprintln!("\n[RAW REQUEST BODY]\n{pretty}\n");
            } else {
                eprintln!("\n[RAW REQUEST BODY]\n{}\n", String::from_utf8_lossy(&body));
            }
        }

        let url = format!("{}/chat/completions", self.base_url);
        let request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "text/event-stream")
            .body(body)
            .build()?;

        let policy = RetryPolicy::default();
        let (tx, rx) = mpsc::channel(128);
        let mut retry_tx = tx.clone();
        let mut resp = None;

        for attempt in 0.. {
            let req = request.try_clone().ok_or_else(|| {
                ProviderError::Message("request cannot be cloned for retry".to_string())
            })?;
            let r = self.client.execute(req).await?;
            if r.status().is_success() {
                resp = Some(r);
                break;
            }
            let status = r.status().as_u16();
            let data = r.text().await.unwrap_or_default();
            let (backoff, retry) = policy.should_retry(status, attempt);
            if !retry {
                let (kind, msg) = classify_error(status, &data);
                let _ = retry_tx
                    .send(ProviderEvent::Error(mew_message::MessageError {
                        kind,
                        message: msg.clone(),
                    }))
                    .await;
                return Err(ProviderError::Classified { kind, message: msg });
            }
            let _ = retry_tx
                .send(ProviderEvent::RetryWait {
                    attempt: attempt as u32 + 1,
                    max_attempts: 4,
                    delay_secs: backoff.as_secs(),
                    reason: classify_reason(status),
                })
                .await;
            tokio::time::sleep(backoff).await;
        }

        drop(retry_tx);
        let resp = resp.unwrap();
        let dump = self.dump;
        tokio::spawn(async move {
            Self::read_stream(dump, resp, tx).await;
        });

        Ok(Box::pin(rx))
    }

    async fn list_models(&self) -> Result<Vec<mew_provider::ModelInfo>, ProviderError> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let (kind, msg) = mew_provider::classify_error(status, &body);
            return Err(ProviderError::Classified { kind, message: msg });
        }

        #[derive(serde::Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        #[derive(serde::Deserialize)]
        struct ModelEntry {
            id: String,
            #[serde(default)]
            owned_by: String,
        }

        let payload: ModelsResponse = resp.json().await?;
        Ok(payload
            .data
            .into_iter()
            .map(|m| mew_provider::ModelInfo {
                id: m.id,
                owned_by: m.owned_by,
            })
            .collect())
    }
}

impl Adapter {
    fn find_tool_output(messages: &[Message], call_id: &str) -> String {
        for m in messages {
            for p in &m.parts {
                if let Part::ToolCall(tc) = p {
                    if tc.call_id == call_id {
                        return tc.state.output().unwrap_or("").to_string();
                    }
                }
            }
        }
        String::new()
    }

    async fn build_request_body(&self, req: &Request) -> Result<Vec<u8>, ProviderError> {
        let mut messages: Vec<serde_json::Value> = Vec::new();
        for m in &req.messages {
            let msgs = self.build_wire_message(&req.messages, m).await;
            messages.extend(msgs);
        }

        if !req.system.is_empty() {
            messages.insert(0, json!({"role": "system", "content": req.system}));
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });

        if !req.tools.is_empty() {
            let tools: Vec<serde_json::Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.schema,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(tools);
        }

        if let Some(ref reasoning) = req.reasoning {
            if let Some(body_obj) = body.as_object_mut() {
                for (k, v) in &reasoning.params {
                    body_obj.insert(k.clone(), v.clone());
                }
            }
        }

        // Sampling params from the `on_chat_params` hook. Each field is
        // optional; only set keys are added so the provider defaults
        // (which the model was trained against) win when the plugin
        // doesn't override.
        if let Some(ref params) = req.params {
            if let Some(body_obj) = body.as_object_mut() {
                if let Some(t) = params.temperature {
                    body_obj.insert("temperature".into(), json!(t));
                }
                if let Some(p) = params.top_p {
                    body_obj.insert("top_p".into(), json!(p));
                }
                if let Some(m) = params.max_tokens {
                    body_obj.insert("max_tokens".into(), json!(m));
                }
                if let Some(tc) = params.tool_choice {
                    let v = match tc {
                        mew_provider::ToolChoice::Auto => json!("auto"),
                        mew_provider::ToolChoice::Required => json!("required"),
                        mew_provider::ToolChoice::None_ => json!("none"),
                    };
                    body_obj.insert("tool_choice".into(), v);
                }
            }
        }

        serde_json::to_vec(&body).map_err(ProviderError::Json)
    }

    async fn build_wire_message(&self, all: &[Message], m: &Message) -> Vec<serde_json::Value> {
        let mut out: Vec<serde_json::Value> = Vec::new();

        match m.role {
            Role::System => {
                let text = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Text(pt) => Some(pt.text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                if !text.is_empty() {
                    out.push(json!({"role": "system", "content": text}));
                }
            }
            Role::User => {
                let mut text_content = String::new();
                let mut image_blocks: Vec<serde_json::Value> = Vec::new();
                let mut tool_results: Vec<serde_json::Value> = Vec::new();

                for p in &m.parts {
                    match p {
                        Part::Text(pt) => {
                            text_content.push_str(&pt.text);
                        }
                        Part::File(pt) => {
                            if pt.mime.starts_with("image/") {
                                if let Ok((mime, b64)) =
                                    mew_provider::imageutil::resolve(&pt.url).await
                                {
                                    image_blocks.push(json!({
                                        "type": "image_url",
                                        "image_url": {
                                            "url": format!("data:{};base64,{}", mime, b64),
                                        }
                                    }));
                                }
                            } else {
                                let filename = pt.filename.as_deref().unwrap_or("unnamed");
                                text_content.push_str(&format!("\n[File: {}]", filename));
                            }
                        }
                        Part::ToolResult(pt) => {
                            let output = Self::find_tool_output(all, &pt.call_id);
                            tool_results.push(json!({
                                "role": "tool",
                                "content": output,
                                "tool_call_id": pt.call_id,
                            }));
                        }
                        _ => {}
                    }
                }

                if !image_blocks.is_empty() {
                    let mut content: Vec<serde_json::Value> = Vec::new();
                    if !text_content.is_empty() {
                        content.push(json!({"type": "text", "text": text_content}));
                    }
                    content.extend(image_blocks);
                    out.push(json!({"role": "user", "content": content}));
                } else if !text_content.is_empty() {
                    out.push(json!({"role": "user", "content": text_content}));
                }
                out.extend(tool_results);
            }
            Role::Assistant => {
                let mut content = String::new();
                let mut reasoning = String::new();
                let mut tool_calls: Vec<serde_json::Value> = Vec::new();

                for p in &m.parts {
                    match p {
                        Part::Text(pt) => {
                            content.push_str(&pt.text);
                        }
                        Part::Reasoning(pt) => {
                            reasoning.push_str(&pt.text);
                        }
                        Part::ToolCall(pt) => {
                            // Backends reject non-object arguments (sessions
                            // persisted before object-input was enforced can
                            // still carry Null, which stringifies to "null").
                            let input = pt.state.input();
                            let arguments = if input.is_object() {
                                input.to_string()
                            } else {
                                "{}".to_string()
                            };
                            tool_calls.push(json!({
                                "id": pt.call_id,
                                "type": "function",
                                "function": {
                                    "name": pt.tool_name,
                                    "arguments": arguments,
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let mut msg = serde_json::Map::new();
                msg.insert("role".to_string(), json!("assistant"));
                if content.is_empty() {
                    msg.insert("content".to_string(), serde_json::Value::Null);
                } else {
                    msg.insert("content".to_string(), json!(content));
                }
                msg.insert("reasoning".to_string(), json!(reasoning));
                msg.insert("reasoning_content".to_string(), json!(reasoning));
                if !tool_calls.is_empty() {
                    msg.insert("tool_calls".to_string(), json!(tool_calls));
                }
                out.push(serde_json::Value::Object(msg));
            }
        }

        out
    }

    async fn read_stream(dump: bool, resp: reqwest::Response, mut tx: mpsc::Sender<ProviderEvent>) {
        let mut stream = resp.bytes_stream().eventsource();

        let mut current_text_part: Option<TextPart> = None;
        let mut current_reasoning_part: Option<ReasoningPart> = None;
        let mut current_tool_calls: HashMap<u32, ToolCallAccumulator> = HashMap::new();

        while let Some(item) = stream.next().await {
            let event = match item {
                Ok(e) => e,
                Err(e) => {
                    let _ = tx
                        .send(ProviderEvent::Error(MessageError {
                            kind: ErrorKind::Network,
                            message: format!("sse stream error: {e}"),
                        }))
                        .await;
                    break;
                }
            };

            if dump {
                tracing::debug!("[RAW SSE] {}", event.data);
            }

            if event.data == "[DONE]" {
                Self::finalize_all(
                    &mut current_text_part,
                    &mut current_reasoning_part,
                    &mut current_tool_calls,
                    &mut tx,
                    Finish::Stop,
                )
                .await;
                return;
            }

            let chunk: CompletionChunk = match serde_json::from_str(&event.data) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx
                        .send(ProviderEvent::Error(MessageError {
                            kind: ErrorKind::ProviderApi,
                            message: format!("unmarshal chunk: {e}"),
                        }))
                        .await;
                    break;
                }
            };

            if chunk.choices.is_empty() {
                continue;
            }

            let delta = &chunk.choices[0].delta;

            // Text and reasoning parts are created lazily by their respective
            // content handlers below. We do NOT eagerly create a text part
            // on the {"role":"assistant"} chunk, because reasoning may arrive
            // in this delta or the next — if we create the text part first,
            // it renders above reasoning instead of below.

            if let Some(content) = &delta.content {
                if !content.is_empty() {
                    if let Some(rp) = current_reasoning_part.take() {
                        let _ = tx
                            .send(ProviderEvent::PartEnd {
                                part_id: rp.base.id,
                            })
                            .await;
                    }
                    if current_text_part.is_none() {
                        let part = new_text_part();
                        let _ = tx
                            .send(ProviderEvent::PartStart {
                                part: Part::Text(part.clone()),
                            })
                            .await;
                        current_text_part = Some(part);
                    }
                    if let Some(tp) = &current_text_part {
                        let _ = tx
                            .send(ProviderEvent::PartDelta {
                                part_id: tp.base.id,
                                field: "text",
                                delta: content.clone(),
                            })
                            .await;
                    }
                }
            }

            if let Some(reasoning) = &delta.reasoning {
                if !reasoning.is_empty() {
                    if current_reasoning_part.is_none() {
                        let part = new_reasoning_part();
                        let _ = tx
                            .send(ProviderEvent::PartStart {
                                part: Part::Reasoning(part.clone()),
                            })
                            .await;
                        current_reasoning_part = Some(part);
                    }
                    if let Some(rp) = &current_reasoning_part {
                        let _ = tx
                            .send(ProviderEvent::PartDelta {
                                part_id: rp.base.id,
                                field: "text",
                                delta: reasoning.clone(),
                            })
                            .await;
                    }
                }
            }

            if let Some(tool_calls) = &delta.tool_calls {
                if let Some(rp) = current_reasoning_part.take() {
                    let _ = tx
                        .send(ProviderEvent::PartEnd {
                            part_id: rp.base.id,
                        })
                        .await;
                }
                for tc_delta in tool_calls {
                    let idx = tc_delta.index;

                    if let std::collections::hash_map::Entry::Vacant(e) =
                        current_tool_calls.entry(idx)
                    {
                        let part = new_tool_call_part();
                        let acc = ToolCallAccumulator {
                            part: part.clone(),
                            id: String::new(),
                            typ: String::new(),
                            name: String::new(),
                            arguments: String::new(),
                        };
                        let _ = tx
                            .send(ProviderEvent::PartStart {
                                part: Part::ToolCall(part),
                            })
                            .await;
                        e.insert(acc);
                    }

                    let acc = current_tool_calls.get_mut(&idx).unwrap();
                    if let Some(id) = &tc_delta.id {
                        acc.id.clone_from(id);
                        let _ = tx
                            .send(ProviderEvent::PartDelta {
                                part_id: acc.part.base.id,
                                field: "call_id",
                                delta: id.clone(),
                            })
                            .await;
                    }
                    if let Some(typ) = &tc_delta.typ {
                        acc.typ.clone_from(typ);
                    }
                    if let Some(function) = &tc_delta.function {
                        if let Some(name) = &function.name {
                            acc.name.clone_from(name);
                            let _ = tx
                                .send(ProviderEvent::PartDelta {
                                    part_id: acc.part.base.id,
                                    field: "tool_name",
                                    delta: name.clone(),
                                })
                                .await;
                        }
                        if let Some(arguments) = &function.arguments {
                            acc.arguments.push_str(arguments);
                            let _ = tx
                                .send(ProviderEvent::PartDelta {
                                    part_id: acc.part.base.id,
                                    field: "arguments",
                                    delta: arguments.clone(),
                                })
                                .await;
                        }
                    }
                }
            }

            if let Some(finish_reason) = &chunk.choices[0].finish_reason {
                let finish = map_finish_reason(finish_reason);
                Self::finalize_all(
                    &mut current_text_part,
                    &mut current_reasoning_part,
                    &mut current_tool_calls,
                    &mut tx,
                    finish,
                )
                .await;
                return;
            }
        }

        // Stream ended without [DONE] or finish_reason.
        Self::finalize_all(
            &mut current_text_part,
            &mut current_reasoning_part,
            &mut current_tool_calls,
            &mut tx,
            Finish::Stop,
        )
        .await;
    }

    async fn finalize_all(
        current_text_part: &mut Option<TextPart>,
        current_reasoning_part: &mut Option<ReasoningPart>,
        current_tool_calls: &mut HashMap<u32, ToolCallAccumulator>,
        tx: &mut mpsc::Sender<ProviderEvent>,
        finish: Finish,
    ) {
        if let Some(tp) = current_text_part.take() {
            let _ = tx
                .send(ProviderEvent::PartEnd {
                    part_id: tp.base.id,
                })
                .await;
        }
        if let Some(rp) = current_reasoning_part.take() {
            let _ = tx
                .send(ProviderEvent::PartEnd {
                    part_id: rp.base.id,
                })
                .await;
        }
        for (_, mut acc) in std::mem::take(current_tool_calls) {
            acc.finalize();
            let _ = tx
                .send(ProviderEvent::PartEnd {
                    part_id: acc.part.base.id,
                })
                .await;
        }
        let _ = tx
            .send(ProviderEvent::MessageEnd {
                finish,
                usage: Tokens::default(),
                cost: 0.0,
            })
            .await;
    }
}

#[derive(Debug, serde::Deserialize)]
struct CompletionChunk {
    choices: Vec<Choice>,
}

#[derive(Debug, serde::Deserialize)]
struct Choice {
    delta: Delta,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Delta {
    #[allow(dead_code)]
    role: Option<String>,
    content: Option<String>,
    #[serde(alias = "reasoning_content")]
    reasoning: Option<String>,
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, serde::Deserialize)]
struct ToolCallDelta {
    index: u32,
    id: Option<String>,
    #[serde(rename = "type")]
    typ: Option<String>,
    function: Option<FunctionDelta>,
}

#[derive(Debug, serde::Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug)]
struct ToolCallAccumulator {
    part: ToolCallPart,
    id: String,
    typ: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn finalize(&mut self) {
        self.part.tool_name.clone_from(&self.name);
        self.part.call_id.clone_from(&self.id);
        if !self.arguments.is_empty() {
            if let Ok(input) = serde_json::from_str(&self.arguments) {
                self.part.state = ToolState::Pending(ToolStatePending {
                    input,
                    time: ToolTime {
                        start: chrono::Utc::now().timestamp_millis(),
                        end: None,
                    },
                });
            }
        }
    }
}

fn map_finish_reason(reason: &str) -> Finish {
    match reason {
        "stop" => Finish::Stop,
        "length" => Finish::Length,
        "tool_calls" => Finish::ToolUse,
        "content_filter" => Finish::Error,
        _ => Finish::Error,
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
    }
}

fn new_tool_call_part() -> ToolCallPart {
    ToolCallPart {
        base: PartBase {
            id: ulid::Ulid::new(),
            message_id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
        },
        tool_name: String::new(),
        call_id: String::new(),
        state: ToolState::Pending(ToolStatePending {
            // Backends require function.arguments to be a JSON object, even
            // when no argument deltas ever arrive. Null here poisons the
            // history and 400s on replay.
            input: serde_json::json!({}),
            time: ToolTime {
                start: chrono::Utc::now().timestamp_millis(),
                end: None,
            },
        }),
        raw_input: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::AssistantMeta;

    fn make_minimal_png() -> Vec<u8> {
        vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01, 0xF6, 0x17, 0xA4,
            0x49, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ]
    }

    #[tokio::test]
    async fn test_build_wire_message_user() {
        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "model".to_string(),
            "key".to_string(),
        );
        let msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::User,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                text: "Hello".to_string(),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        let wire = adapter.build_wire_message(&[], &msg).await;
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[0]["content"], "Hello");
    }

    #[tokio::test]
    async fn test_build_wire_message_assistant_with_tool() {
        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "model".to_string(),
            "key".to_string(),
        );
        let msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![
                Part::Text(TextPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: "Calling tool".to_string(),
                    synthetic: false,
                }),
                Part::ToolCall(ToolCallPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    tool_name: "echo".to_string(),
                    call_id: "call_123".to_string(),
                    state: ToolState::Pending(ToolStatePending {
                        input: serde_json::json!({"input": "hi"}),
                        time: ToolTime {
                            start: 0,
                            end: None,
                        },
                    }),
                    raw_input: String::new(),
                }),
            ],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: Some(AssistantMeta {
                provider_id: String::new(),
                model_id: String::new(),
                cost: 0.0,
                tokens: Tokens::default(),
                finish: None,
                error: None,
                manifest: None,
            }),
        };
        let wire = adapter.build_wire_message(&[], &msg).await;
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "assistant");
        assert!(wire[0]["tool_calls"].is_array());
        assert_eq!(wire[0]["tool_calls"][0]["id"], "call_123");
        assert_eq!(wire[0]["tool_calls"][0]["function"]["name"], "echo");
    }

    #[tokio::test]
    async fn test_build_wire_message_null_tool_input_becomes_object() {
        // A tool call whose arguments never streamed carried `input: Null`.
        // Replaying that as `function.arguments = "null"` is rejected by
        // backends ("arguments must be a JSON object"). It must be "{}".
        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "model".to_string(),
            "key".to_string(),
        );
        let msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: "write".to_string(),
                call_id: "call_null".to_string(),
                state: ToolState::Pending(ToolStatePending {
                    input: serde_json::Value::Null,
                    time: ToolTime {
                        start: 0,
                        end: None,
                    },
                }),
                raw_input: String::new(),
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: Some(AssistantMeta {
                provider_id: String::new(),
                model_id: String::new(),
                cost: 0.0,
                tokens: Tokens::default(),
                finish: None,
                error: None,
                manifest: None,
            }),
        };
        let wire = adapter.build_wire_message(&[], &msg).await;
        assert_eq!(wire.len(), 1);
        assert_eq!(
            wire[0]["tool_calls"][0]["function"]["arguments"], "{}",
            "null tool input must serialize as \"{{}}\""
        );
    }

    #[tokio::test]
    async fn test_build_wire_message_user_with_image() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test.png");
        tokio::fs::write(&img_path, make_minimal_png())
            .await
            .unwrap();
        let img_url = format!("file://{}", img_path.display());

        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "model".to_string(),
            "key".to_string(),
        );
        let msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::User,
            parts: vec![
                Part::Text(TextPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: "Describe this image".to_string(),
                    synthetic: false,
                }),
                Part::File(mew_message::FilePart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    mime: "image/png".to_string(),
                    filename: Some("test.png".to_string()),
                    url: img_url,
                }),
            ],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        let wire = adapter.build_wire_message(&[], &msg).await;
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
        let content = wire[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Describe this image");
        assert_eq!(content[1]["type"], "image_url");
        let data_url = content[1]["image_url"]["url"].as_str().unwrap();
        assert!(data_url.starts_with("data:image/png;base64,"));
        assert!(data_url.len() > "data:image/png;base64,".len());
    }

    // -----------------------------------------------------------------------
    // SSE fixture replay tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_fixture_text_only() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture =
            std::fs::read_to_string("src/testdata/text-only.sse").expect("read text-only fixture");

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "text/event-stream"))
            .mount(&mock_server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            mock_server.uri(),
            "test-model".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "test-model".into(),
            messages: vec![],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            ..Default::default()
        };

        let mut stream = adapter.stream(req).await.expect("stream");
        let mut events: Vec<ProviderEvent> = Vec::new();
        while let Some(ev) = futures::StreamExt::next(&mut stream).await {
            events.push(ev);
        }

        // Should have PartStart(Text), PartDelta(text) x3, PartEnd, MessageEnd
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::PartStart { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::PartDelta { field: "text", .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::PartEnd { .. })));
        assert!(events.iter().any(|e| matches!(
            e,
            ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn test_fixture_reasoning_content_alias() {
        // Kimi K3 and DeepSeek emit reasoning under `reasoning_content` in the
        // streaming delta (not `reasoning`). The Delta struct aliases both
        // field names so reasoning is captured regardless of which the provider
        // uses. This fixture uses `reasoning_content` exclusively.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture = std::fs::read_to_string("src/testdata/reasoning-content.sse")
            .expect("read reasoning-content fixture");

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "text/event-stream"))
            .mount(&mock_server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            mock_server.uri(),
            "k3".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "k3".into(),
            messages: vec![],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            ..Default::default()
        };

        let mut stream = adapter.stream(req).await.expect("stream");
        let mut events: Vec<ProviderEvent> = Vec::new();
        while let Some(ev) = futures::StreamExt::next(&mut stream).await {
            events.push(ev);
        }

        // Reasoning should be captured from reasoning_content deltas.
        // PartStart(Reasoning) proves the reasoning_content field was
        // deserialized and a reasoning part was opened.
        let has_reasoning_start = events.iter().any(|e| {
            matches!(e, ProviderEvent::PartStart { part: Part::Reasoning(_) })
        });
        assert!(
            has_reasoning_start,
            "expected reasoning PartStart from reasoning_content deltas; events: {:?}",
            events.iter().map(|e| format!("{:?}", e)).collect::<Vec<_>>()
        );

        // The reasoning part must be closed with a PartEnd, and a text part
        // must also appear (the fixture has both reasoning_content and content).
        let part_end_count = events
            .iter()
            .filter(|e| matches!(e, ProviderEvent::PartEnd { .. }))
            .count();
        assert!(
            part_end_count >= 2,
            "expected at least 2 PartEnd events (reasoning + text); got {}",
            part_end_count
        );

        let has_text_start = events.iter().any(|e| {
            matches!(e, ProviderEvent::PartStart { part: Part::Text(_) })
        });
        assert!(
            has_text_start,
            "expected text PartStart from content deltas"
        );
        assert!(events.iter().any(|e| matches!(
            e,
            ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn test_fixture_tool_call() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture =
            std::fs::read_to_string("src/testdata/tool-call.sse").expect("read tool-call fixture");

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "text/event-stream"))
            .mount(&mock_server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            mock_server.uri(),
            "test-model".to_string(),
            "test-key".to_string(),
        );

        let req = Request {
            model: "test-model".into(),
            messages: vec![],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            ..Default::default()
        };

        let mut stream = adapter.stream(req).await.expect("stream");
        let mut events: Vec<ProviderEvent> = Vec::new();
        while let Some(ev) = futures::StreamExt::next(&mut stream).await {
            events.push(ev);
        }

        println!("events: {events:?}");

        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::PartStart { .. })));
        assert!(events.iter().any(|e| matches!(
            e,
            ProviderEvent::PartDelta {
                field: "arguments",
                ..
            }
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            ProviderEvent::PartDelta {
                field: "call_id",
                ..
            }
        )));
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::MessageEnd { .. })));
    }
}
