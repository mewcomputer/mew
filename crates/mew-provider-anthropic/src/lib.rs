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
                        // Return the output for Completed/Running states.
                        // For Error state, return the error message so the
                        // provider sees a non-empty tool result.
                        return match &tc.state {
                            ToolState::Error(e) => e.error.clone(),
                            _ => tc.state.output().unwrap_or("").to_string(),
                        };
                    }
                }
            }
        }
        String::new()
    }

    async fn build_request_body(&self, req: &Request) -> Result<Vec<u8>, ProviderError> {
        let mut messages: Vec<serde_json::Value> = Vec::new();
        // Track tool_use ids issued by the most recent assistant message.
        // tool_result blocks are only emitted if they match — the API rejects
        // tool_results that don't follow a preceding tool_use.
        let mut last_assistant_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for m in &req.messages {
            if let Some(msg) = self
                .build_wire_message(&req.messages, m, &last_assistant_call_ids)
                .await
            {
                if m.role == Role::Assistant {
                    last_assistant_call_ids.clear();
                    for p in &m.parts {
                        if let Part::ToolCall(tc) = p {
                            if !matches!(tc.state, ToolState::Pending(_)) {
                                last_assistant_call_ids.insert(tc.call_id.clone());
                            }
                        }
                    }
                }
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

        if !req.system.is_empty() || req.messages.iter().any(|m| m.role == Role::System) {
            let mut system = req.system.clone();
            for message in &req.messages {
                if message.role == Role::System {
                    let text = message
                        .parts
                        .iter()
                        .filter_map(|part| match part {
                            Part::Text(text) => Some(text.text.as_str()),
                            _ => None,
                        })
                        .collect::<String>();
                    if !text.is_empty() {
                        if !system.is_empty() {
                            system.push_str("\n\n");
                        }
                        system.push_str(&text);
                    }
                }
            }
            body["system"] = json!(system);
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
                // Extended thinking rejects tool_choice "any"/"tool". The
                // reasoning truncator forces Required to break deliberation
                // loops, but that is incompatible with thinking — drop it
                // rather than fail the request.
                let drop_required_in_thinking = matches!(tc, mew_provider::ToolChoice::Required)
                    && thinking_mode_active(req.reasoning.as_ref());
                if !drop_required_in_thinking {
                    let v = match tc {
                        mew_provider::ToolChoice::Auto => json!("auto"),
                        mew_provider::ToolChoice::Required => json!("any"),
                        mew_provider::ToolChoice::None_ => json!("none"),
                    };
                    body["tool_choice"] = v;
                }
            }
        }

        serde_json::to_vec(&body).map_err(ProviderError::Json)
    }

    async fn build_wire_message(
        &self,
        all: &[Message],
        m: &Message,
        last_assistant_call_ids: &std::collections::HashSet<String>,
    ) -> Option<serde_json::Value> {
        let mut content: Vec<serde_json::Value> = Vec::new();

        match m.role {
            Role::System => None,
            Role::User => {
                for p in &m.parts {
                    match p {
                        Part::Text(pt) => {
                            content.push(json!({"type": "text", "text": pt.text}));
                        }
                        Part::ToolResult(pt) if last_assistant_call_ids.contains(&pt.call_id) => {
                            // Only emit tool_result blocks that respond to a
                            // tool_use in the immediately preceding assistant
                            // message. The API rejects tool_results without
                            // a preceding tool_use.
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
                            if let Some(ref encrypted) = pt.encrypted_content {
                                // Redacted thinking block — opaque data that
                                // must be round-tripped verbatim.
                                content.push(json!({
                                    "type": "redacted_thinking",
                                    "data": encrypted,
                                }));
                            } else {
                                // Normal thinking block with signature.
                                let mut block = serde_json::Map::new();
                                block.insert("type".to_string(), json!("thinking"));
                                block.insert("thinking".to_string(), json!(pt.text));
                                if let Some(sig) = &pt.signature {
                                    block.insert("signature".to_string(), json!(sig));
                                }
                                content.push(serde_json::Value::Object(block));
                            }
                        }
                        Part::ToolCall(pt) => {
                            // Skip tool calls that are still Pending — they
                            // have no result yet. Emitting a tool_use without
                            // a matching tool_result block causes API errors
                            // on replay after interrupted sessions.
                            if matches!(pt.state, ToolState::Pending(_)) {
                                continue;
                            }
                            // The API rejects non-object input (sessions
                            // persisted before object-input was enforced can
                            // still carry Null).
                            let input = match pt.state.input() {
                                v @ serde_json::Value::Object(_) => v.clone(),
                                _ => json!({}),
                            };
                            content.push(json!({
                                "type": "tool_use",
                                "id": pt.call_id,
                                "name": pt.tool_name,
                                "input": input,
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
            // Still deserialized for raw-dump mode and future use, but no
            // longer used to populate `call_id` — see the tool_use arm.
            #[allow(dead_code)]
            id: Option<String>,
            name: Option<String>,
            // redacted_thinking carries opaque data in content_block_start.
            data: Option<String>,
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
            "redacted_thinking" => {
                // Redacted thinking blocks arrive complete in content_block_start
                // with an opaque `data` field. No deltas follow. We create a
                // reasoning part, emit the data as an encrypted_content delta,
                // and immediately finalize it (content_block_stop will fire next).
                let part = new_reasoning_part();
                let part_id = part.base.id;
                let _ = tx
                    .send(ProviderEvent::PartStart {
                        part: Part::Reasoning(part),
                    })
                    .await;
                if let Some(data) = event.content_block.data {
                    let _ = tx
                        .send(ProviderEvent::PartDelta {
                            part_id,
                            field: "encrypted_content",
                            delta: data,
                        })
                        .await;
                }
                // redacted_thinking has no text; clear any dangling state.
                // content_block_stop will emit PartEnd.
            }
            "tool_use" => {
                let part = ToolCallPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    tool_name: event.content_block.name.unwrap_or_default(),
                    // Some Anthropic-compatible providers (notably Kimi) emit
                    // tool-call IDs containing spaces and colons (e.g.
                    // "handoff plan:29"). Anthropic's tool_use ID format
                    // rules reject these on the next-turn replay, producing
                    // "toolcallids did not have response messages" errors —
                    // the API rejects its own ID. We replace every incoming
                    // ID with a fresh `toolu_`-prefixed ULID. The fresh ID
                    // round-trips consistently: the agent matches tool
                    // results to calls by `call_id`, and `build_request_body`
                    // serializes both `tool_use.id` and
                    // `tool_result.tool_use_id` from this same field, so the
                    // pair always matches.
                    call_id: format!("toolu_{}", ulid::Ulid::new()),
                    state: ToolState::Pending(ToolStatePending {
                        // The API requires tool_use input to be a JSON object,
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

/// Detect whether the request has extended thinking enabled, based on the
/// Anthropic reasoning params. Extended thinking rejects `tool_choice` set to
/// "any" or a specific tool, so the adapter uses this to avoid sending an
/// incompatible value.
fn thinking_mode_active(reasoning: Option<&mew_provider::ReasoningConfig>) -> bool {
    let Some(cfg) = reasoning else {
        return false;
    };
    // Anthropic shape: {"thinking": {"type": "enabled", "budget_tokens": N}}
    if let Some(t) = cfg.params.get("thinking") {
        if let Some(obj) = t.as_object() {
            if let Some(typ) = obj.get("type").and_then(|v| v.as_str()) {
                return typ == "enabled";
            }
        }
        return true;
    }
    false
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
        encrypted_content: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{AssistantMeta, TextPart, ToolResultPart};

    fn empty_call_ids() -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }

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
        let wire = adapter
            .build_wire_message(&[], &msg, &empty_call_ids())
            .await;
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
                    encrypted_content: None,
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
                    state: ToolState::Completed(mew_message::ToolStateCompleted {
                        input: serde_json::json!({"input": "hi"}),
                        output: String::new(),
                        metadata: None,
                        diff: None,
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
        let wire = adapter
            .build_wire_message(&[], &msg, &empty_call_ids())
            .await;
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
    async fn test_build_wire_message_redacted_thinking_round_trip() {
        // A reasoning part with encrypted_content (from a redacted_thinking
        // block) should be sent back as {"type": "redacted_thinking", "data": ...}
        // not as a regular thinking block.
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
                    text: String::new(),
                    signature: None,
                    encrypted_content: Some("opaque_redacted_data_blob".to_string()),
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
            ],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        let wire = adapter
            .build_wire_message(&[], &msg, &empty_call_ids())
            .await;
        assert!(wire.is_some());
        let wire = wire.unwrap();
        let content = wire["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "redacted_thinking");
        assert_eq!(content[0]["data"], "opaque_redacted_data_blob");
        // Should NOT have thinking fields.
        assert!(content[0].get("thinking").is_none());
        assert!(content[0].get("signature").is_none());
        // Text block follows.
        assert_eq!(content[1]["type"], "text");
    }

    #[tokio::test]
    async fn test_build_wire_message_null_tool_input_becomes_object() {
        // A tool call whose arguments never streamed (or failed to parse)
        // historically carried `input: Null`. Replaying that as
        // `"input": null` is rejected by the API ("input must be a JSON
        // object"), killing the session. Null must serialize as `{}`.
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
                state: ToolState::Completed(mew_message::ToolStateCompleted {
                    input: serde_json::Value::Null,
                    output: String::new(),
                    metadata: None,
                    diff: None,
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
        let wire = adapter
            .build_wire_message(&[], &msg, &empty_call_ids())
            .await
            .unwrap();
        let content = wire["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_use");
        assert!(
            content[0]["input"].is_object(),
            "null tool input must serialize as an object, got {}",
            content[0]["input"]
        );
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
                manifest: None,
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
        // The preceding assistant message issued call_789, so it must be in
        // the tracking set for the tool_result to be emitted.
        let mut call_ids = std::collections::HashSet::new();
        call_ids.insert("call_789".to_string());
        let wire = adapter.build_wire_message(&all, &user_msg, &call_ids).await;
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
        let wire = adapter
            .build_wire_message(&[], &msg, &empty_call_ids())
            .await;
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

    #[tokio::test]
    async fn test_fixture_tool_call_no_arguments_starts_with_object_input() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture = std::fs::read_to_string("src/testdata/tool-call-empty-input.sse")
            .expect("read tool-call-empty-input fixture");

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

        // The agent only ever sees the part from PartStart (arguments arrive
        // as deltas). When no deltas come, the input it was born with is what
        // gets executed and replayed — it must be `{}`, never Null.
        let tool_input = events.iter().find_map(|e| {
            if let ProviderEvent::PartStart {
                part: Part::ToolCall(tc),
            } = e
            {
                Some(tc.state.input().clone())
            } else {
                None
            }
        });
        let tool_input = tool_input.expect("expected tool call part start");
        assert!(
            tool_input.is_object(),
            "tool call with no argument deltas must start with object input, got {tool_input}"
        );
    }

    // Kimi's Anthropic-compatible API emits tool-call IDs containing spaces
    // and colons (e.g. "handoff plan:29"). Anthropic rejects these on the
    // next-turn replay with "toolcallids did not have response messages".
    // The adapter must replace every incoming ID with a fresh
    // `toolu_`-prefixed ULID so the round-trip stays compliant.
    #[tokio::test]
    async fn test_fixture_tool_call_nonconformant_id_is_sanitized() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture = std::fs::read_to_string("src/testdata/tool-call-nonconformant-id.sse")
            .expect("read tool-call-nonconformant-id fixture");

        Mock::given(method("POST"))
            .and(path("/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "text/event-stream"))
            .mount(&mock_server)
            .await;

        let adapter = Adapter::new(
            "kimi".to_string(),
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
            params: None,
            headers: Default::default(),
        };

        let mut stream = adapter.stream(req).await.expect("stream");
        let mut events: Vec<ProviderEvent> = Vec::new();
        while let Some(ev) = futures::StreamExt::next(&mut stream).await {
            events.push(ev);
        }

        let call_id = events
            .iter()
            .find_map(|e| {
                if let ProviderEvent::PartStart {
                    part: Part::ToolCall(tc),
                } = e
                {
                    Some(tc.call_id.clone())
                } else {
                    None
                }
            })
            .expect("expected a tool call part start");

        assert!(
            call_id.starts_with("toolu_"),
            "call_id should be a toolu_-prefixed ULID, got {call_id}"
        );
        assert!(
            !call_id.contains(' ') && !call_id.contains(':'),
            "call_id must not contain spaces or colons, got {call_id}"
        );
        assert_ne!(
            call_id, "handoff plan:29",
            "call_id must not be the raw provider ID"
        );
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

    // Kimi K3 sends thinking effort as a top-level `reasoning_effort` field
    // (not Anthropic's nested `thinking` object). The catalog produces
    // `{"reasoning_effort": "low"|"high"|"max"}` params for k3, and the
    // adapter must forward them verbatim to the top level of the request
    // body — Kimi's API reads them there. This guards against a refactor of
    // the reasoning-params loop silently dropping or nesting the field.
    #[tokio::test]
    async fn test_anthropic_adapter_forwards_reasoning_effort_top_level() {
        let adapter = Adapter::new(
            "kimi".into(),
            "https://api.kimi.com/coding/v1".into(),
            "k3".into(),
            "test-key".into(),
        );
        let mut reasoning = ReasoningConfig::default();
        reasoning
            .params
            .insert("reasoning_effort".into(), json!("high"));
        let body = adapter
            .build_request_body(&sample_request(Some(8192), Some(reasoning)))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // reasoning_effort must be present at the top level, not nested.
        assert_eq!(
            v.get("reasoning_effort").and_then(|x| x.as_str()),
            Some("high"),
            "reasoning_effort should be forwarded top-level; body: {}",
            v
        );

        // Kimi does not use Anthropic-style thinking blocks, so the
        // thinking-budget bump should not have injected a `thinking` object.
        assert!(
            v.get("thinking").is_none(),
            "k3 reasoning_effort must not trigger Anthropic thinking block; body: {}",
            v
        );
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

    fn tool_choice_request(
        tool_choice: mew_provider::ToolChoice,
        thinking: Option<mew_provider::ReasoningConfig>,
    ) -> Request {
        Request {
            model: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            system: String::new(),
            reasoning: thinking,
            params: Some(ChatParams {
                temperature: None,
                top_p: None,
                max_tokens: None,
                tool_choice: Some(tool_choice),
            }),
            headers: reqwest::header::HeaderMap::new(),
        }
    }

    fn anthropic_thinking_on() -> mew_provider::ReasoningConfig {
        let mut cfg = mew_provider::ReasoningConfig::default();
        cfg.params.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": 16000}),
        );
        cfg
    }

    #[tokio::test]
    async fn test_tool_choice_required_dropped_in_thinking_mode() {
        let adapter = Adapter::new(
            "anthropic".into(),
            "https://example.com".into(),
            "test-model".into(),
            "test-key".into(),
        );
        // Thinking on + Required -> tool_choice must be omitted.
        let req = tool_choice_request(
            mew_provider::ToolChoice::Required,
            Some(anthropic_thinking_on()),
        );
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            body_json.get("tool_choice").is_none(),
            "tool_choice must be omitted in thinking mode, got {:?}",
            body_json.get("tool_choice")
        );

        // Thinking off + Required -> tool_choice stays "any".
        let req = tool_choice_request(mew_provider::ToolChoice::Required, None);
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body_json["tool_choice"], "any");

        // Thinking on + Auto -> tool_choice stays "auto".
        let req = tool_choice_request(
            mew_provider::ToolChoice::Auto,
            Some(anthropic_thinking_on()),
        );
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body_json["tool_choice"], "auto");
    }
}
