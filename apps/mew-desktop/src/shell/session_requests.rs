use super::drafts::{DraftOwner, PromptDraft};
use super::PromptHistory;
use mew_protocol::{Attachment, ClientKind, ClientMessage};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Default)]
pub(super) struct SessionRequests {
    pending: Option<PendingSession>,
    reconnect_required: bool,
    generation: usize,
}

enum PendingSession {
    Attach(String),
    Create {
        owner: DraftOwner,
        follow: bool,
        initial_prompt: Option<PromptDraft>,
    },
}

pub(super) struct ReadySession {
    pub(super) created_for: Option<DraftOwner>,
    pub(super) follow_creation: bool,
    pub(super) initial_prompt: Option<PromptDraft>,
}

impl SessionRequests {
    pub(super) fn generation(&self) -> usize {
        self.generation
    }

    pub(super) fn accepts_source(&self, source: usize) -> bool {
        self.generation == source
    }

    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some() || self.reconnect_required
    }

    pub(super) fn needs_reconnect(&self) -> bool {
        self.reconnect_required
    }

    pub(super) fn attach(
        &mut self,
        sender: Option<&UnboundedSender<ClientMessage>>,
        session_id: &str,
    ) -> bool {
        if self.is_pending() {
            return false;
        }
        let Some(sender) = sender else { return false };
        if sender
            .send(ClientMessage::AttachSession {
                session_id: session_id.into(),
                client_kind: ClientKind::Desktop,
            })
            .is_err()
        {
            return false;
        }
        self.pending = Some(PendingSession::Attach(session_id.into()));
        true
    }

    pub(super) fn create(
        &mut self,
        sender: Option<&UnboundedSender<ClientMessage>>,
        owner: DraftOwner,
        cwd: Option<String>,
        group_id: Option<String>,
    ) -> bool {
        if self.is_pending() {
            return false;
        }
        let Some(sender) = sender else { return false };
        let command = match group_id {
            Some(group_id) => ClientMessage::NewSessionInGroup {
                cwd,
                group_id,
                client_kind: ClientKind::Desktop,
            },
            None => ClientMessage::NewSession {
                cwd,
                client_kind: ClientKind::Desktop,
            },
        };
        if sender.send(command).is_err() {
            return false;
        }
        self.pending = Some(PendingSession::Create {
            owner,
            follow: true,
            initial_prompt: None,
        });
        true
    }

    pub(super) fn set_initial_prompt(&mut self, current: &DraftOwner, prompt: PromptDraft) -> bool {
        match &mut self.pending {
            Some(PendingSession::Create {
                owner,
                follow: true,
                initial_prompt,
            }) if owner == current => {
                *initial_prompt = Some(prompt);
                true
            }
            _ => false,
        }
    }

    pub(super) fn cancel_initial_prompt(&mut self) {
        if let Some(PendingSession::Create { initial_prompt, .. }) = &mut self.pending {
            *initial_prompt = None;
        }
    }

    pub(super) fn navigate(&mut self) {
        if let Some(PendingSession::Create {
            follow,
            initial_prompt,
            ..
        }) = &mut self.pending
        {
            *follow = false;
            *initial_prompt = None;
        }
    }

    pub(super) fn acknowledge_from(
        &mut self,
        source: usize,
        session_id: &str,
    ) -> Option<ReadySession> {
        if !self.accepts_source(source) || self.reconnect_required {
            return None;
        }
        match self.pending.as_ref()? {
            PendingSession::Attach(expected) if expected != session_id => return None,
            _ => {}
        }
        Some(match self.pending.take()? {
            PendingSession::Attach(_) => ReadySession {
                created_for: None,
                follow_creation: false,
                initial_prompt: None,
            },
            PendingSession::Create {
                owner,
                follow,
                initial_prompt,
            } => ReadySession {
                created_for: Some(owner),
                follow_creation: follow,
                initial_prompt,
            },
        })
    }

    pub(super) fn ambiguous_error(&mut self) {
        if self.pending.is_some() {
            self.reconnect_required = true;
            self.navigate();
        }
    }

    pub(super) fn reset_connection(&mut self) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            ..Self::default()
        };
    }
}

pub(super) fn queue_initial_prompt(
    initial: PromptDraft,
    sender: Option<&UnboundedSender<ClientMessage>>,
    text: &mut String,
    attachments: &mut Vec<Attachment>,
    history: &mut PromptHistory,
) -> Option<bool> {
    if initial.text != *text || initial.attachments != *attachments {
        return None;
    }
    Some(queue_prompt(sender, text, attachments, history))
}

// Success means accepted by this client's command queue, not acknowledged by the daemon.
pub(super) fn queue_prompt(
    sender: Option<&UnboundedSender<ClientMessage>>,
    text: &mut String,
    attachments: &mut Vec<Attachment>,
    history: &mut PromptHistory,
) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let Some(sender) = sender else { return false };
    let history_text = text.trim().to_owned();
    match sender.send(ClientMessage::Prompt {
        text: std::mem::take(text),
        attachments: std::mem::take(attachments),
    }) {
        Ok(()) => {
            history.record(history_text);
            true
        }
        Err(error) => {
            if let ClientMessage::Prompt {
                text: rejected_text,
                attachments: rejected,
            } = error.0
            {
                *text = rejected_text;
                *attachments = rejected;
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::drafts::PromptDraft;
    use crate::shell::{DraftOwner, PromptHistory};
    use mew_protocol::{Attachment, ClientMessage};

    #[test]
    fn rapid_navigation_queues_only_one_attachment_until_its_acknowledgement() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut requests = SessionRequests::default();
        assert!(requests.attach(Some(&tx), "a"));
        assert!(!requests.attach(Some(&tx), "b"));
        assert!(!requests.attach(Some(&tx), "a"));
        assert!(
            matches!(rx.try_recv().unwrap(), ClientMessage::AttachSession { session_id, .. } if session_id == "a")
        );
        assert!(rx.try_recv().is_err());
        assert!(requests
            .acknowledge_from(requests.generation(), "b")
            .is_none());
        assert!(requests.is_pending());
        assert!(requests
            .acknowledge_from(requests.generation(), "a")
            .is_some());
        assert!(!requests.is_pending());
        assert!(requests.attach(Some(&tx), "b"));
    }

    #[test]
    fn creation_stays_exclusive_and_navigation_cancels_its_automatic_submission() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let owner = DraftOwner::new_task("local");
        let mut requests = SessionRequests::default();
        assert!(requests.create(
            Some(&tx),
            owner.clone(),
            Some("workspace".into()),
            Some("group".into())
        ));
        assert!(requests.set_initial_prompt(
            &owner,
            PromptDraft {
                text: "initial".into(),
                attachments: vec![]
            }
        ));
        assert!(!requests.create(Some(&tx), owner.clone(), None, None));
        assert!(!requests.attach(Some(&tx), "other"));
        requests.navigate();
        let ready = requests
            .acknowledge_from(requests.generation(), "created")
            .unwrap();
        assert_eq!(ready.created_for, Some(owner));
        assert!(!ready.follow_creation);
        assert!(ready.initial_prompt.is_none());
        assert!(
            matches!(rx.try_recv().unwrap(), ClientMessage::NewSessionInGroup { cwd: Some(cwd), group_id, .. } if cwd == "workspace" && group_id == "group")
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn edits_cancel_initial_submission_without_canceling_the_created_task() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let owner = DraftOwner::new_task("local");
        let mut requests = SessionRequests::default();
        assert!(requests.create(Some(&tx), owner.clone(), None, None));
        assert!(requests.set_initial_prompt(
            &owner,
            PromptDraft {
                text: "initial".into(),
                attachments: vec![]
            }
        ));
        requests.cancel_initial_prompt();
        let ready = requests
            .acknowledge_from(requests.generation(), "created")
            .unwrap();
        assert!(ready.follow_creation);
        assert!(ready.initial_prompt.is_none());
    }

    #[test]
    fn ambiguous_error_requires_reconnect_and_never_releases_a_queued_prompt() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut requests = SessionRequests::default();
        assert!(requests.attach(Some(&tx), "a"));
        requests.ambiguous_error();
        assert!(requests.needs_reconnect());
        assert!(requests
            .acknowledge_from(requests.generation(), "a")
            .is_none());
        assert!(!requests.attach(Some(&tx), "b"));
        requests.reset_connection();
        assert!(!requests.needs_reconnect());
        assert!(requests.attach(Some(&tx), "b"));
        assert!(requests
            .acknowledge_from(requests.generation(), "b")
            .is_some());
    }

    #[test]
    fn rejected_local_queue_preserves_exact_composer_files_and_history() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        drop(rx);
        let mut text = "  unsent follow-up\n".into();
        let mut attachments = vec![Attachment {
            path: "image.png".into(),
            mime: Some("image/png".into()),
        }];
        let mut history = PromptHistory::default();
        assert!(!queue_prompt(
            Some(&tx),
            &mut text,
            &mut attachments,
            &mut history
        ));
        assert_eq!(text, "  unsent follow-up\n");
        assert_eq!(attachments[0].path, "image.png");
        assert!(history.recall_older("draft").is_none());
        let mut requests = SessionRequests::default();
        assert!(!requests.create(Some(&tx), DraftOwner::new_task("local"), None, None));
        assert!(!requests.is_pending());
    }

    #[test]
    fn accepted_prompt_queues_once_and_consumes_its_composer() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut text = "follow up".into();
        let mut attachments = vec![Attachment {
            path: "image.png".into(),
            mime: None,
        }];
        let mut history = PromptHistory::default();
        assert!(queue_prompt(
            Some(&tx),
            &mut text,
            &mut attachments,
            &mut history
        ));
        assert!(!queue_prompt(
            Some(&tx),
            &mut text,
            &mut attachments,
            &mut history
        ));
        assert!(text.is_empty());
        assert!(attachments.is_empty());
        assert!(
            matches!(rx.try_recv().unwrap(), ClientMessage::Prompt { text, attachments } if text == "follow up" && attachments[0].path == "image.png")
        );
        assert!(rx.try_recv().is_err());
        assert_eq!(history.recall_older(""), Some("follow up".into()));
    }

    #[test]
    fn creation_submits_only_the_unchanged_text_and_files_after_ready() {
        for changed in [false, true] {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let owner = DraftOwner::new_task("local");
            let mut requests = SessionRequests::default();
            let mut text = String::from("initial prompt");
            let mut files = vec![Attachment {
                path: "initial.txt".into(),
                mime: None,
            }];
            assert!(requests.create(Some(&tx), owner.clone(), None, None));
            requests.set_initial_prompt(
                &owner,
                PromptDraft {
                    text: text.clone(),
                    attachments: files.clone(),
                },
            );
            if changed {
                files[0].path = "edited.txt".into();
            }
            let prompt = requests
                .acknowledge_from(requests.generation(), "created")
                .unwrap()
                .initial_prompt
                .unwrap();
            let mut history = PromptHistory::default();
            assert_eq!(
                queue_initial_prompt(prompt, Some(&tx), &mut text, &mut files, &mut history),
                if changed { None } else { Some(true) }
            );
            assert!(matches!(
                rx.try_recv().unwrap(),
                ClientMessage::NewSession { .. }
            ));
            if changed {
                assert_eq!(text, "initial prompt");
                assert_eq!(files[0].path, "edited.txt");
                assert!(rx.try_recv().is_err());
            } else {
                assert!(matches!(
                    rx.try_recv().unwrap(),
                    ClientMessage::Prompt { .. }
                ));
                assert!(text.is_empty());
            }
        }
    }

    #[test]
    fn reconnect_rejects_buffered_ready_from_the_previous_client() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut requests = SessionRequests::default();
        let old_source = requests.generation();
        requests.attach(Some(&tx), "a");
        requests.reset_connection();
        let new_source = requests.generation();
        requests.attach(Some(&tx), "a");
        assert!(!requests.accepts_source(old_source));
        assert!(requests.acknowledge_from(old_source, "a").is_none());
        assert!(requests.is_pending());
        assert!(requests.accepts_source(new_source));
        assert!(requests.acknowledge_from(new_source, "a").is_some());
    }
}
