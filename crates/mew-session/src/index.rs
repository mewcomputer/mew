//! Whole-session turn index.
//!
//! History paging hands a frontend messages one page at a time, so a long
//! session only reveals its shape after the reader scrolls all the way back.
//! This index summarizes every turn in one pass over the transcript, so a
//! conversation minimap can render the whole session from the first frame.
//!
//! A turn is one user message that carries text plus the messages that follow
//! it, up to the next such user message. Tool results and compaction markers
//! arrive as user messages without text; they belong to the open turn and
//! never start one.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use mew_message::{Message, Part, Role};

use crate::{decode_transcript, SessionError};

/// Longest prompt or reply snippet a [`TurnSummary`] carries, in characters.
pub const TURN_SUMMARY_TEXT_LIMIT: usize = 300;

/// One turn's preview text, numbered by position in the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnSummary {
    /// Id of the turn's first message. Stable across pages, so a frontend can
    /// use it to key ticks, bookmarks, and scroll targets.
    pub id: String,
    /// Zero-based ordinal of the turn within the session.
    pub index: usize,
    /// Messages in the turn, so a minimap can space ticks by turn size.
    pub message_count: usize,
    /// Opening text of the user message.
    pub prompt: String,
    /// Opening text of the turn's assistant messages, joined in order.
    pub reply: String,
}

/// Summarize every turn in a decoded transcript.
pub fn turn_summaries(messages: &[Message]) -> Vec<TurnSummary> {
    let mut turns: Vec<TurnSummary> = Vec::new();
    for message in messages {
        if message.role == Role::User {
            let prompt = truncate(&message_text(message), TURN_SUMMARY_TEXT_LIMIT);
            if prompt.is_empty() {
                // Tool results, compaction markers, and other text-free user
                // messages continue the open turn instead of starting one.
                if let Some(turn) = turns.last_mut() {
                    turn.message_count += 1;
                }
                continue;
            }
            turns.push(TurnSummary {
                id: message.id.to_string(),
                index: turns.len(),
                message_count: 1,
                prompt,
                reply: String::new(),
            });
        } else if let Some(turn) = turns.last_mut() {
            turn.message_count += 1;
            if message.role == Role::Assistant {
                append_reply(turn, &message_text(message));
            }
        }
    }
    turns
}

/// Read a session transcript file and summarize every turn in it.
///
/// Blocking file I/O and JSON decoding: call this from a blocking thread.
pub fn read_turn_index(path: &Path) -> Result<Vec<TurnSummary>, SessionError> {
    let mut file = File::open(path)?;
    // The transcript is append-only. Reading under the writer's shared lock
    // keeps a record that is still being committed out of the decode.
    file.lock_shared()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    Ok(turn_summaries(&decode_transcript(&data)?))
}

/// Concatenated text of a message's text parts, collapsed to one line.
fn message_text(message: &Message) -> String {
    let mut text = String::new();
    for part in &message.parts {
        let Part::Text(part) = part else { continue };
        let words = part.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if words.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&words);
    }
    text
}

/// Append assistant text to an open turn, stopping at the snippet limit.
fn append_reply(turn: &mut TurnSummary, text: &str) {
    if text.is_empty() {
        return;
    }
    let used = turn.reply.chars().count();
    if used >= TURN_SUMMARY_TEXT_LIMIT {
        return;
    }
    if used > 0 {
        turn.reply.push(' ');
    }
    let room = TURN_SUMMARY_TEXT_LIMIT - used;
    turn.reply.extend(text.chars().take(room));
}

/// Keep the first `limit` characters, never splitting one in half.
fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{PartBase, TextPart, Time, ToolResultPart};
    use ulid::Ulid;

    fn message(role: Role, parts: Vec<Part>) -> Message {
        let id = Ulid::new();
        let session_id = Ulid::new();
        Message {
            id,
            session_id,
            role,
            parts,
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        }
    }

    fn text_message(role: Role, text: &str) -> Message {
        let mut message = message(role, Vec::new());
        message.parts = vec![Part::Text(TextPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: message.id,
                session_id: message.session_id,
            },
            text: text.to_string(),
            synthetic: false,
        })];
        message
    }

    fn tool_result_message() -> Message {
        let mut message = message(Role::User, Vec::new());
        message.parts = vec![Part::ToolResult(ToolResultPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: message.id,
                session_id: message.session_id,
            },
            call_id: "call-1".into(),
        })];
        message
    }

    #[test]
    fn pairs_each_user_prompt_with_the_replies_that_follow_it() {
        let messages = vec![
            text_message(Role::User, "first question"),
            text_message(Role::Assistant, "first answer"),
            tool_result_message(),
            text_message(Role::Assistant, "second answer"),
            text_message(Role::User, "second question"),
            text_message(Role::Assistant, "third answer"),
        ];

        let turns = turn_summaries(&messages);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, messages[0].id.to_string());
        assert_eq!(turns[0].index, 0);
        assert_eq!(turns[0].message_count, 4);
        assert_eq!(turns[0].prompt, "first question");
        assert_eq!(turns[0].reply, "first answer second answer");
        assert_eq!(turns[1].id, messages[4].id.to_string());
        assert_eq!(turns[1].index, 1);
        assert_eq!(turns[1].message_count, 2);
        assert_eq!(turns[1].prompt, "second question");
        assert_eq!(turns[1].reply, "third answer");
    }

    #[test]
    fn joins_multi_part_text_and_collapses_line_breaks() {
        let mut user = text_message(Role::User, "  line one\n\nline two  ");
        let mut extra = user.parts[0].clone();
        if let Part::Text(part) = &mut extra {
            part.text = "third".into();
        }
        user.parts.push(extra);

        let turns = turn_summaries(&[user]);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].prompt, "line one line two third");
        assert!(turns[0].reply.is_empty());
    }

    #[test]
    fn truncates_snippets_at_the_character_limit() {
        // 'é' is two bytes, so byte slicing at 300 would panic or split it.
        let long = "é".repeat(TURN_SUMMARY_TEXT_LIMIT + 50);
        let messages = vec![
            text_message(Role::User, &long),
            text_message(Role::Assistant, &long),
        ];

        let turns = turn_summaries(&messages);

        assert_eq!(turns[0].prompt.chars().count(), TURN_SUMMARY_TEXT_LIMIT);
        assert_eq!(turns[0].reply.chars().count(), TURN_SUMMARY_TEXT_LIMIT);
        assert!(turns[0].prompt.chars().all(|c| c == 'é'));
    }

    #[test]
    fn reply_stops_growing_once_the_snippet_is_full() {
        let chunk = "x".repeat(TURN_SUMMARY_TEXT_LIMIT);
        let messages = vec![
            text_message(Role::User, "go"),
            text_message(Role::Assistant, &chunk),
            text_message(Role::Assistant, "later narration"),
        ];

        let turns = turn_summaries(&messages);

        assert_eq!(turns[0].reply, chunk);
        assert_eq!(turns[0].message_count, 3);
    }

    #[test]
    fn tool_result_only_messages_never_start_a_turn() {
        let messages = vec![
            tool_result_message(),
            text_message(Role::Assistant, "orphan reply"),
            tool_result_message(),
        ];

        assert!(turn_summaries(&messages).is_empty());
    }

    #[test]
    fn empty_transcript_has_no_turns() {
        assert!(turn_summaries(&[]).is_empty());
    }

    #[test]
    fn reads_a_transcript_file_and_ignores_a_torn_trailing_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        let user = text_message(Role::User, "check the build");
        let assistant = text_message(Role::Assistant, "the build is green");
        let mut data = String::new();
        for message in [&user, &assistant] {
            data.push_str(&serde_json::to_string(message).unwrap());
            data.push('\n');
        }
        // A writer mid-append leaves a partial line behind.
        data.push_str("{\"id\":\"01J");
        std::fs::write(&path, data).unwrap();

        let turns = read_turn_index(&path).unwrap();

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, user.id.to_string());
        assert_eq!(turns[0].prompt, "check the build");
        assert_eq!(turns[0].reply, "the build is green");
        assert_eq!(turns[0].message_count, 2);
    }

    #[test]
    fn missing_transcript_is_an_error_not_a_panic() {
        let directory = tempfile::tempdir().unwrap();
        let error = read_turn_index(&directory.path().join("absent.jsonl")).unwrap_err();
        assert!(matches!(error, SessionError::Io(_)));
    }
}
