use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

/// Events that drive the TUI.
#[derive(Debug)]
pub enum Event {
    /// A crossterm input event.
    Input(CrosstermEvent),
    /// An agent event.
    Agent(mew_agent::AgentEvent),
    /// A tick for rendering.
    Tick,
    /// Application should quit.
    Quit,
}

/// Drives the event loop, merging crossterm and tick events.
/// Agent events are forwarded into this loop from outside.
pub struct EventLoop {
    pub tx: mpsc::Sender<Event>,
}

impl EventLoop {
    pub fn new() -> (Self, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel(256);
        (Self { tx }, rx)
    }

    /// Spawn the crossterm and tick event readers.
    pub fn spawn(&self) {
        let tx = self.tx.clone();

        // Crossterm event reader.
        tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            loop {
                match reader.next().await {
                    Some(Ok(event)) => {
                        if matches!(event, CrosstermEvent::Key(_)) {
                            if tx.send(Event::Input(event)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!("crossterm error: {}", e);
                    }
                    None => break,
                }
            }
        });

        // Tick generator (60fps).
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(16));
            loop {
                interval.tick().await;
                if tx.send(Event::Tick).await.is_err() {
                    break;
                }
            }
        });
    }

    /// Forward agent events from a receiver into the event loop.
    pub fn forward_agent_events(&self, mut agent_rx: mpsc::Receiver<mew_agent::AgentEvent>) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Some(event) = agent_rx.recv().await {
                if tx.send(Event::Agent(event)).await.is_err() {
                    break;
                }
            }
        });
    }
}

/// Process a crossterm key event and return an action.
pub fn handle_key_event(
    app: &mut crate::app::App,
    key: KeyEvent,
) -> Option<Action> {
    match app.mode {
        crate::app::Mode::PermissionPrompt => handle_permission_key(app, key),
        crate::app::Mode::Normal | crate::app::Mode::SlashCommand => handle_normal_key(app, key),
    }
}

fn handle_permission_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.send_permission_decision(mew_hooks::PermissionDecision::AllowOnce);
            None
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.send_permission_decision(mew_hooks::PermissionDecision::AllowSession);
            None
        }
        KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Esc => {
            app.send_permission_decision(mew_hooks::PermissionDecision::Deny);
            None
        }
        KeyCode::Down | KeyCode::Tab => {
            app.permission_next();
            None
        }
        KeyCode::Up => {
            app.permission_prev();
            None
        }
        KeyCode::Enter => {
            let decision = app.permission.as_ref().map(|p| match p.selected {
                0 => mew_hooks::PermissionDecision::AllowOnce,
                1 => mew_hooks::PermissionDecision::AllowSession,
                _ => mew_hooks::PermissionDecision::Deny,
            });
            if let Some(d) = decision {
                app.send_permission_decision(d);
            }
            None
        }
        _ => None,
    }
}

fn handle_normal_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    // Global shortcuts.
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.streaming {
                return Some(Action::Cancel);
            } else if !app.input.is_empty() {
                app.input.clear();
                app.cursor = 0;
                return None;
            } else {
                app.should_quit = true;
                return Some(Action::Quit);
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) && app.input.is_empty() => {
            app.should_quit = true;
            return Some(Action::Quit);
        }
        _ => {}
    }

    // Input handling.
    match key.code {
        KeyCode::Enter => {
            if let Some(text) = app.submit_input() {
                if text.starts_with('/') {
                    return Some(Action::SlashCommand(text));
                }
                return Some(Action::Submit(text));
            }
            None
        }
        KeyCode::Char(c) => {
            app.insert_char(c);
            if app.input.starts_with('/') {
                app.mode = crate::app::Mode::SlashCommand;
            }
            None
        }
        KeyCode::Backspace => {
            app.backspace();
            if !app.input.starts_with('/') {
                app.mode = crate::app::Mode::Normal;
            }
            None
        }
        KeyCode::Delete => {
            app.delete_char();
            None
        }
        KeyCode::Left => {
            app.cursor_left();
            None
        }
        KeyCode::Right => {
            app.cursor_right();
            None
        }
        KeyCode::Home => {
            app.cursor_home();
            None
        }
        KeyCode::End => {
            app.cursor_end();
            None
        }
        KeyCode::Up => {
            app.history_up();
            None
        }
        KeyCode::Down => {
            app.history_down();
            None
        }
        KeyCode::PageUp => {
            app.scroll_up(10);
            None
        }
        KeyCode::PageDown => {
            app.scroll_down(10);
            None
        }
        _ => None,
    }
}

/// High-level actions produced by input handling.
#[derive(Debug, Clone)]
pub enum Action {
    /// Submit a user message.
    Submit(String),
    /// Execute a slash command.
    SlashCommand(String),
    /// Cancel the current streaming turn.
    Cancel,
    /// Quit the application.
    Quit,
}
