//! Terminal window titles identify the current conversation, project and attention state.

use crate::app::App;
use mew_client_core::ConnectionStatus;
use std::fmt::Write as _;
use std::io::{self, Write};
use unicode_segmentation::UnicodeSegmentation;

const MAX_TITLE_GRAPHEMES: usize = 160;
const MAX_TITLE_BYTES: usize = 4096;

/// Set the terminal title via the xterm escape sequence `\x1b]0;{title}\x07`.
/// This works on macOS Terminal, iTerm2, and most Linux terminals that
/// support the OSC 0 sequence. Terminals that don't support it silently
/// ignore it.
pub fn set_terminal_title(title: &str) {
    // Use stderr so it doesn't interfere with ratatui's stdout rendering.
    let _ = write_terminal_title(&mut io::stderr().lock(), title);
}

fn write_terminal_title(output: &mut impl Write, title: &str) -> io::Result<()> {
    let mut clean = String::with_capacity(title.len().min(512));
    append_clean(&mut clean, title, MAX_TITLE_GRAPHEMES, MAX_TITLE_BYTES);
    write!(output, "\x1b]0;{clean}\x07")
}

/// Keep the current conversation identifiable even while it needs attention.
pub fn title_for_app(app: &App) -> String {
    let id = &app.status.session_id;
    let session = app
        .daemon_sessions
        .iter()
        .find(|session| &session.session_id == id);
    let (permissions, questions) = app.session_attention.get(id).copied().unwrap_or_else(|| {
        session.map_or((0, 0), |session| {
            (session.pending_permissions, session.pending_questions)
        })
    });
    let state = match &app.connection {
        Some(ConnectionStatus::Backoff { .. }) => "reconnecting",
        Some(ConnectionStatus::Disconnected) => "disconnected",
        Some(ConnectionStatus::Connecting) => "connecting",
        _ if app.attaching_session => "attaching",
        _ if app.permission.is_some() || permissions > 0 => "approval needed",
        _ if app.user_question.is_some() || questions > 0 => "answer needed",
        _ if app.streaming
            || session
                .is_some_and(|session| session.state == mew_protocol::SessionState::Running) =>
        {
            "running"
        }
        _ if app.conversation_failed(id) => "needs attention",
        _ if app.unread_completions.contains(id) => "result ready",
        _ => "idle",
    };
    let conversation = [
        app.session_titles.get(id).map(String::as_str),
        app.session_summaries.get(id).map(String::as_str),
        session.and_then(|session| session.summary.as_deref()),
        session.and_then(|session| session.first_message.as_deref()),
    ]
    .into_iter()
    .flatten()
    .find(|value| value.chars().any(visible_character))
    .unwrap_or("New conversation");
    let (needed, results) = app.notification_counts();
    let current_needs_input = app.conversation_needs_input(id);
    let needed = needed.saturating_sub(usize::from(current_needs_input));
    let results = results.saturating_sub(usize::from(
        !current_needs_input && app.unread_completions.contains(id),
    ));
    let background = match (needed, results) {
        (0, 0) => String::new(),
        (0, n) => format!(" · {n} {}", if n == 1 { "result" } else { "results" }),
        (n, 0) => format!(" · {n} needs you"),
        (n, r) => format!(
            " · {n} needs you · {r} {}",
            if r == 1 { "result" } else { "results" }
        ),
    };
    let mut title = String::with_capacity(192);
    write!(title, "mew · {state} · ").unwrap();
    append_clean(
        &mut title,
        conversation,
        72usize.saturating_sub(background.len()),
        1536,
    );
    if app.short_cwd.chars().any(visible_character) {
        title.push_str(" · ");
        append_clean(&mut title, &app.short_cwd, 48, 768);
    }
    title.push_str(&background);
    title
}

fn safe_character(character: char) -> bool {
    !character.is_control()
        && !matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn visible_character(character: char) -> bool {
    safe_character(character) && !character.is_whitespace()
}

/// Strip terminal controls and collapse whitespace without cutting a grapheme in half.
fn append_clean(output: &mut String, value: &str, limit: usize, max_bytes: usize) {
    let start = output.len();
    let mut count = 0;
    let mut last_boundary = output.len();
    let mut pending_space = false;
    for grapheme in value.graphemes(true) {
        if !grapheme.chars().any(visible_character) {
            pending_space |= grapheme.chars().any(char::is_whitespace);
            continue;
        }
        // A single grapheme can contain arbitrarily many combining marks.
        let space_bytes = usize::from(pending_space && count > 0);
        if output.len() - start + grapheme.len() + space_bytes + '…'.len_utf8() > max_bytes {
            output.push('…');
            return;
        }
        if pending_space && count > 0 {
            if count == limit {
                output.truncate(last_boundary);
                output.push('…');
                return;
            }
            last_boundary = output.len();
            output.push(' ');
            count += 1;
        }
        if count == limit {
            output.truncate(last_boundary);
            output.push('…');
            return;
        }
        last_boundary = output.len();
        output.extend(
            grapheme
                .chars()
                .filter(|&character| visible_character(character)),
        );
        count += 1;
        pending_space = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, DecisionView, PermissionState, UserQuestionState};
    use mew_client_core::{ConnectionStatus, TurnOutcome};
    use unicode_segmentation::UnicodeSegmentation;

    fn app() -> App {
        let mut app = App::new();
        app.status.session_id = "current".into();
        app.short_cwd = "~/code/mew".into();
        app.daemon_sessions = vec![serde_json::from_value(serde_json::json!({
            "session_id": "current", "state": "idle", "created_at": 0,
            "client_count": 1, "summary": "Saved summary", "first_message": "First prompt"
        }))
        .unwrap()];
        app
    }

    #[test]
    fn title_names_current_conversation_and_project() {
        let mut app = app();
        app.session_titles
            .insert("current".into(), "Native browser lifecycle".into());
        assert_eq!(
            title_for_app(&app),
            "mew · idle · Native browser lifecycle · ~/code/mew"
        );
        app.session_titles.clear();
        assert!(title_for_app(&app).contains("Saved summary"));
        app.daemon_sessions[0].summary = Some(" \n ".into());
        assert!(title_for_app(&app).contains("First prompt"));
        app.daemon_sessions.clear();
        assert!(title_for_app(&app).contains("New conversation"));
    }

    #[test]
    fn current_attention_takes_priority_over_work_and_background_attention() {
        let mut app = app();
        app.streaming = true;
        app.session_attention.insert("elsewhere".into(), (2, 2));
        assert!(title_for_app(&app).starts_with("mew · running ·"));
        app.session_attention.insert("current".into(), (0, 1));
        assert!(title_for_app(&app).starts_with("mew · answer needed ·"));
        app.permission = Some(PermissionState {
            tool_name: "shell".into(),
            call_id: "call".into(),
            input: serde_json::json!({}),
            tx: None,
            selected: 0,
            scroll: 0,
            view: DecisionView::default(),
        });
        assert!(title_for_app(&app).starts_with("mew · approval needed ·"));
        app.attaching_session = true;
        assert!(title_for_app(&app).starts_with("mew · attaching ·"));
        app.connection = Some(ConnectionStatus::Backoff {
            attempt: 2,
            error: "offline".into(),
        });
        assert!(title_for_app(&app).starts_with("mew · reconnecting ·"));
    }

    #[test]
    fn an_active_question_is_visible_before_session_counts_arrive() {
        let mut app = app();
        app.user_question = Some(UserQuestionState {
            subagent: None,
            call_id: "question".into(),
            questions: vec![],
            answers: vec![],
            page: 0,
            selected: 0,
            freeform_text: String::new(),
            freeform_cursor: 0,
            freeform_undo: Default::default(),
            review: false,
            review_selected: 0,
            tx: None,
        });
        assert!(title_for_app(&app).starts_with("mew · answer needed ·"));
        for (connection, state) in [
            (ConnectionStatus::Connecting, "connecting"),
            (ConnectionStatus::Disconnected, "disconnected"),
            (ConnectionStatus::Connected, "answer needed"),
        ] {
            app.connection = Some(connection);
            assert!(title_for_app(&app).starts_with(&format!("mew · {state} ·")));
        }
    }

    #[test]
    fn session_counts_and_unread_outcomes_are_specific_to_current_conversation() {
        let mut app = app();
        app.daemon_sessions[0].pending_permissions = 1;
        assert!(title_for_app(&app).contains("approval needed"));
        app.session_attention.insert("current".into(), (0, 0));
        app.unread_completions.insert("current".into());
        assert!(title_for_app(&app).contains("result ready"));
        app.session_turn_outcomes
            .insert("current".into(), TurnOutcome::Failed { detail: None });
        assert!(title_for_app(&app).contains("needs attention"));
        app.streaming = true;
        assert!(title_for_app(&app).contains("running"));
        app.streaming = false;
        app.session_turn_outcomes.clear();
        app.unread_completions.clear();
        assert!(title_for_app(&app).contains("idle"));
    }

    #[test]
    fn saved_failure_is_visible_after_resume_and_running_metadata_takes_precedence() {
        let mut app = app();
        app.daemon_sessions[0].last_turn_failed = true;
        assert!(title_for_app(&app).contains("needs attention"));
        app.daemon_sessions[0].state = mew_protocol::SessionState::Running;
        assert!(title_for_app(&app).contains("running"));
    }

    #[test]
    fn title_reports_background_attention_only_for_conversations_open_here() {
        let mut app = app();
        app.streaming = true;
        app.open_sessions = vec!["current".into(), "background".into()];
        app.session_attention.insert("background".into(), (0, 1));
        app.session_attention.insert("other-window".into(), (1, 0));
        app.unread_completions.insert("other-window".into());
        assert_eq!(
            title_for_app(&app),
            "mew · running · Saved summary · ~/code/mew · 1 needs you"
        );
        app.session_attention.remove("background");
        app.unread_completions.insert("background".into());
        assert!(title_for_app(&app).ends_with(" · 1 result"));
        app.open_sessions.pop();
        assert!(title_for_app(&app).ends_with(" · ~/code/mew"));
    }

    #[test]
    fn terminal_output_cannot_inject_control_sequences() {
        let mut output = Vec::new();
        write_terminal_title(
            &mut output,
            "name\x1b]2;injected\x07\nnext\u{9c}\u{202e}text",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output, "\x1b]0;name]2;injected nexttext\x07");
        let mut app = app();
        app.session_titles
            .insert("current".into(), "Hello\x07\x1b\nworld".into());
        app.short_cwd = "~/project\u{9b}\tname".into();
        let title = title_for_app(&app);
        assert!(!title.chars().any(char::is_control));
        assert!(title.contains("Hello world"));
    }

    #[test]
    fn long_unicode_titles_remain_bounded_without_splitting_graphemes() {
        let mut app = app();
        app.session_titles
            .insert("current".into(), "👩🏽‍💻".repeat(500));
        app.short_cwd = format!("~/{}", "日本語".repeat(500));
        let title = title_for_app(&app);
        assert!(title.graphemes(true).count() <= MAX_TITLE_GRAPHEMES);
        assert!(title.contains("👩🏽‍💻… · ~/"));
        assert!(title.ends_with('…'));
        app.open_sessions = vec!["current".into(), "background".into()];
        app.session_attention.insert("background".into(), (1, 0));
        let title = title_for_app(&app);
        assert!(title.graphemes(true).count() <= MAX_TITLE_GRAPHEMES);
        assert!(title.ends_with(" · 1 needs you"));
        let mut output = Vec::new();
        write_terminal_title(&mut output, &"👩🏽‍💻".repeat(500)).unwrap();
        let output = String::from_utf8(output).unwrap();
        let payload = output
            .strip_prefix("\x1b]0;")
            .unwrap()
            .strip_suffix('\x07')
            .unwrap();
        assert_eq!(payload.graphemes(true).count(), MAX_TITLE_GRAPHEMES);
        assert!(payload.ends_with("👩🏽‍💻…"));
    }

    #[test]
    fn an_oversized_combining_sequence_cannot_bypass_the_title_bound() {
        let mut output = Vec::new();
        let title = format!("e{}", "\u{301}".repeat(10_000));
        write_terminal_title(&mut output, &title).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "\x1b]0;…\x07");
    }
}
