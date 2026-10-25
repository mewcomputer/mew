use mew_protocol::Attachment;
use std::{
    collections::{BTreeMap, VecDeque},
    ops::Range,
    time::{Duration, Instant},
};

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DraftEditKind {
    Insert,
    Backspace,
    Delete,
    Replace,
    Composition,
}

struct DraftSnapshot {
    text: String,
    selection: Range<usize>,
    reversed: bool,
}

#[derive(Default)]
pub(super) struct DraftEdits {
    undo: VecDeque<DraftSnapshot>,
    redo: Vec<DraftSnapshot>,
    group: Option<(DraftEditKind, Instant, Range<usize>)>,
}

impl DraftEdits {
    pub(super) fn record(
        &mut self,
        text: &str,
        selection: Range<usize>,
        reversed: bool,
        kind: DraftEditKind,
        now: Instant,
    ) {
        let merge = self.group.as_ref().is_some_and(|(previous, time, cursor)| {
            *previous == kind
                && kind != DraftEditKind::Replace
                && (kind == DraftEditKind::Composition
                    || (now.duration_since(*time) < Duration::from_secs(1)
                        && selection.is_empty()
                        && *cursor == selection))
        });
        if !merge {
            if self.undo.len() == 100 {
                self.undo.pop_front();
            }
            self.undo.push_back(DraftSnapshot {
                text: text.to_owned(),
                selection: selection.clone(),
                reversed,
            });
        }
        self.redo.clear();
        self.group = Some((kind, now, selection));
    }

    pub(super) fn finish_edit(&mut self, selection: &Range<usize>) {
        if let Some((_, _, cursor)) = self.group.as_mut() {
            *cursor = selection.clone();
        }
    }

    pub(super) fn end_group(&mut self) {
        self.group = None;
    }

    pub(super) fn restore(
        &mut self,
        redo: bool,
        text: &mut String,
        selection: &mut Range<usize>,
        reversed: &mut bool,
    ) -> bool {
        let snapshot = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        let Some(mut snapshot) = snapshot else {
            return false;
        };
        std::mem::swap(text, &mut snapshot.text);
        std::mem::swap(selection, &mut snapshot.selection);
        std::mem::swap(reversed, &mut snapshot.reversed);
        if redo {
            self.undo.push_back(snapshot);
        } else {
            self.redo.push(snapshot);
        }
        self.end_group();
        true
    }

    pub(super) fn end_composition(&mut self, text: &str) {
        if self
            .group
            .as_ref()
            .is_some_and(|(kind, _, _)| *kind == DraftEditKind::Composition)
            && self
                .undo
                .back()
                .is_some_and(|snapshot| snapshot.text == text)
        {
            self.undo.pop_back();
        }
        self.end_group();
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Default)]
struct SavedDraft {
    draft: PromptDraft,
    edits: DraftEdits,
}

pub(super) struct TaskDrafts {
    owner: DraftOwner,
    saved: BTreeMap<DraftOwner, SavedDraft>,
    pub(super) edits: DraftEdits,
}

impl TaskDrafts {
    pub(super) fn new(owner: DraftOwner) -> Self {
        Self {
            owner,
            saved: BTreeMap::new(),
            edits: DraftEdits::default(),
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
        self.edits.end_group();
        let edits = std::mem::take(&mut self.edits);
        if !current.text.is_empty()
            || !current.attachments.is_empty()
            || !edits.undo.is_empty()
            || !edits.redo.is_empty()
        {
            self.saved.insert(
                self.owner.clone(),
                SavedDraft {
                    draft: current,
                    edits,
                },
            );
        }
        self.owner = owner;
        let restored = self.saved.remove(&self.owner).unwrap_or_default();
        self.edits = restored.edits;
        restored.draft
    }

    pub(super) fn bind_created_task(
        &mut self,
        creation: &DraftOwner,
        session_id: &str,
        follow: bool,
    ) {
        let owner = DraftOwner::session(&creation.profile, session_id);
        if let Some(draft) = self.saved.remove(creation) {
            self.saved.insert(owner.clone(), draft);
        }
        if follow && self.owner == *creation {
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
    fn undo_groups_nearby_typing_but_splits_after_a_pause_or_cursor_move() {
        let mut edits = DraftEdits::default();
        let now = Instant::now();
        edits.record("", 0..0, false, DraftEditKind::Insert, now);
        edits.finish_edit(&(1..1));
        edits.record(
            "a",
            1..1,
            false,
            DraftEditKind::Insert,
            now + Duration::from_millis(100),
        );
        edits.finish_edit(&(2..2));
        edits.record(
            "ab",
            2..2,
            false,
            DraftEditKind::Insert,
            now + Duration::from_millis(1100),
        );
        edits.finish_edit(&(3..3));
        let mut text = "abc".to_owned();
        let mut selection = 3..3;
        let mut reversed = false;
        assert!(edits.restore(false, &mut text, &mut selection, &mut reversed));
        assert_eq!(text, "ab");
        assert_eq!(selection, 2..2);
        assert!(edits.restore(false, &mut text, &mut selection, &mut reversed));
        assert_eq!(text, "");
        assert!(!edits.restore(false, &mut text, &mut selection, &mut reversed));
        assert!(edits.restore(true, &mut text, &mut selection, &mut reversed));
        assert_eq!(text, "ab");
        edits.end_group();
        edits.record(
            &text,
            0..0,
            false,
            DraftEditKind::Insert,
            now + Duration::from_millis(1200),
        );
        text.insert(0, 'x');
        selection = 1..1;
        assert!(edits.restore(false, &mut text, &mut selection, &mut reversed));
        assert_eq!(text, "ab");
        assert_eq!(selection, 0..0);
        assert!(edits.restore(true, &mut text, &mut selection, &mut reversed));
        assert_eq!(text, "xab");
        assert!(!edits.restore(true, &mut text, &mut selection, &mut reversed));
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
        history.record("previous prompt".into());
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
        drafts.bind_created_task(&creation, "created", true);
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
        drafts.bind_created_task(&creation, "created", true);
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
