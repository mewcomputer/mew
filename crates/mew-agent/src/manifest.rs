//! Context window manifest builder.
//!
//! Captures a per-turn snapshot of what's in the model's context window
//! at prompt assembly time. The manifest is a tree of `Segment`s mirroring
//! the assembled prompt structure: system prompt sections, tool schemas,
//! and message history (with per-part children).
//!
//! Token counts are estimated using `tiktoken` — the `~` prefix in the UI
//! signals that these are local-tokenizer estimates, not exact provider counts.
//! For non-OpenAI models (DeepSeek, Qwen, Llama, etc.), tiktoken selects the
//! appropriate encoding via `encoding_for_model`. Unknown models fall back to
//! `cl100k_base`. Usage tokens (`input_tokens`, etc.) are `None` until
//! backfilled after the API response.

use mew_message::{Message, MessageId, Part, Segment, SegmentKind, TurnManifest};
use mew_provider::Request;
use ulid::Ulid;

/// Cache of token counts per message ID. Messages are immutable once
/// appended to history, so their token count is stable and can be
/// reused across turns. Cleared on compaction.
type TokenCountCache = std::sync::Mutex<std::collections::HashMap<MessageId, u32>>;

/// Count tokens in `text` using the tokenizer for `model_id`.
///
/// Tries `tiktoken::encoding_for_model(model_id)` first (supports DeepSeek,
/// Qwen, Llama, Mistral, and OpenAI models). Falls back to `cl100k_base`
/// (the most common encoding) for unknown models. Returns 0 on total failure.
fn count_tokens(text: &str, model_id: &str) -> u32 {
    let enc =
        tiktoken::encoding_for_model(model_id).or_else(|| tiktoken::get_encoding("cl100k_base"));
    match enc {
        Some(e) => e.count(text) as u32,
        None => 0,
    }
}

/// Create a leaf segment with `source_id: None`, `tokens_scaled: 0`, no children.
fn seg(label: impl Into<String>, kind: SegmentKind, tokens: u32) -> Segment {
    Segment {
        label: label.into(),
        kind,
        source_id: None,
        tokens,
        tokens_scaled: 0,
        children: vec![],
    }
}

/// Create a segment with children, summing child tokens for the parent total.
fn seg_with_children(
    label: impl Into<String>,
    kind: SegmentKind,
    source_id: Option<Ulid>,
    children: Vec<Segment>,
) -> Segment {
    let total_tokens: u32 = children.iter().map(|s| s.tokens).sum();
    Segment {
        label: label.into(),
        kind,
        source_id,
        tokens: total_tokens,
        tokens_scaled: 0,
        children,
    }
}

/// Classify a text chunk from the system prompt into one or two segments.
///
/// If the chunk contains `<available_skills>`, splits into scaffold + skills.
/// Otherwise emits a single scaffold segment. Empty/whitespace chunks are
/// skipped (returns empty vec).
fn classify_prefix(text: &str, model_id: &str) -> Vec<Segment> {
    if text.trim().is_empty() {
        return vec![];
    }
    if let Some(split_pos) = text.find("<available_skills>") {
        let non_skill = &text[..split_pos];
        let skills_text = &text[split_pos..];
        let mut result = vec![];
        if !non_skill.trim().is_empty() {
            result.push(seg(
                "scaffold",
                SegmentKind::Scaffold,
                count_tokens(non_skill, model_id),
            ));
        }
        result.push(seg(
            "skills",
            SegmentKind::Skill,
            count_tokens(skills_text, model_id),
        ));
        result
    } else {
        vec![seg(
            "scaffold",
            SegmentKind::Scaffold,
            count_tokens(text, model_id),
        )]
    }
}

/// Build a `TurnManifest` from the assembled `Request`.
///
/// Called at `turn.rs:304` (after Request assembly, before `provider.stream()`).
/// `model_id` and `context_window` come from the `Agent` struct, NOT from
/// `Request.model` (which is an empty string at this point).
///
/// Token counts are estimated using tiktoken. Usage tokens (`input_tokens`,
/// etc.) are `None` until backfilled after the API response.
pub fn build_manifest(
    req: &Request,
    model_id: &str,
    context_window: u32,
    cache: &TokenCountCache,
) -> TurnManifest {
    let segments = build_system_segments(req, model_id);
    let mut all_segments = segments;
    all_segments.push(build_tools_segment(req, model_id));
    all_segments.push(build_history_segment(req, model_id, cache));

    TurnManifest {
        model: model_id.to_string(),
        context_window,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        reasoning_tokens: None,
        segments: all_segments,
    }
}

/// Segment the system prompt by its known XML tags.
///
/// The system prompt is assembled as:
///   [persona body]\n\n[base system]
/// where base system contains:
///   - `<context source="...">` blocks (context files)
///   - `<available_skills>` block (skills)
///   - scaffold/boilerplate text
///
/// For v1, we split by these XML tags. This is slightly fragile (breaks if
/// tags change) but avoids threading a new type through the assembly path.
fn build_system_segments(req: &Request, model_id: &str) -> Vec<Segment> {
    let system = &req.system;
    let mut segments = Vec::new();

    // Split by `<context source="...">` tags.
    let mut last_end = 0;
    let mut cursor = 0;

    while cursor < system.len() {
        // Find the next `<context` tag.
        if let Some(tag_start) = system[cursor..].find("<context source=\"") {
            let abs_start = cursor + tag_start;

            // Emit any text before this tag.
            if abs_start > last_end {
                let prefix = &system[last_end..abs_start];
                segments.extend(classify_prefix(prefix, model_id));
            }

            // Find the closing `</context>` tag.
            if let Some(close) = system[abs_start..].find("</context>") {
                let abs_close = abs_start + close + "</context>".len();

                // Extract the source attribute for the label.
                let tag_content = &system[abs_start..abs_close];
                let source = tag_content
                    .find("source=\"")
                    .and_then(|i| {
                        let start = i + "source=\"".len();
                        tag_content[start..]
                            .find('"')
                            .map(|end| &tag_content[start..start + end])
                    })
                    .unwrap_or("unknown");

                let tokens = count_tokens(tag_content, model_id);
                segments.push(seg(
                    format!("context: {}", source),
                    SegmentKind::ContextFile,
                    tokens,
                ));

                last_end = abs_close;
                cursor = abs_close;
            } else {
                // Malformed — skip past the tag start (not one byte at a time).
                cursor = abs_start + "<context source=\"".len();
            }
        } else {
            break;
        }
    }

    // Emit any remaining text after the last tag.
    if last_end < system.len() {
        let suffix = &system[last_end..];
        segments.extend(classify_prefix(suffix, model_id));
    }

    // If no segments were created (empty system prompt), emit nothing.
    // If the system prompt had no XML tags at all, it's all scaffold.
    if segments.is_empty() && !system.trim().is_empty() {
        segments.push(seg(
            "scaffold",
            SegmentKind::Scaffold,
            count_tokens(system, model_id),
        ));
    }

    segments
}

/// Build the Tools segment from the request's tool definitions.
fn build_tools_segment(req: &Request, model_id: &str) -> Segment {
    let children: Vec<Segment> = req
        .tools
        .iter()
        .map(|t| {
            let schema_str = serde_json::to_string(&t.schema).unwrap_or_default();
            let tokens = count_tokens(&format!("{} {}", t.name, schema_str), model_id);
            seg(t.name.clone(), SegmentKind::Tools, tokens)
        })
        .collect();
    seg_with_children(
        format!("tools ({})", req.tools.len()),
        SegmentKind::Tools,
        None,
        children,
    )
}

/// Build the History segment from the request's message list.
///
/// Uses `cache` to avoid re-tokenizing messages that were already counted
/// in a prior turn. Messages are immutable once appended, so their token
/// count is stable.
fn build_history_segment(req: &Request, model_id: &str, cache: &TokenCountCache) -> Segment {
    let message_segments: Vec<Segment> = req
        .messages
        .iter()
        .map(|m| build_message_segment(m, model_id, cache))
        .collect();
    seg_with_children(
        format!("history ({} messages)", req.messages.len()),
        SegmentKind::Message,
        None,
        message_segments,
    )
}

/// Build a Message segment with Part children.
///
/// Checks the token count cache first. On a miss, computes the count and
/// stores it for future turns.
fn build_message_segment(msg: &Message, model_id: &str, cache: &TokenCountCache) -> Segment {
    let label = match msg.role {
        mew_message::Role::User => "user".to_string(),
        mew_message::Role::Assistant => "assistant".to_string(),
    };

    // Detect compaction summary: synthetic TextPart at index 0 with
    // "Previous conversation has been compacted" prefix.
    let is_compaction = msg.parts.iter().any(|p| {
        if let Part::Text(tp) = p {
            tp.synthetic
                && tp
                    .text
                    .starts_with("Previous conversation has been compacted")
        } else {
            false
        }
    });

    let kind = if is_compaction {
        SegmentKind::CompactionSummary
    } else {
        SegmentKind::Message
    };

    // Check cache for this message's total token count.
    let cached = {
        let guard = cache.lock().unwrap();
        guard.get(&msg.id).copied()
    };
    let (part_children, total_tokens) = {
        if let Some(cached) = cached {
            // Cache hit — build skeleton children without tokenizing.
            // Parent total comes from the cache, not from summing children.
            let children: Vec<Segment> = msg
                .parts
                .iter()
                .map(|p| {
                    let (label, kind) = part_label_kind(p);
                    let mut s = seg(label, kind, 0);
                    s.source_id = Some(p.id());
                    s
                })
                .collect();
            (children, cached)
        } else {
            // Cache miss — tokenize all parts and cache the total.
            let children: Vec<Segment> = msg
                .parts
                .iter()
                .map(|p| build_part_segment(p, model_id))
                .collect();
            let total: u32 = children.iter().map(|s| s.tokens).sum();
            cache.lock().unwrap().insert(msg.id, total);
            (children, total)
        }
    };

    Segment {
        label,
        kind,
        source_id: Some(msg.id),
        tokens: total_tokens,
        tokens_scaled: 0,
        children: part_children,
    }
}

/// Extract label and kind from a Part without tokenizing.
fn part_label_kind(part: &Part) -> (String, SegmentKind) {
    match part {
        Part::Text(_) => ("text".to_string(), SegmentKind::Part),
        Part::Reasoning(_) => ("reasoning".to_string(), SegmentKind::Part),
        Part::File(_) => ("file".to_string(), SegmentKind::Part),
        Part::ToolCall(tc) => (tool_call_label(tc), SegmentKind::Part),
        Part::ToolResult(_) => ("tool result".to_string(), SegmentKind::Part),
        Part::Compaction(_) => ("compaction".to_string(), SegmentKind::CompactionSummary),
    }
}

/// Build the display label for a tool call. Subagent calls are labeled
/// `"subagent: {name}"` using the `name` field from the tool input;
/// all other tools use `"tool: {tool_name}"`.
fn tool_call_label(tc: &mew_message::ToolCallPart) -> String {
    if tc.tool_name == "subagent" {
        let name = tc
            .state
            .input()
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("subagent");
        format!("subagent: {}", name)
    } else {
        format!("tool: {}", tc.tool_name)
    }
}

/// Build a Part segment.
fn build_part_segment(part: &Part, model_id: &str) -> Segment {
    let (label, kind, text) = match part {
        Part::Text(tp) => ("text".to_string(), SegmentKind::Part, tp.text.clone()),
        Part::Reasoning(rp) => ("reasoning".to_string(), SegmentKind::Part, rp.text.clone()),
        Part::File(fp) => (
            "file".to_string(),
            SegmentKind::Part,
            fp.filename.clone().unwrap_or_default(),
        ),
        Part::ToolCall(tc) => {
            // Use state.input() (parsed JSON), not raw_input (#[serde(skip)]).
            let input_str = serde_json::to_string(tc.state.input()).unwrap_or_default();
            (
                tool_call_label(tc),
                SegmentKind::Part,
                format!("{} {}", tc.tool_name, input_str),
            )
        }
        Part::ToolResult(_) => ("tool result".to_string(), SegmentKind::Part, String::new()),
        Part::Compaction(_) => (
            "compaction".to_string(),
            SegmentKind::CompactionSummary,
            String::new(),
        ),
    };

    let tokens = count_tokens(&text, model_id);
    let mut s = seg(label, kind, tokens);
    s.source_id = Some(part.id());
    s
}

/// Backfill a manifest with usage data from the API response and scale
/// segment token counts so siblings sum to the reported input total.
///
/// Called after `MessageEnd` in the turn loop. Before this call, all
/// `tokens` and `tokens_scaled` fields are 0 (step 2) and all usage
/// fields are `None`. After this call:
///
/// - `input_tokens`/`output_tokens`/`cache_*`/`reasoning_tokens` are `Some`.
/// - `tokens_scaled` on top-level segments are scaled so they sum to
///   `input_tokens` (proportional to their `tokens` estimates).
///
/// ## Cache-derived prior refinement (Step 6)
///
/// When `cache_read_tokens` (and optionally `cache_write_tokens`) are
/// nonzero, the scaling is split into two groups instead of a single
/// global factor:
///
/// - **Warm prefix** — the first N segments whose cumulative local-token
///   proportion best matches the cache proportion
///   `(cache_read + cache_write) / input`. These are scaled against
///   `cache_read_tokens + cache_write_tokens`.
/// - **Cold suffix** — the remaining segments, scaled against
///   `input_tokens - cache_read_tokens - cache_write_tokens`.
///
/// This tightens estimates for the big static segments (scaffold, tools,
/// context files) whose true token total is known from the cache read.
/// On OpenAI (which reports a single `cached_tokens`), the same logic
/// applies because `cache_read_tokens` is populated from that field.
///
/// When `cache_read_tokens` is 0, or a clean split cannot be found (the
/// best boundary has excessive ratio error, or the split is all-warm /
/// all-cold), the function degrades gracefully to the original single-
/// factor global scaling.
///
/// When all `tokens` are 0, scaling is a no-op (avoids division by zero).
/// The segment structure is still valid; counts stay at 0.
pub fn backfill_manifest(manifest: &mut TurnManifest, usage: &mew_message::Tokens) {
    manifest.input_tokens = Some(usage.input);
    manifest.output_tokens = Some(usage.output);
    manifest.cache_read_tokens = Some(usage.cache_read);
    manifest.cache_write_tokens = Some(usage.cache_write);
    manifest.reasoning_tokens = Some(usage.reasoning);

    let total_local: u32 = manifest.segments.iter().map(|s| s.tokens).sum();
    if total_local == 0 || usage.input == 0 {
        return;
    }

    let warm_target = usage.cache_read.saturating_add(usage.cache_write);

    if warm_target == 0 {
        // No cache info → single global factor.
        let scale = usage.input as f64 / total_local as f64;
        for seg in &mut manifest.segments {
            scale_segment_recursive(seg, scale);
        }
        return;
    }

    // Try to find a clean warm/cold split point.
    if let Some(split_idx) = find_cache_split(&manifest.segments, warm_target, usage.input) {
        apply_split_scaling(&mut manifest.segments, split_idx, warm_target, usage.input);
    } else {
        // Fallback: global scaling.
        let scale = usage.input as f64 / total_local as f64;
        for seg in &mut manifest.segments {
            scale_segment_recursive(seg, scale);
        }
    }
}

/// Find the index at which to split segments into warm prefix [0..split_idx)
/// and cold suffix [split_idx..len).
///
/// The split point is chosen so that the cumulative local-token proportion
/// of the warm prefix best matches the cache proportion
/// `warm_target / input_tokens`. We evaluate every possible boundary
/// (0, 1, 2, …, len) and pick the one with the smallest absolute ratio
/// error.
///
/// Returns `None` when:
/// - The split would be all-warm (split_idx == len) or all-cold
///   (split_idx == 0), because there's nothing to split.
/// - The best achievable ratio error exceeds 50% of the target ratio
///   (the cache proportion doesn't correspond to any segment boundary,
///   so the split would be misleading).
/// - `warm_target >= input_tokens` (the entire input is cached; there's
///   no cold suffix to scale separately).
fn find_cache_split(segments: &[Segment], warm_target: u32, input_tokens: u32) -> Option<usize> {
    if warm_target >= input_tokens || segments.len() < 2 {
        return None;
    }

    let target_ratio = warm_target as f64 / input_tokens as f64;
    // Error tolerance: reject splits whose ratio error exceeds half the
    // target ratio. If the cache is 80% of input, the warm prefix's
    // local proportion must be within ±40 percentage points of that.
    // This is generous enough to accommodate tokenizer drift while
    // preventing nonsensical splits (e.g., warm = 10% when cache = 80%).
    let tolerance = (target_ratio / 2.0).max(0.15);

    let total_local: u64 = segments.iter().map(|s| s.tokens as u64).sum();
    if total_local == 0 {
        return None;
    }

    let mut best_idx: Option<usize> = None;
    let mut best_error = f64::MAX;

    for i in 0..segments.len() {
        // Split at i means warm = [0..i), cold = [i..len).
        // Skip degenerate splits (all-warm or all-cold).
        if i == 0 || i == segments.len() {
            continue;
        }
        let cumulative: u64 = segments[..i].iter().map(|s| s.tokens as u64).sum();
        let warm_ratio = cumulative as f64 / total_local as f64;
        let error = (warm_ratio - target_ratio).abs();

        if error < best_error {
            best_error = error;
            best_idx = Some(i);
        }
    }

    let best_idx = best_idx?;

    if best_error > tolerance {
        return None;
    }

    Some(best_idx)
}

/// Apply split scaling: warm segments [0..split_idx) scaled against
/// `warm_target`, cold segments [split_idx..len) scaled against
/// `input_tokens - warm_target`.
///
/// Each group's segments are scaled proportionally within the group.
/// Children are scaled with the same factor as their parent.
fn apply_split_scaling(
    segments: &mut [Segment],
    split_idx: usize,
    warm_target: u32,
    input_tokens: u32,
) {
    let warm_local: u32 = segments[..split_idx].iter().map(|s| s.tokens).sum();
    let cold_target = input_tokens.saturating_sub(warm_target);
    let cold_local: u32 = segments[split_idx..].iter().map(|s| s.tokens).sum();

    let warm_scale = if warm_local > 0 {
        warm_target as f64 / warm_local as f64
    } else {
        0.0
    };
    let cold_scale = if cold_local > 0 {
        cold_target as f64 / cold_local as f64
    } else {
        0.0
    };

    for (i, seg) in segments.iter_mut().enumerate() {
        let scale = if i < split_idx {
            warm_scale
        } else {
            cold_scale
        };
        scale_segment_recursive(seg, scale);
    }
}

/// Recursively scale `tokens_scaled` on a segment and all its children.
fn scale_segment_recursive(seg: &mut Segment, scale: f64) {
    seg.tokens_scaled = (seg.tokens as f64 * scale).round() as u32;
    for child in &mut seg.children {
        scale_segment_recursive(child, scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{
        Message, PartBase, PartId, Role, TextPart, Time, ToolCallPart, ToolState, ToolStatePending,
    };
    use mew_provider::{Request, ToolDef};

    fn empty_cache() -> TokenCountCache {
        std::sync::Mutex::new(std::collections::HashMap::new())
    }

    fn make_message(role: Role, text: &str) -> Message {
        Message {
            id: Ulid::new(),
            session_id: Ulid::new(),
            role,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
                    session_id: Ulid::new(),
                },
                text: text.into(),
                synthetic: false,
            })],
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        }
    }

    fn make_request(system: &str, messages: Vec<Message>, tools: Vec<ToolDef>) -> Request {
        Request {
            model: String::new(),
            messages,
            tools,
            system: system.into(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
        }
    }

    #[test]
    fn test_build_manifest_basic() {
        let msg = make_message(Role::User, "Hello");
        let req = make_request("You are helpful.", vec![msg], vec![]);
        let manifest = build_manifest(&req, "test-model", 128000, &empty_cache());

        assert_eq!(manifest.model, "test-model");
        assert_eq!(manifest.context_window, 128000);
        assert!(manifest.input_tokens.is_none());
        assert!(manifest.output_tokens.is_none());
        // Should have: system segments + tools + history
        assert!(manifest.segments.len() >= 2); // at least scaffold + history
    }

    #[test]
    fn test_system_prompt_segmentation() {
        let system = r#"You are an assistant.

<context source="CLAUDE.md">
Project instructions here.
</context>
<available_skills>
<skill><name>test</name></skill>
</available_skills>"#;
        let req = make_request(system, vec![], vec![]);
        let segments = build_system_segments(&req, "gpt-4o");

        // Should have: scaffold + context file + skills
        assert!(segments.iter().any(|s| s.kind == SegmentKind::Scaffold));
        assert!(segments.iter().any(|s| s.kind == SegmentKind::ContextFile));
        assert!(segments.iter().any(|s| s.kind == SegmentKind::Skill));
    }

    #[test]
    fn test_tools_segment() {
        let tool = ToolDef {
            name: "bash".into(),
            description: "Run a command".into(),
            schema: serde_json::json!({}),
        };
        let req = make_request("system", vec![], vec![tool]);
        let seg = build_tools_segment(&req, "gpt-4o");

        assert_eq!(seg.kind, SegmentKind::Tools);
        assert!(seg.label.contains("1"));
        assert_eq!(seg.children.len(), 1);
        assert_eq!(seg.children[0].label, "bash");
    }

    #[test]
    fn test_history_segment() {
        let msg1 = make_message(Role::User, "Hello");
        let msg2 = make_message(Role::Assistant, "Hi there");
        let req = make_request("system", vec![msg1, msg2], vec![]);
        let seg = build_history_segment(&req, "gpt-4o", &empty_cache());

        assert_eq!(seg.kind, SegmentKind::Message);
        assert!(seg.label.contains("2"));
        assert_eq!(seg.children.len(), 2);
        assert_eq!(seg.children[0].label, "user");
        assert_eq!(seg.children[1].label, "assistant");
    }

    #[test]
    fn test_compaction_detection() {
        let mut msg = make_message(Role::User, "");
        msg.parts[0] = Part::Text(TextPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: Ulid::new(),
                session_id: Ulid::new(),
            },
            text: "Previous conversation has been compacted. Summary: ...".into(),
            synthetic: true,
        });
        let req = make_request("system", vec![msg], vec![]);
        let seg = build_history_segment(&req, "gpt-4o", &empty_cache());

        // The compaction message should be detected as CompactionSummary
        assert_eq!(seg.children[0].kind, SegmentKind::CompactionSummary);
    }

    #[test]
    fn test_empty_system_prompt() {
        let req = make_request("", vec![], vec![]);
        let segments = build_system_segments(&req, "gpt-4o");
        assert!(segments.is_empty());
    }

    #[test]
    fn test_manifest_tokens_nonzero_after_build() {
        let msg = make_message(Role::User, "Hello world this is a test message");
        let req = make_request("You are a helpful assistant.", vec![msg], vec![]);
        let manifest = build_manifest(&req, "gpt-4o", 1000, &empty_cache());

        // With tiktoken integrated, segments should have nonzero token counts
        // for non-empty text.
        let has_nonzero = manifest.segments.iter().any(|s| s.tokens > 0);
        assert!(
            has_nonzero,
            "at least one segment should have nonzero tokens"
        );
    }

    #[test]
    fn test_backfill_manifest_sets_usage_fields() {
        let msg = make_message(Role::User, "Hello");
        let req = make_request("system", vec![msg], vec![]);
        let mut manifest = build_manifest(&req, "gpt-4o", 1000, &empty_cache());

        // Before backfill: all usage fields are None.
        assert!(manifest.input_tokens.is_none());
        assert!(manifest.output_tokens.is_none());
        assert!(manifest.cache_read_tokens.is_none());

        let usage = mew_message::Tokens {
            input: 5000,
            output: 1200,
            reasoning: 300,
            cache_read: 2000,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // After backfill: usage fields are Some.
        assert_eq!(manifest.input_tokens, Some(5000));
        assert_eq!(manifest.output_tokens, Some(1200));
        assert_eq!(manifest.cache_read_tokens, Some(2000));
        assert_eq!(manifest.cache_write_tokens, Some(0));
        assert_eq!(manifest.reasoning_tokens, Some(300));
    }

    #[test]
    fn test_backfill_manifest_no_scaling_when_tokens_zero() {
        // When all segment tokens are 0 (e.g., empty content), scaling is a
        // no-op (avoids division by zero). tokens_scaled stays 0.
        let msg = make_message(Role::User, "Hello");
        let req = make_request("system", vec![msg], vec![]);
        let mut manifest = build_manifest(&req, "gpt-4o", 1000, &empty_cache());

        // Manually zero out tokens to test the division-by-zero guard.
        for seg in &mut manifest.segments {
            seg.tokens = 0;
        }

        let usage = mew_message::Tokens {
            input: 5000,
            output: 1000,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // tokens_scaled should still be 0 — no tokenizer estimates.
        for seg in &manifest.segments {
            assert_eq!(
                seg.tokens_scaled, 0,
                "segment '{}' scaled but tokens=0",
                seg.label
            );
        }
    }

    #[test]
    fn test_backfill_manifest_scales_when_tokens_nonzero() {
        // When segment tokens are nonzero (step 4 — tokenizer integrated),
        // scaling distributes input_tokens proportionally.
        let msg = make_message(Role::User, "Hello");
        let req = make_request("system", vec![msg], vec![]);
        let mut manifest = build_manifest(&req, "gpt-4o", 1000, &empty_cache());

        // Simulate tokenizer having assigned estimates.
        // Give first segment 300 tokens, second 100 (total 400).
        assert!(
            manifest.segments.len() >= 2,
            "test requires at least 2 segments, got {}",
            manifest.segments.len()
        );
        manifest.segments[0].tokens = 300;
        manifest.segments[1].tokens = 100;

        let usage = mew_message::Tokens {
            input: 800, // 2x the local total
            output: 200,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // 800 * (300/400) = 600, 800 * (100/400) = 200
        // Allow ±1 for rounding.
        assert!(
            (manifest.segments[0].tokens_scaled as i32 - 600).abs() <= 1,
            "expected ~600, got {}",
            manifest.segments[0].tokens_scaled
        );
        assert!(
            (manifest.segments[1].tokens_scaled as i32 - 200).abs() <= 1,
            "expected ~200, got {}",
            manifest.segments[1].tokens_scaled
        );
        // Siblings sum to input_tokens (±1 for rounding).
        let total: u32 = manifest.segments.iter().map(|s| s.tokens_scaled).sum();
        assert!(
            (total as i32 - 800).abs() <= 1,
            "expected ~800, got {}",
            total
        );
    }

    #[test]
    fn test_count_tokens_returns_nonzero() {
        let count = count_tokens("Hello world, this is a test message.", "gpt-4o");
        assert!(
            count > 0,
            "token count should be nonzero for non-empty text"
        );
    }

    #[test]
    fn test_count_tokens_unknown_model_falls_back() {
        // Unknown model should fall back to cl100k_base and still return > 0.
        let count = count_tokens("Hello world, this is a test message.", "bogus-model-xyz");
        assert!(count > 0, "unknown model should fall back to cl100k_base");
    }

    #[test]
    fn test_backfill_manifest_zero_input_tokens() {
        // When usage.input == 0, scaling is skipped (div-by-zero guard)
        // but usage fields are still set. tokens_scaled stays 0.
        let msg = make_message(Role::User, "Hello");
        let req = make_request("system", vec![msg], vec![]);
        let mut manifest = build_manifest(&req, "gpt-4o", 1000, &empty_cache());

        // Manually set nonzero tokens to verify they're NOT scaled.
        for seg in &mut manifest.segments {
            seg.tokens = 100;
        }

        let usage = mew_message::Tokens {
            input: 0,
            output: 0,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // input_tokens is Some(0) — the UI shows "error · structure below".
        assert_eq!(manifest.input_tokens, Some(0));
        // tokens_scaled stays 0 — no scaling when input is 0.
        for seg in &manifest.segments {
            assert_eq!(
                seg.tokens_scaled, 0,
                "segment '{}' should not be scaled",
                seg.label
            );
        }
    }

    #[test]
    fn test_backfill_manifest_scales_children_recursively() {
        // After backfill, children should also have tokens_scaled set
        // (proportional to the same scale factor as the parent).
        let msg1 = make_message(Role::User, "Hello world");
        let msg2 = make_message(Role::Assistant, "Hi there");
        let req = make_request("system", vec![msg1, msg2], vec![]);
        let mut manifest = build_manifest(&req, "gpt-4o", 1000, &empty_cache());

        // Set known token counts on the history segment and its children.
        let history = manifest
            .segments
            .iter_mut()
            .find(|s| s.label.starts_with("history"))
            .expect("history segment should exist");
        history.tokens = 400;
        for child in &mut history.children {
            child.tokens = 200;
            for grandchild in &mut child.children {
                grandchild.tokens = 200;
            }
        }

        // Set system segment tokens to 100 so total_local = 500.
        let system = manifest
            .segments
            .iter_mut()
            .find(|s| s.kind == SegmentKind::Scaffold)
            .expect("scaffold segment should exist");
        system.tokens = 100;

        let usage = mew_message::Tokens {
            input: 1000, // 2x the local total (500)
            output: 200,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Scale = 1000/500 = 2.0
        // History: 400 * 2.0 = 800
        // System: 100 * 2.0 = 200
        let history = manifest
            .segments
            .iter()
            .find(|s| s.label.starts_with("history"))
            .unwrap();
        assert!(
            (history.tokens_scaled as i32 - 800).abs() <= 1,
            "history should be ~800, got {}",
            history.tokens_scaled
        );

        // Children should also be scaled: 200 * 2.0 = 400
        for child in &history.children {
            assert!(
                (child.tokens_scaled as i32 - 400).abs() <= 1,
                "child '{}' should be ~400, got {}",
                child.label,
                child.tokens_scaled
            );
            // Grandchildren too: 200 * 2.0 = 400
            for grandchild in &child.children {
                assert!(
                    (grandchild.tokens_scaled as i32 - 400).abs() <= 1,
                    "grandchild should be ~400, got {}",
                    grandchild.tokens_scaled
                );
            }
        }
    }

    #[test]
    fn test_token_count_cache_avoids_retokenization() {
        // On the second call to build_manifest with the same messages,
        // cached token counts should be reused (not re-tokenized).
        let msg = make_message(Role::User, "Hello world this is a test message");
        let req = make_request("system prompt", vec![msg], vec![]);
        let cache = empty_cache();

        let manifest1 = build_manifest(&req, "gpt-4o", 1000, &cache);
        let history1 = manifest1
            .segments
            .iter()
            .find(|s| s.label.starts_with("history"))
            .unwrap();

        // Cache should now contain an entry for this message.
        assert!(
            !cache.lock().unwrap().is_empty(),
            "cache should have an entry after first build"
        );

        // Second build with the same message — should use cached total.
        let manifest2 = build_manifest(&req, "gpt-4o", 1000, &cache);
        let history2 = manifest2
            .segments
            .iter()
            .find(|s| s.label.starts_with("history"))
            .unwrap();

        // Token counts should match (cache returned the same value).
        assert_eq!(
            history1.tokens, history2.tokens,
            "cached token count should be identical on second build"
        );
    }

    // ── Cache-derived prior refinement tests (Step 6) ──

    /// Helper: build a manifest with N segments of known token counts,
    /// all usage fields set to None.
    fn make_manifest_with_tokens(segment_tokens: &[u32]) -> TurnManifest {
        let segments = segment_tokens
            .iter()
            .map(|&t| seg(format!("seg{}", t), SegmentKind::Scaffold, t))
            .collect();
        TurnManifest {
            model: "test".into(),
            context_window: 10000,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
            segments,
        }
    }

    #[test]
    fn test_cache_split_scales_warm_and_cold_separately() {
        // 3 segments: 300 + 100 + 100 = 500 local total.
        // cache_read = 400 (80% of input), input = 500.
        // Target ratio = 400/500 = 0.8.
        // Best split: warm = first 2 segments (300+100=400, ratio=0.8).
        // Perfect match — warm scaled by 400/400 = 1.0, cold by 100/100 = 1.0.
        let mut manifest = make_manifest_with_tokens(&[300, 100, 100]);
        let usage = mew_message::Tokens {
            input: 500,
            output: 50,
            reasoning: 0,
            cache_read: 400,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Warm segments: 300*1.0=300, 100*1.0=100. Cold: 100*1.0=100.
        assert_eq!(manifest.segments[0].tokens_scaled, 300);
        assert_eq!(manifest.segments[1].tokens_scaled, 100);
        assert_eq!(manifest.segments[2].tokens_scaled, 100);
        // Sum invariant.
        let total: u32 = manifest.segments.iter().map(|s| s.tokens_scaled).sum();
        assert_eq!(total, 500);
    }

    #[test]
    fn test_cache_split_scales_with_different_ratios() {
        // 2 segments: 300 + 100 = 400 local total.
        // cache_read = 600 (75% of input=800), input = 800.
        // Target ratio = 600/800 = 0.75. Best split: warm = seg0 (300/400=0.75).
        // Warm scale = 600/300 = 2.0. Cold scale = 200/100 = 2.0.
        let mut manifest = make_manifest_with_tokens(&[300, 100]);
        let usage = mew_message::Tokens {
            input: 800,
            output: 100,
            reasoning: 0,
            cache_read: 600,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Warm: 300 * 2.0 = 600. Cold: 100 * 2.0 = 200.
        assert_eq!(manifest.segments[0].tokens_scaled, 600);
        assert_eq!(manifest.segments[1].tokens_scaled, 200);
        let total: u32 = manifest.segments.iter().map(|s| s.tokens_scaled).sum();
        assert_eq!(total, 800);
    }

    #[test]
    fn test_cache_split_degrades_to_global_when_no_cache() {
        // cache_read = 0 → should use global scaling.
        let mut manifest = make_manifest_with_tokens(&[300, 100]);
        let usage = mew_message::Tokens {
            input: 800,
            output: 100,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Global scale = 800/400 = 2.0.
        assert_eq!(manifest.segments[0].tokens_scaled, 600);
        assert_eq!(manifest.segments[1].tokens_scaled, 200);
    }

    #[test]
    fn test_cache_split_degrades_when_all_warm() {
        // cache_read = input → warm_target >= input_tokens → no split.
        // Should fall back to global scaling.
        let mut manifest = make_manifest_with_tokens(&[300, 100]);
        let usage = mew_message::Tokens {
            input: 400,
            output: 50,
            reasoning: 0,
            cache_read: 400, // 100% of input
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Global scale = 400/400 = 1.0.
        assert_eq!(manifest.segments[0].tokens_scaled, 300);
        assert_eq!(manifest.segments[1].tokens_scaled, 100);
    }

    #[test]
    fn test_cache_split_degrades_when_ratio_mismatch() {
        // 2 segments: 10 + 990 = 1000 local.
        // cache_read = 800 (80% of input=1000). Target ratio = 0.8.
        // Only possible non-degenerate split: warm = seg0 (10/1000 = 0.01).
        // Error = |0.01 - 0.8| = 0.79 >> tolerance (0.4).
        // Should fall back to global scaling.
        let mut manifest = make_manifest_with_tokens(&[10, 990]);
        let usage = mew_message::Tokens {
            input: 1000,
            output: 100,
            reasoning: 0,
            cache_read: 800,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Global scale = 1000/1000 = 1.0.
        assert_eq!(manifest.segments[0].tokens_scaled, 10);
        assert_eq!(manifest.segments[1].tokens_scaled, 990);
    }

    #[test]
    fn test_cache_split_with_cache_write() {
        // cache_read = 300, cache_write = 100 → warm_target = 400.
        // 3 segments: 200 + 200 + 200 = 600 local.
        // Target ratio = 400/600 ≈ 0.667.
        // Best split: warm = first 2 segs (400/600 = 0.667). Perfect match.
        // Warm scale = 400/400 = 1.0. Cold scale = 200/200 = 1.0.
        let mut manifest = make_manifest_with_tokens(&[200, 200, 200]);
        let usage = mew_message::Tokens {
            input: 600,
            output: 50,
            reasoning: 0,
            cache_read: 300,
            cache_write: 100,
        };
        backfill_manifest(&mut manifest, &usage);

        assert_eq!(manifest.segments[0].tokens_scaled, 200);
        assert_eq!(manifest.segments[1].tokens_scaled, 200);
        assert_eq!(manifest.segments[2].tokens_scaled, 200);
        let total: u32 = manifest.segments.iter().map(|s| s.tokens_scaled).sum();
        assert_eq!(total, 600);
    }

    #[test]
    fn test_cache_split_with_very_small_cold_suffix() {
        // 3 segments: 300 + 300 + 10 = 610 local.
        // cache_read = 600 (input=610). Target ratio = 600/610 ≈ 0.984.
        // Best split: warm = first 2 (600/610 ≈ 0.984). Error ≈ 0.
        // Warm scale = 600/600 = 1.0. Cold scale = 10/10 = 1.0.
        let mut manifest = make_manifest_with_tokens(&[300, 300, 10]);
        let usage = mew_message::Tokens {
            input: 610,
            output: 10,
            reasoning: 0,
            cache_read: 600,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Warm: 300+300=600. Cold: 10.
        let warm_sum: u32 = manifest.segments[..2].iter().map(|s| s.tokens_scaled).sum();
        assert!(
            (warm_sum as i32 - 600).abs() <= 1,
            "warm should be ~600, got {}",
            warm_sum
        );
        assert!(
            (manifest.segments[2].tokens_scaled as i32 - 10).abs() <= 1,
            "cold should be ~10, got {}",
            manifest.segments[2].tokens_scaled
        );
    }

    #[test]
    fn test_cache_split_sum_invariant() {
        // Verify that warm + cold = input_tokens after split scaling,
        // even with messy numbers.
        let mut manifest = make_manifest_with_tokens(&[350, 150, 200, 300]);
        let usage = mew_message::Tokens {
            input: 1234,
            output: 56,
            reasoning: 0,
            cache_read: 700,
            cache_write: 50,
        };
        backfill_manifest(&mut manifest, &usage);

        let total: u32 = manifest.segments.iter().map(|s| s.tokens_scaled).sum();
        assert!(
            (total as i32 - 1234).abs() <= 2,
            "scaled total should be ~1234, got {}",
            total
        );
    }

    #[test]
    fn test_cache_split_finds_best_boundary_with_multiple_options() {
        // 4 segments: 100 + 100 + 300 + 500 = 1000 local.
        // cache_read = 200 (input=1000). Target ratio = 0.2.
        // Best split: warm = first 2 segs (200/1000 = 0.2). Perfect match.
        // Warm scale = 200/200 = 1.0. Cold scale = 800/800 = 1.0.
        let mut manifest = make_manifest_with_tokens(&[100, 100, 300, 500]);
        let usage = mew_message::Tokens {
            input: 1000,
            output: 100,
            reasoning: 0,
            cache_read: 200,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // With perfect ratio match, each segment scales by 1.0.
        for (i, seg) in manifest.segments.iter().enumerate() {
            assert!(
                (seg.tokens_scaled as i32 - seg.tokens as i32).abs() <= 1,
                "segment {} should be ~{}, got {}",
                i,
                seg.tokens,
                seg.tokens_scaled
            );
        }
    }

    #[test]
    fn test_cache_split_children_scaled_with_parent_group() {
        // Verify that children inside a warm-prefix segment are scaled
        // with the warm factor, not the cold factor.
        let mut manifest = make_manifest_with_tokens(&[400, 100]);
        // Add children to the first (warm) segment.
        manifest.segments[0].children = vec![
            seg("child1", SegmentKind::Part, 250),
            seg("child2", SegmentKind::Part, 150),
        ];
        let usage = mew_message::Tokens {
            input: 1000,
            output: 100,
            reasoning: 0,
            cache_read: 800,
            cache_write: 0,
        };
        backfill_manifest(&mut manifest, &usage);

        // Target ratio = 800/1000 = 0.8. Best split: warm = seg0 (400/500=0.8).
        // Warm scale = 800/400 = 2.0. Cold scale = 200/100 = 2.0.
        assert_eq!(manifest.segments[0].tokens_scaled, 800);
        // Children should be scaled by warm factor (2.0).
        assert_eq!(manifest.segments[0].children[0].tokens_scaled, 500);
        assert_eq!(manifest.segments[0].children[1].tokens_scaled, 300);
    }

    #[test]
    fn test_find_cache_split_returns_none_for_single_segment() {
        let segments = vec![seg("only", SegmentKind::Scaffold, 100)];
        assert!(find_cache_split(&segments, 50, 100).is_none());
    }

    #[test]
    fn test_find_cache_split_returns_none_when_warm_exceeds_input() {
        let segments = vec![
            seg("a", SegmentKind::Scaffold, 50),
            seg("b", SegmentKind::Scaffold, 50),
        ];
        // warm_target = 150 > input = 100.
        assert!(find_cache_split(&segments, 150, 100).is_none());
    }

    // ── Calibration harness ──────────────────────────────────────────
    //
    // These tests verify that `count_tokens` produces counts consistent
    // with direct `tiktoken::get_encoding(...)` calls for the expected
    // encoding per model. At least one fixture uses an independently-
    // verified count ("Hello world" = 2 tokens for cl100k_base, per
    // OpenAI's published tokenizer) to establish a non-circular baseline.
    // If tiktoken's encoding tables change (version bump), these tests
    // will fail and the hardcoded counts must be updated.

    // Fixture: short string. "Hello world" = 2 tokens for cl100k_base
    // (independently verified via OpenAI's tokenizer web UI).
    // cl100k and o200k happen to agree on this string, but the JSON
    // fixture below distinguishes them.
    const FIXTURE_HELLO: &str = "Hello world";
    const EXPECTED_CL100K_HELLO: u32 = 2;
    const EXPECTED_O200K_HELLO: u32 = 2;

    // Fixture: multi-paragraph text (~150 tokens). Tests longer input.
    const FIXTURE_LONG_TEXT: &str = "\
The quick brown fox jumps over the lazy dog. Pack my box with five dozen \
liquor jugs. How vexingly quick daft zebras jump! The five boxing wizards \
jump quickly. Sphinx of black quartz, judge my vow. Waltz, bad nymph, for \
quick jigs vex. Glib jocks quiz nymph to vex dwarf. Quick zephyrs blow, \
vexing daft Jim. Two driven jocks help fax my big quiz. The jay, pig, fox, \
zebra, and my wolves quack! Blowzy red vixens fight for a quick jump. \
Joaquin Phoenix was gazed by me for the length of the movie. Crazy \
Fredrick bought many very exquisite opal jewels.";
    const EXPECTED_CL100K_LONG: u32 = 146;

    // Fixture: code snippet (structured text with special tokens).
    const FIXTURE_CODE: &str = "\
fn main() {
    let mut v = Vec::new();
    for i in 0..100 {
        v.push(i * 2);
    }
    println!(\"{:?}\", v);
}";
    const EXPECTED_O200K_CODE: u32 = 40;

    // Fixture: JSON schema text (simulates tool schema segment).
    // This is where cl100k (36) and o200k (38) diverge — useful for
    // verifying routing.
    const FIXTURE_JSON: &str = r#"{"type":"object","properties":{"name":{"type":"string","description":"The name of the person"},"age":{"type":"integer","minimum":0}},"required":["name"]}"#;
    const EXPECTED_CL100K_JSON: u32 = 36;
    const EXPECTED_O200K_JSON: u32 = 38;

    #[test]
    fn test_calibration_cl100k_short_string() {
        // Independently verified: "Hello world" = 2 tokens for cl100k_base.
        assert_eq!(
            count_tokens(FIXTURE_HELLO, "gpt-3.5-turbo"),
            EXPECTED_CL100K_HELLO
        );
    }

    #[test]
    fn test_calibration_cl100k_long_text() {
        assert_eq!(
            count_tokens(FIXTURE_LONG_TEXT, "gpt-3.5-turbo"),
            EXPECTED_CL100K_LONG
        );
    }

    #[test]
    fn test_calibration_o200k_short_string() {
        assert_eq!(count_tokens(FIXTURE_HELLO, "gpt-4o"), EXPECTED_O200K_HELLO);
    }

    #[test]
    fn test_calibration_o200k_code_snippet() {
        assert_eq!(count_tokens(FIXTURE_CODE, "gpt-4o"), EXPECTED_O200K_CODE);
    }

    #[test]
    fn test_calibration_json_schema_text() {
        // JSON schema text — cl100k and o200k diverge here (36 vs 38).
        // Verify both encodings produce their expected counts.
        assert_eq!(
            count_tokens(FIXTURE_JSON, "gpt-3.5-turbo"),
            EXPECTED_CL100K_JSON
        );
        assert_eq!(count_tokens(FIXTURE_JSON, "gpt-4o"), EXPECTED_O200K_JSON);
    }

    #[test]
    fn test_model_encoding_routing() {
        // Verify model→encoding routing by comparing count_tokens (which
        // uses encoding_for_model internally) against direct get_encoding
        // calls. The JSON fixture distinguishes cl100k (36) from o200k (38).

        // gpt-4o → o200k_base
        let direct_o200k =
            tiktoken::get_encoding("o200k_base").expect("o200k_base encoding should exist");
        assert_eq!(
            count_tokens(FIXTURE_JSON, "gpt-4o"),
            direct_o200k.count(FIXTURE_JSON) as u32,
        );

        // gpt-3.5-turbo → cl100k_base
        let direct_cl100k =
            tiktoken::get_encoding("cl100k_base").expect("cl100k_base encoding should exist");
        assert_eq!(
            count_tokens(FIXTURE_JSON, "gpt-3.5-turbo"),
            direct_cl100k.count(FIXTURE_JSON) as u32,
        );

        // Unknown model → cl100k_base fallback
        assert_eq!(
            count_tokens(FIXTURE_JSON, "totally-unknown-model"),
            direct_cl100k.count(FIXTURE_JSON) as u32,
        );
    }

    // ── Subagent label detection ─────────────────────────────────────

    fn make_tool_call_part(tool_name: &str, input: serde_json::Value) -> ToolCallPart {
        ToolCallPart {
            base: PartBase {
                id: PartId::new(),
                message_id: mew_message::MessageId::new(),
                session_id: mew_message::SessionId::new(),
            },
            tool_name: tool_name.into(),
            call_id: "test-call".into(),
            state: ToolState::Pending(ToolStatePending {
                input,
                time: mew_message::ToolTime {
                    start: 0,
                    end: None,
                },
            }),
            raw_input: String::new(),
        }
    }

    #[test]
    fn test_manifest_labels_subagent_tool_calls() {
        // A subagent tool call with a "name" field should be labeled
        // "subagent: {name}", not "tool: subagent".
        let tc = make_tool_call_part(
            "subagent",
            serde_json::json!({"name": "researcher", "prompt": "find the bug"}),
        );
        let label = tool_call_label(&tc);
        assert_eq!(label, "subagent: researcher");

        // A regular tool call should still use "tool: {name}".
        let tc2 = make_tool_call_part("bash", serde_json::json!({"command": "ls"}));
        assert_eq!(tool_call_label(&tc2), "tool: bash");

        // A subagent call without a "name" field falls back to "subagent: subagent".
        let tc3 = make_tool_call_part("subagent", serde_json::json!({"prompt": "do something"}));
        assert_eq!(tool_call_label(&tc3), "subagent: subagent");
    }
}
