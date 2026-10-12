use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityTarget {
    Subagent(String),
    Job(String),
    Tool {
        message_id: mew_message::MessageId,
        part_id: PartId,
    },
    Review,
}

#[derive(Debug)]
pub struct ActivityView {
    pub session_id: String,
    pub target: ActivityTarget,
    pub scroll: usize,
    pub max_scroll: usize,
    pub view: DecisionView,
    pub transcript: Option<SubagentTranscript>,
    pub job_output: Option<JobOutputView>,
    pub review: Option<ReviewView>,
    pub(crate) rendered: Option<RenderedActivity>,
}

#[derive(Debug)]
pub(crate) struct RenderedActivity {
    pub width: u16,
    pub dirty: Option<u64>,
    pub rows: Vec<ratatui::text::Line<'static>>,
}

#[derive(Debug)]
pub struct JobOutputView {
    pub output: String,
    pub offset: u64,
    pub done: bool,
    pub incomplete: bool,
    pub error: Option<String>,
    request: ActivityRead,
}

#[derive(Debug)]
pub struct ReviewView {
    pub files: Vec<mew_diff::FileDiff>,
    pub loaded: bool,
    pub error: Option<String>,
    request: ActivityRead,
}

#[derive(Debug)]
struct ActivityRead {
    pending: Option<(String, Instant)>,
    next_at: Option<Instant>,
    queued: bool,
}

impl ActivityRead {
    fn new() -> Self {
        Self {
            pending: None,
            next_at: Some(Instant::now()),
            queued: false,
        }
    }

    fn take(&mut self, now: Instant) -> Result<Option<String>, ()> {
        if let Some((_, started)) = &self.pending {
            if now.saturating_duration_since(*started) >= Duration::from_secs(3) {
                self.pending = None;
                self.next_at = None;
                self.queued = false;
                return Err(());
            }
            return Ok(None);
        }
        if self.next_at.is_none_or(|at| now < at) {
            return Ok(None);
        }
        let id = ulid::Ulid::new().to_string();
        self.pending = Some((id.clone(), now));
        self.next_at = None;
        Ok(Some(id))
    }

    fn matches(&self, id: &str) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(pending, _)| pending == id)
    }

    fn finish(&mut self, delay: Option<Duration>) {
        self.pending = None;
        self.next_at = if self.queued {
            Some(Instant::now())
        } else {
            delay.map(|delay| Instant::now() + delay)
        };
        self.queued = false;
    }

    fn refresh(&mut self) {
        if self.pending.is_some() {
            self.queued = true;
        } else {
            self.next_at = Some(Instant::now());
        }
    }
}

#[derive(Debug)]
pub struct SubagentTranscript {
    pub child_session_id: String,
    pub messages: Vec<Message>,
    pub error: Option<String>,
    pub loaded: bool,
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

    pub fn open_tool(&mut self, message_id: mew_message::MessageId, part_id: PartId) {
        self.open_activity(ActivityTarget::Tool {
            message_id,
            part_id,
        });
    }

    pub fn open_tool_selection(&mut self, id: &str) {
        if let Ok(part_id) = id.parse::<PartId>() {
            if let Some(message) = self.messages.iter().find(|message| {
                message
                    .parts
                    .iter()
                    .any(|part| matches!(part, Part::ToolCall(call) if call.base.id==part_id))
            }) {
                self.open_tool(message.id, part_id);
            }
        }
    }

    pub fn open_tool_picker(&mut self) {
        let items = self
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(|part| {
                if let Part::ToolCall(call) = part {
                    Some(PickerItem {
                        id: call.base.id.to_string(),
                        label: call.tool_name.clone(),
                        description: call.call_id.clone(),
                        ..Default::default()
                    })
                } else {
                    None
                }
            })
            .collect();
        self.open_activity_picker("tool", items);
    }

    pub fn open_review(&mut self) {
        self.open_activity(ActivityTarget::Review);
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
                    pending: None,
                    refresh_at: Some(Instant::now()),
                    refresh_requested: false,
                }),
            ActivityTarget::Job(_) | ActivityTarget::Tool { .. } | ActivityTarget::Review => None,
        };
        let job_output = matches!(target, ActivityTarget::Job(_)).then(|| JobOutputView {
            output: String::new(),
            offset: 0,
            done: false,
            incomplete: false,
            error: None,
            request: ActivityRead::new(),
        });
        let review = matches!(target, ActivityTarget::Review).then(|| ReviewView {
            files: Vec::new(),
            loaded: false,
            error: None,
            request: ActivityRead::new(),
        });
        self.activity_view = Some(ActivityView {
            session_id: self.status.session_id.clone(),
            target,
            scroll: 0,
            max_scroll: 0,
            job_output,
            review,
            rendered: None,
            view: DecisionView::default(),
            transcript,
        });
        self.mode = Mode::Activity;
    }

    pub fn take_activity_request(&mut self, now: Instant) -> Option<crate::events::Action> {
        if self.mode != Mode::Activity || self.attaching_session {
            return None;
        }
        let view = self.activity_view.as_mut()?;
        if view.session_id != self.status.session_id {
            return None;
        }
        if let Some(output) = &mut view.job_output {
            let id = match output.request.take(now) {
                Ok(id) => id?,
                Err(()) => {
                    output.error =
                        Some("output not confirmed; reconnect the daemon, then press r".into());
                    return None;
                }
            };
            let ActivityTarget::Job(job_id) = &view.target else {
                return None;
            };
            return Some(crate::events::Action::ReadJobOutput {
                session_id: view.session_id.clone(),
                job_id: job_id.clone(),
                request_id: id,
                offset: output.offset,
            });
        }
        if let Some(review) = &mut view.review {
            let id = match review.request.take(now) {
                Ok(id) => id?,
                Err(()) => {
                    review.error =
                        Some("review not confirmed; reconnect the daemon, then press r".into());
                    return None;
                }
            };
            return Some(crate::events::Action::ReadSessionDiff {
                session_id: view.session_id.clone(),
                request_id: id,
            });
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
                view.rendered = None;
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
        let Some(view) = &mut self.activity_view else {
            return;
        };
        if let Some(transcript) = &mut view.transcript {
            if transcript.pending.is_some() {
                transcript.refresh_requested = true;
            } else {
                transcript.refresh_at = Some(Instant::now());
            }
        }
        if let Some(output) = &mut view.job_output {
            output.request.refresh();
        }
        if let Some(review) = &mut view.review {
            review.request.refresh();
        }
    }

    pub fn invalidate_review(&mut self, session_id: &str) {
        if let Some(view) = &mut self.activity_view {
            if view.session_id == session_id && self.status.session_id == session_id {
                if let Some(review) = &mut view.review {
                    review.request.refresh();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_job_output(
        &mut self,
        session_id: &str,
        job_id: &str,
        request_id: &str,
        offset: u64,
        next_offset: u64,
        output: &str,
        done: bool,
        incomplete: bool,
    ) {
        let Some(view) = &mut self.activity_view else {
            return;
        };
        if session_id != self.status.session_id
            || session_id != view.session_id
            || !matches!(&view.target,ActivityTarget::Job(id) if id==job_id)
        {
            return;
        }
        let Some(state) = &mut view.job_output else {
            return;
        };
        if !state.request.matches(request_id)
            || offset != state.offset
            || next_offset.checked_sub(offset) != Some(output.len() as u64)
            || output.len() > 65_536
        {
            return;
        }
        if !output.is_empty() {
            view.rendered = None;
        }
        state.output.push_str(output);
        state.offset = next_offset;
        state.done = done;
        state.incomplete = incomplete;
        state.error = None;
        state
            .request
            .finish((!done).then_some(if output.len() >= 65_533 {
                Duration::ZERO
            } else {
                Duration::from_millis(250)
            }));
    }

    pub fn apply_job_output_error(
        &mut self,
        session_id: &str,
        job_id: &str,
        request_id: &str,
        message: &str,
    ) {
        let Some(view) = &mut self.activity_view else {
            return;
        };
        if session_id != self.status.session_id
            || session_id != view.session_id
            || !matches!(&view.target,ActivityTarget::Job(id) if id==job_id)
        {
            return;
        }
        if let Some(output) = &mut view.job_output {
            if output.request.matches(request_id) {
                view.rendered = None;
                output.error = Some(message.into());
                output.request.finish(None);
            }
        }
    }

    pub fn apply_session_diff(
        &mut self,
        session_id: &str,
        request_id: &str,
        files: &[mew_diff::FileDiff],
        error: Option<&str>,
    ) {
        let Some(view) = &mut self.activity_view else {
            return;
        };
        if session_id != self.status.session_id || session_id != view.session_id {
            return;
        }
        if let Some(review) = &mut view.review {
            if !review.request.matches(request_id) {
                return;
            }
            view.rendered = None;
            review.error = error.map(str::to_owned);
            if error.is_none() {
                review.files.clear();
                review.files.extend_from_slice(files);
                review.loaded = true;
            }
            review.request.finish(None);
        }
    }

    pub fn scroll_activity(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;
        if let Some(view) = &mut self.activity_view {
            let page = usize::from(view.view.page_rows.max(1));
            view.scroll = match code {
                KeyCode::Up => view.scroll.saturating_sub(1),
                KeyCode::Down => view.scroll.saturating_add(1).min(view.max_scroll),
                KeyCode::PageUp => view.scroll.saturating_sub(page),
                KeyCode::PageDown => view.scroll.saturating_add(page).min(view.max_scroll),
                KeyCode::Home => 0,
                KeyCode::End => view.max_scroll,
                _ => view.scroll,
            };
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
        view.rendered = None;
        transcript.pending = None;
        transcript.error = error.map(str::to_owned);
        if error.is_none() {
            transcript.loaded = true;
            if transcript.messages != messages {
                transcript.messages.clear();
                transcript.messages.extend_from_slice(messages);
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
