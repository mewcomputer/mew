use super::{App, Mode};
use crate::events::Action;
use mew_client_core::TurnOutcome;

impl App {
    pub fn conversation_is_open(&self, id: &str) -> bool {
        !id.is_empty()
            && (id == self.status.session_id || self.open_sessions.iter().any(|open| open == id))
    }

    pub(super) fn confirm_window_conversation(&mut self, id: &str) {
        if let Some(closed) = self
            .closing_conversation
            .take()
            .filter(|closed| closed != id)
        {
            self.open_sessions.retain(|open| open != &closed);
        }
        if !self.open_sessions.iter().any(|open| open == id) {
            self.open_sessions.push(id.to_owned());
        }
    }

    /// Closing a view never cancels or archives daemon work or deletes its draft.
    pub fn close_window_conversation(&mut self, id: &str) -> Option<Action> {
        if self.attaching_session || self.closing_conversation.is_some() {
            self.set_alert("waiting for the conversation to be confirmed");
            return None;
        }
        if !self.conversation_is_open(id) {
            return None;
        }
        if id == self.status.session_id {
            if let Some(next) = self
                .open_sessions
                .iter()
                .rev()
                .find(|open| open.as_str() != id)
            {
                let next = next.clone();
                self.closing_conversation = Some(id.to_owned());
                return Some(Action::AttachSession(next));
            }
            return Some(Action::Quit);
        }
        self.open_sessions.retain(|open| open != id);
        self.refresh_session_picker();
        None
    }

    pub fn cancel_window_close(&mut self) {
        self.closing_conversation = None;
    }

    pub fn adjacent_window_conversation(&self, direction: isize) -> Option<Action> {
        let index = self
            .open_sessions
            .iter()
            .position(|id| id == &self.status.session_id)?;
        if self.open_sessions.len() < 2 || self.attaching_session {
            return None;
        }
        let next = (index as isize + direction).rem_euclid(self.open_sessions.len() as isize);
        Some(Action::AttachSession(
            self.open_sessions[next as usize].clone(),
        ))
    }

    pub fn acknowledge_visible_result(&mut self) {
        if self.terminal_focused == Some(true) {
            self.acknowledge_result_on_interaction();
        }
    }

    pub(crate) fn acknowledge_result_on_interaction(&mut self) {
        if !self.attaching_session
            && !self.awaiting_transcript
            && matches!(self.mode, Mode::Normal | Mode::SlashCommand)
            && self.picker.is_none()
            && (self.auto_scroll || self.scroll >= self.max_scroll)
            && self.unread_completions.remove(&self.status.session_id)
        {
            self.refresh_session_picker();
        }
    }

    pub(super) fn record_window_outcome(&mut self, id: &str, outcome: &TurnOutcome) {
        self.session_turn_outcomes
            .insert(id.to_owned(), outcome.clone());
        self.unread_completions.insert(id.to_owned());
        self.acknowledge_visible_result();
    }

    pub fn conversation_failed(&self, id: &str) -> bool {
        let session = self
            .daemon_sessions
            .iter()
            .find(|session| session.session_id == id);
        if session.is_some_and(|session| session.state == mew_protocol::SessionState::Running)
            || (id == self.status.session_id && self.streaming)
        {
            return false;
        }
        matches!(
            self.session_turn_outcomes.get(id),
            Some(TurnOutcome::Failed { .. })
        ) || session.is_some_and(|session| session.last_turn_failed)
    }

    pub fn conversation_needs_input(&self, id: &str) -> bool {
        let (permissions, questions) =
            self.session_attention.get(id).copied().unwrap_or_else(|| {
                self.daemon_sessions
                    .iter()
                    .find(|session| session.session_id == id)
                    .map_or((0, 0), |session| {
                        (session.pending_permissions, session.pending_questions)
                    })
            });
        permissions > 0
            || questions > 0
            || self.conversation_failed(id)
            || (id == self.status.session_id
                && (self.permission.is_some() || self.user_question.is_some()))
    }

    pub fn notification_counts(&self) -> (usize, usize) {
        let mut needed = 0;
        let mut results = 0;
        for id in self.open_sessions.iter().chain(
            std::iter::once(&self.status.session_id)
                .filter(|id| !id.is_empty() && !self.open_sessions.contains(id)),
        ) {
            if self.conversation_needs_input(id) {
                needed += 1;
            } else if self.unread_completions.contains(id) {
                results += 1;
            }
        }
        (needed, results)
    }

    pub fn notification_summary(&self) -> String {
        let (needed, results) = self.notification_counts();
        match (needed, results) {
            (0, _) => format!(
                "{results} {}",
                if results == 1 { "result" } else { "results" }
            ),
            (_, 0) => format!("{needed} needs you"),
            _ => format!(
                "{needed} needs you · {results} {}",
                if results == 1 { "result" } else { "results" }
            ),
        }
    }
}
