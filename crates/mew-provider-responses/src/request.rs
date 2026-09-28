//! Request body construction for `POST /v1/responses`.

use crate::Adapter;
use mew_message::{Message, Part, Role, ToolState};
use mew_provider::{ProviderError, Request};
use serde_json::json;

impl Adapter {
    /// Build a lookup map of call_id → tool output string, scanning all
    /// messages once. O(N×P) instead of O(N²×P) when called per tool result.
    fn build_tool_output_map(
        messages: &[Message],
    ) -> std::collections::HashMap<&str, (String, Vec<mew_message::ToolImage>)> {
        let mut map = std::collections::HashMap::new();
        for m in messages {
            for p in &m.parts {
                if let Part::ToolCall(tc) = p {
                    // For Error state, return the error message so the
                    // provider sees a non-empty tool result.
                    let output = match &tc.state {
                        ToolState::Error(e) => (e.error.clone(), vec![]),
                        ToolState::Completed(c) => (c.output.clone(), c.images.clone()),
                        _ => (tc.state.output().unwrap_or("").to_string(), vec![]),
                    };
                    map.insert(tc.call_id.as_str(), output);
                }
            }
        }
        map
    }

    pub(crate) async fn build_request_body(&self, req: &Request) -> Result<Vec<u8>, ProviderError> {
        let mut input: Vec<serde_json::Value> = Vec::new();

        // Build the tool output lookup once for O(1) access in build_wire_message.
        let tool_outputs = Self::build_tool_output_map(&req.messages);

        // Track call_ids issued by the most recent assistant message.
        // function_call_output items are only emitted if they match — the API
        // rejects outputs that don't follow a preceding function_call.
        let mut last_assistant_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for m in &req.messages {
            self.build_wire_message(
                m,
                &tool_outputs,
                &mut input,
                &last_assistant_call_ids,
                req.supports_vision,
            )
            .await;
            if m.role == Role::Assistant {
                last_assistant_call_ids.clear();
                for p in &m.parts {
                    if let Part::ToolCall(tc) = p {
                        // Mirror the wire builder: only Completed/Error
                        // calls are emitted as function_call, so only those
                        // belong in the set used to pair function_call_output.
                        if matches!(tc.state, ToolState::Completed(_) | ToolState::Error(_)) {
                            last_assistant_call_ids.insert(tc.call_id.clone());
                        }
                    }
                }
            }
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
        //
        // Responses Lite models always require reasoning.context = "all_turns"
        // (and the matching `include` array), even when reasoning is None or
        // explicitly disabled. Codex's build_reasoning always returns
        // Some(Reasoning { context: AllTurns }) when the model supports
        // reasoning — we mirror that here.
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
            } else if self.use_responses_lite {
                // Reasoning is configured but didn't match the two shapes above
                // (e.g. {"type": "disabled"} for title generation). Lite still
                // requires the context field.
                body["reasoning"] = json!({"context": "all_turns"});
                body["include"] = json!(["reasoning.encrypted_content"]);
            }
        } else if self.use_responses_lite {
            // No reasoning configured at all. Lite models still require
            // reasoning.context = "all_turns".
            body["reasoning"] = json!({"context": "all_turns"});
            body["include"] = json!(["reasoning.encrypted_content"]);
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
        tool_outputs: &std::collections::HashMap<&str, (String, Vec<mew_message::ToolImage>)>,
        input: &mut Vec<serde_json::Value>,
        last_assistant_call_ids: &std::collections::HashSet<String>,
        supports_vision: bool,
    ) {
        match m.role {
            Role::System => {
                let text = m
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                if !text.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "system",
                        "content": [{"type": "input_text", "text": text}],
                    }));
                }
            }
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
                            if fp.mime.starts_with("image/") && supports_vision {
                                text_content.push(json!({
                                    "type": "input_image",
                                    "image_url": fp.url,
                                }));
                            } else if fp.mime.starts_with("image/") {
                                let filename = fp.filename.as_deref().unwrap_or("image");
                                text_content.push(json!({
                                    "type": "input_text",
                                    "text": format!(
                                        "[Image attached: {} ({}); omitted — current model does not support vision]",
                                        filename, fp.mime
                                    ),
                                }));
                            } else {
                                let filename = fp.filename.as_deref().unwrap_or("file");
                                text_content.push(json!({
                                    "type": "input_text",
                                    "text": format!("[File: {}]", filename),
                                }));
                            }
                        }
                        Part::ToolResult(tr)
                            if last_assistant_call_ids.contains(tr.call_id.as_str()) =>
                        {
                            // Only emit function_call_output items that respond
                            // to a function_call in the immediately preceding
                            // assistant message. The API rejects outputs that
                            // don't follow a preceding function_call.
                            let (text, images) = tool_outputs
                                .get(tr.call_id.as_str())
                                .cloned()
                                .unwrap_or_default();
                            // The Responses API function_call_output only
                            // accepts a string. When images are present and
                            // the model supports vision, encode them as data
                            // URLs appended to the text. When the model
                            // doesn't support vision, demote to a text
                            // annotation so the model gets a coherent, non-
                            // broken payload (and the base64 bytes don't
                            // leak into the response).
                            let output = if images.is_empty() {
                                text
                            } else if supports_vision {
                                let mut combined = text;
                                for img in &images {
                                    if !combined.is_empty() {
                                        combined.push('\n');
                                    }
                                    combined.push_str(&format!(
                                        "data:{};base64,{}",
                                        img.mime, img.data
                                    ));
                                }
                                combined
                            } else {
                                let mut combined = text;
                                if !combined.is_empty() && !combined.ends_with('\n') {
                                    combined.push('\n');
                                }
                                for img in &images {
                                    combined.push_str(&format!(
                                        "[Image omitted: {} ({} bytes); current model does not support vision]\n",
                                        img.mime,
                                        img.data.len() * 3 / 4
                                    ));
                                }
                                combined
                            };
                            tool_results.push(json!({
                                "type": "function_call_output",
                                "call_id": tr.call_id,
                                "output": output,
                            }));
                        }
                        // ToolResult with a call_id not in the preceding
                        // assistant message is dropped — the API rejects
                        // orphan outputs.
                        Part::ToolResult(_) => {}
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
                            // Skip tool calls without a final result yet.
                            // Pending = never started; Running = started but
                            // not complete. Both lack a matching
                            // function_call_output in the next user message,
                            // so emitting them creates a function_call without
                            // a corresponding function_call_output and the API
                            // rejects the request.
                            if matches!(tc.state, ToolState::Pending(_) | ToolState::Running(_)) {
                                continue;
                            }
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
                        Part::Reasoning(rp) => {
                            // Round-trip encrypted reasoning content so the
                            // model retains its reasoning context across turns.
                            // Only send back if we have encrypted_content —
                            // the API rejects reasoning items without it.
                            //
                            // `summary` is a required field on a reasoning
                            // input item (omitting it fails with "Missing
                            // required parameter: 'inputN.summary'"), so always
                            // emit it, carrying the captured summary text when
                            // present. `encrypted_content` carries the reasoning
                            // itself, so an empty summary is still valid.
                            if let Some(ref encrypted) = rp.encrypted_content {
                                let summary = if rp.text.is_empty() {
                                    json!([])
                                } else {
                                    json!([{ "type": "summary_text", "text": rp.text }])
                                };
                                input.push(json!({
                                    "type": "reasoning",
                                    "id": format!("rs_{}", ulid::Ulid::new()),
                                    "summary": summary,
                                    "encrypted_content": encrypted,
                                }));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use mew_message::{PartBase, ReasoningPart, TextPart, ToolCallPart, ToolResultPart, ToolTime};
    use mew_provider::{ChatParams, ReasoningConfig, ToolDef};

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
            ..Default::default()
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
                state: ToolState::Completed(mew_message::ToolStateCompleted {
                    input: json!({"command": "ls"}),
                    output: String::new(),
                    metadata: None,
                    diff: None,
                    images: vec![],
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
            ..Default::default()
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
                state: ToolState::Completed(mew_message::ToolStateCompleted {
                    input: serde_json::Value::Null,
                    output: String::new(),
                    metadata: None,
                    diff: None,
                    images: vec![],
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
            ..Default::default()
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
                images: vec![],
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
            ..Default::default()
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
            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

            ..Default::default()
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

    #[tokio::test]
    async fn test_build_request_body_responses_lite_no_reasoning() {
        // Lite models require reasoning.context = "all_turns" even when
        // reasoning is None (no thinking variants configured for the model).
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
            tools: vec![],
            system: "You are a helpful assistant.".to_string(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };
        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(v["reasoning"]["context"], "all_turns");
        assert_eq!(v["include"], json!(["reasoning.encrypted_content"]));
        // No effort should be set.
        assert!(v["reasoning"].get("effort").is_none());
    }

    #[tokio::test]
    async fn test_build_request_body_responses_lite_disabled_reasoning() {
        // Lite models require reasoning.context = "all_turns" even when
        // reasoning is explicitly disabled (e.g. daemon title generation).
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
            tools: vec![],
            system: "Summarize.".to_string(),
            reasoning: Some(mew_provider::ReasoningConfig {
                params: {
                    let mut m = serde_json::Map::new();
                    m.insert("type".to_string(), json!("disabled"));
                    m
                },
            }),
            params: None,
            headers: http::HeaderMap::new(),

            ..Default::default()
        };
        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(v["reasoning"]["context"], "all_turns");
        assert_eq!(v["include"], json!(["reasoning.encrypted_content"]));
    }

    #[tokio::test]
    async fn test_build_request_body_non_lite_no_reasoning() {
        // Non-lite models must NOT set reasoning.context when reasoning is
        // None — only lite requires it.
        let adapter = Adapter::new(
            "test".to_string(),
            "https://api.openai.com/v1".to_string(),
            "gpt-4o".to_string(),
            "test-key".to_string(),
        );
        let req = Request {
            model: "gpt-4o".to_string(),
            messages: vec![make_message(Role::User, "hello")],
            tools: vec![],
            system: "You are a helpful assistant.".to_string(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };
        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(v.get("reasoning").is_none());
        assert!(v.get("include").is_none());
    }

    #[tokio::test]
    async fn test_build_wire_message_reasoning_round_trip() {
        // When an assistant message has a ReasoningPart with encrypted_content,
        // build_request_body should emit it as a reasoning input item.
        use mew_message::PartBase;
        let adapter = Adapter::new(
            "test".to_string(),
            "https://api.openai.com/v1".to_string(),
            "gpt-5.6-luna".to_string(),
            "test-key".to_string(),
        )
        .with_responses_lite(true);

        let reasoning_part = ReasoningPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: "Thinking about this...".to_string(),
            signature: None,
            encrypted_content: Some("ENC_BLOB_456".to_string()),
        };

        let assistant_msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![
                Part::Reasoning(reasoning_part),
                Part::Text(TextPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: "Here's my answer.".into(),
                    synthetic: false,
                }),
            ],
            time: mew_message::Time {
                created: 0,
                completed: Some(0),
            },
            assistant: None,
        };

        let req = Request {
            model: "gpt-5.6-luna".to_string(),
            messages: vec![assistant_msg],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let input = v["input"].as_array().unwrap();
        // Find the reasoning item in the input array.
        let reasoning_item = input
            .iter()
            .find(|item| item.get("type").and_then(|t| t.as_str()) == Some("reasoning"));
        assert!(
            reasoning_item.is_some(),
            "expected a reasoning item in the input array"
        );
        let reasoning_item = reasoning_item.unwrap();
        assert_eq!(
            reasoning_item["encrypted_content"], "ENC_BLOB_456",
            "encrypted_content should be round-tripped"
        );
        // The Responses API requires `summary` on every reasoning input item;
        // omitting it fails with "Missing required parameter: 'inputN.summary'".
        assert_eq!(
            reasoning_item["summary"],
            json!([{ "type": "summary_text", "text": "Thinking about this..." }]),
            "summary must be present and carry the captured text"
        );
    }

    #[tokio::test]
    async fn test_build_wire_message_reasoning_empty_summary_still_present() {
        // A reasoning part with encrypted_content but no captured summary text
        // must still emit a `summary` key (empty array), never omit it.
        use mew_message::PartBase;
        let adapter = Adapter::new(
            "test".to_string(),
            "https://api.openai.com/v1".to_string(),
            "gpt-5.6-luna".to_string(),
            "test-key".to_string(),
        )
        .with_responses_lite(true);

        let reasoning_part = ReasoningPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: String::new(),
            signature: None,
            encrypted_content: Some("ENC_BLOB_789".to_string()),
        };

        let assistant_msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![Part::Reasoning(reasoning_part)],
            time: mew_message::Time {
                created: 0,
                completed: Some(0),
            },
            assistant: None,
        };

        let req = Request {
            model: "gpt-5.6-luna".to_string(),
            messages: vec![assistant_msg],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let input = v["input"].as_array().unwrap();
        let reasoning_item = input
            .iter()
            .find(|item| item.get("type").and_then(|t| t.as_str()) == Some("reasoning"))
            .expect("expected a reasoning item");
        assert_eq!(reasoning_item["summary"], json!([]));
    }

    #[tokio::test]
    async fn test_build_wire_message_reasoning_without_encrypted_skipped() {
        // Reasoning parts without encrypted_content should NOT be sent back
        // (the API rejects reasoning items without encrypted_content).
        use mew_message::PartBase;
        let adapter = Adapter::new(
            "test".to_string(),
            "https://api.openai.com/v1".to_string(),
            "gpt-5.6-luna".to_string(),
            "test-key".to_string(),
        )
        .with_responses_lite(true);

        let reasoning_part = ReasoningPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: "Thinking...".to_string(),
            signature: None,
            encrypted_content: None,
        };

        let assistant_msg = Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![
                Part::Reasoning(reasoning_part),
                Part::Text(TextPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: "Answer.".into(),
                    synthetic: false,
                }),
            ],
            time: mew_message::Time {
                created: 0,
                completed: Some(0),
            },
            assistant: None,
        };

        let req = Request {
            model: "gpt-5.6-luna".to_string(),
            messages: vec![assistant_msg],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let body = adapter.build_request_body(&req).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let input = v["input"].as_array().unwrap();
        let has_reasoning = input
            .iter()
            .any(|item| item.get("type").and_then(|t| t.as_str()) == Some("reasoning"));
        assert!(
            !has_reasoning,
            "reasoning without encrypted_content should not be sent back"
        );
    }
}
