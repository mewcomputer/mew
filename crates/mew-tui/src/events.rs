use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use futures::StreamExt;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;

/// Events that drive the TUI.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
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
                        if matches!(
                            event,
                            CrosstermEvent::Key(_)
                                | CrosstermEvent::Mouse(_)
                                | CrosstermEvent::Paste(_)
                        ) && tx.send(Event::Input(event)).await.is_err()
                        {
                            break;
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

/// Process any crossterm input event and return an action.
pub fn handle_input_event(app: &mut crate::app::App, event: CrosstermEvent) -> Option<Action> {
    match event {
        CrosstermEvent::Key(key) => handle_key_event(app, key),
        CrosstermEvent::Mouse(mouse) => handle_mouse_event(app, mouse),
        CrosstermEvent::Paste(text) => handle_paste_event(app, text),
        _ => None,
    }
}

/// Process a crossterm key event and return an action.
pub fn handle_key_event(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    match app.mode {
        crate::app::Mode::PermissionPrompt => handle_permission_key(app, key),
        crate::app::Mode::UserQuestion => handle_user_question_key(app, key),
        crate::app::Mode::CommandPalette => handle_picker_key(app, key),
        crate::app::Mode::PersonaSwitchConfirm => handle_persona_confirm_key(app, key),
        crate::app::Mode::Help => handle_help_key(app, key),
        crate::app::Mode::HistorySearch => handle_history_search_key(app, key),
        crate::app::Mode::PasteConfirm => handle_paste_confirm_key(app, key),
        crate::app::Mode::Normal | crate::app::Mode::SlashCommand => handle_normal_key(app, key),
        // Settings mode key handling is done by ConfigEditor in main.rs
        crate::app::Mode::Settings => None,
    }
}

/// Handle keys while the help overlay is open.
fn handle_help_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    use crossterm::event::{KeyCode, KeyModifiers};
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Char(' ') => {
            app.mode = crate::app::Mode::Normal;
        }
        _ => {}
    }
    let _ = KeyModifiers::empty();
    None
}

/// Handle keys during Ctrl+R history search.
fn handle_history_search_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    use crossterm::event::{KeyCode, KeyModifiers};
    match key.code {
        KeyCode::Esc => {
            app.history_search_cancel();
        }
        KeyCode::Enter => {
            app.history_search_confirm();
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+R again: cycle to next match.
            app.history_search_next();
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+S: cycle to previous match.
            app.history_search_prev();
        }
        KeyCode::Up => {
            app.history_search_prev();
        }
        KeyCode::Down => {
            app.history_search_next();
        }
        KeyCode::Char(c) => {
            app.history_search_query.push(c);
            // Reset to first match.
            app.history_search_index = if app.history_search_matches().is_empty() {
                None
            } else {
                Some(0)
            };
        }
        KeyCode::Backspace => {
            app.history_search_query.pop();
            app.history_search_index = if app.history_search_matches().is_empty() {
                None
            } else {
                Some(0)
            };
        }
        _ => {}
    }
    None
}

/// Handle keys during large-paste confirmation.
fn handle_paste_confirm_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            if let Some(pending) = app.pending_paste.take() {
                insert_paste(app, &pending);
            }
            app.mode = crate::app::Mode::Normal;
        }
        _ => {
            // Any other key cancels the paste.
            app.pending_paste = None;
            app.mode = crate::app::Mode::Normal;
        }
    }
    None
}

/// Handle keys while the persona-switch confirm modal is open. The modal
/// owns its own mode and dispatches the confirmed name back to the main
/// event loop via `Action::PersonaSwitchConfirmed`.
fn handle_persona_confirm_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    use crossterm::event::{KeyCode, KeyModifiers};
    match key.code {
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.persona_switch_confirm = None;
            app.mode = crate::app::Mode::Normal;
            None
        }
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            // Enter confirms whichever button is focused; Y is unambiguous
            // confirmation regardless of focus.
            if key.code == KeyCode::Enter {
                let name = app.take_confirmed_persona_switch();
                if let Some(name) = name {
                    return Some(Action::PersonaSwitchConfirmed(name));
                }
            } else {
                // y/Y forces the confirm path even if "Cancel" is focused.
                if let Some(state) = app.persona_switch_confirm.take() {
                    app.mode = crate::app::Mode::Normal;
                    return Some(Action::PersonaSwitchConfirmed(state.target.name));
                }
            }
            None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.persona_confirm_focus(-1);
            None
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
            app.persona_confirm_focus(1);
            None
        }
        KeyCode::BackTab => {
            app.persona_confirm_focus(-1);
            None
        }
        _ if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') => {
            // Ctrl-C cancels, matching the rest of the app.
            app.persona_switch_confirm = None;
            app.mode = crate::app::Mode::Normal;
            None
        }
        _ => None,
    }
}

fn handle_paste_event(app: &mut crate::app::App, text: String) -> Option<Action> {
    // If the whole paste is a path to an image, turn it into an @mention.
    let candidate = text.trim().strip_prefix("file://").unwrap_or(text.trim());
    let is_image = std::path::Path::new(candidate)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            )
        })
        .unwrap_or(false);
    let content = if is_image {
        format!("@{}", candidate)
    } else {
        text
    };

    // Large paste confirmation: if >2000 chars, ask the user first.
    const PASTE_CONFIRM_THRESHOLD: usize = 2000;
    if content.len() > PASTE_CONFIRM_THRESHOLD {
        app.pending_paste = Some(content);
        app.mode = crate::app::Mode::PasteConfirm;
        return None;
    }

    insert_paste(app, &content);
    None
}

/// Insert pasted content into the input buffer (single undo entry).
fn insert_paste(app: &mut crate::app::App, content: &str) {
    app.push_undo();
    for c in content.chars() {
        app.input.insert(app.cursor, c);
        app.cursor += c.len_utf8();
    }
    if app.input.starts_with('/') {
        app.mode = crate::app::Mode::SlashCommand;
    }
}

fn handle_mouse_event(app: &mut crate::app::App, mouse: MouseEvent) -> Option<Action> {
    if !matches!(
        app.mode,
        crate::app::Mode::Normal | crate::app::Mode::SlashCommand
    ) {
        return None;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_up(1);
            None
        }
        MouseEventKind::ScrollDown => {
            app.scroll_down(1);
            None
        }
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Right) => {
            let row = mouse.row.saturating_sub(1);
            let col = mouse.column.saturating_sub(1);

            // Scrollbar click → seek.
            let scrollbar_col = app.chat_area.right().saturating_sub(1);
            if col == scrollbar_col && app.chat_area.height > 0 && app.max_scroll > 0 {
                let rel_y = row.saturating_sub(app.chat_area.y) as f64;
                let ratio = (rel_y / app.chat_area.height as f64).clamp(0.0, 1.0);
                let target = (app.max_scroll as f64 * ratio).round() as u16;
                let target = target.min(app.max_scroll);
                if target >= app.max_scroll {
                    app.auto_scroll = true;
                    app.scroll = app.max_scroll;
                } else {
                    app.auto_scroll = false;
                    app.scroll = target;
                }
                return None;
            }

            // Input click → position cursor and clear selection.
            if app.input_area.contains((col, row).into()) {
                app.clear_selection();
                let rel_x = col.saturating_sub(app.input_area.x) as usize;
                let rel_y = row.saturating_sub(app.input_area.y) as usize;
                // The input has a 1-cell border on each side and a 2-cell
                // prefix (`> `) on the first visual row of each logical
                // line. Convert the mouse cell to a (visual_row, visual_col)
                // pair in the wrapped content grid.
                let content_width = app.input_area.width.saturating_sub(2);
                let visual_row = rel_y.saturating_sub(1);
                let visual_col = rel_x.saturating_sub(1).saturating_sub(2);
                let total = app.input_visual_line_count(content_width);
                if visual_row < total {
                    app.cursor = app
                        .visual_to_byte_offset(visual_row, visual_col, content_width)
                        .min(app.input.len());
                }
                return None;
            }

            // Sidebar click → toggle section.
            if app.sidebar_rect.contains((col, row).into()) {
                for (header_row, section) in &app.sidebar_header_rows.clone() {
                    if row == *header_row {
                        app.toggle_sidebar_section(section);
                        return None;
                    }
                }
            }

            // Chat area click → start selection.
            if app.chat_area.contains((col, row).into()) && !app.chat_rows.is_empty() {
                let rel_row = row.saturating_sub(app.chat_area.y) as usize;
                let visual_row = (app.scroll as usize + rel_row).min(app.chat_rows.len() - 1);

                // Check if this click landed on a reasoning block header.
                if let Some(&(id, _)) = app
                    .reasoning_header_rows
                    .iter()
                    .find(|(_, header_row)| *header_row == visual_row)
                {
                    if app.reasoning_expanded.contains(&id) {
                        app.reasoning_expanded.remove(&id);
                    } else {
                        app.reasoning_expanded.insert(id);
                    }
                    return None;
                }

                let rel_col = col.saturating_sub(app.chat_area.x) as usize;
                app.sel_anchor_row = Some(visual_row);
                app.sel_anchor_col = Some(rel_col);
                app.sel_end_row = Some(visual_row);
                app.sel_end_col = Some(rel_col);
                return None;
            }
            None
        }
        MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Right) => {
            let row = mouse.row.saturating_sub(1);
            let col = mouse.column.saturating_sub(1);
            if app.chat_area.contains((col, row).into())
                && !app.chat_rows.is_empty()
                && app.sel_anchor_row.is_some()
            {
                let rel_row = row.saturating_sub(app.chat_area.y) as usize;
                let visual_row = (app.scroll as usize + rel_row).min(app.chat_rows.len() - 1);
                let rel_col = col.saturating_sub(app.chat_area.x) as usize;
                app.sel_end_row = Some(visual_row);
                app.sel_end_col = Some(rel_col);
                return None;
            }
            None
        }
        MouseEventKind::Up(MouseButton::Left) | MouseEventKind::Up(MouseButton::Right) => {
            let text = app.selected_text();
            if !text.is_empty() {
                return Some(Action::CopySelection(text));
            }
            None
        }
        _ => None,
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

fn handle_user_question_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    let on_review = app
        .user_question
        .as_ref()
        .map(|uq| uq.review)
        .unwrap_or(false);

    match key.code {
        KeyCode::Enter => {
            app.user_question_confirm();
            None
        }
        KeyCode::Esc => {
            app.cancel_user_question();
            None
        }
        KeyCode::Tab => {
            if on_review {
                app.user_question_review_next();
            } else {
                app.user_question_select_next();
            }
            None
        }
        KeyCode::BackTab => {
            if on_review {
                app.user_question_review_prev();
            } else {
                app.user_question_select_prev();
            }
            None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !on_review {
                app.user_question_select_next();
            }
            None
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if !on_review {
                app.user_question_select_prev();
            }
            None
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if on_review {
                app.user_question_review_next();
            }
            None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if on_review {
                app.user_question_review_prev();
            }
            None
        }
        KeyCode::Char('y') => {
            if on_review {
                app.submit_user_question();
            }
            None
        }
        KeyCode::Char('n') => {
            if on_review {
                app.cancel_user_question();
            }
            None
        }
        KeyCode::Backspace => {
            if !on_review {
                app.user_question_backspace();
            }
            None
        }
        KeyCode::Char(c) if !on_review && c.is_ascii_digit() && c != '0' => {
            if let Some(n) = c.to_digit(10) {
                app.user_question_jump(n as usize);
            }
            None
        }
        KeyCode::Char(c) => {
            if !on_review {
                app.user_question_type_char(c);
            }
            None
        }
        _ => None,
    }
}

fn handle_normal_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    // Any key other than Esc dismisses the pending-cancel hint.
    if key.code != KeyCode::Esc {
        app.esc_cancel_pending = None;
    }

    // Global shortcuts.
    match key.code {
        KeyCode::Esc => {
            // If there's an active selection, clear it on Esc.
            if app.sel_anchor_row.is_some() {
                app.clear_selection();
                return None;
            }
            if app.streaming {
                if app.esc_cancel_pending.is_some() {
                    app.esc_cancel_pending = None;
                    return Some(Action::Cancel);
                } else {
                    app.esc_cancel_pending = Some(Instant::now());
                }
            }
            return None;
        }
        // Ctrl+1/2/3: toggle sidebar sections
        KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Some(Action::ToggleSidebarContext);
        }
        KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Some(Action::ToggleSidebarTools);
        }
        KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Some(Action::ToggleSidebarMcp);
        }
        // Ctrl+Shift+C: copy selected text to clipboard.
        KeyCode::Char('C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let text = app.selected_text();
            if !text.is_empty() {
                return Some(Action::CopySelection(text));
            }
            return None;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.streaming {
                if app.ctrl_c_quit_pending.is_some() {
                    app.should_quit = true;
                    return Some(Action::Quit);
                } else {
                    app.ctrl_c_quit_pending = Some(Instant::now());
                    return None;
                }
            } else if !app.input.is_empty() {
                app.input.clear();
                app.cursor = 0;
                return None;
            } else {
                app.should_quit = true;
                return Some(Action::Quit);
            }
        }
        KeyCode::Char('d')
            if key.modifiers.contains(KeyModifiers::CONTROL) && app.input.is_empty() =>
        {
            app.should_quit = true;
            return Some(Action::Quit);
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.open_command_palette();
            return None;
        }
        // `?` opens the keyboard shortcuts overlay (when input is empty
        // so it doesn't shadow literal `?` in chat messages).
        KeyCode::Char('?') if app.input.is_empty() => {
            app.mode = crate::app::Mode::Help;
            return None;
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.toggle_bash_expanded();
            return None;
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.toggle_reasoning_expanded();
            return None;
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.auto_scroll = true;
            app.scroll = app.max_scroll;
            return None;
        }
        KeyCode::Char('x') if app.input.is_empty() => {
            // Cancel the most recently started running subagent. Only
            // triggers when the input is empty so it doesn't shadow the
            // literal character in chat messages.
            if let Some(task_id) = app.most_recent_running_subagent() {
                return Some(Action::CancelMostRecentSubagent(task_id));
            }
            return None;
        }
        _ => {}
    }

    // Input handling.
    match key.code {
        KeyCode::Enter => {
            // Alt+Enter inserts a newline (multiline input).
            if key.modifiers.contains(KeyModifiers::ALT) {
                app.insert_newline();
                return None;
            }
            // If slash autocomplete is showing, select the highlighted command.
            if app.mode == crate::app::Mode::SlashCommand
                && !app.filtered_slash_commands().is_empty()
            {
                app.apply_slash_completion();
                if let Some(text) = app.submit_input() {
                    return Some(Action::SlashCommand(text));
                }
                return None;
            }
            if let Some(text) = app.submit_input() {
                if text.starts_with('/') {
                    return Some(Action::SlashCommand(text));
                }
                return Some(Action::Submit(text));
            }
            None
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_home();
            None
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_end();
            None
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_right();
            None
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_left();
            None
        }
        // Ctrl+R: reverse search through input history.
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.start_history_search();
            None
        }
        // Ctrl+Z: undo. Coalesces consecutive edits within 500ms.
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.undo();
            None
        }
        // Ctrl+Shift+Z (or Ctrl+Y): redo.
        KeyCode::Char('Z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.redo();
            None
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.redo();
            None
        }
        // Ctrl+U: clear to beginning of line (macOS Cmd+Backspace sends this).
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.push_undo();
            app.input.clear();
            app.cursor = 0;
            app.mode = crate::app::Mode::Normal;
            None
        }
        // Ctrl+W: delete word backward (macOS Option+Delete sends this).
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.push_undo();
            // Delete back to the previous word boundary.
            let before = &app.input[..app.cursor];
            let new_cursor = before
                .trim_end_matches(|c: char| c.is_whitespace())
                .rfind(|c: char| c.is_whitespace())
                .map(|i| i + 1)
                .unwrap_or(0);
            app.input.replace_range(new_cursor..app.cursor, "");
            app.cursor = new_cursor;
            if !app.input.starts_with('/') {
                app.mode = crate::app::Mode::Normal;
            }
            None
        }
        // Ctrl+K: kill from cursor to end of line (readline convention).
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.cursor < app.input.len() {
                app.push_undo();
                app.input.truncate(app.cursor);
                if !app.input.starts_with('/') {
                    app.mode = crate::app::Mode::Normal;
                }
            }
            None
        }
        // Ctrl+D: delete char forward (readline convention). Also quits
        // on empty input when not streaming (matching shell behavior).
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.input.is_empty() && !app.streaming {
                app.should_quit = true;
                return Some(Action::Quit);
            }
            app.delete_char();
            None
        }
        KeyCode::Char(c) => {
            app.insert_char(c);
            if app.input.starts_with('/') {
                app.mode = crate::app::Mode::SlashCommand;
                app.slash_selected = 0;
                app.slash_scroll = 0;
            }
            // Auto-open file picker when @ is typed at start or after a space.
            if c == '@' {
                let before = &app.input[..app.cursor.saturating_sub(1)];
                if before.is_empty() || before.ends_with(' ') {
                    app.open_file_picker("");
                }
            }
            None
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
            app.push_undo();
            app.delete_word_left();
            if !app.input.starts_with('/') {
                app.mode = crate::app::Mode::Normal;
            }
            None
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::META) => {
            app.push_undo();
            app.input.clear();
            app.cursor = 0;
            app.mode = crate::app::Mode::Normal;
            None
        }
        KeyCode::Backspace => {
            app.backspace();
            if !app.input.starts_with('/') {
                app.mode = crate::app::Mode::Normal;
            }
            app.slash_selected = 0;
            app.slash_scroll = 0;
            None
        }
        KeyCode::Delete => {
            app.delete_char();
            None
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_word_left();
            None
        }
        KeyCode::Left => {
            app.cursor_left();
            None
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_word_right();
            None
        }
        KeyCode::Right => {
            app.cursor_right();
            None
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.auto_scroll = false;
            app.scroll = 0;
            None
        }
        KeyCode::Home => {
            app.cursor_home();
            None
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.auto_scroll = true;
            app.scroll = app.max_scroll;
            None
        }
        KeyCode::End => {
            app.cursor_end();
            None
        }
        KeyCode::Tab if app.mode == crate::app::Mode::SlashCommand => {
            app.apply_slash_completion();
            None
        }
        KeyCode::Up => {
            if app.mode == crate::app::Mode::SlashCommand
                && !app.filtered_slash_commands().is_empty()
            {
                app.slash_prev();
            } else {
                // If input is multi-line, move cursor between visual lines.
                // Only navigate history when on the first visual line.
                let content_width = app.input_area.width.saturating_sub(2);
                if app.input_visual_line_count(content_width) > 1
                    && !app.input.is_empty()
                    && app.cursor_visual_up(content_width)
                {
                    // Cursor moved up within the input.
                } else if app.input.is_empty() {
                    app.scroll_up(1);
                } else {
                    app.history_up();
                }
            }
            None
        }
        KeyCode::Down => {
            if app.mode == crate::app::Mode::SlashCommand
                && !app.filtered_slash_commands().is_empty()
            {
                app.slash_next();
            } else {
                // If input is multi-line, move cursor between visual lines.
                // Only navigate history when on the last visual line.
                let content_width = app.input_area.width.saturating_sub(2);
                if app.input_visual_line_count(content_width) > 1
                    && !app.input.is_empty()
                    && app.cursor_visual_down(content_width)
                {
                    // Cursor moved down within the input.
                } else if app.input.is_empty() {
                    app.scroll_down(1);
                } else {
                    app.history_down();
                }
            }
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

fn handle_picker_key(app: &mut crate::app::App, key: KeyEvent) -> Option<Action> {
    // Ctrl+P in the thinking variant picker cycles to the next variant.
    if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if let Some(ref picker) = app.picker {
            if picker.kind == "thinking_variant" {
                let items_len = picker.items.len();
                if items_len > 0 {
                    let next = (picker.selected + 1) % items_len;
                    if let Some(ref mut picker) = app.picker {
                        picker.selected = next;
                        picker.adjust_scroll();
                    }
                    // Return the selected variant as an action.
                    let item = app
                        .picker
                        .as_ref()
                        .and_then(|p| p.selected_item().map(|i| i.id.clone()));
                    if let Some(variant) = item {
                        return Some(Action::SetThinkingVariant(variant));
                    }
                }
            }
        }
        return None;
    }
    match key.code {
        KeyCode::Esc => {
            app.close_picker();
            None
        }
        KeyCode::Enter => {
            let picker_data = app.picker.as_ref().and_then(|p| {
                p.selected_item()
                    .map(|item| (item.id.clone(), p.kind.clone(), item.label.clone()))
            });
            if let Some((id, kind, label)) = picker_data {
                app.close_picker();
                if kind == "command" {
                    match id.as_str() {
                        "switch-model" => {
                            app.open_model_picker();
                            None
                        }
                        "thinking-variant" => {
                            app.open_thinking_variant_picker();
                            None
                        }
                        "settings" => Some(Action::OpenSettings),
                        "clear" => Some(Action::Clear),
                        "quit" => {
                            app.should_quit = true;
                            Some(Action::Quit)
                        }
                        _ => None,
                    }
                } else if kind == "model" {
                    Some(Action::SwitchModel(id))
                } else if kind == "thinking_variant" {
                    Some(Action::SetThinkingVariant(id))
                } else if kind == "permission_mode" {
                    mew_hooks::PermissionMode::from_id(&id).map(Action::SetPermissionMode)
                } else if kind == "file" {
                    if label.contains("[subagent]") {
                        Some(Action::InsertSubagentMention(id))
                    } else {
                        Some(Action::InsertAtMention(format!("@{}", id)))
                    }
                } else if kind == "session" {
                    Some(Action::AttachSession(id))
                } else {
                    None
                }
            } else {
                None
            }
        }
        KeyCode::Up => {
            app.picker_up();
            None
        }
        KeyCode::Down => {
            app.picker_down();
            None
        }
        KeyCode::Char(c) => {
            app.picker_insert(c);
            None
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::META) => {
            if let Some(ref mut p) = app.picker {
                p.filter.clear();
                p.cursor = 0;
            }
            None
        }
        KeyCode::Backspace => {
            app.picker_backspace();
            None
        }
        KeyCode::Left => {
            app.picker_cursor_left();
            None
        }
        KeyCode::Right => {
            app.picker_cursor_right();
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
    /// Clear the chat.
    Clear,
    /// Switch to a different model.
    SwitchModel(String),
    /// Insert an @mention path into the input.
    InsertAtMention(String),
    /// Insert an @mention subagent reference.
    InsertSubagentMention(String),
    /// Copy selected text to clipboard.
    CopySelection(String),
    /// Toggle sidebar section collapsed state.
    ToggleSidebarContext,
    ToggleSidebarTools,
    ToggleSidebarMcp,
    /// Save settings from the settings page.
    SaveSettings,
    /// Start editing a field in the settings page.
    SettingsEditStart,
    /// Complete editing a field in the settings page.
    SettingsEditComplete,
    /// Open the settings page (populate plugins).
    OpenSettings,
    /// Cancel the most recently started running subagent. Carries the
    /// task id so the main loop can resolve which task to cancel without
    /// re-deriving it from app state.
    CancelMostRecentSubagent(String),
    /// User confirmed a persona switch from the confirm modal. The string
    /// is the target persona name.
    PersonaSwitchConfirmed(String),
    /// Apply a permission mode change. Fires from `/permissions <mode>` and
    /// from the permission-mode picker.
    SetPermissionMode(mew_hooks::PermissionMode),
    /// Set or clear the thinking variant. Fires from `/thinking <variant>`,
    /// the thinking variant picker, or Ctrl+P cycling.
    SetThinkingVariant(String),
    /// Attach to a different daemon session (switch chat). Only meaningful
    /// in daemon mode; local `run_tui` handles it as a no-op with an alert.
    AttachSession(String),
}
