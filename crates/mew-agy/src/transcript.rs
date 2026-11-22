//! Projects Mew history into an inert, structured AGY replay payload.

use std::collections::{HashMap, HashSet};

use mew_message::{Message, Part, Role, ToolState};
use mew_provider::Request;
use serde_json::{json, Value};

/// Render a history as JSON. The caller must frame this as historical context,
/// not as fresh user instructions or tool requests.
pub fn project(request: &Request) -> Result<String, String> {
    let messages = project_messages(&request.messages)?;
    serde_json::to_string(&json!({
        "format": "mew_transcript_v1",
        "messages": messages,
    }))
    .map_err(|error| format!("cannot serialize transcript: {error}"))
}

/// Combine the request's system prompt and explicit system-message text for
/// the agent instructions. Reusing an AGY process requires this to remain identical.
pub fn system_instructions(request: &Request) -> Result<String, String> {
    fn collect<'a>(messages: &'a [Message], pieces: &mut Vec<&'a str>) -> Result<(), String> {
        for message in messages {
            for part in &message.parts {
                match part {
                    Part::Text(text) if message.role == Role::System => pieces.push(&text.text),
                    Part::Compaction(part) => collect(&part.preserved_context, pieces)?,
                    Part::File(_) if message.role == Role::System => {
                        return Err("file attachments are not supported".into())
                    }
                    Part::ToolCall(_) | Part::ToolResult(_) if message.role == Role::System => {
                        return Err("tool actions in system messages are not supported".into())
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    let mut pieces = Vec::new();
    if !request.system.is_empty() {
        pieces.push(request.system.as_str());
    }
    collect(&request.messages, &mut pieces)?;
    Ok(pieces.join("\n\n"))
}

fn project_messages(messages: &[Message]) -> Result<Vec<Value>, String> {
    let mut result = Vec::with_capacity(messages.len());
    let mut seen = HashSet::new();
    let mut pending: HashMap<&str, (&str, bool)> = HashMap::new();
    for message in messages {
        if !pending.is_empty() && message.role != Role::User {
            return Err("tool calls must be followed by their results".into());
        }
        let mut content = Vec::new();
        for part in &message.parts {
            match part {
                Part::Text(text) if message.role != Role::System => {
                    content.push(json!({"type":"text", "text":text.text}))
                }
                Part::Text(_) => {}
                Part::Reasoning(_) => {}
                Part::File(_) => return Err("file attachments are not supported".into()),
                Part::ToolCall(call) => {
                    if message.role != Role::Assistant
                        || call.call_id.is_empty()
                        || !seen.insert(call.call_id.as_str())
                        || !call.state.input().is_object()
                    {
                        return Err("invalid or duplicate tool call".into());
                    }
                    let (output, is_error) = match &call.state {
                        ToolState::Completed(state) if state.images.is_empty() => {
                            (state.output.as_str(), false)
                        }
                        ToolState::Completed(_) => {
                            return Err("tool images are not supported".into())
                        }
                        ToolState::Error(state) => (state.error.as_str(), true),
                        _ => return Err("unfinished tool call".into()),
                    };
                    if call.tool_name.is_empty() {
                        return Err("empty tool name".into());
                    }
                    pending.insert(&call.call_id, (output, is_error));
                    content.push(json!({"type":"tool_use", "id":call.call_id, "name":call.tool_name, "input":call.state.input()}));
                }
                Part::ToolResult(part) => {
                    if message.role != Role::User {
                        return Err("tool result has invalid role".into());
                    }
                    let (output, is_error) = pending
                        .remove(part.call_id.as_str())
                        .ok_or("orphan or duplicate tool result")?;
                    content.push(json!({"type":"tool_result", "tool_use_id":part.call_id, "content":output, "is_error":is_error}));
                }
                Part::Compaction(part) => {
                    if let Some(summary) = &part.summary {
                        let already_rendered = message.role != Role::System
                            && message
                                .parts
                                .iter()
                                .any(|p| matches!(p, Part::Text(text) if text.text == *summary));
                        if !already_rendered {
                            content.push(json!({"type":"compaction", "summary":summary}));
                        }
                    }
                    if !part.preserved_context.is_empty() {
                        content.push(json!({"type":"preserved_context", "messages":project_messages(&part.preserved_context)?}));
                    }
                }
            }
        }
        if message.role == Role::User && !pending.is_empty() {
            return Err("tool result batch is incomplete".into());
        }
        if !content.is_empty() {
            let role = match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            };
            result.push(json!({"role":role,"content":content}));
        }
    }
    if !pending.is_empty() {
        return Err("tool calls are missing results".into());
    }
    Ok(result)
}
