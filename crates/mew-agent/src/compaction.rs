use std::collections::HashSet;

use chrono::Utc;
use mew_message::{Message, Part, PartBase, Role, TextPart, ToolState};
use mew_provider::Request;
use ulid::Ulid;

const MAX_TOOL_VALUE_CHARS: usize = 1_500;
const MAX_TRANSCRIPT_CHARS: usize = 40_000;
const MAX_OPENING_TASK_CHARS: usize = 600;
const MAX_COMMAND_CHARS: usize = 120;
const MAX_ERROR_LINE_CHARS: usize = 120;
const MAX_FAILED_COMMANDS: usize = 5;
const STALE_TOOL_OUTPUT_PRUNE_BYTES: usize = 2_000;
pub(crate) const OFFLOAD_TOOL_OUTPUT_BYTES: usize = 48_000;
const OFFLOAD_HEAD_CHARS: usize = 6_000;
const OFFLOAD_TAIL_CHARS: usize = 4_000;
const PRUNED_TOOL_OUTPUT_PREFIX: &str = "[Pruned tool result:";
const MAX_TOOL_LABEL_CHARS: usize = 80;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactTrail {
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub failed_commands: Vec<String>,
}

impl ArtifactTrail {
    pub(crate) fn format_blocks(&self) -> String {
        let mut sections = Vec::new();
        if !self.modified_files.is_empty() {
            let body = self
                .modified_files
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("<modified-files>\n{body}\n</modified-files>"));
        }
        if !self.read_files.is_empty() {
            let body = self
                .read_files
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("<read-files>\n{body}\n</read-files>"));
        }
        if !self.failed_commands.is_empty() {
            let body = self
                .failed_commands
                .iter()
                .map(|entry| format!("- {entry}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("<failed-commands>\n{body}\n</failed-commands>"));
        }
        sections.join("\n\n")
    }
}

fn push_unique(list: &mut Vec<String>, item: String) {
    let trimmed = item.trim();
    if trimmed.is_empty() {
        return;
    }
    if !list.iter().any(|existing| existing == trimmed) {
        list.push(trimmed.to_string());
    }
}

fn parse_tag_block(summary: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut items = Vec::new();
    let mut in_block = false;
    for raw_line in summary.lines() {
        let line = raw_line.trim();
        if line == open {
            in_block = true;
            continue;
        }
        if line == close {
            in_block = false;
            continue;
        }
        if in_block {
            let item = line.strip_prefix("- ").unwrap_or(line).trim();
            if !item.is_empty() {
                items.push(item.to_string());
            }
        }
    }
    items
}

fn strip_artifact_blocks(summary: &str) -> String {
    let mut kept = Vec::new();
    let mut active_close: Option<&str> = None;
    for line in summary.lines() {
        let trimmed = line.trim();
        if let Some(close) = active_close {
            if trimmed == close {
                active_close = None;
            }
            continue;
        }
        match trimmed {
            "<modified-files>" => {
                active_close = Some("</modified-files>");
                continue;
            }
            "<read-files>" => {
                active_close = Some("</read-files>");
                continue;
            }
            "<failed-commands>" => {
                active_close = Some("</failed-commands>");
                continue;
            }
            _ => kept.push(line),
        }
    }
    kept.join("\n").trim().to_string()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}…")
}

fn parse_hashline_patch_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        let trimmed = line.trim();
        let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
            continue;
        };
        if let Some((path, hash)) = inner.rsplit_once('#') {
            let path = path.trim();
            if !path.is_empty() && !hash.trim().is_empty() {
                push_unique(&mut paths, path.to_string());
            }
        }
    }
    paths
}

pub(crate) fn extract_artifact_trail(dropped: &[Message]) -> ArtifactTrail {
    let mut trail = ArtifactTrail::default();

    for message in dropped {
        for part in &message.parts {
            match part {
                Part::Compaction(cp) => {
                    if let Some(summary) = cp.summary.as_deref() {
                        for path in parse_tag_block(summary, "modified-files") {
                            push_unique(&mut trail.modified_files, path);
                        }
                        for path in parse_tag_block(summary, "read-files") {
                            push_unique(&mut trail.read_files, path);
                        }
                    }
                }
                Part::ToolCall(tc) => {
                    let input = tc.state.input();
                    match tc.tool_name.as_str() {
                        "read" | "read_file" => {
                            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                                push_unique(&mut trail.read_files, path.to_string());
                            }
                        }
                        "write" | "edit" | "edit_str_replace" => {
                            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                                push_unique(&mut trail.modified_files, path.to_string());
                            }
                        }
                        "edit_hashline" => {
                            if let Some(patch) = input.get("patch").and_then(|v| v.as_str()) {
                                for path in parse_hashline_patch_paths(patch) {
                                    push_unique(&mut trail.modified_files, path);
                                }
                            }
                        }
                        "bash" | "shell" | "shell_background" => {
                            if let ToolState::Error(err) = &tc.state {
                                let command = input
                                    .get("command")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&tc.tool_name);
                                let first_err_line = err
                                    .error
                                    .lines()
                                    .map(str::trim)
                                    .find(|line| !line.is_empty())
                                    .unwrap_or("failed");
                                let entry = format!(
                                    "{}: {}",
                                    truncate_chars(command.trim(), MAX_COMMAND_CHARS),
                                    truncate_chars(first_err_line, MAX_ERROR_LINE_CHARS)
                                );
                                trail.failed_commands.push(entry);
                                if trail.failed_commands.len() > MAX_FAILED_COMMANDS {
                                    let excess = trail.failed_commands.len() - MAX_FAILED_COMMANDS;
                                    trail.failed_commands.drain(..excess);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let modified: HashSet<String> = trail.modified_files.iter().cloned().collect();
    trail.read_files.retain(|path| !modified.contains(path));
    trail
}

pub(crate) fn extract_previous_summary(dropped: &[Message]) -> Option<String> {
    for message in dropped.iter().rev() {
        for part in &message.parts {
            if let Part::Compaction(cp) = part {
                if let Some(raw) = cp.summary.as_deref() {
                    let stripped = strip_artifact_blocks(raw);
                    if !stripped.is_empty() {
                        return Some(stripped);
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn extract_opening_task(dropped: &[Message]) -> Option<String> {
    for message in dropped {
        if message.role != Role::User {
            continue;
        }
        if message
            .parts
            .iter()
            .any(|part| matches!(part, Part::Compaction(_)))
        {
            continue;
        }
        let texts: Vec<&str> = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) if !text.synthetic && !text.text.trim().is_empty() => {
                    Some(text.text.trim())
                }
                _ => None,
            })
            .collect();
        if !texts.is_empty() {
            return Some(truncate_chars(&texts.join("\n"), MAX_OPENING_TASK_CHARS));
        }
    }
    None
}

fn format_message_for_transcript(message: &Message) -> Option<String> {
    let role_tag = match message.role {
        Role::User => "User",
        Role::Assistant => "Assistant",
        Role::System => "System",
    };
    let mut lines = Vec::new();
    for part in &message.parts {
        match part {
            Part::Compaction(_) => continue,
            Part::Text(tp) => {
                let text = tp.text.trim();
                if !text.is_empty() {
                    lines.push(format!(
                        "[{role_tag}] {}",
                        truncate_chars(text, MAX_TOOL_VALUE_CHARS)
                    ));
                }
            }
            Part::Reasoning(_) => {}
            Part::ToolCall(tc) => {
                let input_str = serde_json::to_string(tc.state.input()).unwrap_or_default();
                let outcome = match &tc.state {
                    ToolState::Completed(s) => {
                        format!(
                            "output: {}",
                            truncate_chars(s.output.trim(), MAX_TOOL_VALUE_CHARS)
                        )
                    }
                    ToolState::Error(s) => {
                        format!(
                            "error: {}",
                            truncate_chars(s.error.trim(), MAX_TOOL_VALUE_CHARS)
                        )
                    }
                    ToolState::Running(s) => {
                        format!(
                            "running: {}",
                            truncate_chars(s.output.trim(), MAX_TOOL_VALUE_CHARS)
                        )
                    }
                    ToolState::Pending(_) => "pending".to_string(),
                };
                lines.push(format!(
                    "[ToolCall {}] input: {} | {}",
                    tc.tool_name,
                    truncate_chars(&input_str, 400),
                    outcome
                ));
            }
            Part::ToolResult(tr) => {
                lines.push(format!("[ToolResult {}]", tr.call_id));
            }
            Part::File(file) => {
                let name = file.filename.as_deref().unwrap_or(&file.url);
                lines.push(format!("[File: {} ({})]", name, file.mime));
            }
            Part::Artifact(artifact) => {
                lines.push(format!("[Artifact: {} ({})]", artifact.title, artifact.url));
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

pub(crate) fn build_summarization_request(
    dropped: &[Message],
    previous_summary: Option<&str>,
    opening_task: Option<&str>,
    focus_instructions: Option<&str>,
    model_id: &str,
) -> Request {
    let system = mew_prompts::compact::prompt().to_string();

    // Keep the most recent dropped turns up to MAX_TRANSCRIPT_CHARS so the
    // summarization prompt stays safely inside context limits.
    let mut chunks = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;
    for message in dropped.iter().rev() {
        let Some(formatted) = format_message_for_transcript(message) else {
            continue;
        };
        let char_len = formatted.chars().count();
        if used + char_len > MAX_TRANSCRIPT_CHARS && !chunks.is_empty() {
            omitted += 1;
            continue;
        }
        used += char_len + 2;
        chunks.push(formatted);
    }
    chunks.reverse();

    let mut prompt_sections = Vec::new();
    if let Some(opening) = opening_task.filter(|s| !s.trim().is_empty()) {
        prompt_sections.push(format!("## Initial Task\n{}", opening.trim()));
    }
    if let Some(previous) = previous_summary.filter(|s| !s.trim().is_empty()) {
        prompt_sections.push(format!("## Previous Summary\n{}", previous.trim()));
    }
    if let Some(focus) = focus_instructions.filter(|s| !s.trim().is_empty()) {
        prompt_sections.push(format!("## Focus Instructions\n{}", focus.trim()));
    }
    let transcript_body = if omitted > 0 {
        format!(
            "[... {omitted} earlier dropped messages omitted for summarization budget ...]\n\n{}",
            chunks.join("\n\n")
        )
    } else {
        chunks.join("\n\n")
    };
    prompt_sections.push(format!("## Dropped Conversation\n{transcript_body}"));

    let msg_id = Ulid::new();
    let user_msg = Message {
        id: msg_id,
        session_id: Ulid::new(),
        role: Role::User,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: msg_id,
                session_id: Ulid::new(),
            },
            text: prompt_sections.join("\n\n"),
            synthetic: false,
        })],
        time: mew_message::Time {
            created: Utc::now().timestamp_millis(),
            completed: None,
        },
        assistant: None,
    };

    Request {
        session_id: None,
        cancel_token: Default::default(),
        model: model_id.to_string(),
        messages: vec![user_msg],
        tools: Vec::new(),
        system,
        reasoning: None,
        params: None,
        headers: http::HeaderMap::new(),
        supports_vision: false,
    }
}

fn extract_tool_label(input: &serde_json::Value) -> Option<String> {
    for key in ["path", "file_path", "command", "pattern"] {
        if let Some(raw) = input.get(key).and_then(|v| v.as_str()) {
            let single_line = raw.replace(['\r', '\n'], " ");
            let trimmed = single_line.trim();
            if !trimmed.is_empty() {
                return Some(truncate_chars(trimmed, MAX_TOOL_LABEL_CHARS));
            }
        }
    }
    None
}

/// Return whether `output` has already been replaced with a pruned-tool stub.
pub(crate) fn is_pruned_tool_output(output: &str) -> bool {
    output.starts_with(PRUNED_TOOL_OUTPUT_PREFIX)
}

/// Replace bulky completed tool outputs strictly older than the compaction
/// boundary with compact stubs while preserving `ToolCall` / `ToolResult`
/// parts and pairing IDs.
pub(crate) fn prune_stale_tool_outputs(messages: &mut [Message], keep_turns: usize) -> usize {
    let start = crate::turn::compaction_start(messages, keep_turns);
    if start == 0 {
        return 0;
    }

    let mut pruned = 0usize;
    for message in &mut messages[..start] {
        for part in &mut message.parts {
            let Part::ToolCall(tc) = part else {
                continue;
            };
            let ToolState::Completed(completed) = &mut tc.state else {
                continue;
            };
            if completed.output.len() <= STALE_TOOL_OUTPUT_PRUNE_BYTES
                || completed.output.starts_with(PRUNED_TOOL_OUTPUT_PREFIX)
            {
                continue;
            }
            let lines = completed.output.lines().count();
            let bytes = completed.output.len();
            completed.output = match extract_tool_label(&completed.input) {
                Some(label) => format!(
                    "{PRUNED_TOOL_OUTPUT_PREFIX} {}({}), {lines} lines / {bytes} bytes — re-read if needed]",
                    tc.tool_name, label
                ),
                None => format!(
                    "{PRUNED_TOOL_OUTPUT_PREFIX} {}, {lines} lines / {bytes} bytes — re-read if needed]",
                    tc.tool_name
                ),
            };
            pruned += 1;
        }
    }
    pruned
}

fn sanitize_call_id(call_id: &str) -> String {
    let sanitized: String = call_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.trim_matches('_').is_empty() {
        "tool".to_string()
    } else {
        sanitized
    }
}

/// Offload oversized tool output at ingestion time into `<session_dir>/tool_outputs/<call_id>.txt`
/// and return a bounded head+tail preview with a pointer for targeted reads.
pub(crate) async fn offload_large_tool_output(
    call_id: &str,
    output: &str,
    session_dir: Option<&std::path::Path>,
) -> String {
    if output.len() <= OFFLOAD_TOOL_OUTPUT_BYTES {
        return output.to_string();
    }

    let head_end = output
        .char_indices()
        .nth(OFFLOAD_HEAD_CHARS)
        .map(|(idx, _)| idx)
        .unwrap_or(output.len());
    let tail_start = output
        .char_indices()
        .rev()
        .nth(OFFLOAD_TAIL_CHARS.saturating_sub(1))
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    if tail_start <= head_end {
        return output.to_string();
    }

    let head = &output[..head_end];
    let middle = &output[head_end..tail_start];
    let tail = &output[tail_start..];
    let total_lines = output.lines().count();
    let total_bytes = output.len();
    let omitted_lines = middle.lines().count();
    let omitted_bytes = middle.len();

    if let Some(dir) = session_dir {
        let outputs_dir = dir.join("tool_outputs");
        let file_path = outputs_dir.join(format!("{}.txt", sanitize_call_id(call_id)));
        match tokio::fs::create_dir_all(&outputs_dir).await {
            Ok(()) => match tokio::fs::write(&file_path, output).await {
                Ok(()) => {
                    return format!(
                        "[Output offloaded: {total_lines} lines / {total_bytes} bytes saved to {} — use read tool with offset/limit to inspect]\n\n{head}\n\n[... middle omitted ({omitted_lines} lines / {omitted_bytes} bytes) ...]\n\n{tail}",
                        file_path.display()
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        path = %file_path.display(),
                        error = %error,
                        "failed to write offloaded tool output; falling back to head/tail truncation"
                    );
                }
            },
            Err(error) => {
                tracing::warn!(
                    path = %outputs_dir.display(),
                    error = %error,
                    "failed to create tool_outputs directory; falling back to head/tail truncation"
                );
            }
        }
    }

    format!(
        "[Output truncated: {total_lines} lines / {total_bytes} bytes — showing head and tail]\n\n{head}\n\n[... middle omitted ({omitted_lines} lines / {omitted_bytes} bytes) ...]\n\n{tail}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{
        CompactionPart, Time, ToolCallPart, ToolStateCompleted, ToolStateError, ToolTime,
    };

    fn base_msg(role: Role, parts: Vec<Part>) -> Message {
        let id = Ulid::new();
        Message {
            id,
            session_id: Ulid::new(),
            role,
            parts,
            time: Time {
                created: 0,
                completed: Some(0),
            },
            assistant: None,
        }
    }

    fn text_part(text: &str, synthetic: bool) -> Part {
        Part::Text(TextPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: Ulid::new(),
                session_id: Ulid::new(),
            },
            text: text.to_string(),
            synthetic,
        })
    }

    fn completed_tool(name: &str, input: serde_json::Value, output: &str) -> Part {
        Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: Ulid::new(),
                session_id: Ulid::new(),
            },
            tool_name: name.to_string(),
            call_id: Ulid::new().to_string(),
            state: ToolState::Completed(ToolStateCompleted {
                input,
                output: output.to_string(),
                metadata: None,
                diff: None,
                images: Vec::new(),
                time: ToolTime {
                    start: 0,
                    end: Some(1),
                },
            }),
            raw_input: String::new(),
        })
    }

    fn errored_tool(name: &str, input: serde_json::Value, error: &str) -> Part {
        Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: Ulid::new(),
                session_id: Ulid::new(),
            },
            tool_name: name.to_string(),
            call_id: Ulid::new().to_string(),
            state: ToolState::Error(ToolStateError {
                input,
                error: error.to_string(),
                time: ToolTime {
                    start: 0,
                    end: Some(1),
                },
            }),
            raw_input: String::new(),
        })
    }

    #[test]
    fn extract_artifact_trail_collects_reads_writes_hashlines_failed_commands_and_prior_compactions(
    ) {
        let prior_compaction = base_msg(
            Role::System,
            vec![Part::Compaction(CompactionPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
                    session_id: Ulid::new(),
                },
                auto: true,
                overflow: false,
                tail_start_id: None,
                summary: Some(
                    "## Goal & Constraints\nBuild feature\n\n<modified-files>\n- src/old.rs\n</modified-files>\n\n<read-files>\n- src/edited_later.rs\n- src/unchanged.rs\n</read-files>"
                        .to_string(),
                ),
                removed_count: Some(3),
                preserved_context: Vec::new(),
            })],
        );
        let assistant_tools = base_msg(
            Role::Assistant,
            vec![
                completed_tool(
                    "read",
                    serde_json::json!({"path": "src/lib.rs"}),
                    "contents",
                ),
                completed_tool(
                    "read_file",
                    serde_json::json!({"path": "src/edited_later.rs"}),
                    "contents",
                ),
                completed_tool("write", serde_json::json!({"path": "src/new.rs"}), "ok"),
                completed_tool(
                    "edit_hashline",
                    serde_json::json!({"patch": "[src/edited_later.rs#AB12]\nADD\n+hello\n[src/another.rs#CD34]\nDEL"}),
                    "ok",
                ),
                errored_tool(
                    "bash",
                    serde_json::json!({"command": "cargo test"}),
                    "error[E0425]: missing foo\nmore details",
                ),
            ],
        );

        let trail = extract_artifact_trail(&[prior_compaction, assistant_tools]);
        assert_eq!(
            trail.modified_files,
            vec![
                "src/old.rs",
                "src/new.rs",
                "src/edited_later.rs",
                "src/another.rs"
            ]
        );
        assert_eq!(trail.read_files, vec!["src/unchanged.rs", "src/lib.rs"]);
        assert_eq!(
            trail.failed_commands,
            vec!["cargo test: error[E0425]: missing foo"]
        );

        let formatted = trail.format_blocks();
        assert!(formatted.contains("<modified-files>\n- src/old.rs"));
        assert!(formatted.contains("<read-files>\n- src/unchanged.rs"));
        assert!(formatted.contains("<failed-commands>\n- cargo test: error[E0425]: missing foo"));
    }

    #[test]
    fn extract_previous_summary_strips_artifact_tags_and_ignores_older_markers() {
        let older = base_msg(
            Role::System,
            vec![Part::Compaction(CompactionPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
                    session_id: Ulid::new(),
                },
                auto: true,
                overflow: false,
                tail_start_id: None,
                summary: Some("Older summary".into()),
                removed_count: Some(1),
                preserved_context: Vec::new(),
            })],
        );
        let latest = base_msg(
            Role::System,
            vec![Part::Compaction(CompactionPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
                    session_id: Ulid::new(),
                },
                auto: true,
                overflow: false,
                tail_start_id: None,
                summary: Some(
                    "## Goal & Constraints\nKeep moving\n\n<modified-files>\n- a.rs\n</modified-files>\n\n<failed-commands>\n- cmd: err\n</failed-commands>"
                        .into(),
                ),
                removed_count: Some(2),
                preserved_context: Vec::new(),
            })],
        );

        assert_eq!(
            extract_previous_summary(&[older, latest]),
            Some("## Goal & Constraints\nKeep moving".to_string())
        );
    }

    #[test]
    fn build_summarization_request_includes_opening_previous_focus_and_drops_compaction_markers() {
        let dropped = vec![
            base_msg(
                Role::User,
                vec![text_part("Initial task: refactor parser", false)],
            ),
            base_msg(
                Role::System,
                vec![Part::Compaction(CompactionPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id: Ulid::new(),
                        session_id: Ulid::new(),
                    },
                    auto: true,
                    overflow: false,
                    tail_start_id: None,
                    summary: Some(
                        "## Goal & Constraints\nPrior\n\n<read-files>\n- foo.rs\n</read-files>"
                            .into(),
                    ),
                    removed_count: Some(1),
                    preserved_context: Vec::new(),
                })],
            ),
            base_msg(
                Role::Assistant,
                vec![text_part("Updated ast module", false)],
            ),
        ];
        let previous = extract_previous_summary(&dropped);
        let opening = extract_opening_task(&dropped);
        let req = build_summarization_request(
            &dropped,
            previous.as_deref(),
            opening.as_deref(),
            Some("focus on parser edge cases"),
            "test-model",
        );

        assert!(req.tools.is_empty());
        assert_eq!(req.model, "test-model");
        assert!(req.system.contains("CONTEXT CHECKPOINT COMPACTION"));
        assert!(!req.system.contains("software engineering"));
        assert!(req.system.contains("## Goal"));
        assert!(req.system.contains("## Decisions"));
        let Part::Text(user_text) = &req.messages[0].parts[0] else {
            panic!("expected text part");
        };
        assert!(user_text
            .text
            .contains("## Initial Task\nInitial task: refactor parser"));
        assert!(user_text
            .text
            .contains("## Previous Summary\n## Goal & Constraints\nPrior"));
        assert!(user_text
            .text
            .contains("## Focus Instructions\nfocus on parser edge cases"));
        assert!(user_text.text.contains("[Assistant] Updated ast module"));
        assert!(!user_text.text.contains("<read-files>"));
    }

    #[test]
    fn prune_stale_tool_outputs_respects_keep_turns_preserves_pairing_and_is_idempotent() {
        let big_output = "line of output\n".repeat(300);
        let small_output = "ok".to_string();

        let old_big_call = completed_tool(
            "read",
            serde_json::json!({"path": "src/lib.rs"}),
            &big_output,
        );
        let old_big_call_id = match &old_big_call {
            Part::ToolCall(tc) => tc.call_id.clone(),
            _ => unreachable!(),
        };
        let old_small_call = completed_tool(
            "read",
            serde_json::json!({"path": "src/small.rs"}),
            &small_output,
        );
        let recent_big_call = completed_tool(
            "bash",
            serde_json::json!({"command": "cargo test"}),
            &big_output,
        );
        let recent_call_id = match &recent_big_call {
            Part::ToolCall(tc) => tc.call_id.clone(),
            _ => unreachable!(),
        };

        let mut messages = vec![
            base_msg(Role::User, vec![text_part("initial request", false)]),
            base_msg(Role::Assistant, vec![old_big_call, old_small_call]),
            base_msg(
                Role::User,
                vec![Part::ToolResult(mew_message::ToolResultPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id: Ulid::new(),
                        session_id: Ulid::new(),
                    },
                    call_id: old_big_call_id.clone(),
                })],
            ),
            base_msg(Role::User, vec![text_part("follow up", false)]),
            base_msg(Role::Assistant, vec![recent_big_call]),
            base_msg(
                Role::User,
                vec![Part::ToolResult(mew_message::ToolResultPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id: Ulid::new(),
                        session_id: Ulid::new(),
                    },
                    call_id: recent_call_id,
                })],
            ),
        ];

        // keep_turns = 2 keeps the final assistant ToolCall + ToolResult pair intact.
        let pruned = prune_stale_tool_outputs(&mut messages, 2);
        assert_eq!(pruned, 1);

        // Old big call was replaced with compact stub while preserving call_id and tool_name.
        let Part::ToolCall(tc_old_big) = &messages[1].parts[0] else {
            panic!("expected tool call");
        };
        assert_eq!(tc_old_big.call_id, old_big_call_id);
        let ToolState::Completed(old_big_state) = &tc_old_big.state else {
            panic!("expected completed");
        };
        assert!(old_big_state
            .output
            .starts_with("[Pruned tool result: read(src/lib.rs), 300 lines /"));
        assert!(old_big_state.output.contains("re-read if needed]"));

        // Matching ToolResult remains intact and paired.
        assert!(matches!(
            &messages[2].parts[0],
            Part::ToolResult(tr) if tr.call_id == old_big_call_id
        ));

        // Old small output was left untouched.
        let Part::ToolCall(tc_old_small) = &messages[1].parts[1] else {
            panic!("expected tool call");
        };
        let ToolState::Completed(old_small_state) = &tc_old_small.state else {
            panic!("expected completed");
        };
        assert_eq!(old_small_state.output, "ok");

        // Recent bulky call in kept tail was left untouched.
        let Part::ToolCall(tc_recent) = &messages[4].parts[0] else {
            panic!("expected tool call");
        };
        let ToolState::Completed(recent_state) = &tc_recent.state else {
            panic!("expected completed");
        };
        assert_eq!(recent_state.output, big_output);

        // Idempotent on second call.
        assert_eq!(prune_stale_tool_outputs(&mut messages, 2), 0);
    }

    #[tokio::test]
    async fn offload_large_tool_output_spills_to_disk_and_falls_back_cleanly() {
        let small = "hello world";
        assert_eq!(offload_large_tool_output("c1", small, None).await, small);

        // Include multi-byte UTF-8 characters across the head/tail cut boundaries.
        let line = "éééé log line content\n";
        let large = line.repeat(3_500);
        assert!(large.len() > OFFLOAD_TOOL_OUTPUT_BYTES);

        let truncated = offload_large_tool_output("c-none", &large, None).await;
        assert!(truncated.starts_with("[Output truncated:"));
        assert!(truncated.contains("[... middle omitted"));
        assert!(truncated.len() < large.len());

        let temp = tempfile::tempdir().expect("tempdir");
        let offloaded = offload_large_tool_output("call/1:abc", &large, Some(temp.path())).await;
        let expected_path = temp.path().join("tool_outputs").join("call_1_abc.txt");
        assert!(expected_path.exists());
        assert_eq!(
            std::fs::read_to_string(&expected_path).expect("saved output"),
            large
        );
        assert!(offloaded.starts_with("[Output offloaded:"));
        assert!(offloaded.contains(&expected_path.display().to_string()));
        assert!(offloaded.contains("[... middle omitted"));
        assert!(offloaded.len() < large.len());
    }
}
