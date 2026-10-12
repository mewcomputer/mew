use super::{editor::UndoHistory, App, Mode};

pub(super) struct Submission {
    session_id: String,
    message_id: Option<mew_message::MessageId>,
    display: String,
    attachments: Vec<mew_message::Part>,
    draft: Option<String>,
    queued: bool,
    accepted: bool,
}

#[derive(Default)]
pub(super) struct ConversationDraft {
    input: String,
    cursor: usize,
    undo: UndoHistory,
    history: Vec<String>,
    queue: Vec<String>,
    context_files: Vec<String>,
    pending_paste: Option<String>,
    ready: bool,
}

impl App {
    pub fn restore_queued_for_session(&mut self, session_id: &str, text: String) {
        if session_id == self.status.session_id {
            self.queued_messages.insert(0, text);
        } else {
            self.session_drafts
                .entry(session_id.to_owned())
                .or_default()
                .queue
                .insert(0, text);
        }
    }

    pub fn accept_editor_input(&mut self, text: &str) {
        if self.input.trim() == text && !self.input.is_empty() {
            self.history.push(text.to_owned());
            self.input.clear();
            self.cursor = 0;
            self.history_index = None;
            self.mode = Mode::Normal;
        }
    }
    pub fn submission_pending(&self) -> bool {
        self.submissions.values().any(|submission| {
            submission.session_id == self.status.session_id && !submission.accepted
        })
    }

    pub fn begin_submission(
        &mut self,
        request_id: String,
        display: String,
        attachments: Vec<mew_message::Part>,
        queued: bool,
    ) {
        self.submissions.insert(
            request_id,
            Submission {
                session_id: self.status.session_id.clone(),
                message_id: None,
                display,
                attachments,
                draft: (!queued && !self.input.is_empty()).then(|| self.input.clone()),
                queued,
                accepted: false,
            },
        );
    }

    pub fn resolve_submission(
        &mut self,
        session_id: &str,
        request_id: &str,
        status: mew_protocol::PromptStatus,
        message: Option<&str>,
    ) {
        let Some(submission) = self.submissions.get_mut(request_id) else {
            return;
        };
        if submission.session_id != session_id {
            return;
        }
        if status == mew_protocol::PromptStatus::Accepted {
            if submission.accepted {
                return;
            }
            submission.accepted = true;
            if let Some(input) = &submission.draft {
                if self.status.session_id == session_id {
                    self.history.push(input.trim().to_owned());
                    if &self.input == input {
                        self.input.clear();
                        self.cursor = 0;
                        self.history_index = None;
                        self.mode = Mode::Normal;
                    }
                } else if let Some(draft) = self.session_drafts.get_mut(session_id) {
                    draft.history.push(input.trim().to_owned());
                    if &draft.input == input {
                        draft.input.clear();
                        draft.cursor = 0;
                    }
                }
            }
            if self.status.session_id == session_id {
                self.streaming = true;
            }
            self.present_submission(request_id);
            return;
        }
        let submission = self
            .submissions
            .remove(request_id)
            .expect("matched submission");
        if submission.queued {
            if self.status.session_id == session_id {
                self.queued_messages.insert(0, submission.display);
            } else if let Some(draft) = self.session_drafts.get_mut(session_id) {
                draft.queue.insert(0, submission.display);
            }
        }
        self.set_alert(
            message.unwrap_or(
                "submission was not confirmed; inspect the conversation before retrying",
            ),
        );
    }

    pub(super) fn record_submission_echo(
        &mut self,
        session_id: &str,
        request_id: &str,
        message_id: mew_message::MessageId,
    ) {
        let Some(submission) = self.submissions.get_mut(request_id) else {
            return;
        };
        if submission.session_id != session_id {
            return;
        }
        submission.message_id = Some(message_id);
        self.present_submission(request_id);
    }

    fn present_submission(&mut self, request_id: &str) {
        let Some(submission) = self.submissions.get(request_id).filter(|submission| {
            submission.accepted && submission.session_id == self.status.session_id
        }) else {
            return;
        };
        let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| Some(message.id) == submission.message_id)
        else {
            return;
        };
        let submission = self
            .submissions
            .remove(request_id)
            .expect("matched submission");
        if let Some(mew_message::Part::Text(part)) = message
            .parts
            .iter_mut()
            .find(|part| matches!(part, mew_message::Part::Text(_)))
        {
            part.text = submission.display;
        }
        message.parts.extend(submission.attachments);
        self.mark_chat_dirty();
    }

    pub(super) fn switch_conversation_draft(&mut self, session_id: &str) {
        self.kill_ring.end_sequence();
        let current = ConversationDraft {
            input: std::mem::take(&mut self.input),
            cursor: std::mem::take(&mut self.cursor),
            undo: std::mem::take(&mut self.undo_history),
            history: std::mem::take(&mut self.history),
            queue: std::mem::take(&mut self.queued_messages),
            context_files: std::mem::take(&mut self.context_files),
            pending_paste: self.pending_paste.take(),
            ready: std::mem::take(&mut self.pending_queued_send),
        };
        self.session_drafts
            .insert(self.status.session_id.clone(), current);
        let draft = self.session_drafts.remove(session_id).unwrap_or_default();
        self.input = draft.input;
        self.cursor = draft.cursor;
        self.undo_history = draft.undo;
        self.history = draft.history;
        self.queued_messages = draft.queue;
        self.context_files = draft.context_files;
        self.pending_paste = draft.pending_paste;
        self.streaming = self.daemon_sessions.iter().any(|session| {
            session.session_id == session_id && session.state == mew_protocol::SessionState::Running
        });
        self.pending_queued_send = draft.ready && !self.streaming;
        self.cancelling = false;
        self.up_press = None;
        self.history_index = None;
        self.history_draft = None;
        self.history_search_saved = None;
        self.picker = None;
        self.mode = if self.pending_paste.is_some() {
            Mode::PasteConfirm
        } else if self.input.starts_with('/') {
            Mode::SlashCommand
        } else {
            Mode::Normal
        };
        self.clear_selection();
        self.clear_messages();
    }

    /// Completion can release only the queue owned by the finished session.
    pub fn finish_session_turn(&mut self, session_id: &str) {
        if self.status.session_id == session_id {
            self.finish_turn();
        } else if let Some(draft) = self.session_drafts.get_mut(session_id) {
            draft.ready = !draft.queue.is_empty();
        }
    }
}
