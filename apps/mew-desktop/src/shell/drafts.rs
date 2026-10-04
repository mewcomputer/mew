use mew_protocol::Attachment;
use std::collections::BTreeMap;

pub(super) fn may_auto_attach(
    initial_allowed: bool,
    composer: &str,
    attachments: &[Attachment],
) -> bool {
    initial_allowed && composer.is_empty() && attachments.is_empty()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct DraftOwner {
    pub(super) profile: String,
    pub(super) session_id: Option<String>,
}

impl DraftOwner {
    pub(super) fn session(profile: &str, session_id: &str) -> Self {
        Self {
            profile: profile.into(),
            session_id: Some(session_id.into()),
        }
    }

    pub(super) fn new_task(profile: &str) -> Self {
        Self {
            profile: profile.into(),
            session_id: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PromptDraft {
    pub(super) text: String,
    pub(super) attachments: Vec<Attachment>,
}

pub(super) struct TaskDrafts {
    owner: DraftOwner,
    saved: BTreeMap<DraftOwner, PromptDraft>,
}

impl TaskDrafts {
    pub(super) fn new(owner: DraftOwner) -> Self {
        Self {
            owner,
            saved: BTreeMap::new(),
        }
    }

    pub(super) fn owner(&self) -> &DraftOwner {
        &self.owner
    }

    pub(super) fn activate(
        &mut self,
        profile: String,
        session_id: Option<&str>,
        text: &mut String,
        attachments: &mut Vec<Attachment>,
        history: &mut super::PromptHistory,
    ) -> bool {
        if self.owner.profile == profile && self.owner.session_id.as_deref() == session_id {
            return false;
        }
        let owner = DraftOwner {
            profile,
            session_id: session_id.map(str::to_owned),
        };
        let restored = self.switch(
            owner,
            PromptDraft {
                text: std::mem::take(text),
                attachments: std::mem::take(attachments),
            },
        );
        *text = restored.text;
        *attachments = restored.attachments;
        history.reset_recall();
        true
    }

    pub(super) fn switch(&mut self, owner: DraftOwner, current: PromptDraft) -> PromptDraft {
        if self.owner == owner {
            return current;
        }
        if !current.text.is_empty() || !current.attachments.is_empty() {
            self.saved.insert(self.owner.clone(), current);
        }
        self.owner = owner;
        self.saved.remove(&self.owner).unwrap_or_default()
    }

    pub(super) fn bind_created_task(&mut self, creation: &DraftOwner, session_id: &str) {
        let owner = DraftOwner::session(&creation.profile, session_id);
        if let Some(draft) = self.saved.remove(creation) {
            self.saved.insert(owner.clone(), draft);
        }
        if self.owner == *creation {
            self.owner = owner;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(text: &str, file: &str) -> PromptDraft {
        PromptDraft {
            text: text.into(),
            attachments: vec![mew_protocol::Attachment {
                path: file.into(),
                mime: None,
            }],
        }
    }

    #[test]
    fn startup_attachment_never_overrides_a_new_draft_or_explicit_unbound_selection() {
        assert!(may_auto_attach(true, "", &[]));
        assert!(!may_auto_attach(true, "new task", &[]));
        assert!(!may_auto_attach(true, " ", &[]));
        assert!(!may_auto_attach(
            true,
            "",
            &draft("", "chosen.txt").attachments
        ));
        assert!(!may_auto_attach(false, "", &[]));
    }

    #[test]
    fn navigation_restores_each_tasks_text_and_attachments() {
        let a = DraftOwner::session("local", "a");
        let b = DraftOwner::session("local", "b");
        let mut drafts = TaskDrafts::new(a.clone());
        assert_eq!(
            drafts.switch(b.clone(), draft("follow up a", "a.txt")),
            PromptDraft::default()
        );
        assert_eq!(
            drafts.switch(a.clone(), draft("follow up b", "b.txt")),
            draft("follow up a", "a.txt")
        );
        assert_eq!(
            drafts.switch(b, draft("edited a", "a2.txt")),
            draft("follow up b", "b.txt")
        );
        assert_eq!(
            drafts.switch(a, PromptDraft::default()),
            draft("edited a", "a2.txt")
        );
    }

    #[test]
    fn identical_session_ids_in_different_profiles_have_separate_drafts() {
        let local = DraftOwner::session("local", "same-id");
        let remote = DraftOwner::session("remote", "same-id");
        let mut drafts = TaskDrafts::new(local.clone());
        assert_eq!(
            drafts.switch(remote.clone(), draft("local draft", "local.txt")),
            PromptDraft::default()
        );
        assert_eq!(
            drafts.switch(local, draft("remote draft", "remote.txt")),
            draft("local draft", "local.txt")
        );
        assert_eq!(
            drafts.switch(remote, PromptDraft::default()),
            draft("remote draft", "remote.txt")
        );
    }

    #[test]
    fn selected_task_owns_the_composer_before_its_ready_snapshot_arrives() {
        let mut state = mew_client_core::ClientState::default();
        state.apply_server_message(mew_protocol::ServerMessage::SessionList {
            sessions: serde_json::from_value(serde_json::json!([
                { "session_id": "b", "state": "idle", "created_at": 0, "client_count": 0 }
            ]))
            .unwrap(),
        });
        let mut ui = mew_ui_model::UiModel::default();
        ui.sync_client_metadata(&state);
        let mut attachments = Vec::new();
        let mut drafts = TaskDrafts::new(DraftOwner::new_task("local"));
        let mut history = super::super::PromptHistory::default();
        assert!(matches!(
            ui.select_session("b"),
            Some(mew_protocol::ClientMessage::AttachSession { .. })
        ));
        drafts.activate(
            "local".into(),
            ui.selected_session.as_deref(),
            &mut ui.composer,
            &mut attachments,
            &mut history,
        );
        assert_eq!(drafts.owner(), &DraftOwner::session("local", "b"));
        ui.composer = "follow up in b".into();
        attachments.push(mew_protocol::Attachment {
            path: "b.txt".into(),
            mime: None,
        });
        drafts.activate(
            "local".into(),
            Some("a"),
            &mut ui.composer,
            &mut attachments,
            &mut history,
        );
        drafts.activate(
            "local".into(),
            Some("b"),
            &mut ui.composer,
            &mut attachments,
            &mut history,
        );
        assert_eq!(ui.composer, "follow up in b");
        assert_eq!(attachments[0].path, "b.txt");
    }

    #[test]
    fn navigating_after_history_recall_cannot_replace_another_tasks_draft() {
        let mut drafts = TaskDrafts::new(DraftOwner::session("local", "a"));
        let mut history = super::super::PromptHistory::default();
        history.record("previous prompt");
        let mut text = history.recall_older("a's original draft").unwrap();
        let mut attachments = Vec::new();
        drafts.activate(
            "local".into(),
            Some("b"),
            &mut text,
            &mut attachments,
            &mut history,
        );
        text = "b's follow-up".into();
        assert_eq!(history.recall_newer(), None);
        assert_eq!(text, "b's follow-up");
    }

    #[test]
    fn late_creation_ack_binds_its_draft_without_replacing_the_current_task() {
        let creation = DraftOwner::new_task("local");
        let other = DraftOwner::session("local", "other");
        let created = DraftOwner::session("local", "created");
        let mut drafts = TaskDrafts::new(creation.clone());
        assert_eq!(
            drafts.switch(other.clone(), draft("new task", "new.txt")),
            PromptDraft::default()
        );
        drafts.bind_created_task(&creation, "created");
        assert_eq!(drafts.owner(), &other);
        assert_eq!(
            drafts.switch(created, draft("other task", "other.txt")),
            draft("new task", "new.txt")
        );
    }

    #[test]
    fn binding_current_creation_preserves_the_live_draft_and_does_not_leak_it_to_next_new_task() {
        let creation = DraftOwner::new_task("local");
        let created = DraftOwner::session("local", "created");
        let mut drafts = TaskDrafts::new(creation.clone());
        drafts.bind_created_task(&creation, "created");
        assert_eq!(drafts.owner(), &created);
        assert_eq!(
            drafts.switch(
                DraftOwner::new_task("local"),
                draft("current edit", "current.txt")
            ),
            PromptDraft::default()
        );
        assert_eq!(
            drafts.switch(created, PromptDraft::default()),
            draft("current edit", "current.txt")
        );
    }
}
