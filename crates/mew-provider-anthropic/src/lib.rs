use async_trait::async_trait;
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
use tokio::io::AsyncBufReadExt;

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

        let url = format!("{}/messages", self.base_url);
        let request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-API-Key", &self.api_key)
            .header("Anthropic-Version", "2023-06-01")
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
            .header("X-API-Key", &self.api_key)
            .header("Anthropic-Version", "2023-06-01")
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
            #[serde(rename = "display_name")]
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
            if let Some(msg) = self.build_wire_message(&req.messages, m).await {
                messages.push(msg);
            }
        }

        // Baseline `max_tokens` from the request if the dispatcher
        // provided one, otherwise default to 4096. Anthropic requires
        // `max_tokens >= 1`; we honour `Some(0)` as a request-level
        // override (the thinking-budget bump below will rescue it
        // when thinking is on) and fall back to 4096 only when
        // thinking is off — the API would reject 0 otherwise.
        let max_tokens = req
            .params
            .as_ref()
            .and_then(|p| p.max_tokens)
            .map(|v| v as i64)
            .unwrap_or(4096);

        let mut body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": messages,
            "stream": true,
        });

        if let Some(ref reasoning) = req.reasoning {
            if let Some(body_obj) = body.as_object_mut() {
                for (k, v) in &reasoning.params {
                    body_obj.insert(k.clone(), v.clone());
                }
            }
            // If thinking was enabled with a budget, ensure max_tokens is sufficient.
            if let Some(budget) = body
                .get("thinking")
                .and_then(|t| t.get("budget_tokens"))
                .and_then(|b| b.as_u64())
            {
                let min_max = budget + 4096;
                let current = body["max_tokens"].as_u64().unwrap_or(4096);
                if current < min_max {
                    body["max_tokens"] = json!(min_max);
                }
            }
        }

        // If max_tokens ended up at 0 (dispatcher dispatched Some(0)
        // and thinking wasn't on), the API would reject the request.
        // The thinking-budget bump above only fires when reasoning is
        // configured; otherwise we have to floor explicitly.
        if body["max_tokens"].as_i64() == Some(0) {
            body["max_tokens"] = json!(4096);
        }

        if !req.system.is_empty() {
            body["system"] = json!(req.system);
        }

        if !req.tools.is_empty() {
            let tools: Vec<serde_json::Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.schema,
                    })
                })
                .collect();
            body["tools"] = json!(tools);
        }

        // `tool_choice` is an Anthropic top-level field. Values:
        //   "auto" → model decides, "any" → must call a tool,
        //   "tool" → must call a specific named tool, "none" → no tools.
        // We map our 3-variant enum onto "auto"/"any"/"none".
        if let Some(ref params) = req.params {
            if let Some(tc) = params.tool_choice {
                let v = match tc {
                    mew_provider::ToolChoice::Auto => json!("auto"),
                    mew_provider::ToolChoice::Required => json!("any"),
                    mew_provider::ToolChoice::None_ => json!("none"),
                };
                body["tool_choice"] = v;
            }
        }

        serde_json::to_vec(&body).map_err(ProviderError::Json)
    }

    async fn build_wire_message(&self, all: &[Message], m: &Message) -> Option<serde_json::Value> {
        let mut content: Vec<serde_json::Value> = Vec::new();

        match m.role {
            Role::User => {
                for p in &m.parts {
                    match p {
                        Part::Text(pt) => {
                            content.push(json!({"type": "text", "text": pt.text}));
                        }
                        Part::ToolResult(pt) => {
                            let output = Self::find_tool_output(all, &pt.call_id);
                            content.push(json!({
                                "type": "tool_result",
                                "tool_use_id": pt.call_id,
                                "content": output,
                            }));
                        }
                        Part::File(pt) => {
                            if pt.mime.starts_with("image/") {
                                let b64 = self.read_image_data(&pt.url).await;
                                content.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": pt.mime,
                                        "data": b64,
                                    }
                                }));
                            } else {
                                let filename = pt.filename.as_deref().unwrap_or("unnamed");
                                content.push(json!({
                                    "type": "text",
                                    "text": format!("[File: {}]", filename),
                                }));
                            }
                        }
                        _ => {}
                    }
                }
                if content.is_empty() {
                    return None;
                }
                Some(json!({"role": "user", "content": content}))
            }
            Role::Assistant => {
                for p in &m.parts {
                    match p {
                        Part::Text(pt) => {
                            content.push(json!({"type": "text", "text": pt.text}));
                        }
                        Part::Reasoning(pt) => {
                            let mut block = serde_json::Map::new();
                            block.insert("type".to_string(), json!("thinking"));
                            block.insert("thinking".to_string(), json!(pt.text));
                            if let Some(sig) = &pt.signature {
                                block.insert("signature".to_string(), json!(sig));
                            }
                            content.push(serde_json::Value::Object(block));
                        }
                        Part::ToolCall(pt) => {
                            content.push(json!({
                                "type": "tool_use",
                                "id": pt.call_id,
                                "name": pt.tool_name,
                                "input": pt.state.input(),
                            }));
                        }
                        _ => {}
                    }
                }
                if content.is_empty() {
                    return None;
                }
                Some(json!({"role": "assistant", "content": content}))
            }
        }
    }

    async fn read_image_data(&self, url: &str) -> String {
        match mew_provider::imageutil::resolve(url).await {
            Ok((_, b64)) => b64,
            Err(_) => String::new(),
        }
    }

    async fn read_stream(dump: bool, resp: reqwest::Response, mut tx: mpsc::Sender<ProviderEvent>) {
        let stream = resp
            .bytes_stream()
            .map(|res| res.map_err(std::io::Error::other));
        let reader = tokio::io::BufReader::new(tokio_util::io::StreamReader::new(stream));
        let mut lines = reader.lines();

        let mut current_event = String::new();
        let mut current_text_part: Option<TextPart> = None;
        let mut current_reasoning_part: Option<ReasoningPart> = None;
        let mut current_tool_call: Option<ToolCallAccumulator> = None;
        let mut message_end_emitted = false;

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
                tracing::debug!("[RAW SSE] {}", line);
            }

            if let Some(ev) = line.strip_prefix("event: ") {
                current_event = ev.to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };

            match current_event.as_str() {
                "message_start" => {}
                "content_block_start" => {
                    Self::handle_content_block_start(
                        data,
                        &mut tx,
                        &mut current_text_part,
                        &mut current_reasoning_part,
                        &mut current_tool_call,
                    )
                    .await;
                }
                "content_block_delta" => {
                    Self::handle_content_block_delta(
                        data,
                        &mut tx,
                        &current_text_part,
                        &current_reasoning_part,
                        &mut current_tool_call,
                    )
                    .await;
                }
                "content_block_stop" => {
                    Self::handle_content_block_stop(
                        &mut tx,
                        &mut current_text_part,
                        &mut current_reasoning_part,
                        &mut current_tool_call,
                    )
                    .await;
                }
                "message_delta" => {
                    Self::handle_message_delta(data, &mut tx).await;
                    message_end_emitted = true;
                }
                "message_stop" => {}
                "error" => {
                    if let Ok(err_resp) = serde_json::from_str::<ErrorEvent>(data) {
                        let _ = tx
                            .send(ProviderEvent::Error(MessageError {
                                kind: ErrorKind::ProviderApi,
                                message: format!("anthropic error: {}", err_resp.error.message),
                            }))
                            .await;
                    }
                }
                _ => {}
            }
        }

        if !message_end_emitted {
            Self::finalize_open_parts(
                &mut tx,
                &mut current_text_part,
                &mut current_reasoning_part,
                &mut current_tool_call,
            )
            .await;
            let _ = tx
                .send(ProviderEvent::MessageEnd {
                    finish: Finish::Stop,
                    usage: Tokens::default(),
                    cost: 0.0,
                })
                .await;
        }
    }

    async fn handle_content_block_start(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        current_text_part: &mut Option<TextPart>,
        current_reasoning_part: &mut Option<ReasoningPart>,
        current_tool_call: &mut Option<ToolCallAccumulator>,
    ) {
        #[derive(Debug, serde::Deserialize)]
        struct Event {
            #[serde(rename = "content_block")]
            content_block: ContentBlock,
        }
        #[derive(Debug, serde::Deserialize)]
        struct ContentBlock {
            #[serde(rename = "type")]
            typ: String,
            id: Option<String>,
            name: Option<String>,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        match event.content_block.typ.as_str() {
            "text" => {
                let part = new_text_part();
                let _ = tx
                    .send(ProviderEvent::PartStart {
                        part: Part::Text(part.clone()),
                    })
                    .await;
                *current_text_part = Some(part);
            }
            "thinking" => {
                let part = new_reasoning_part();
                let _ = tx
                    .send(ProviderEvent::PartStart {
                        part: Part::Reasoning(part.clone()),
                    })
                    .await;
                *current_reasoning_part = Some(part);
            }
            "tool_use" => {
                let part = ToolCallPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    tool_name: event.content_block.name.unwrap_or_default(),
                    call_id: event.content_block.id.unwrap_or_default(),
                    state: ToolState::Pending(ToolStatePending {
                        input: serde_json::Value::Null,
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
                *current_tool_call = Some(acc);
            }
            _ => {}
        }
    }

    async fn handle_content_block_delta(
        data: &str,
        tx: &mut mpsc::Sender<ProviderEvent>,
        current_text_part: &Option<TextPart>,
        current_reasoning_part: &Option<ReasoningPart>,
        current_tool_call: &mut Option<ToolCallAccumulator>,
    ) {
        #[derive(Debug, serde::Deserialize)]
        struct Event {
            delta: Delta,
        }
        #[derive(Debug, serde::Deserialize)]
        struct Delta {
            #[serde(rename = "type")]
            typ: String,
            text: Option<String>,
            #[serde(rename = "partial_json")]
            partial_json: Option<String>,
            thinking: Option<String>,
            signature: Option<String>,
        }

        let event: Event = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return,
        };

        match event.delta.typ.as_str() {
            "text_delta" => {
                if let Some(tp) = current_text_part {
                    if let Some(text) = event.delta.text {
                        let _ = tx
                            .send(ProviderEvent::PartDelta {
                                part_id: tp.base.id,
                                field: "text",
                                delta: text,
                            })
                            .await;
                    }
                }
            }
            "input_json_delta" => {
                if let Some(acc) = current_tool_call {
                    if let Some(partial) = event.delta.partial_json {
                        acc.json.push_str(&partial);
                        let _ = tx
                            .send(ProviderEvent::PartDelta {
                                part_id: acc.part.base.id,
                                field: "arguments",
                                delta: partial,
                            })
                            .await;
                    }
                }
            }
            "thinking_delta" => {
                if let Some(rp) = current_reasoning_part {
                    if let Some(thinking) = event.delta.thinking {
                        let _ = tx
                            .send(ProviderEvent::PartDelta {
                                part_id: rp.base.id,
                                field: "text",
                                delta: thinking,
                            })
                            .await;
                    }
                }
            }
            "signature_delta" => {
                if let Some(rp) = current_reasoning_part {
                    if let Some(signature) = event.delta.signature {
                        let _ = tx
                            .send(ProviderEvent::PartDelta {
                                part_id: rp.base.id,
                                field: "signature",
                                delta: signature,
                            })
                            .await;
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_content_block_stop(
        tx: &mut mpsc::Sender<ProviderEvent>,
        current_text_part: &mut Option<TextPart>,
        current_reasoning_part: &mut Option<ReasoningPart>,
        current_tool_call: &mut Option<ToolCallAccumulator>,
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
        if let Some(mut acc) = current_tool_call.take() {
            acc.finalize();
            let _ = tx
                .send(ProviderEvent::PartEnd {
                    part_id: acc.part.base.id,
                })
                .await;
        }
    }

    async fn handle_message_delta(data: &str, tx: &mut mpsc::Sender<ProviderEvent>) {
        let v: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return,
        };

        let stop_reason = v["delta"]["stop_reason"].as_str().unwrap_or("");
        let input_tokens = v["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = v["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

        let finish = map_finish_reason(stop_reason);
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
    }

    async fn finalize_open_parts(
        tx: &mut mpsc::Sender<ProviderEvent>,
        current_text_part: &mut Option<TextPart>,
        current_reasoning_part: &mut Option<ReasoningPart>,
        current_tool_call: &mut Option<ToolCallAccumulator>,
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
        if let Some(mut acc) = current_tool_call.take() {
            acc.finalize();
            let _ = tx
                .send(ProviderEvent::PartEnd {
                    part_id: acc.part.base.id,
                })
                .await;
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ErrorEvent {
    error: ErrorDetail,
}

#[derive(Debug, serde::Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    _typ: String,
    message: String,
}

#[derive(Debug)]
struct ToolCallAccumulator {
    part: ToolCallPart,
    json: String,
}

impl ToolCallAccumulator {
    fn finalize(&mut self) {
        if !self.json.is_empty() {
            if let Ok(input) = serde_json::from_str(&self.json) {
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
        "end_turn" | "stop_sequence" => Finish::Stop,
        "max_tokens" => Finish::Length,
        "tool_use" => Finish::ToolUse,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{AssistantMeta, TextPart, ToolResultPart};

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
        assert!(wire.is_some());
        let wire = wire.unwrap();
        assert_eq!(wire["role"], "user");
        assert!(wire["content"].is_array());
        assert_eq!(wire["content"][0]["type"], "text");
        assert_eq!(wire["content"][0]["text"], "Hello");
    }

    #[tokio::test]
    async fn test_build_wire_message_assistant_with_reasoning_and_tool() {
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
                Part::Reasoning(ReasoningPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: "Thinking...".to_string(),
                    signature: Some("sig123".to_string()),
                }),
                Part::Text(TextPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: "Here you go".to_string(),
                    synthetic: false,
                }),
                Part::ToolCall(ToolCallPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    tool_name: "echo".to_string(),
                    call_id: "call_456".to_string(),
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
            }),
        };
        let wire = adapter.build_wire_message(&[], &msg).await;
        assert!(wire.is_some());
        let wire = wire.unwrap();
        assert_eq!(wire["role"], "assistant");
        let content = wire["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "Thinking...");
        assert_eq!(content[0]["signature"], "sig123");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[2]["type"], "tool_use");
        assert_eq!(content[2]["id"], "call_456");
        assert_eq!(content[2]["name"], "echo");
    }

    #[tokio::test]
    async fn test_build_wire_message_tool_result() {
        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "model".to_string(),
            "key".to_string(),
        );
        // First create an assistant message with the tool call so find_tool_output works
        let assistant_msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: "echo".to_string(),
                call_id: "call_789".to_string(),
                state: ToolState::Completed(mew_message::ToolStateCompleted {
                    input: serde_json::json!({"input": "hello"}),
                    output: "echo: hello".to_string(),
                    metadata: None,
                    diff: None,
                    time: ToolTime {
                        start: 0,
                        end: Some(1),
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
            }),
        };
        let user_msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::User,
            parts: vec![Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                call_id: "call_789".to_string(),
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        let all = vec![assistant_msg.clone(), user_msg.clone()];
        let wire = adapter.build_wire_message(&all, &user_msg).await;
        assert!(wire.is_some());
        let wire = wire.unwrap();
        let content = wire["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "call_789");
        assert_eq!(content[0]["content"], "echo: hello");
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
        assert!(wire.is_some());
        let wire = wire.unwrap();
        let content = wire["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Describe this image");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert!(!content[1]["source"]["data"].as_str().unwrap().is_empty());
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
            .and(path("/messages"))
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
            params: None,
            headers: Default::default(),
        };

        let mut stream = adapter.stream(req).await.expect("stream");
        let mut events: Vec<ProviderEvent> = Vec::new();
        while let Some(ev) = futures::StreamExt::next(&mut stream).await {
            events.push(ev);
        }

        println!("events: {events:?}");

        // Should have PartStart(Text), PartDelta(text), PartEnd, MessageEnd
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::PartStart { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::PartDelta { field: "text", .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::PartEnd { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::MessageEnd { .. })));
    }

    #[tokio::test]
    async fn test_fixture_tool_call_reasoning() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture = std::fs::read_to_string("src/testdata/tool-call-reasoning.sse")
            .expect("read tool-call-reasoning fixture");

        Mock::given(method("POST"))
            .and(path("/messages"))
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
            params: None,
            headers: Default::default(),
        };

        let mut stream = adapter.stream(req).await.expect("stream");
        let mut events: Vec<ProviderEvent> = Vec::new();
        while let Some(ev) = futures::StreamExt::next(&mut stream).await {
            events.push(ev);
        }

        println!("events: {events:?}");

        let has_reasoning = events.iter().any(|e| {
            if let ProviderEvent::PartStart { part } = e {
                matches!(part, Part::Reasoning(_))
            } else {
                false
            }
        });
        assert!(has_reasoning, "expected reasoning part start");

        let has_tool_use = events.iter().any(|e| {
            if let ProviderEvent::PartStart { part } = e {
                matches!(part, Part::ToolCall(_))
            } else {
                false
            }
        });
        assert!(has_tool_use, "expected tool call part start");

        assert!(events
            .iter()
            .any(|e| matches!(e, ProviderEvent::MessageEnd { .. })));
    }

    // -- max_output_tokens wire-format tests -------------------------------
    //
    // Verify that the Anthropic adapter actually honours
    // `req.params.max_tokens`. The previous version hard-coded 4096 and
    // silently ignored the field, which made the agent's
    // `default_max_output_tokens` a no-op on this provider.
    //
    // Verify that the Anthropic adapter actually honours
    // `req.params.max_tokens`. The previous version hard-coded 4096 and
    // silently ignored the field, which made the agent's
    // `default_max_output_tokens` a no-op on this provider.

    use mew_provider::{ChatParams, ReasoningConfig};

    fn sample_request(
        max_tokens: Option<i32>,
        thinking: Option<mew_provider::ReasoningConfig>,
    ) -> Request {
        Request {
            model: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            system: String::new(),
            reasoning: thinking,
            params: max_tokens.map(|m| ChatParams {
                temperature: None,
                top_p: None,
                max_tokens: Some(m),
                tool_choice: None,
            }),
            headers: reqwest::header::HeaderMap::new(),
        }
    }

    fn body_max_tokens(body: &[u8]) -> Option<i64> {
        let v: serde_json::Value = serde_json::from_slice(body).unwrap();
        v.get("max_tokens").and_then(|x| x.as_i64())
    }

    fn body_thinking_enabled(body: &[u8]) -> bool {
        let v: serde_json::Value = serde_json::from_slice(body).unwrap();
        v.get("thinking")
            .and_then(|t| t.get("type"))
            .and_then(|x| x.as_str())
            == Some("enabled")
    }

    #[tokio::test]
    async fn test_anthropic_adapter_uses_params_max_tokens() {
        // params.max_tokens = Some(32_768), no thinking → wire body
        // has max_tokens: 32_768.
        let adapter = Adapter::new(
            "anthropic".into(),
            "https://example.com".into(),
            "test-model".into(),
            "test-key".into(),
        );
        let body = adapter
            .build_request_body(&sample_request(Some(32_768), None))
            .await
            .unwrap();
        assert_eq!(body_max_tokens(&body), Some(32_768));
    }

    #[tokio::test]
    async fn test_anthropic_thinking_bump_uses_max_of_both() {
        // params.max_tokens = Some(32_768) + thinking.budget_tokens = 8_000
        // → wire body has max_tokens: 32_768 (default wins; the
        // required min is 8_000 + 4096 = 12_096, well below 32_768).
        let adapter = Adapter::new(
            "anthropic".into(),
            "https://example.com".into(),
            "test-model".into(),
            "test-key".into(),
        );
        let mut thinking = ReasoningConfig::default();
        thinking.params.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": 8_000}),
        );
        let body = adapter
            .build_request_body(&sample_request(Some(32_768), Some(thinking)))
            .await
            .unwrap();
        assert!(body_thinking_enabled(&body));
        assert_eq!(body_max_tokens(&body), Some(32_768));

        // params.max_tokens = Some(32_768) + thinking.budget_tokens = 64_000
        // → wire body has max_tokens: 68_096 (thinking wins; required
        // min is 64_000 + 4096 = 68_096, above the 32_768 default).
        let mut thinking2 = ReasoningConfig::default();
        thinking2.params.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": 64_000}),
        );
        let body2 = adapter
            .build_request_body(&sample_request(Some(32_768), Some(thinking2)))
            .await
            .unwrap();
        assert_eq!(body_max_tokens(&body2), Some(68_096));
    }

    #[tokio::test]
    async fn test_anthropic_adapter_some_zero_without_thinking_falls_back_to_4096() {
        // Some(0) without thinking → API floor (4096). The agent's
        // dispatcher can dispatch Some(0); the adapter must not crash
        // and must not send 0 (Anthropic rejects).
        let adapter = Adapter::new(
            "anthropic".into(),
            "https://example.com".into(),
            "test-model".into(),
            "test-key".into(),
        );
        let body = adapter
            .build_request_body(&sample_request(Some(0), None))
            .await
            .unwrap();
        assert_eq!(body_max_tokens(&body), Some(4096));
    }

    #[tokio::test]
    async fn test_anthropic_adapter_some_zero_with_thinking_lets_bump_handle_it() {
        // Some(0) WITH thinking → the thinking-budget bump handles the
        // floor naturally (max(0, budget + 4096)). No silent 0 leak.
        let adapter = Adapter::new(
            "anthropic".into(),
            "https://example.com".into(),
            "test-model".into(),
            "test-key".into(),
        );
        let mut thinking = ReasoningConfig::default();
        thinking.params.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": 30_000}),
        );
        let body = adapter
            .build_request_body(&sample_request(Some(0), Some(thinking)))
            .await
            .unwrap();
        assert!(body_thinking_enabled(&body));
        assert_eq!(body_max_tokens(&body), Some(34_096));
    }

    #[tokio::test]
    async fn test_anthropic_adapter_no_params_uses_4096_default() {
        // No params at all → 4096 (backward-compatible default).
        let adapter = Adapter::new(
            "anthropic".into(),
            "https://example.com".into(),
            "test-model".into(),
            "test-key".into(),
        );
        let body = adapter
            .build_request_body(&sample_request(None, None))
            .await
            .unwrap();
        assert_eq!(body_max_tokens(&body), Some(4096));
    }
}
