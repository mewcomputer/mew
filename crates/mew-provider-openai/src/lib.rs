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

/// Detect whether the request has thinking/reasoning mode enabled, based on
/// the provider-specific reasoning params merged into the body. Thinking-mode
/// models reject `tool_choice` set to "required" or a specific-tool object, so
/// the adapter uses this to avoid sending an incompatible value.
fn thinking_mode_active(reasoning: Option<&mew_provider::ReasoningConfig>) -> bool {
    let Some(cfg) = reasoning else {
        return false;
    };
    let params = &cfg.params;
    // Qwen/DashScope-style explicit flag.
    if let Some(v) = params.get("enable_thinking") {
        if let Some(b) = v.as_bool() {
            return b;
        }
    }
    // OpenAI-style reasoning effort implies a reasoning model.
    if params.contains_key("reasoning_effort") {
        return true;
    }
    // Object-style thinking config (GLM/MiniMax/Anthropic-shaped).
    if let Some(t) = params.get("thinking") {
        if let Some(obj) = t.as_object() {
            if let Some(typ) = obj.get("type").and_then(|v| v.as_str()) {
                return !matches!(typ, "disabled" | "off" | "none");
            }
        }
        return true;
    }
    false
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
        // Track call_ids issued by the most recent assistant message in the
        // wire. Tool results are only emitted if they match — providers
        // reject role:tool messages that don't follow a preceding
        // tool_calls message.
        let mut last_assistant_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for m in &req.messages {
            let msgs = self
                .build_wire_message(&req.messages, m, &last_assistant_call_ids)
                .await;
            // Update the tracking set if this message emitted tool_calls.
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
            messages.extend(msgs);
        }

        if !req.system.is_empty() {
            messages.insert(0, json!({"role": "system", "content": req.system}));
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            // Ask the provider to return usage in the final chunk. Without
            // this, many OpenAI-compatible providers (Alibaba Qwen, DeepSeek,
            // etc.) omit the `usage` object entirely and the token counter /
            // cost stay at 0.
            "stream_options": { "include_usage": true },
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
                    // Thinking-mode models (Qwen, etc.) reject `tool_choice`
                    // set to "required" or a specific-tool object. The
                    // reasoning truncator forces `Required` to break
                    // deliberation loops, but that is incompatible with
                    // thinking mode — drop it rather than fail the request.
                    let drop_required_in_thinking =
                        matches!(tc, mew_provider::ToolChoice::Required)
                            && thinking_mode_active(req.reasoning.as_ref());
                    if !drop_required_in_thinking {
                        let v = match tc {
                            mew_provider::ToolChoice::Auto => json!("auto"),
                            mew_provider::ToolChoice::Required => json!("required"),
                            mew_provider::ToolChoice::None_ => json!("none"),
                        };
                        body_obj.insert("tool_choice".into(), v);
                    }
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
    ) -> Vec<serde_json::Value> {
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
                        Part::ToolResult(pt) if last_assistant_call_ids.contains(&pt.call_id) => {
                            // Only emit role:tool messages that respond to a
                            // tool_call in the immediately preceding assistant
                            // message. Providers reject role:tool messages
                            // that don't follow a preceding tool_calls message.
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

                // OpenAI requires that `role: tool` messages immediately follow
                // the assistant message that issued the matching tool_calls. Emit
                // any tool results first, then the user message (text/images).
                out.extend(tool_results);
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
                            // Skip tool calls that are still Pending — they have
                            // no result yet, and emitting them creates an
                            // assistant message with tool_calls but no matching
                            // role:tool response, which providers reject with
                            // "insufficient tool messages following toolcalls".
                            if matches!(pt.state, ToolState::Pending(_)) {
                                continue;
                            }
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
                    // Some OpenAI-compatible providers (e.g. Kimi) reject null
                    // content when tool_calls are present, so use an empty string.
                    msg.insert("content".to_string(), json!(""));
                } else {
                    msg.insert("content".to_string(), json!(content));
                }
                // Only send reasoning fields when there is actual reasoning
                // content; empty strings can confuse provider-side validation.
                if !reasoning.is_empty() {
                    msg.insert("reasoning".to_string(), json!(reasoning));
                    msg.insert("reasoning_content".to_string(), json!(reasoning));
                }
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
        // Captured from the final chunk: OpenAI-compatible streams send a
        // `usage` object (prompt_tokens / completion_tokens) in the last
        // chunk, which is what powers the TUI's token counter.
        let mut last_usage: Option<Tokens> = None;
        // The finish reason may arrive in a chunk before the usage chunk and
        // [DONE]. Defer finalization until the stream ends so usage is captured.
        let mut pending_finish: Option<Finish> = None;

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
                    pending_finish.unwrap_or(Finish::Stop),
                    last_usage,
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

            // Capture usage from any chunk that carries it (the final data
            // chunk typically has `choices: []` + a `usage` object).
            if let Some(usage) = &chunk.usage {
                last_usage = Some(Tokens {
                    input: usage.input_tokens.unwrap_or(0),
                    output: usage.output_tokens.unwrap_or(0),
                    ..Default::default()
                });
            }

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
                // Record the finish but don't return yet: the usage chunk (if
                // any) arrives after finish_reason and before [DONE].
                pending_finish = Some(map_finish_reason(finish_reason));
            }
        }

        // Stream ended without [DONE] (or we hit [DONE] which finalizes above).
        Self::finalize_all(
            &mut current_text_part,
            &mut current_reasoning_part,
            &mut current_tool_calls,
            &mut tx,
            pending_finish.unwrap_or(Finish::Stop),
            last_usage,
        )
        .await;
    }

    async fn finalize_all(
        current_text_part: &mut Option<TextPart>,
        current_reasoning_part: &mut Option<ReasoningPart>,
        current_tool_calls: &mut HashMap<u32, ToolCallAccumulator>,
        tx: &mut mpsc::Sender<ProviderEvent>,
        finish: Finish,
        usage: Option<Tokens>,
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
                usage: usage.unwrap_or_default(),
                cost: 0.0,
            })
            .await;
    }
}

#[derive(Debug, serde::Deserialize)]
struct CompletionChunk {
    // `#[serde(default)]`: some OpenAI-compatible providers (vLLM, proxies)
    // emit a usage-only final chunk with NO `choices` key at all. Defaulting to
    // an empty vec lets it deserialize; the read loop already treats empty
    // choices as "nothing to render" and continues after capturing usage.
    #[serde(default)]
    choices: Vec<Choice>,
    /// Present on the final data chunk (with `stream_options.include_usage`).
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

/// OpenAI-compatible usage object. `prompt_tokens`/`completion_tokens` mirror
/// the Anthropic fields; some providers (e.g. Alibaba Qwen) use `input_tokens`
/// / `output_tokens` instead, so both are accepted.
#[derive(Debug, Default, serde::Deserialize)]
struct ChunkUsage {
    #[serde(alias = "prompt_tokens")]
    input_tokens: Option<u32>,
    #[serde(alias = "completion_tokens")]
    output_tokens: Option<u32>,
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
    use mew_message::{ToolResultPart, ToolStateCompleted};

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
                    state: ToolState::Completed(ToolStateCompleted {
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
                state: ToolState::Completed(ToolStateCompleted {
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
            .await;
        assert_eq!(wire.len(), 1);
        assert_eq!(
            wire[0]["tool_calls"][0]["function"]["arguments"], "{}",
            "null tool input must serialize as \"{{}}\""
        );
    }

    #[tokio::test]
    async fn test_build_wire_message_skips_pending_tool_calls() {
        // A Pending tool call (never executed) must NOT be emitted as
        // tool_calls in the wire — it has no matching tool result, and
        // emitting it triggers "insufficient tool messages following
        // toolcalls message" from OpenAI-compatible providers.
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
                    text: "Let me check.".to_string(),
                    synthetic: false,
                }),
                Part::ToolCall(ToolCallPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    tool_name: "read".to_string(),
                    call_id: "call_pending".to_string(),
                    state: ToolState::Pending(ToolStatePending {
                        input: serde_json::json!({"path": "foo.rs"}),
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
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "assistant");
        // Text content should still be emitted.
        assert_eq!(wire[0]["content"], "Let me check.");
        // tool_calls must NOT be present — the Pending call has no result.
        assert!(
            wire[0].get("tool_calls").is_none(),
            "Pending tool calls must not appear in wire messages"
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
        let wire = adapter
            .build_wire_message(&[], &msg, &empty_call_ids())
            .await;
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

    #[tokio::test]
    async fn test_build_wire_message_drops_orphan_tool_results() {
        // A ToolResultPart whose call_id is NOT in the previous assistant
        // message must NOT be emitted as a role:tool message — providers
        // reject tool messages that don't follow a tool_calls message.
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
            parts: vec![Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                call_id: "call_orphan".to_string(),
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        // Pass empty call_ids — no preceding assistant message had this call.
        let wire = adapter
            .build_wire_message(&[], &msg, &empty_call_ids())
            .await;
        // The tool result is dropped; the user message has no text/image,
        // so nothing is emitted at all.
        assert!(
            wire.is_empty(),
            "orphan tool result should be dropped, got: {wire:?}"
        );
    }

    #[tokio::test]
    async fn test_build_wire_message_emits_tool_results_for_preceding_calls() {
        // ToolResultPart whose call_id IS in the preceding assistant
        // message must be emitted as role:tool.
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
            parts: vec![Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                call_id: "call_valid".to_string(),
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        let mut call_ids = std::collections::HashSet::new();
        call_ids.insert("call_valid".to_string());
        let wire = adapter.build_wire_message(&[], &msg, &call_ids).await;
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "tool");
        assert_eq!(wire[0]["tool_call_id"], "call_valid");
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
    async fn test_fixture_usage_parsed_from_final_chunk() {
        // Alibaba Qwen and other OpenAI-compatible providers send a final
        // data chunk with `choices: []` + a `usage` object. The adapter must
        // surface it in MessageEnd so the TUI token counter works.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture =
            std::fs::read_to_string("src/testdata/usage.sse").expect("read usage fixture");

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

        let usage = events
            .iter()
            .find_map(|e| match e {
                ProviderEvent::MessageEnd { usage, .. } => Some(usage),
                _ => None,
            })
            .expect("MessageEnd with usage");
        assert_eq!(usage.input, 1234, "prompt_tokens should map to input");
        assert_eq!(usage.output, 56, "completion_tokens should map to output");
    }

    #[tokio::test]
    async fn test_fixture_usage_chunk_without_choices_field() {
        // vLLM and some OpenAI-compatible proxies emit a final usage data chunk
        // with NO `choices` key at all (not even `choices: []`). Before the
        // `#[serde(default)]` fix this failed to deserialize and aborted the
        // stream with "unmarshal chunk: missing field choices". The stream must
        // now complete and surface usage instead of erroring.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let fixture = std::fs::read_to_string("src/testdata/usage-no-choices.sse")
            .expect("read usage-no-choices fixture");

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

        // No error event must surface — that was the regression.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Error { .. })),
            "choices-less usage chunk must not abort the stream: {:?}",
            events
        );

        let usage = events
            .iter()
            .find_map(|e| match e {
                ProviderEvent::MessageEnd { usage, .. } => Some(usage),
                _ => None,
            })
            .expect("MessageEnd with usage");
        assert_eq!(usage.input, 10);
        assert_eq!(usage.output, 2);
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
            matches!(
                e,
                ProviderEvent::PartStart {
                    part: Part::Reasoning(_)
                }
            )
        });
        assert!(
            has_reasoning_start,
            "expected reasoning PartStart from reasoning_content deltas; events: {:?}",
            events
                .iter()
                .map(|e| format!("{:?}", e))
                .collect::<Vec<_>>()
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
            matches!(
                e,
                ProviderEvent::PartStart {
                    part: Part::Text(_)
                }
            )
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

    #[tokio::test]
    async fn test_build_wire_message_tool_result_pair() {
        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "test-model".to_string(),
            "test-key".to_string(),
        );
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
                tool_name: "bash".to_string(),
                call_id: "bash:14".to_string(),
                state: ToolState::Completed(ToolStateCompleted {
                    input: serde_json::json!({"command": "echo hi"}),
                    output: "hi\n".to_string(),
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
            assistant: None,
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
                call_id: "bash:14".to_string(),
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        let all = vec![assistant_msg.clone(), user_msg.clone()];
        let req = Request {
            model: "test-model".to_string(),
            messages: all.clone(),
            tools: vec![],
            system: String::new(),
            reasoning: None,
            ..Default::default()
        };
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let messages = body_json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert!(messages[0]["tool_calls"].is_array());
        assert_eq!(messages[0]["tool_calls"][0]["id"], "bash:14");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "bash:14");
        assert_eq!(messages[1]["content"], "hi\n");
    }

    #[tokio::test]
    async fn test_build_wire_message_tool_results_before_user_text() {
        // OpenAI requires that `role: tool` messages immediately follow the
        // assistant message that issued the tool_calls. If a user message
        // carries both text and a tool result, emit tool results first.
        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "model".to_string(),
            "key".to_string(),
        );
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
                tool_name: "bash".to_string(),
                call_id: "bash:14".to_string(),
                state: ToolState::Completed(ToolStateCompleted {
                    input: serde_json::json!({"command": "echo hi"}),
                    output: "hi\n".to_string(),
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
            assistant: None,
        };
        let user_msg = Message {
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
                    text: "Here is the result".to_string(),
                    synthetic: false,
                }),
                Part::ToolResult(ToolResultPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    call_id: "bash:14".to_string(),
                }),
            ],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        let all = vec![assistant_msg.clone(), user_msg.clone()];
        let req = Request {
            model: "test-model".to_string(),
            messages: all.clone(),
            tools: vec![],
            system: String::new(),
            reasoning: None,
            ..Default::default()
        };
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let messages = body_json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["tool_calls"][0]["id"], "bash:14");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "bash:14");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], "Here is the result");
    }

    fn tool_choice_request(
        tool_choice: mew_provider::ToolChoice,
        reasoning: Option<mew_provider::ReasoningConfig>,
    ) -> Request {
        Request {
            model: "test-model".to_string(),
            messages: vec![Message {
                id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
                role: Role::User,
                parts: vec![Part::Text(TextPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: "hi".to_string(),
                    synthetic: false,
                })],
                time: mew_message::Time {
                    created: 0,
                    completed: None,
                },
                assistant: None,
            }],
            tools: vec![],
            system: String::new(),
            reasoning,
            params: Some(mew_provider::ChatParams {
                temperature: None,
                top_p: None,
                max_tokens: None,
                tool_choice: Some(tool_choice),
            }),
            ..Default::default()
        }
    }

    fn qwen_thinking_on() -> mew_provider::ReasoningConfig {
        mew_provider::ReasoningConfig {
            params: serde_json::json!({ "enable_thinking": true, "thinking_budget": 8192 })
                .as_object()
                .cloned()
                .unwrap(),
        }
    }

    #[tokio::test]
    async fn test_thinking_mode_detection() {
        // No reasoning config -> not thinking.
        assert!(!thinking_mode_active(None));
        // Qwen thinking on.
        assert!(thinking_mode_active(Some(&qwen_thinking_on())));
        // Qwen thinking explicitly off.
        let off = mew_provider::ReasoningConfig {
            params: serde_json::json!({ "enable_thinking": false })
                .as_object()
                .cloned()
                .unwrap(),
        };
        assert!(!thinking_mode_active(Some(&off)));
        // OpenAI reasoning_effort implies thinking.
        let effort = mew_provider::ReasoningConfig {
            params: serde_json::json!({ "reasoning_effort": "high" })
                .as_object()
                .cloned()
                .unwrap(),
        };
        assert!(thinking_mode_active(Some(&effort)));
    }

    #[tokio::test]
    async fn test_tool_choice_required_dropped_in_thinking_mode() {
        let adapter = Adapter::new(
            "test".to_string(),
            "https://example.com".to_string(),
            "model".to_string(),
            "key".to_string(),
        );
        // Thinking on + Required -> tool_choice must be omitted.
        let req = tool_choice_request(mew_provider::ToolChoice::Required, Some(qwen_thinking_on()));
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            body_json.get("tool_choice").is_none(),
            "tool_choice must be omitted in thinking mode, got {:?}",
            body_json.get("tool_choice")
        );

        // Thinking off + Required -> tool_choice stays "required".
        let req = tool_choice_request(mew_provider::ToolChoice::Required, None);
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body_json["tool_choice"], "required");

        // Thinking on + Auto -> tool_choice stays "auto" (only Required is
        // incompatible).
        let req = tool_choice_request(mew_provider::ToolChoice::Auto, Some(qwen_thinking_on()));
        let body = adapter.build_request_body(&req).await.unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body_json["tool_choice"], "auto");
    }
}
