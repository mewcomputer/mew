use super::{AuxiliaryView, DesktopShell};

impl DesktopShell {
    pub(super) fn open_review(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.model.session_is_ready()
            || self.session_requests.is_pending()
            || !matches!(
                self.model.connection,
                crate::model::ShellConnection::Connected
            )
        {
            return;
        }
        if !self
            .layout
            .reveal_workbench(self.window_width, self.sidebar_width)
        {
            self.model.last_error = Some("Widen the window to review changes.".into());
            cx.notify();
            return;
        }
        self.layout.changes_expanded = true;
        self.select_auxiliary_view(AuxiliaryView::Changes, cx);
        self.workbench_animation_id = self.workbench_animation_id.wrapping_add(1);
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }
}

#[derive(Clone)]
pub(super) struct ReviewRequest {
    pub session_id: String,
    pub request_id: String,
}

pub(super) struct ReviewRequests {
    pending: Option<ReviewRequest>,
    dirty: bool,
}

impl Default for ReviewRequests {
    fn default() -> Self {
        Self {
            pending: None,
            dirty: true,
        }
    }
}

impl ReviewRequests {
    pub fn observe(&mut self, event: &mew_client_core::ClientEvent, attached_session: &str) {
        use mew_client_core::ClientEvent;
        match event {
            ClientEvent::SessionReady { session_id } if session_id == attached_session => {
                self.reset()
            }
            ClientEvent::WorkspaceChanged { session_id }
            | ClientEvent::MessageChanged { session_id }
            | ClientEvent::SessionMetaChanged { session_id }
            | ClientEvent::SessionActivityChanged {
                session_id,
                running: false,
            } if session_id == attached_session => self.invalidate(),
            _ => {}
        }
    }

    pub fn complete_event(
        &mut self,
        event: &mut mew_client_core::ClientEvent,
        attached_session: &str,
    ) -> Option<Result<Vec<mew_diff::FileDiff>, String>> {
        use mew_client_core::ClientEvent;
        match event {
            ClientEvent::SessionDiff {
                session_id,
                request_id,
                files,
            } if session_id == attached_session && self.complete(session_id, request_id) => {
                Some(Ok(std::mem::take(files)))
            }
            ClientEvent::SessionDiffError {
                session_id,
                request_id,
                message,
            } if session_id == attached_session && self.complete(session_id, request_id) => {
                Some(Err(std::mem::take(message)))
            }
            _ => None,
        }
    }

    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn reset(&mut self) {
        self.pending = None;
        self.invalidate();
    }

    pub fn start(&mut self, session_id: &str) -> Option<ReviewRequest> {
        if self.pending.is_some() || !self.dirty {
            return None;
        }
        let request = ReviewRequest {
            session_id: session_id.into(),
            request_id: ulid::Ulid::new().to_string(),
        };
        self.pending = Some(request.clone());
        self.dirty = false;
        Some(request)
    }

    pub fn complete(&mut self, session_id: &str, request_id: &str) -> bool {
        if !self.pending.as_ref().is_some_and(|pending| {
            pending.session_id == session_id && pending.request_id == request_id
        }) {
            return false;
        }
        self.pending = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_review_payload_is_consumed_and_failures_preserve_their_detail() {
        let mut requests = ReviewRequests::default();
        let old = requests.start("old-session").unwrap();
        requests.reset();
        let current = requests.start("current-session").unwrap();
        let mut stale = mew_client_core::ClientEvent::SessionDiffError {
            session_id: old.session_id,
            request_id: old.request_id,
            message: "stale failure".into(),
        };
        assert!(requests
            .complete_event(&mut stale, "current-session")
            .is_none());
        let mut wrong_session = mew_client_core::ClientEvent::SessionDiff {
            session_id: "other-session".into(),
            request_id: current.request_id.clone(),
            files: vec![mew_diff::FileDiff::binary("wrong.txt", None)],
        };
        assert!(requests
            .complete_event(&mut wrong_session, "current-session")
            .is_none());
        let mut fresh = mew_client_core::ClientEvent::SessionDiff {
            session_id: current.session_id,
            request_id: current.request_id,
            files: vec![mew_diff::FileDiff::binary("daemon.txt", None)],
        };
        let files = requests
            .complete_event(&mut fresh, "current-session")
            .unwrap()
            .unwrap();
        assert_eq!(files[0].path, "daemon.txt");
        assert!(requests
            .complete_event(&mut fresh, "current-session")
            .is_none());
        requests.invalidate();
        let next = requests.start("current-session").unwrap();
        let mut failure = mew_client_core::ClientEvent::SessionDiffError {
            session_id: next.session_id,
            request_id: next.request_id,
            message: "repository unavailable".into(),
        };
        assert_eq!(
            requests
                .complete_event(&mut failure, "current-session")
                .unwrap()
                .unwrap_err(),
            "repository unavailable"
        );
        assert!(requests.start("current-session").is_none());
    }

    #[test]
    fn review_refreshes_follow_current_attachment_and_terminal_activity() {
        use mew_client_core::ClientEvent;
        let mut requests = ReviewRequests::default();
        let first = requests.start("a").unwrap();
        assert!(requests.complete("a", &first.request_id));
        for event in [
            ClientEvent::WorkspaceChanged {
                session_id: "b".into(),
            },
            ClientEvent::MessageChanged {
                session_id: "b".into(),
            },
            ClientEvent::SessionActivityChanged {
                session_id: "a".into(),
                running: true,
            },
        ] {
            requests.observe(&event, "a");
            assert!(requests.start("a").is_none());
        }
        for event in [
            ClientEvent::WorkspaceChanged {
                session_id: "a".into(),
            },
            ClientEvent::MessageChanged {
                session_id: "a".into(),
            },
            ClientEvent::SessionActivityChanged {
                session_id: "a".into(),
                running: false,
            },
        ] {
            requests.observe(&event, "a");
            let request = requests.start("a").unwrap();
            assert!(requests.complete("a", &request.request_id));
        }
        requests.invalidate();
        let pending = requests.start("a").unwrap();
        requests.observe(
            &ClientEvent::SessionReady {
                session_id: "a".into(),
            },
            "a",
        );
        let reattached = requests.start("a").unwrap();
        assert!(!requests.complete("a", &pending.request_id));
        assert!(requests.complete("a", &reattached.request_id));
    }

    #[test]
    fn changes_during_load_schedule_one_fresh_request() {
        let mut requests = ReviewRequests::default();
        let first = requests.start("a").unwrap();
        requests.invalidate();
        requests.invalidate();
        assert!(requests.start("a").is_none());
        assert!(requests.complete("a", &first.request_id));
        let second = requests.start("a").unwrap();
        assert_ne!(first.request_id, second.request_id);
        assert!(requests.complete("a", &second.request_id));
        assert!(requests.start("a").is_none());
    }

    #[test]
    fn late_response_and_error_cannot_complete_another_attachment() {
        let mut requests = ReviewRequests::default();
        let old = requests.start("a").unwrap();
        requests.reset();
        let current = requests.start("b").unwrap();
        assert!(!requests.complete("a", &old.request_id));
        assert!(!requests.complete("b", &old.request_id));
        assert!(requests.start("b").is_none());
        assert!(requests.complete("b", &current.request_id));
        requests.reset();
        let reconnect = requests.start("b").unwrap();
        assert!(!requests.complete("b", &current.request_id));
        assert!(requests.complete("b", &reconnect.request_id));
    }
}
