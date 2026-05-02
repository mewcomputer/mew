use async_trait::async_trait;
use futures::{channel::mpsc, SinkExt, StreamExt};
use mew_message::{
    ErrorKind, Finish, Message, MessageError, Part, PartBase, ReasoningPart, Role, TextPart,
    Tokens, ToolCallPart, ToolState, ToolStatePending, ToolTime,
};
use mew_provider::{
    classify_error, EventStream, Provider, ProviderError, ProviderEvent, Request, RetryPolicy,
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
                return Err(ProviderError::Classified { kind, message: msg });
            }
            tokio::time::sleep(backoff).await;
        }

        let resp = resp.unwrap();
        let (tx, rx) = mpsc::channel(128);
        let dump = self.dump;
        tokio::spawn(async move {
            Self::read_stream(dump, resp, tx).await;
        });

        Ok(Box::pin(rx))
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

        let mut body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": messages,
            "stream": true,
        });

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

        Ok(serde_json::to_vec(&body).map_err(ProviderError::Json)?)
    }

    async fn build_wire_message(
        &self,
        all: &[Message],
        m: &Message,
    ) -> Option<serde_json::Value> {
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
                            // z.ai's proxy expects content as an array of text blocks.
                            content.push(json!({
                                "type": "tool_result",
                                "tool_use_id": pt.call_id,
                                "content": [{"type": "text", "text": output}],
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

    async fn read_stream(
        dump: bool,
        resp: reqwest::Response,
        mut tx: mpsc::Sender<ProviderEvent>,
    ) {
        let stream = resp.bytes_stream().map(|res| {
            res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        });
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
            let _ = tx.send(ProviderEvent::PartEnd { part_id: tp.base.id }).await;
        }
        if let Some(rp) = current_reasoning_part.take() {
            let _ = tx.send(ProviderEvent::PartEnd { part_id: rp.base.id }).await;
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
        #[derive(Debug, serde::Deserialize)]
        struct MsgDelta {
            delta: Delta,
            usage: Usage,
        }
        #[derive(Debug, serde::Deserialize)]
        struct Delta {
            #[serde(rename = "stop_reason")]
            #[serde(default)]
            stop_reason: String,
        }
        #[derive(Debug, serde::Deserialize)]
        struct Usage {
            #[serde(rename = "input_tokens")]
            input_tokens: u32,
            #[serde(rename = "output_tokens")]
            output_tokens: u32,
        }

        let msg_delta: MsgDelta = match serde_json::from_str(data) {
            Ok(m) => m,
            Err(_) => return,
        };

        let finish = map_finish_reason(&msg_delta.delta.stop_reason);
        let _ = tx
            .send(ProviderEvent::MessageEnd {
                finish,
                usage: Tokens {
                    input: msg_delta.usage.input_tokens,
                    output: msg_delta.usage.output_tokens,
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
            let _ = tx.send(ProviderEvent::PartEnd { part_id: tp.base.id }).await;
        }
        if let Some(rp) = current_reasoning_part.take() {
            let _ = tx.send(ProviderEvent::PartEnd { part_id: rp.base.id }).await;
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
