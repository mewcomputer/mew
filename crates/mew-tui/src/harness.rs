//! Headless, deterministic driver for the TUI — for tests and for agents that
//! want to exercise the interface without a real terminal, provider, or runtime.
//!
//! A [`Harness`] wraps an [`App`] backed by ratatui's `TestBackend`. It feeds
//! the app synthetic keyboard events and `AgentEvent`s, renders frames to an
//! in-memory buffer, and returns them as plain text you can read back or assert
//! on. There is no network, no async, and no dependency on `main.rs`.
//!
//! # Script format
//!
//! [`run_script`] interprets a line-based script. Blank lines and lines
//! starting with `#` are ignored. Verbs:
//!
//! - `size <w> <h>` — resize the virtual terminal (default 80x24)
//! - `type <text>` — type literal text into the composer
//! - `key <name>` — send a key: `enter`, `esc`, `tab`, `backspace`, `space`,
//!   `up`/`down`/`left`/`right`, `home`/`end`, `ctrl+c`, `alt+x`, or a single char
//! - `submit` — shorthand for `key enter`
//! - `say <text>` — inject a complete assistant text turn (start → deltas → end)
//! - `error <text>` — inject a terminal `AgentEvent::Error`
//! - `snapshot [label]` — render the current frame into the output
//!
//! ```no_run
//! let out = mew_tui::harness::run_script("type hello\nsubmit\nsay hi there\nsnapshot", 80, 24);
//! print!("{out}");
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_agent::AgentEvent;
use mew_message::{
    Finish, Message, MessageId, Part, PartBase, PartId, Role, SessionId, TextPart, Time, Tokens,
};
use mew_provider::ProviderEvent;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::App;
use crate::events::{handle_key_event, Action};
use crate::ui;

/// A headless TUI driven programmatically.
pub struct Harness {
    /// The app under test. Public so tests can inspect or seed state directly.
    pub app: App,
    /// Actions returned by key handling, in order — useful for assertions.
    pub actions: Vec<Action>,
    terminal: Terminal<TestBackend>,
}

impl Harness {
    /// Build a harness with a virtual terminal of the given size.
    pub fn new(width: u16, height: u16) -> Self {
        let terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        Self {
            app: App::new(),
            actions: Vec::new(),
            terminal,
        }
    }

    /// Resize the virtual terminal.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.terminal.backend_mut().resize(width, height);
    }

    /// Send one key event, applying the harness-visible effect of any returned
    /// action (echoing the user message on submit, quitting, clearing).
    pub fn key(&mut self, key: KeyEvent) {
        if let Some(action) = handle_key_event(&mut self.app, key) {
            self.apply_action(action);
        }
    }

    fn apply_action(&mut self, action: Action) {
        match &action {
            Action::Submit(text) => {
                self.push_user_message(text.clone());
                self.app.streaming = true;
            }
            Action::Quit => self.app.should_quit = true,
            Action::Clear => self.app.clear_messages(),
            _ => {}
        }
        self.actions.push(action);
    }

    /// Type literal text into the composer, one character at a time.
    pub fn type_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
    }

    /// Inject a complete assistant text turn: `PartStart` → `PartDelta`s →
    /// `PartEnd` → `MessageEnd`, mirroring how a real provider streams text.
    pub fn say(&mut self, text: &str) {
        let part_id = PartId::new();
        let base = PartBase {
            id: part_id,
            message_id: MessageId::new(),
            session_id: SessionId::new(),
        };
        self.agent(AgentEvent::Provider(ProviderEvent::PartStart {
            part: Part::Text(TextPart {
                base,
                text: String::new(),
                synthetic: false,
            }),
        }));
        for chunk in text.chars().collect::<Vec<_>>().chunks(8) {
            self.agent(AgentEvent::Provider(ProviderEvent::PartDelta {
                part_id,
                field: "text",
                delta: chunk.iter().collect(),
            }));
        }
        self.agent(AgentEvent::Provider(ProviderEvent::PartEnd { part_id }));
        self.agent(AgentEvent::Provider(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            usage: Tokens::default(),
            cost: 0.0,
        }));
        self.app.streaming = false;
    }

    /// Inject a terminal error event.
    pub fn error(&mut self, message: &str) {
        self.agent(AgentEvent::Error(message.to_string()));
    }

    /// Inject a completed tool call with `output` and optional `diff`.
    /// Mirrors the real event sequence: PartStart(ToolCall) → ToolStart →
    /// ToolEnd → PartUpdated(Completed). The resulting `ToolDisplayState`
    /// carries the output + diff that the chat renderer reads.
    pub fn say_tool_call(&mut self, tool_name: &str, output: &str, diff: Option<&str>) {
        use mew_message::{PartBase, ToolCallPart, ToolState, ToolStateCompleted, ToolTime};
        let part_id = PartId::new();
        let msg_id = MessageId::new();
        let session_id = SessionId::new();
        let call_id = format!("call_{}", ulid::Ulid::new());
        let base = PartBase {
            id: part_id,
            message_id: msg_id,
            session_id,
        };
        // Ensure an assistant message exists to hold the tool call part.
        if !self.app.messages.iter().any(|m| m.role == Role::Assistant) {
            self.app.messages.push(Message {
                id: msg_id,
                session_id,
                role: Role::Assistant,
                parts: vec![],
                time: Time {
                    created: chrono::Utc::now().timestamp_millis(),
                    completed: None,
                },
                assistant: None,
            });
        }
        let part = Part::ToolCall(ToolCallPart {
            base: base.clone(),
            tool_name: tool_name.to_string(),
            call_id: call_id.clone(),
            state: ToolState::Pending(mew_message::ToolStatePending {
                input: serde_json::json!({}),
                time: ToolTime {
                    start: 0,
                    end: None,
                },
            }),
            raw_input: String::new(),
        });
        self.agent(AgentEvent::Provider(ProviderEvent::PartStart {
            part: part.clone(),
        }));
        self.agent(AgentEvent::ToolStart {
            call_id: call_id.clone(),
        });
        self.agent(AgentEvent::ToolEnd {
            call_id: call_id.clone(),
            success: true,
        });
        let completed = Part::ToolCall(ToolCallPart {
            base: base.clone(),
            tool_name: tool_name.to_string(),
            call_id: call_id.clone(),
            state: ToolState::Completed(ToolStateCompleted {
                input: serde_json::json!({}),
                output: output.to_string(),
                metadata: None,
                diff: diff.map(|s| s.to_string()),
                time: ToolTime {
                    start: 0,
                    end: Some(0),
                },
            }),
            raw_input: String::new(),
        });
        self.agent(AgentEvent::PartUpdated {
            part_id,
            part: completed,
        });
    }

    /// Inject a reasoning/thinking part with `text`. The block renders
    /// collapsed by default (header line only) unless expanded.
    pub fn say_reasoning(&mut self, text: &str) {
        use mew_message::{PartBase, ReasoningPart};
        let part_id = PartId::new();
        let msg_id = MessageId::new();
        let session_id = SessionId::new();
        if !self.app.messages.iter().any(|m| m.role == Role::Assistant) {
            self.app.messages.push(Message {
                id: msg_id,
                session_id,
                role: Role::Assistant,
                parts: vec![],
                time: Time {
                    created: chrono::Utc::now().timestamp_millis(),
                    completed: None,
                },
                assistant: None,
            });
        }
        self.agent(AgentEvent::Provider(ProviderEvent::PartStart {
            part: Part::Reasoning(ReasoningPart {
                base: PartBase {
                    id: part_id,
                    message_id: msg_id,
                    session_id,
                },
                text: text.to_string(),
                signature: None,
            }),
        }));
        self.agent(AgentEvent::Provider(ProviderEvent::PartEnd { part_id }));
    }

    /// Feed a raw `AgentEvent` to the app, exactly as the real event loop does.
    pub fn agent(&mut self, event: AgentEvent) {
        self.app.handle_agent_event(event);
    }

    /// Render the current state and return the frame as text, with trailing
    /// whitespace trimmed from each row.
    pub fn render(&mut self) -> String {
        let app = &mut self.app;
        self.terminal.draw(|f| ui::draw(f, app)).expect("draw");
        buffer_to_string(self.terminal.backend().buffer())
    }

    fn push_user_message(&mut self, text: String) {
        let msg_id = MessageId::new();
        let session_id = SessionId::new();
        self.app.messages.push(Message {
            id: msg_id,
            session_id,
            role: Role::User,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: PartId::new(),
                    message_id: msg_id,
                    session_id,
                },
                text,
                synthetic: false,
            })],
            time: Time {
                created: chrono::Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        });
    }
}

/// Render a ratatui buffer to text, one line per row, trailing spaces trimmed.
fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area();
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let start = out.len();
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        while out.len() > start && out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}

/// Run a line-based script against a fresh harness and return the accumulated
/// snapshots as text. See the module docs for the verb list.
pub fn run_script(script: &str, width: u16, height: u16) -> String {
    let mut h = Harness::new(width, height);
    let mut out = String::new();

    for (i, raw) in script.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (verb, rest) = match line.split_once(char::is_whitespace) {
            Some((v, r)) => (v, r.trim()),
            None => (line, ""),
        };
        match verb {
            "size" | "resize" => match parse_size(rest) {
                Some((w, hh)) => h.resize(w, hh),
                None => out.push_str(&format!("!! line {}: bad size '{rest}'\n", i + 1)),
            },
            "type" => h.type_str(rest),
            "key" => match parse_key(rest) {
                Some(key) => h.key(key),
                None => out.push_str(&format!("!! line {}: unknown key '{rest}'\n", i + 1)),
            },
            "submit" => h.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            "say" => h.say(rest),
            "error" => h.error(rest),
            "snapshot" => {
                let label = if rest.is_empty() {
                    String::new()
                } else {
                    format!(" {rest}")
                };
                out.push_str(&format!("--- snapshot{label} ---\n"));
                out.push_str(&h.render());
                out.push_str("---\n");
            }
            other => out.push_str(&format!("!! line {}: unknown verb '{other}'\n", i + 1)),
        }
    }
    out
}

fn parse_size(s: &str) -> Option<(u16, u16)> {
    let mut parts = s
        .split(|c: char| c == 'x' || c.is_whitespace())
        .filter(|p| !p.is_empty());
    let w = parts.next()?.parse().ok()?;
    let h = parts.next()?.parse().ok()?;
    Some((w, h))
}

/// Parse a key name into a `KeyEvent`. Supports named keys, `ctrl+`/`alt+`
/// modifier prefixes, and single characters.
fn parse_key(name: &str) -> Option<KeyEvent> {
    let name = name.trim();
    let (mods, key) = if let Some(rest) = name.strip_prefix("ctrl+") {
        (KeyModifiers::CONTROL, rest)
    } else if let Some(rest) = name.strip_prefix("alt+") {
        (KeyModifiers::ALT, rest)
    } else {
        (KeyModifiers::NONE, name)
    };
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None; // multi-char, not a known name
            }
            KeyCode::Char(c)
        }
    };
    Some(KeyEvent::new(code, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_appears_in_composer() {
        let mut h = Harness::new(80, 24);
        h.type_str("hello world");
        assert!(h.render().contains("hello world"));
        assert_eq!(h.app.input, "hello world");
    }

    #[test]
    fn submit_echoes_user_message_and_streams() {
        let mut h = Harness::new(80, 24);
        h.type_str("do the thing");
        h.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(h.app.streaming);
        assert!(matches!(h.actions.last(), Some(Action::Submit(_))));
        assert!(h.render().contains("do the thing"));
    }

    #[test]
    fn assistant_text_turn_renders() {
        let mut h = Harness::new(80, 24);
        h.say("the answer is 42");
        assert!(!h.app.streaming);
        assert!(h.render().contains("the answer is 42"));
    }

    #[test]
    fn script_runs_and_snapshots() {
        let out = run_script(
            "# a quick session\ntype hi\nsubmit\nsay hello back\nsnapshot result",
            80,
            24,
        );
        assert!(out.contains("--- snapshot result ---"));
        assert!(out.contains("hello back"));
    }

    #[test]
    fn parse_key_handles_modifiers_and_names() {
        assert_eq!(
            parse_key("ctrl+c"),
            Some(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        );
        assert_eq!(
            parse_key("enter"),
            Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        );
        assert_eq!(parse_key("nope-not-a-key"), None);
    }
}
