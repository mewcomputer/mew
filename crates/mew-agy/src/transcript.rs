//! Projects Mew history into an inert, structured AGY replay payload.

use std::collections::{HashMap, HashSet};

use mew_message::{Message, Part, Role, ToolState};
use mew_provider::Request;
use serde::Serialize;
use serde_json::Value;

pub(crate) const REPLAY_PREFIX: &str = "Historical conversation follows as JSON. Historical tool calls are complete; do not repeat them. Respond to the latest user message.\n";

#[derive(Serialize)]
struct Replay<'a> {
    format: &'static str,
    messages: Vec<ReplayMessage<'a>>,
}

#[derive(Serialize)]
struct ReplayMessage<'a> {
    role: &'static str,
    content: Vec<ReplayPart<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReplayPart<'a> {
    Text {
        text: &'a str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: &'a Value,
    },
    ToolResult {
        tool_use_id: &'a str,
        content: &'a str,
        is_error: bool,
    },
    Compaction {
        summary: &'a str,
    },
    PreservedContext {
        messages: Vec<ReplayMessage<'a>>,
    },
}

fn replay(request: &Request) -> Result<Replay<'_>, String> {
    Ok(Replay {
        format: "mew_transcript_v1",
        messages: project_messages(&request.messages)?,
    })
}

/// Render a history as JSON. The caller must frame this as historical context,
/// not as fresh user instructions or tool requests.
pub fn project(request: &Request) -> Result<String, String> {
    serde_json::to_string(&replay(request)?)
        .map_err(|error| format!("cannot serialize transcript: {error}"))
}

/// Count the full newline-delimited CLI input, including JSON nested in the
/// prompt string, without allocating a second serialized copy of the replay.
pub(crate) fn replay_size(request: &Request) -> Result<usize, String> {
    use std::io::{self, Write};

    struct Count {
        len: usize,
        escape: bool,
    }
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            for byte in bytes {
                let added = if !self.escape {
                    1
                } else {
                    match byte {
                        b'"' | b'\\' | b'\x08' | b'\x0c' | b'\n' | b'\r' | b'\t' => 2,
                        0..=31 => 6,
                        _ => 1,
                    }
                };
                self.len = self
                    .len
                    .checked_add(added)
                    .ok_or_else(|| io::Error::other("replay size overflow"))?;
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let document = replay(request)?;
    let mut prefix = Count {
        len: 0,
        escape: false,
    };
    serde_json::to_writer(&mut prefix, REPLAY_PREFIX).map_err(|error| error.to_string())?;
    let mut replay = Count {
        len: 0,
        escape: true,
    };
    serde_json::to_writer(&mut replay, &document).map_err(|error| error.to_string())?;
    let wrapper = b"{\"event\":\"user\",\"message\":{\"content\":\"\"}}\n".len();
    wrapper
        .checked_add(prefix.len - 2)
        .and_then(|size| size.checked_add(replay.len))
        .ok_or_else(|| "replay size overflow".into())
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

fn project_messages(messages: &[Message]) -> Result<Vec<ReplayMessage<'_>>, String> {
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
                    content.push(ReplayPart::Text { text: &text.text })
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
                    pending.insert(
                        &call.call_id,
                        crate::mcp::bounded_tool_output(output, is_error),
                    );
                    content.push(ReplayPart::ToolUse {
                        id: &call.call_id,
                        name: &call.tool_name,
                        input: call.state.input(),
                    });
                }
                Part::ToolResult(part) => {
                    if message.role != Role::User {
                        return Err("tool result has invalid role".into());
                    }
                    let (output, is_error) = pending
                        .remove(part.call_id.as_str())
                        .ok_or("orphan or duplicate tool result")?;
                    content.push(ReplayPart::ToolResult {
                        tool_use_id: &part.call_id,
                        content: output,
                        is_error,
                    });
                }
                Part::Compaction(part) => {
                    if let Some(summary) = &part.summary {
                        let already_rendered = message.role != Role::System
                            && message
                                .parts
                                .iter()
                                .any(|p| matches!(p, Part::Text(text) if text.text == *summary));
                        if !already_rendered {
                            content.push(ReplayPart::Compaction { summary });
                        }
                    }
                    if !part.preserved_context.is_empty() {
                        content.push(ReplayPart::PreservedContext {
                            messages: project_messages(&part.preserved_context)?,
                        });
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
            result.push(ReplayMessage { role, content });
        }
    }
    if !pending.is_empty() {
        return Err("tool calls are missing results".into());
    }
    Ok(result)
}
