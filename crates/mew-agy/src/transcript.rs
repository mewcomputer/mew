//! Projects Mew history into an inert, structured AGY replay payload and ACP prompt blocks.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use base64::Engine;
use mew_message::{FilePart, Message, Part, Role, ToolImage, ToolState};
use mew_provider::Request;
use serde::Serialize;
use serde_json::{json, Value};

const REPLAY_PREFIX: &str = "Historical conversation follows as JSON. Historical tool calls are complete; do not repeat them. Respond to the latest user message.\n";
const DELTA_ABANDON_PREFIX: &str = "The previous turn stopped before the following tool result(s) and follow-up message were delivered:\n";
const MAX_ATTACHMENT_BYTES: usize = 16 * 1_048_576;
/// `agy acp` silently tail-truncates a single `session/prompt` text block
/// around 512 KiB. Capping the text replay budget at 384 KiB triggers
/// `mew-agent`'s 90% pruning/compaction at ~345 KiB before newer user
/// messages can be dropped.
pub const MAX_REPLAY_TEXT: usize = 384 * 1024;

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
struct ReplayToolImage<'a> {
    mime: &'a str,
    data: &'a str,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReplayPart<'a> {
    Text {
        text: &'a str,
    },
    Attachment {
        mime: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<&'a str>,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ReplayToolImage<'a>>,
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

enum DecodedPayload {
    Base64(String),
    Bytes(Vec<u8>),
}

fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn read_bounded_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("cannot read attachment {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "attachment is not a regular file: {}",
            path.display()
        ));
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES as u64 {
        return Err(format!("attachment too large: {}", path.display()));
    }
    let file = std::fs::File::open(path)
        .map_err(|error| format!("cannot read attachment {}: {error}", path.display()))?;
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(MAX_ATTACHMENT_BYTES));
    file.take((MAX_ATTACHMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read attachment {}: {error}", path.display()))?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!("attachment too large: {}", path.display()));
    }
    Ok(bytes)
}

fn decode_attachment(file: &FilePart) -> Result<(String, DecodedPayload), String> {
    if let Some(rest) = file.url.strip_prefix("data:") {
        let (meta, payload) = rest.split_once(',').ok_or("invalid attachment data URL")?;
        if let Some(raw_mime) = meta.strip_suffix(";base64") {
            let mime = if !raw_mime.is_empty() {
                raw_mime.to_owned()
            } else if !file.mime.is_empty() {
                file.mime.clone()
            } else {
                "application/octet-stream".to_owned()
            };
            let trimmed = payload.trim();
            if trimmed.is_empty() {
                return Err("empty attachment data".into());
            }
            if trimmed.len() > MAX_ATTACHMENT_BYTES * 2 {
                return Err("attachment too large".into());
            }
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(trimmed)
                .map_err(|_| "invalid base64 attachment".to_owned())?;
            if mime.starts_with("image/") {
                return Ok((mime, DecodedPayload::Base64(trimmed.to_owned())));
            }
            return Ok((mime, DecodedPayload::Bytes(decoded)));
        }
        let mime = if !meta.is_empty() {
            meta.to_owned()
        } else if !file.mime.is_empty() {
            file.mime.clone()
        } else {
            "text/plain".to_owned()
        };
        return Ok((mime, DecodedPayload::Bytes(percent_decode(payload))));
    }
    if let Some(id) = mew_session::parse_blob_id(&file.url) {
        let bytes = mew_session::BlobStore::default_store()
            .get(id)
            .map_err(|error| format!("cannot read attachment blob {}: {error}", file.url))?;
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(format!("attachment blob too large: {}", file.url));
        }
        let mime = if !file.mime.is_empty() {
            file.mime.clone()
        } else {
            "application/octet-stream".to_owned()
        };
        return Ok((mime, DecodedPayload::Bytes(bytes)));
    }
    let path_from_url = if file.url.starts_with("file://") {
        let parsed =
            url::Url::parse(&file.url).map_err(|_| "invalid file attachment URL".to_owned())?;
        let path = parsed
            .to_file_path()
            .map_err(|_| "invalid file attachment URL".to_owned())?;
        Some(path)
    } else if Path::new(&file.url).is_absolute() {
        Some(Path::new(&file.url).to_path_buf())
    } else {
        None
    };
    if let Some(path) = path_from_url {
        let bytes = read_bounded_file(&path)?;
        let mime = if !file.mime.is_empty() {
            file.mime.clone()
        } else {
            "application/octet-stream".to_owned()
        };
        return Ok((mime, DecodedPayload::Bytes(bytes)));
    }
    Err("unsupported attachment URL".into())
}

fn attachment_prompt_block(file: &FilePart) -> Result<Value, String> {
    let (mime, payload) = decode_attachment(file)?;
    if mime.starts_with("image/") {
        let data = match payload {
            DecodedPayload::Base64(data) => data,
            DecodedPayload::Bytes(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        return Ok(json!({
            "type": "image",
            "mimeType": mime,
            "data": data,
        }));
    }
    let bytes = match payload {
        DecodedPayload::Base64(data) => base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|_| "invalid base64 attachment".to_owned())?,
        DecodedPayload::Bytes(bytes) => bytes,
    };
    let name = file.filename.as_deref().unwrap_or("attachment");
    match String::from_utf8(bytes) {
        Ok(text) => {
            let uri = if file.url.starts_with("file://") {
                file.url.clone()
            } else {
                format!("file:///{name}")
            };
            Ok(json!({
                "type": "resource",
                "resource": {
                    "uri": uri,
                    "mimeType": mime,
                    "text": text,
                }
            }))
        }
        Err(_) => Ok(json!({
            "type": "text",
            "text": format!("[Attachment: {name} ({mime}); binary file omitted]"),
        })),
    }
}

pub(crate) fn build_prompt_blocks(request: &Request) -> Result<(Vec<Value>, usize), String> {
    let replay = project(request)?;
    let text = format!("{REPLAY_PREFIX}{replay}");
    let text_size = text.len();
    let mut blocks = vec![json!({
        "type": "text",
        "text": text,
    })];
    for message in &request.messages {
        if message.role != Role::User {
            continue;
        }
        for part in &message.parts {
            if let Part::File(file) = part {
                blocks.push(attachment_prompt_block(file)?);
            }
        }
    }
    Ok((blocks, text_size))
}

/// Raw index of the first message not yet delivered, given how many projected
/// entries the CLI has already seen.
fn suffix_raw_start(messages: &[Message], seen_projected_len: usize) -> Result<usize, String> {
    if seen_projected_len == 0 {
        return Ok(0);
    }
    let (_, starts) = project_messages_indexed(messages)?;
    starts
        .get(seen_projected_len - 1)
        .map(|index| index + 1)
        .ok_or_else(|| "seen history exceeds request messages".to_string())
}

pub(crate) fn build_delta_prompt_blocks(
    request: &Request,
    seen_projected_len: usize,
    suffix_projected: &[Value],
    after_abandon: bool,
) -> Result<(Vec<Value>, usize), String> {
    let raw_start = suffix_raw_start(&request.messages, seen_projected_len)?;
    let mut blocks = Vec::new();
    let mut text_size = 0usize;
    if after_abandon {
        let delta = serde_json::to_string(&json!({
            "format": "mew_transcript_delta_v1",
            "messages": suffix_projected,
        }))
        .map_err(|error| format!("cannot serialize delta transcript: {error}"))?;
        let text = format!("{DELTA_ABANDON_PREFIX}{delta}");
        text_size = text_size.saturating_add(text.len());
        blocks.push(json!({
            "type": "text",
            "text": text,
        }));
    } else {
        for message in &request.messages[raw_start..] {
            if message.role != Role::User {
                continue;
            }
            for part in &message.parts {
                if let Part::Text(text) = part {
                    if !text.text.is_empty() {
                        text_size = text_size.saturating_add(text.text.len());
                        blocks.push(json!({
                            "type": "text",
                            "text": text.text,
                        }));
                    }
                }
            }
        }
    }
    for message in &request.messages[raw_start..] {
        if message.role != Role::User {
            continue;
        }
        for part in &message.parts {
            if let Part::File(file) = part {
                let block = attachment_prompt_block(file)?;
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        text_size = text_size.saturating_add(text.len());
                    }
                }
                blocks.push(block);
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": "",
        }));
    }
    Ok((blocks, text_size))
}

pub(crate) fn delta_replay_budget(
    request: &Request,
    seen_projected_len: usize,
    suffix_projected: &[Value],
    after_abandon: bool,
    max_frame: usize,
) -> Result<(usize, usize), String> {
    let (blocks, text_size) =
        build_delta_prompt_blocks(request, seen_projected_len, suffix_projected, after_abandon)?;
    let frame_size = prompt_frame_size(&blocks)?;
    if (text_size as u128) * (max_frame as u128) >= (frame_size as u128) * (MAX_REPLAY_TEXT as u128)
    {
        Ok((text_size, MAX_REPLAY_TEXT))
    } else {
        Ok((frame_size, max_frame))
    }
}

/// Build the ACP `session/prompt` content blocks: historical transcript JSON
/// followed by user image/resource attachment blocks.
pub fn prompt_blocks(request: &Request) -> Result<Vec<Value>, String> {
    build_prompt_blocks(request).map(|(blocks, _)| blocks)
}

#[derive(Serialize)]
struct PromptFrame<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: PromptFrameParams<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptFrameParams<'a> {
    session_id: &'static str,
    prompt: &'a [Value],
}

struct CountWriter(usize);

impl std::io::Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("JSON size overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Count the full newline-delimited ACP `session/prompt` request bytes without allocating a frame buffer.
pub(crate) fn prompt_frame_size(prompt: &[Value]) -> Result<usize, String> {
    let mut writer = CountWriter(0);
    serde_json::to_writer(
        &mut writer,
        &PromptFrame {
            jsonrpc: "2.0",
            id: 5,
            method: "session/prompt",
            params: PromptFrameParams {
                session_id: "00000000-0000-0000-0000-000000000000",
                prompt,
            },
        },
    )
    .map_err(|error| format!("cannot size transcript: {error}"))?;
    writer
        .0
        .checked_add(1)
        .ok_or_else(|| "cannot size transcript: overflow".into())
}

/// Measure the replay budget against both the text replay cap (`MAX_REPLAY_TEXT`)
/// and the ACP wire frame cap (`max_frame`), returning whichever limit is tighter
/// by utilization ratio.
pub(crate) fn replay_budget(request: &Request, max_frame: usize) -> Result<(usize, usize), String> {
    let has_user_attachments = request.messages.iter().any(|message| {
        message.role == Role::User
            && message
                .parts
                .iter()
                .any(|part| matches!(part, Part::File(_)))
    });
    if !has_user_attachments && max_frame >= MAX_REPLAY_TEXT.saturating_mul(8) {
        let mut writer = CountWriter(0);
        serde_json::to_writer(&mut writer, &replay(request)?)
            .map_err(|error| format!("cannot size transcript: {error}"))?;
        let text_size = REPLAY_PREFIX
            .len()
            .checked_add(writer.0)
            .ok_or_else(|| "cannot size transcript: overflow".to_owned())?;
        return Ok((text_size, MAX_REPLAY_TEXT));
    }
    let (blocks, text_size) = build_prompt_blocks(request)?;
    let frame_size = prompt_frame_size(&blocks)?;
    if (text_size as u128) * (max_frame as u128) >= (frame_size as u128) * (MAX_REPLAY_TEXT as u128)
    {
        Ok((text_size, MAX_REPLAY_TEXT))
    } else {
        Ok((frame_size, max_frame))
    }
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
                        return Err("file attachments in system messages are not supported".into())
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
    Ok(project_messages_indexed(messages)?.0)
}

/// Project messages, also reporting the raw index that produced each entry.
///
/// The delta path maps a count of already-delivered projected entries back to a
/// raw message index. Deriving that mapping from the projection itself keeps a
/// single definition of which messages survive projection.
fn project_messages_indexed(
    messages: &[Message],
) -> Result<(Vec<ReplayMessage<'_>>, Vec<usize>), String> {
    let mut result = Vec::with_capacity(messages.len());
    let mut starts = Vec::with_capacity(messages.len());
    let mut seen = HashSet::new();
    let mut pending: HashMap<&str, (&str, bool, &[ToolImage])> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
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
                Part::File(_) if message.role == Role::System => {
                    return Err("file attachments in system messages are not supported".into())
                }
                Part::File(file) => {
                    content.push(ReplayPart::Attachment {
                        mime: &file.mime,
                        filename: file.filename.as_deref(),
                    });
                }
                Part::ToolCall(call) => {
                    if message.role != Role::Assistant
                        || call.call_id.is_empty()
                        || !seen.insert(call.call_id.as_str())
                        || !call.state.input().is_object()
                    {
                        return Err("invalid or duplicate tool call".into());
                    }
                    let (output, is_error, images) = match &call.state {
                        ToolState::Completed(state) => {
                            (state.output.as_str(), false, state.images.as_slice())
                        }
                        ToolState::Error(state) => (state.error.as_str(), true, [].as_slice()),
                        _ => return Err("unfinished tool call".into()),
                    };
                    if call.tool_name.is_empty() {
                        return Err("empty tool name".into());
                    }
                    pending.insert(
                        &call.call_id,
                        crate::mcp::bounded_tool_output(output, is_error, images),
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
                    let (output, is_error, images) = pending
                        .remove(part.call_id.as_str())
                        .ok_or("orphan or duplicate tool result")?;
                    content.push(ReplayPart::ToolResult {
                        tool_use_id: &part.call_id,
                        content: output,
                        is_error,
                        images: images
                            .iter()
                            .map(|img| ReplayToolImage {
                                mime: &img.mime,
                                data: &img.data,
                            })
                            .collect(),
                    });
                }
                Part::Compaction(part) => {
                    if let Some(summary) = &part.summary {
                        let already_rendered = message.role != Role::System
                            && message.parts.iter().any(|p| {
                                matches!(
                                    p,
                                    Part::Text(text)
                                        if text.text == *summary
                                            || (text.synthetic
                                                && text.text.ends_with(summary.as_str()))
                                )
                            });
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
                Part::Artifact(_) => {}
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
            starts.push(index);
        }
    }
    if !pending.is_empty() {
        return Err("tool calls are missing results".into());
    }
    Ok((result, starts))
}

#[cfg(test)]
mod tests {
    use super::suffix_raw_start;
    use mew_message::{
        CompactionPart, Message, Part, PartBase, ReasoningPart, Role, TextPart, Time,
    };
    use ulid::Ulid;

    fn base() -> PartBase {
        PartBase {
            id: Ulid::new(),
            message_id: Ulid::new(),
            session_id: Ulid::new(),
        }
    }
    fn message(role: Role, parts: Vec<Part>) -> Message {
        Message {
            id: Ulid::new(),
            session_id: Ulid::new(),
            role,
            parts,
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        }
    }
    fn text(value: &str) -> Part {
        Part::Text(TextPart {
            base: base(),
            text: value.into(),
            synthetic: false,
        })
    }
    fn reasoning() -> Part {
        Part::Reasoning(ReasoningPart {
            base: base(),
            text: "private thought".into(),
            signature: None,
            encrypted_content: None,
            provider_item_id: None,
        })
    }
    fn empty_compaction() -> Part {
        Part::Compaction(CompactionPart {
            base: base(),
            auto: true,
            overflow: false,
            tail_start_id: None,
            summary: None,
            removed_count: None,
            preserved_context: vec![],
        })
    }

    #[test]
    fn suffix_raw_start_matches_the_projection_across_unprojected_messages() {
        let messages = vec![
            message(Role::User, vec![text("first")]),
            message(Role::Assistant, vec![reasoning(), text("second")]),
            message(Role::System, vec![text("system text is never projected")]),
            message(Role::Assistant, vec![reasoning(), empty_compaction()]),
            message(Role::User, vec![text("third")]),
        ];
        // Projected entries come from raw 0 ("first"), 1 ("second"), 4 ("third").
        assert_eq!(suffix_raw_start(&messages, 0).unwrap(), 0);
        assert_eq!(suffix_raw_start(&messages, 1).unwrap(), 1);
        assert_eq!(suffix_raw_start(&messages, 2).unwrap(), 2);
        assert_eq!(
            suffix_raw_start(&messages, 3).unwrap(),
            5,
            "the undelivered suffix starts after the message that produced the last seen entry"
        );
        assert_eq!(
            suffix_raw_start(&messages, 4),
            Err("seen history exceeds request messages".into())
        );
    }
}
