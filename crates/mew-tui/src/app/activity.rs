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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarItem {
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
        self.activity_view = Some(ActivityView {
            session_id: self.status.session_id.clone(),
            target,
            scroll: 0,
            view: DecisionView::default(),
        });
        self.mode = Mode::Activity;
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
