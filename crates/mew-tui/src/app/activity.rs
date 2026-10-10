use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityTarget {
    Subagent(String),
    Job(String),
}

#[derive(Debug)]
pub struct ActivityView {
    pub session_id: String,
    pub target: ActivityTarget,
    pub scroll: u16,
    pub view: DecisionView,
    pub transcript: Option<SubagentTranscript>,
}

#[derive(Debug)]
pub struct SubagentTranscript {
    pub child_session_id: String,
    pub messages: Vec<Message>,
    pub error: Option<String>,
    pub loaded: bool,
    pub(crate) rendered: Option<(u16, Vec<ratatui::text::Line<'static>>)>,
    pending: Option<(String, Instant)>,
    refresh_at: Option<Instant>,
    refresh_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarItem {
    Sessions,
    Attention,
    Session(String),
    Todo(usize),
    Subagent(String),
    Job(String),
}

impl App {
    pub fn open_subagent(&mut self, task_id: String) {
        self.open_activity(ActivityTarget::Subagent(task_id));
    }

    pub fn open_job(&mut self, job_id: String) {
        self.open_activity(ActivityTarget::Job(job_id));
    }

    fn open_activity(&mut self, target: ActivityTarget) {
        self.picker = None;
        let transcript = match &target {
            ActivityTarget::Subagent(id) => self
                .subagents
                .iter()
                .find(|state| &state.task_id == id)
                .map(|state| SubagentTranscript {
                    child_session_id: state.child_session_id.clone(),
                    messages: Vec::new(),
                    error: None,
                    loaded: false,
                    rendered: None,
                    pending: None,
                    refresh_at: Some(Instant::now()),
                    refresh_requested: false,
                }),
            ActivityTarget::Job(_) => None,
        };
        self.activity_view = Some(ActivityView {
            session_id: self.status.session_id.clone(),
            target,
            scroll: 0,
            view: DecisionView::default(),
            transcript,
        });
        self.mode = Mode::Activity;
    }

    pub fn take_activity_request(&mut self, now: Instant) -> Option<crate::events::Action> {
        if self.mode != Mode::Activity {
            return None;
        }
        let view = self.activity_view.as_mut()?;
        if view.session_id != self.status.session_id {
            return None;
        }
        let transcript = view.transcript.as_mut()?;
        if !self.subagents.iter().any(|state| {
            matches!(&view.target, ActivityTarget::Subagent(id) if id == &state.task_id)
                && state.child_session_id == transcript.child_session_id
        }) {
            return None;
        }
        if let Some((_, started)) = &transcript.pending {
            if now.saturating_duration_since(*started) >= Duration::from_secs(3) {
                transcript.pending = None;
                transcript.refresh_at = None;
                transcript.refresh_requested = false;
                transcript.error = Some(
                    "trace not confirmed; update or reconnect the daemon, then press r".into(),
                );
                view.scroll = 0;
            }
            return None;
        }
        if transcript.refresh_at.is_none_or(|at| now < at) {
            return None;
        }
        let request_id = ulid::Ulid::new().to_string();
        transcript.pending = Some((request_id.clone(), now));
        transcript.refresh_at = None;
        Some(crate::events::Action::ReadSubagentHistory {
            session_id: view.session_id.clone(),
            child_session_id: transcript.child_session_id.clone(),
            request_id,
        })
    }

    pub fn refresh_activity(&mut self) {
        if let Some(transcript) = self
            .activity_view
            .as_mut()
            .and_then(|view| view.transcript.as_mut())
        {
            if transcript.pending.is_some() {
                transcript.refresh_requested = true;
            } else {
                transcript.refresh_at = Some(Instant::now());
            }
        }
    }

    pub fn apply_subagent_history(
        &mut self,
        session_id: &str,
        child_session_id: &str,
        request_id: &str,
        messages: &[Message],
        error: Option<&str>,
    ) {
        let Some(view) = self.activity_view.as_mut() else {
            return;
        };
        if session_id != self.status.session_id || session_id != view.session_id {
            return;
        }
        let Some(transcript) = view.transcript.as_mut() else {
            return;
        };
        if !self.subagents.iter().any(|state| {
            matches!(&view.target, ActivityTarget::Subagent(id) if id == &state.task_id)
                && state.child_session_id == transcript.child_session_id
        }) {
            return;
        }
        if child_session_id != transcript.child_session_id
            || !transcript
                .pending
                .as_ref()
                .is_some_and(|(id, _)| id == request_id)
        {
            return;
        }
        transcript.pending = None;
        transcript.error = error.map(str::to_owned);
        if error.is_none() {
            transcript.loaded = true;
            if transcript.messages != messages {
                transcript.messages.clear();
                transcript.messages.extend_from_slice(messages);
                transcript.rendered = None;
            }
            let running = self.subagents.iter().any(|state| {
                matches!(&view.target, ActivityTarget::Subagent(id) if id == &state.task_id)
                    && matches!(state.status, SubagentStatus::Running)
            });
            transcript.refresh_at = if transcript.refresh_requested {
                Some(Instant::now())
            } else {
                running.then(|| Instant::now() + Duration::from_secs(2))
            };
        }
        transcript.refresh_requested = false;
    }

    pub fn close_activity(&mut self) {
        self.activity_view = None;
        self.mode = Mode::Normal;
        self.prune_sidebar_state();
    }

    pub fn subagent_visible(&self, state: &SubagentState) -> bool {
        match state.status {
            SubagentStatus::Running => true,
            SubagentStatus::Failed { .. } => state
                .finished_at
                .is_some_and(|at| at.elapsed() <= self.finished_ttl()),
            SubagentStatus::Completed | SubagentStatus::Cancelled => false,
        }
    }

    pub fn open_subagent_picker(&mut self) {
        let items = self
            .subagents
            .iter()
            .filter(|state| self.subagent_visible(state))
            .map(|state| PickerItem {
                id: state.task_id.clone(),
                label: state.display_name.as_ref().unwrap_or(&state.name).clone(),
                description: state
                    .last_progress
                    .clone()
                    .unwrap_or_else(|| state.name.clone()),
                ..Default::default()
            })
            .collect();
        self.open_activity_picker("subagent", items);
    }

    pub fn open_job_picker(&mut self) {
        let items = self
            .background_jobs
            .iter()
            .filter(|state| self.background_job_visible(state))
            .map(|state| PickerItem {
                id: state.job_id.clone(),
                label: state.command.clone(),
                description: state.job_id.clone(),
                ..Default::default()
            })
            .collect();
        self.open_activity_picker("job", items);
    }

    fn open_activity_picker(&mut self, kind: &str, items: Vec<PickerItem>) {
        self.picker = Some(PickerState {
            kind: kind.into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            filter_undo: editor::UndoHistory::default(),
            scroll: 0,
            details_scroll: 0,
            visible_details: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: Some("Enter inspect".into()),
            budget: None,
        });
        self.mode = Mode::CommandPalette;
    }
}
