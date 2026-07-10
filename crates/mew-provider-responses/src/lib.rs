//! OpenAI Responses API adapter.
//!
//! Implements the `POST /v1/responses` wire protocol used by Codex CLI
//! and GPT-5-codex models. This is a distinct API surface from
//! chat/completions — different request body, different SSE event
//! grammar, different role mapping.
//!
//! Supports both API-key auth (Phase 1) and ChatGPT OAuth (Phase 2).

pub mod oauth;

mod openai_oauth;

use async_trait::async_trait;
use futures::{channel::mpsc, SinkExt, StreamExt};
use mew_message::{
    ErrorKind, Finish, Message, MessageError, Part, PartBase, ReasoningPart, Role, TextPart,
    Tokens, ToolCallPart, ToolState, ToolStatePending, ToolTime,
};
use mew_provider::auth::OAuthProvider;
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
    auth: AdapterAuth,
    /// OAuth provider reference for token refresh. None for API-key auth.
    oauth_provider: Option<std::sync::Arc<dyn OAuthProvider>>,
    client: reqwest::Client,
    dump: bool,
    /// True for Codex models that require the Responses Lite transport
    /// (e.g. gpt-5.6-sol/terra/luna).
    use_responses_lite: bool,
}

impl Adapter {
    pub fn with_responses_lite(mut self, v: bool) -> Self {
        self.use_responses_lite = v;
        self
    }
}

/// The auth state held by the adapter. OAuth tokens are wrapped in
/// RwLock so `&self` methods can refresh them in place.
enum AdapterAuth {
    ApiKey(String),
    OAuth {
        tokens: tokio::sync::RwLock<mew_provider::auth::TokenSet>,
        extra_headers: tokio::sync::RwLock<Vec<(String, String)>>,
    },
}

impl Adapter {
    pub fn new(name: String, base_url: String, model: String, api_key: String) -> Self {
        Self {
            name,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            auth: AdapterAuth::ApiKey(api_key),
            oauth_provider: None,
            client: reqwest::Client::new(),
            dump: false,
            use_responses_lite: false,
        }
    }

    /// Create an adapter authenticated via OAuth.
    pub fn new_oauth(
        name: String,
        model: String,
        tokens: mew_provider::auth::TokenSet,
        extra_headers: Vec<(String, String)>,
        provider: std::sync::Arc<dyn OAuthProvider>,
    ) -> Self {
        Self {
            name,
            base_url: provider.oauth_base_url().to_string(),
            model,
            auth: AdapterAuth::OAuth {
                tokens: tokio::sync::RwLock::new(tokens),
                extra_headers: tokio::sync::RwLock::new(extra_headers),
            },
            oauth_provider: Some(provider),
            client: reqwest::Client::new(),
            dump: false,
            use_responses_lite: false,
        }
    }

    pub fn set_dump(&mut self, v: bool) {
        self.dump = v;
    }

    /// Build the authorization header value(s) for the current auth kind.
    /// For OAuth, also refreshes tokens if expired.
    async fn build_auth_headers(&self) -> Result<(String, Vec<(String, String)>), ProviderError> {
        match &self.auth {
            AdapterAuth::ApiKey(key) => Ok((format!("Bearer {key}"), vec![])),
            AdapterAuth::OAuth {
                tokens,
                extra_headers,
            } => {
                if let Some(provider) = &self.oauth_provider {
                    mew_provider::auth::refresh_if_needed(provider.as_ref(), tokens, extra_headers)
                        .await
                        .map_err(|e| {
                            ProviderError::Message(format!("oauth refresh failed: {e}"))
                        })?;
                }
                let guard = tokens.read().await;
                let headers = extra_headers.read().await;
                Ok((format!("Bearer {}", guard.access_token), headers.clone()))
            }
        }
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

        let url = format!("{}/responses", self.base_url);
        let (auth_header, extra_headers) = self.build_auth_headers().await?;
        let mut request_builder = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", auth_header)
            .header("Accept", "text/event-stream");
        for (name, value) in &extra_headers {
            request_builder = request_builder.header(name, value);
        }
        if self.use_responses_lite {
            request_builder =
                request_builder.header("x-openai-internal-codex-responses-lite", "true");
        }
        let request = request_builder.body(body).build()?;

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
                    .send(ProviderEvent::Error(MessageError {
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
        let resp = resp.ok_or_else(|| {
            ProviderError::Message("retry loop exited without response".to_string())
        })?;
        let dump = self.dump;
        tokio::spawn(async move {
            Self::read_stream(dump, resp, tx).await;
        });

        Ok(Box::pin(rx))
    }

    async fn list_models(&self) -> Result<Vec<mew_provider::ModelInfo>, ProviderError> {
        let url = format!("{}/models", self.base_url);
        let (auth_header, extra_headers) = self.build_auth_headers().await?;
        let mut request_builder = self.client.get(&url).header("Authorization", auth_header);
        for (name, value) in &extra_headers {
            request_builder = request_builder.header(name, value);
        }
        let resp = request_builder.send().await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let (kind, msg) = classify_error(status, &body);
            return Err(ProviderError::Classified { kind, message: msg });
        }

        // The ChatGPT (OAuth) backend returns codex's `ModelsResponse` shape
        // { "models": [...] }; the API-key backend returns OpenAI's
        // { "data": [...] }. The OAuth path also refreshes the codex catalog
        // cache with the live, plan-filtered response so the daemon path
        // (which reads the catalog, not list_models) benefits next launch.
        if self.oauth_provider.is_some() {
            let body = resp.text().await.unwrap_or_default();
            let _ = mew_catalog::write_codex_cache(&body);
            let models = mew_catalog::parse_codex(body.as_bytes())
                .map_err(|e| ProviderError::Message(format!("codex models parse failed: {e}")))?;
            return Ok(models
                .into_iter()
                .map(|m| mew_provider::ModelInfo {
                    id: m.id,
                    owned_by: "openai".to_string(),
                })
                .collect());
        }

        #[derive(serde::Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }
        #[derive(serde::Deserialize)]
        struct ModelEntry {
            id: String,
            owned_by: Option<String>,
        }

        let models: ModelsResponse = resp.json().await?;
        Ok(models
            .data
            .into_iter()
            .map(|m| mew_provider::ModelInfo {
                id: m.id,
                owned_by: m.owned_by.unwrap_or_default(),
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Request body builder
// ---------------------------------------------------------------------------

impl Adapter {
    /// Build a lookup map of call_id → tool output string, scanning all
    /// messages once. O(N×P) instead of O(N²×P) when called per tool result.
    fn build_tool_output_map(messages: &[Message]) -> std::collections::HashMap<&str, &str> {
        let mut map = std::collections::HashMap::new();
        for m in messages {
            for p in &m.parts {
                if let Part::ToolCall(tc) = p {
                    map.insert(tc.call_id.as_str(), tc.state.output().unwrap_or(""));
                }
            }
        }
        map
    }

    async fn build_request_body(&self, req: &Request) -> Result<Vec<u8>, ProviderError> {
        let mut input: Vec<serde_json::Value> = Vec::new();

        // Build the tool output lookup once for O(1) access in build_wire_message.
        let tool_outputs = Self::build_tool_output_map(&req.messages);

        for m in &req.messages {
            self.build_wire_message(m, &tool_outputs, &mut input).await;
        }

        let mut body = json!({
            "model": self.model,
            "input": input,
            "stream": true,
            "parallel_tool_calls": !self.use_responses_lite,
        });

        // The ChatGPT (OAuth) subscription backend rejects requests without
        // store=false (it doesn't persist responses). The API-key backend
        // (api.openai.com) accepts the default, so this is OAuth-only.
        if self.oauth_provider.is_some() {
            body["store"] = json!(false);
        }

        // Tools — flat shape with strict: false.
        let tools_json: Vec<serde_json::Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.schema,
                    "strict": false,
                })
            })
            .collect();

        if self.use_responses_lite {
            // Responses Lite moves tools and the system prompt into the input
            // array and omits the top-level `tools`/`instructions` keys.
            // Order matches OpenAI's Codex CLI: additional_tools first, then the
            // developer message with instructions.
            let mut prefix: Vec<serde_json::Value> = Vec::new();
            if !tools_json.is_empty() {
                prefix.push(json!({
                    "type": "additional_tools",
                    "role": "developer",
                    "tools": tools_json,
                }));
            }
            if !req.system.is_empty() {
                prefix.push(json!({
                    "type": "message",
                    "role": "developer",
                    "content": [{"type": "input_text", "text": req.system}],
                }));
            }
            input.splice(0..0, prefix);
            // Re-assign the now-prefixed input back into the body.
            body["input"] = json!(input);
        } else {
            // Standard Responses shape: instructions + top-level tools array.
            if !req.system.is_empty() {
                body["instructions"] = json!(req.system);
            }
            if !tools_json.is_empty() {
                body["tools"] = json!(tools_json);
            }
        }

        // Reasoning — restructure from flat params to nested object.
        // The catalog stores params in chat/completions format
        // ({"reasoning_effort": "high"}). The Responses API needs
        // {"reasoning": {"effort": "high"}}.
        if let Some(ref reasoning) = req.reasoning {
            if let Some(effort) = reasoning.params.get("reasoning_effort") {
                let mut reasoning_obj = json!({"effort": effort});
                if self.use_responses_lite {
                    reasoning_obj["context"] = json!("all_turns");
                    body["include"] = json!(["reasoning.encrypted_content"]);
                }
                body["reasoning"] = reasoning_obj;
            } else if let Some(reasoning_obj) = reasoning.params.get("reasoning") {
                let mut reasoning_obj = reasoning_obj.clone();
                if self.use_responses_lite && reasoning_obj.get("context").is_none() {
                    reasoning_obj["context"] = json!("all_turns");
                    body["include"] = json!(["reasoning.encrypted_content"]);
                }
                body["reasoning"] = reasoning_obj;
            }
        }

        // Sampling params. Note: max_output_tokens (not max_tokens).
        if let Some(ref params) = req.params {
            if let Some(body_obj) = body.as_object_mut() {
                if let Some(t) = params.temperature {
                    body_obj.insert("temperature".into(), json!(t));
                }
                if let Some(p) = params.top_p {
                    body_obj.insert("top_p".into(), json!(p));
                }
                if let Some(m) = params.max_tokens {
                    body_obj.insert("max_output_tokens".into(), json!(m));
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

    async fn build_wire_message(
        &self,
        m: &Message,
        tool_outputs: &std::collections::HashMap<&str, &str>,
        input: &mut Vec<serde_json::Value>,
    ) {
        match m.role {
            Role::User => {
                let mut text_content: Vec<serde_json::Value> = Vec::new();
                let mut tool_results: Vec<serde_json::Value> = Vec::new();

                for p in &m.parts {
                    match p {
                        Part::Text(pt) => {
                            if !pt.text.is_empty() {
                                text_content.push(json!({
                                    "type": "input_text",
                                    "text": pt.text,
                                }));
                            }
                        }
                        Part::File(fp) => {
                            text_content.push(json!({
                                "type": "input_image",
                                "image_url": fp.url,
                            }));
                        }
                        Part::ToolResult(tr) => {
                            let output =
                                tool_outputs.get(tr.call_id.as_str()).copied().unwrap_or("");
                            tool_results.push(json!({
                                "type": "function_call_output",
                                "call_id": tr.call_id,
                                "output": output,
                            }));
                        }
                        Part::Compaction(_) | Part::Reasoning(_) => {}
                        Part::ToolCall(_) => {
                            // Tool calls from the user role are unusual;
                            // skip them in the input.
                        }
                    }
                }

                // Tool results are top-level input items, not nested in
                // a message.
                input.extend(tool_results);

                if !text_content.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": text_content,
                    }));
                }
            }
            Role::Assistant => {
                for p in &m.parts {
                    match p {
                        Part::Text(pt) => {
                            if !pt.text.is_empty() {
                                input.push(json!({
                                    "type": "message",
                                    "role": "assistant",
                                    "content": [{
                                        "type": "output_text",
                                        "text": pt.text,
                                    }],
                                }));
                            }
                        }
                        Part::ToolCall(tc) => {
                            // Backends reject non-object arguments (sessions
                            // persisted before object-input was enforced can
                            // still carry Null, which stringifies to "null").
                            let tc_input = tc.state.input();
                            let args = if tc_input.is_object() {
                                tc_input.to_string()
                            } else {
                                "{}".to_string()
                            };
                            input.push(json!({
                                "type": "function_call",
                                "call_id": tc.call_id,
                                "name": tc.tool_name,
                                "arguments": args,
                            }));
                        }
                        Part::Reasoning(_) => {
                            // Reasoning is output-only; we don't send it
                            // back as input.
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SSE event parser
// ---------------------------------------------------------------------------

/// Tracks per-item state during SSE streaming.
struct StreamState {
    /// item_id → (PartId, text buffer)
    text_parts: std::collections::HashMap<String, TextPart>,
    /// item_id → (PartId, reasoning buffer)
    reasoning_parts: std::collections::HashMap<String, ReasoningPart>,
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
            text_parts: std::collections::HashMap::new(),
            reasoning_parts: std::collections::HashMap::new(),
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
    async fn read_stream(dump: bool, resp: reqwest::Response, mut tx: mpsc::Sender<ProviderEvent>) {
        let stream = resp
            .bytes_stream()
            .map(|res| res.map_err(std::io::Error::other));
        let reader = tokio::io::BufReader::new(tokio_util::io::StreamReader::new(stream));
        let mut lines = reader.lines();

        let mut current_event = String::new();
        let mut state = StreamState::new();

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
                    // Item already handled by its sub-events.
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
        }
        // Message and reasoning items are handled when their content
        // parts arrive (content_part.added / reasoning_summary).
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    use mew_message::{Message, TextPart, ToolResultPart};
    use mew_provider::{ChatParams, ReasoningConfig, ToolDef};
    use serde_json::json;

    fn make_adapter() -> Adapter {
        Adapter::new(
            "test".to_string(),
            "https://api.openai.com/v1".to_string(),
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        )
    }

    fn make_message(role: Role, text: &str) -> Message {
        Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                text: text.to_string(),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        }
    }

    // --- Request body unit tests ---

    #[tokio::test]
    async fn test_build_request_body_text_only() {
        let adapter = make_adapter();
        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hello")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(v["model"], "gpt-5-codex");
        assert_eq!(v["stream"], true);
        assert_eq!(v["parallel_tool_calls"], true);

        let input = v["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "hello");
    }

    #[tokio::test]
    async fn test_build_request_body_with_tool_call() {
        let adapter = make_adapter();
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
                call_id: "call_123".to_string(),
                state: ToolState::Pending(ToolStatePending {
                    input: json!({"command": "ls"}),
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
            assistant: None,
        };

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![assistant_msg],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let input = v["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["call_id"], "call_123");
        assert_eq!(input[0]["name"], "bash");
        assert!(input[0]["arguments"].is_string());
    }

    #[tokio::test]
    async fn test_build_request_body_null_tool_input_becomes_object() {
        // A tool call whose arguments never streamed carried `input: Null`.
        // Replaying that as `arguments: "null"` is rejected by backends
        // ("arguments must be a JSON object"). It must be "{}".
        let adapter = make_adapter();
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
            assistant: None,
        };

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![assistant_msg],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let input = v["input"].as_array().unwrap();
        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(
            input[0]["arguments"], "{}",
            "null tool input must serialize as \"{{}}\""
        );
    }

    #[tokio::test]
    async fn test_build_request_body_with_tool_result() {
        let adapter = make_adapter();

        // We need a ToolCall with output + a ToolResult referencing it.
        let call_id = "call_456";
        let tool_call = ToolCallPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            tool_name: "bash".to_string(),
            call_id: call_id.to_string(),
            state: ToolState::Completed(mew_message::ToolStateCompleted {
                input: json!({"command": "echo hi"}),
                output: "hi\n".to_string(),
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: 0,
                    end: Some(1),
                },
            }),
            raw_input: String::new(),
        };

        let assistant_msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![Part::ToolCall(tool_call)],
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
                call_id: call_id.to_string(),
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![assistant_msg, user_msg],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let input = v["input"].as_array().unwrap();
        // function_call_output should be in the input (from the user msg's
        // ToolResult). function_call should also be there (from the
        // assistant msg's ToolCall).
        let has_call_output = input.iter().any(|item| {
            item["type"] == "function_call_output"
                && item["call_id"] == call_id
                && item["output"] == "hi\n"
        });
        assert!(has_call_output, "expected function_call_output in input");
    }

    #[tokio::test]
    async fn test_build_request_body_system_prompt() {
        let adapter = make_adapter();
        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: "You are a helpful assistant.".to_string(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // System prompt goes in `instructions`, not in `input`.
        assert_eq!(v["instructions"], "You are a helpful assistant.");

        // Input should NOT contain a system-role message.
        let input = v["input"].as_array().unwrap();
        assert!(input
            .iter()
            .all(|item| item["role"].as_str() != Some("system")));
    }

    #[tokio::test]
    async fn test_build_request_body_tools() {
        let adapter = make_adapter();
        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![ToolDef {
                name: "bash".to_string(),
                description: "Run a shell command".to_string(),
                schema: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
            }],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let tools = v["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "bash");
        assert_eq!(tools[0]["description"], "Run a shell command");
        assert_eq!(tools[0]["strict"], false);
        // Parameters should be at the top level, not nested under "function".
        assert!(tools[0]["parameters"].is_object());
        assert!(tools[0].get("function").is_none());
    }

    #[tokio::test]
    async fn test_build_request_body_reasoning() {
        let adapter = make_adapter();

        // Test 1: reasoning_effort param → nested reasoning object
        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: Some(ReasoningConfig {
                params: serde_json::Map::from_iter(vec![(
                    "reasoning_effort".to_string(),
                    json!("high"),
                )]),
            }),
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["reasoning"]["effort"], "high");
        // Should NOT have reasoning_effort at the top level.
        assert!(v.get("reasoning_effort").is_none());

        // Test 2: pre-shaped "reasoning" key → passthrough
        let req2 = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: Some(ReasoningConfig {
                params: serde_json::Map::from_iter(vec![(
                    "reasoning".to_string(),
                    json!({"effort": "low"}),
                )]),
            }),
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body2 = adapter.build_request_body(&req2).await.unwrap();
        let v2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(v2["reasoning"]["effort"], "low");
    }

    #[tokio::test]
    async fn test_build_request_body_max_output_tokens() {
        let adapter = make_adapter();
        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: Some(ChatParams {
                max_tokens: Some(4096),
                temperature: Some(0.7),
                top_p: None,
                tool_choice: None,
            }),
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // max_tokens → max_output_tokens (not max_tokens)
        assert_eq!(v["max_output_tokens"], 4096);
        assert!(v.get("max_tokens").is_none());
        assert_eq!(v["temperature"], 0.7);
    }

    #[tokio::test]
    async fn test_build_request_body_oauth_sets_store_false() {
        // The ChatGPT subscription backend rejects requests without store=false.
        let provider = std::sync::Arc::new(TestOAuthProvider {
            base_url: "http://unused".into(),
        });
        let adapter = Adapter::new_oauth(
            "test".into(),
            "gpt-5.6-sol".into(),
            mew_provider::auth::TokenSet {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_at: 9_999_999_999,
            },
            vec![],
            provider,
        );
        let req = Request {
            model: "gpt-5.6-sol".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };
        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["store"], false, "OAuth path must set store=false");
    }

    #[tokio::test]
    async fn test_build_request_body_apikey_omits_store() {
        // The API-key backend keeps the default; store must not be forced.
        let adapter = make_adapter();
        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };
        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("store").is_none(), "API-key path must not set store");
    }

    #[tokio::test]
    async fn test_build_request_body_responses_lite() {
        let adapter = Adapter::new(
            "test".to_string(),
            "https://api.openai.com/v1".to_string(),
            "gpt-5.6-luna".to_string(),
            "test-key".to_string(),
        )
        .with_responses_lite(true);
        let req = Request {
            model: "gpt-5.6-luna".to_string(),
            messages: vec![make_message(Role::User, "hello")],
            tools: vec![ToolDef {
                name: "bash".to_string(),
                description: "Run a command".to_string(),
                schema: json!({"type": "object"}),
            }],
            system: "You are a helpful coding assistant.".to_string(),
            reasoning: Some(mew_provider::ReasoningConfig {
                params: {
                    let mut m = serde_json::Map::new();
                    m.insert("reasoning_effort".to_string(), json!("medium"));
                    m
                },
            }),
            params: None,
            headers: http::HeaderMap::new(),
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(v["model"], "gpt-5.6-luna");
        assert_eq!(v["parallel_tool_calls"], false);
        assert!(
            v.get("instructions").is_none(),
            "lite uses developer message, not instructions"
        );
        assert!(
            v.get("tools").is_none(),
            "lite uses additional_tools input item, not top-level tools"
        );

        let input = v["input"].as_array().unwrap();
        assert_eq!(input[0]["type"], "additional_tools");
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[0]["tools"][0]["name"], "bash");
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[1]["role"], "developer");
        assert_eq!(
            input[1]["content"][0]["text"],
            "You are a helpful coding assistant."
        );
        assert_eq!(input[2]["type"], "message");
        assert_eq!(input[2]["role"], "user");

        assert_eq!(v["reasoning"]["effort"], "medium");
        assert_eq!(v["reasoning"]["context"], "all_turns");
        assert_eq!(v["include"], json!(["reasoning.encrypted_content"]));
    }

    // --- Integration tests using wiremock ---

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

    #[tokio::test]
    async fn test_auth_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string("{\"error\":{\"message\":\"Invalid API key\"}}"),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "bad-key".to_string(),
        );

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        };

        let result = adapter.stream(req).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(matches!(
            err,
            ProviderError::Classified {
                kind: ErrorKind::ProviderAuth,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_list_models() {
        let server = MockServer::start().await;

        let models_body = serde_json::json!({
            "data": [
                {"id": "gpt-5-codex", "owned_by": "openai"},
                {"id": "gpt-5.5", "owned_by": "openai"},
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(serde_json::to_string(&models_body).unwrap()),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        );

        let models = adapter.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5-codex");
        assert_eq!(models[0].owned_by, "openai");
        assert_eq!(models[1].id, "gpt-5.5");
    }

    /// Saves and restores the codex cache file so the best-effort cache
    /// write inside `list_models` (OAuth path) can't pollute the developer's
    /// real cache — even if the test panics.
    struct CodexCacheRestore {
        path: std::path::PathBuf,
        original: Option<Vec<u8>>,
    }
    impl CodexCacheRestore {
        fn new(path: std::path::PathBuf) -> Self {
            let original = std::fs::read(&path).ok();
            Self { path, original }
        }
    }
    impl Drop for CodexCacheRestore {
        fn drop(&mut self) {
            match &self.original {
                Some(data) => {
                    let _ = std::fs::write(&self.path, data);
                }
                None => {
                    let _ = std::fs::remove_file(&self.path);
                }
            }
        }
    }

    /// A minimal `OAuthProvider` whose `oauth_base_url` is the mock server,
    /// so `Adapter::new_oauth` points at the wiremock instance.
    struct TestOAuthProvider {
        base_url: String,
    }
    #[async_trait]
    impl mew_provider::auth::OAuthProvider for TestOAuthProvider {
        fn display_name(&self) -> &str {
            "test"
        }
        fn slug(&self) -> &str {
            "test"
        }
        fn oauth_base_url(&self) -> &str {
            &self.base_url
        }
        async fn login(&self, _: bool) -> anyhow::Result<mew_provider::auth::OAuthSession> {
            anyhow::bail!("not used in tests")
        }
        fn extra_headers(&self, _: &mew_provider::auth::TokenSet) -> Vec<(String, String)> {
            vec![]
        }
        async fn refresh(&self, _: &str) -> anyhow::Result<mew_provider::auth::TokenSet> {
            anyhow::bail!("not used in tests")
        }
        fn token_file_path(&self) -> std::path::PathBuf {
            std::path::PathBuf::from("/tmp/mew-test-oauth-not-used")
        }
    }

    #[tokio::test]
    async fn test_list_models_oauth_parses_codex_response_and_refreshes_cache() {
        let server = MockServer::start().await;

        // Codex ModelsResponse shape: { "models": [...] }.
        let codex_body = json!({
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "visibility": "list",
                    "supported_in_api": true,
                    "context_window": 372000,
                    "default_reasoning_level": "low",
                    "supported_reasoning_levels": [{"effort": "low"}, {"effort": "high"}],
                    "input_modalities": ["text", "image"],
                    "supports_parallel_tool_calls": true
                },
                {"slug": "hidden-one", "visibility": "hidden", "supported_in_api": true, "context_window": 1}
            ]
        });

        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer test-access"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(serde_json::to_string(&codex_body).unwrap()),
            )
            .mount(&server)
            .await;

        let provider = std::sync::Arc::new(TestOAuthProvider {
            base_url: server.uri(),
        });
        let tokens = mew_provider::auth::TokenSet {
            access_token: "test-access".to_string(),
            refresh_token: "test-refresh".to_string(),
            // Far-future expiry so refresh_if_needed is a no-op (no file IO).
            expires_at: 9_999_999_999,
        };
        let adapter = Adapter::new_oauth(
            "test".to_string(),
            "gpt-5.6-sol".to_string(),
            tokens,
            vec![],
            provider,
        );

        let _guard = CodexCacheRestore::new(mew_catalog::codex_cache_path());

        let models = adapter.list_models().await.unwrap();
        // The hidden model is filtered out; only the visible one returns.
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-5.6-sol");
        assert_eq!(models[0].owned_by, "openai");

        // The live response refreshed the codex catalog cache. The cache holds
        // the raw body; filtering happens at parse time (proven above).
        let cached = std::fs::read_to_string(mew_catalog::codex_cache_path()).unwrap();
        assert!(cached.contains("gpt-5.6-sol"));
    }

    #[tokio::test]
    async fn test_list_models_oauth_non_2xx_returns_err() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let provider = std::sync::Arc::new(TestOAuthProvider {
            base_url: server.uri(),
        });
        let tokens = mew_provider::auth::TokenSet {
            access_token: "test-access".to_string(),
            refresh_token: "test-refresh".to_string(),
            expires_at: 9_999_999_999,
        };
        let adapter = Adapter::new_oauth(
            "test".to_string(),
            "gpt-5.6-sol".to_string(),
            tokens,
            vec![],
            provider,
        );

        let _guard = CodexCacheRestore::new(mew_catalog::codex_cache_path());
        let result = adapter.list_models().await;
        assert!(result.is_err(), "non-2xx should surface an error");
    }
}
