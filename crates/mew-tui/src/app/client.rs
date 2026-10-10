use super::App;
use mew_agent::AgentEvent;
use mew_client_core::{ClientEvent, ClientState, ConnectionStatus};
use mew_message::{Message, Role};
use mew_protocol::ServerMessage;
use mew_provider::ProviderEvent;

impl App {
    pub fn client_connected(&self) -> bool {
        self.connection
            .as_ref()
            .is_none_or(|connection| matches!(connection, ConnectionStatus::Connected))
    }

    pub fn project_cwd(&self) -> std::path::PathBuf {
        self.session_cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    /// Project canonical client events into the terminal's merged display entries.
    pub fn apply_client_update(
        &mut self,
        events: &[ClientEvent],
        state: &ClientState,
        changed: Option<&Message>,
    ) {
        for event in events {
            match event {
                ClientEvent::ConnectionChanged(connection) => {
                    self.connection = Some(connection.clone());
                    if !matches!(connection, ConnectionStatus::Connected) {
                        self.attaching_session = true;
                    }
                    match connection {
                        ConnectionStatus::Connected => self.set_alert("connected"),
                        ConnectionStatus::Backoff { attempt, error } => {
                            self.set_alert(format!("reconnecting ({attempt}): {error}"))
                        }
                        ConnectionStatus::Connecting => self.set_alert("connecting"),
                        ConnectionStatus::Disconnected => self.set_alert("disconnected"),
                    }
                }
                ClientEvent::SessionReady { session_id } => {
                    let Some(session) = state.session(session_id) else {
                        continue;
                    };
                    self.apply_daemon_notification(&ServerMessage::SessionReady {
                        session_id: session_id.clone(),
                        cwd: session.cwd.clone(),
                        model: session.model.clone(),
                        provider: session.provider.clone(),
                        permission_mode: session.permission_mode.clone(),
                    });
                    self.confirm_project(session.cwd.as_deref());
                    self.streaming = session.running;
                    self.hydrate_activity(session);
                    self.unread_completions = state.unread_completions.clone();
                    self.refresh_session_picker();
                }
                ClientEvent::SessionListChanged | ClientEvent::SessionMetaChanged { .. } => {
                    self.apply_daemon_notification(&ServerMessage::SessionList {
                        sessions: state.session_list.clone(),
                    });
                    self.session_titles = state
                        .session_titles
                        .iter()
                        .map(|(id, title)| (id.clone(), title.clone()))
                        .collect();
                    self.session_attention = state
                        .session_attention
                        .iter()
                        .map(|(id, counts)| (id.clone(), *counts))
                        .collect();
                    for session in &state.session_list {
                        self.session_attention
                            .entry(session.session_id.clone())
                            .or_insert((session.pending_permissions, session.pending_questions));
                    }
                    self.session_turn_outcomes = state.session_turn_outcomes.clone();
                    self.unread_completions = state.unread_completions.clone();
                    self.refresh_session_picker();
                }
                ClientEvent::ActivitySnapshot { session_id }
                    if session_id == &self.status.session_id =>
                {
                    if let Some(session) = state.session(session_id) {
                        self.hydrate_activity(session);
                    }
                }
                ClientEvent::SessionActivityChanged {
                    session_id,
                    running,
                } => {
                    if let Some(session) = self
                        .daemon_sessions
                        .iter_mut()
                        .find(|session| &session.session_id == session_id)
                    {
                        session.state = if *running {
                            mew_protocol::SessionState::Running
                        } else {
                            mew_protocol::SessionState::Idle
                        };
                    }
                    if *running {
                        if session_id == &self.status.session_id {
                            self.streaming = true;
                        }
                    } else {
                        self.finish_session_turn(session_id);
                    }
                }
                ClientEvent::MessageChanged { session_id }
                    if session_id == &self.status.session_id =>
                {
                    if let Some(message) = changed {
                        self.project_message(message);
                    }
                }
                ClientEvent::UserMessage {
                    session_id,
                    request_id: Some(request_id),
                    message_id,
                } => {
                    self.record_submission_echo(session_id, request_id, *message_id);
                }
                ClientEvent::SessionCleared { session_id }
                    if session_id == &self.status.session_id =>
                {
                    self.clear_messages()
                }
                ClientEvent::PartUpdated {
                    session_id,
                    part_id,
                    part,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::PartUpdated {
                        part_id: *part_id,
                        part: part.as_ref().clone(),
                    });
                }
                ClientEvent::TextDelta {
                    session_id,
                    part_id,
                    delta,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::Provider(ProviderEvent::PartDelta {
                        part_id: *part_id,
                        field: "text",
                        delta: delta.clone(),
                    }));
                }
                ClientEvent::ProviderMessageEnded {
                    session_id,
                    cost,
                    input_tokens,
                    output_tokens,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::Provider(ProviderEvent::MessageEnd {
                        finish: changed
                            .and_then(|message| message.assistant.as_ref())
                            .and_then(|meta| meta.finish)
                            .unwrap_or(mew_message::Finish::Stop),
                        usage: mew_message::Tokens {
                            input: *input_tokens,
                            output: *output_tokens,
                            ..Default::default()
                        },
                        cost: *cost,
                    }));
                }
                ClientEvent::ProviderPartEnded {
                    session_id,
                    part_id,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::Provider(ProviderEvent::PartEnd {
                        part_id: *part_id,
                    }));
                }
                ClientEvent::ToolProgress {
                    session_id,
                    call_id,
                    chunk,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::ToolProgress {
                        call_id: call_id.clone(),
                        chunk: chunk.clone(),
                    });
                }
                ClientEvent::RetryWait { session_id, retry }
                    if session_id == &self.status.session_id =>
                {
                    self.handle_agent_event(AgentEvent::Provider(ProviderEvent::RetryWait {
                        attempt: retry.attempt,
                        max_attempts: retry.max_attempts,
                        delay_secs: retry.delay_secs,
                        reason: retry.reason.clone(),
                    }));
                }
                ClientEvent::JobUpdated { session_id, job }
                    if session_id == &self.status.session_id =>
                {
                    self.handle_agent_event(AgentEvent::JobUpdate {
                        job_id: job.job_id.clone(),
                        command: job.command.clone(),
                        state: job.state.clone(),
                    });
                }
                ClientEvent::SubagentStarted {
                    session_id,
                    parent_call_id,
                    child_session_id,
                    name,
                    display_name,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::SubagentStart {
                        parent_call_id: parent_call_id.clone(),
                        child_session_id: child_session_id.clone(),
                        name: name.clone(),
                        display_name: display_name.clone(),
                    });
                }
                ClientEvent::SubagentStatus {
                    session_id,
                    parent_call_id,
                    tool_name,
                    message,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::SubagentStatus {
                        parent_call_id: parent_call_id.clone(),
                        tool_name: tool_name.clone(),
                        message: message.clone(),
                    });
                }
                ClientEvent::SubagentEnded {
                    session_id,
                    parent_call_id,
                    child_session_id,
                    outcome,
                    manifests,
                } if session_id == &self.status.session_id => {
                    let outcome = match outcome {
                        mew_protocol::SubagentOutcome::Completed => {
                            mew_subagents::SubagentOutcome::Completed
                        }
                        mew_protocol::SubagentOutcome::Cancelled => {
                            mew_subagents::SubagentOutcome::Cancelled
                        }
                        mew_protocol::SubagentOutcome::Failed { reason } => {
                            mew_subagents::SubagentOutcome::Failed {
                                reason: reason.clone(),
                            }
                        }
                    };
                    self.handle_agent_event(AgentEvent::SubagentEnd {
                        parent_call_id: parent_call_id.clone(),
                        child_session_id: child_session_id.clone(),
                        outcome,
                        manifests: manifests.clone(),
                    });
                }
                ClientEvent::SubagentHistory {
                    session_id,
                    child_session_id,
                    request_id,
                    messages,
                    error,
                } => {
                    self.apply_subagent_history(
                        session_id,
                        child_session_id,
                        request_id,
                        messages,
                        error.as_deref(),
                    );
                }
                ClientEvent::TodosChanged { session_id }
                | ClientEvent::TodoUpdated { session_id, .. }
                    if session_id == &self.status.session_id =>
                {
                    if let Some(session) = state.session(session_id) {
                        self.apply_daemon_notification(&ServerMessage::TodosUpdated {
                            todos: session.todos.clone(),
                        });
                    }
                }
                ClientEvent::FlaggedFilesChanged { session_id }
                    if session_id == &self.status.session_id =>
                {
                    if let Some(session) = state.session(session_id) {
                        self.apply_daemon_notification(&ServerMessage::FlaggedFilesChanged {
                            session_id: session_id.clone(),
                            files: session.flagged_files.clone(),
                        });
                    }
                }
                ClientEvent::UsageChanged { session_id }
                    if session_id == &self.status.session_id =>
                {
                    if let Some(session) = state.session(session_id) {
                        self.status.cost = session.usage.cost;
                        if let Some(tokens) = session.context_tokens {
                            self.status.context_tokens = u32::try_from(tokens).unwrap_or(u32::MAX);
                        }
                    }
                }
                ClientEvent::SessionHistoryLoaded { session_id }
                    if session_id == &self.status.session_id =>
                {
                    if let Some(session) = state.session(session_id) {
                        self.apply_daemon_notification(&ServerMessage::SessionHistory {
                            messages: session.messages.clone(),
                        });
                    }
                }
                ClientEvent::SessionHistoryPageLoaded {
                    session_id,
                    replace,
                } if session_id == &self.status.session_id => {
                    if let Some(session) = state.session(session_id) {
                        let count = if *replace {
                            session.messages.len()
                        } else {
                            self.messages
                                .first()
                                .and_then(|first| {
                                    session
                                        .messages
                                        .iter()
                                        .position(|message| message.id == first.id)
                                })
                                .unwrap_or(session.messages.len())
                        };
                        self.apply_daemon_notification(&ServerMessage::SessionHistoryPage {
                            session_id: session_id.clone(),
                            messages: session.messages[..count].to_vec(),
                            next_cursor: session.history_before,
                            replace: *replace,
                        });
                    }
                }
                ClientEvent::RequestResolved { request_id } => {
                    self.apply_daemon_notification(&ServerMessage::RequestResolved {
                        request_id: request_id.clone(),
                    })
                }
                ClientEvent::PromptReceipt {
                    session_id,
                    request_id,
                    status,
                    message,
                } => self.resolve_submission(session_id, request_id, *status, message.as_deref()),
                ClientEvent::ProjectListChanged => {
                    self.apply_daemon_notification(&ServerMessage::ProjectList {
                        projects: state.projects.clone(),
                    })
                }
                ClientEvent::GroupsChanged => self.groups = state.groups.clone(),
                ClientEvent::ModelListChanged => {
                    self.apply_daemon_notification(&ServerMessage::ModelList {
                        models: state.models.clone(),
                    })
                }
                ClientEvent::PermissionModeChanged { mode } => {
                    if let Some(mode) = mew_hooks::PermissionMode::from_id(mode) {
                        self.permission_mode = mode;
                    }
                }
                ClientEvent::SessionDiff {
                    session_id,
                    request_id,
                    files,
                } => self.apply_session_diff(session_id, request_id, files, None),
                ClientEvent::SessionDiffError {
                    session_id,
                    request_id,
                    message,
                } => self.apply_session_diff(session_id, request_id, &[], Some(message)),
                ClientEvent::WorkspaceChanged { session_id } => {
                    self.invalidate_review(session_id);
                }
                ClientEvent::JobOutput {
                    session_id,
                    job_id,
                    request_id,
                    offset,
                    next_offset,
                    output,
                    done,
                    incomplete,
                } => self.apply_job_output(
                    session_id,
                    job_id,
                    request_id,
                    *offset,
                    *next_offset,
                    output,
                    *done,
                    *incomplete,
                ),
                ClientEvent::JobOutputError {
                    session_id,
                    job_id,
                    request_id,
                    message,
                } => self.apply_job_output_error(session_id, job_id, request_id, message),
                ClientEvent::TurnError {
                    session_id,
                    message,
                } if session_id == &self.status.session_id => {
                    self.handle_agent_event(AgentEvent::Error(message.clone()));
                }
                ClientEvent::Error(message) => {
                    self.attaching_session = false;
                    self.set_alert(message);
                }
                _ => {}
            }
        }
        if let Some(provider) = &state.current_provider {
            if self.status.provider != *provider {
                self.status.provider.clone_from(provider);
            }
        }
        if let Some(model) = &state.current_model {
            if self.status.model != *model {
                self.status.model.clone_from(model);
            }
        }
        self.active_thinking_variant
            .clone_from(&state.thinking_variant);
        self.active_persona.clone_from(&state.current_persona);
        let color = state
            .personas
            .iter()
            .find(|persona| Some(&persona.name) == state.current_persona.as_ref())
            .and_then(|persona| persona.color.as_ref());
        if self.active_persona_color.as_ref() != color {
            self.active_persona_color = color.cloned();
        }
        if self.personas.len() != state.personas.len()
            || self
                .personas
                .iter()
                .zip(&state.personas)
                .any(|((name, description), persona)| {
                    name != &persona.name || description != &persona.description
                })
        {
            self.personas = state
                .personas
                .iter()
                .map(|persona| (persona.name.clone(), persona.description.clone()))
                .collect();
        }
        if let Some(session) = state.session(&self.status.session_id) {
            if session.retry.is_none() {
                self.retry_status = None;
            }
        }
    }

    fn hydrate_activity(&mut self, session: &mew_client_core::ClientSession) {
        self.subagents.retain(|entry| {
            session.subagents.iter().any(|known| {
                known.task_id == entry.task_id
                    && known.child_session_id.as_deref().unwrap_or_default()
                        == entry.child_session_id
            })
        });
        for known in &session.subagents {
            if !self
                .subagents
                .iter()
                .any(|entry| entry.task_id == known.task_id)
            {
                self.handle_agent_event(AgentEvent::SubagentStart {
                    parent_call_id: known.task_id.clone(),
                    child_session_id: known.child_session_id.clone().unwrap_or_default(),
                    name: known.name.clone(),
                    display_name: known.display_name.clone(),
                });
            }
            if let Some(entry) = self
                .subagents
                .iter_mut()
                .find(|entry| entry.task_id == known.task_id)
            {
                entry.last_progress = (known.status != "running").then(|| known.status.clone());
            }
        }
        self.background_jobs.retain(|entry| {
            session
                .jobs
                .iter()
                .any(|known| known.job_id == entry.job_id)
        });
        for known in &session.jobs {
            self.handle_agent_event(AgentEvent::JobUpdate {
                job_id: known.job_id.clone(),
                command: known.command.clone(),
                state: known.state.clone(),
            });
        }
        self.apply_daemon_notification(&ServerMessage::TodosUpdated {
            todos: session.todos.clone(),
        });
        self.apply_daemon_notification(&ServerMessage::FlaggedFilesChanged {
            session_id: session.session_id.clone(),
            files: session.flagged_files.clone(),
        });
        self.status.cost = session.usage.cost;
        self.status.context_tokens = session
            .context_tokens
            .map(|tokens| u32::try_from(tokens).unwrap_or(u32::MAX))
            .unwrap_or_default();
        self.retry_status = None;
        if let Some(retry) = &session.retry {
            self.handle_agent_event(AgentEvent::Provider(ProviderEvent::RetryWait {
                attempt: retry.attempt,
                max_attempts: retry.max_attempts,
                delay_secs: retry.delay_secs,
                reason: retry.reason.clone(),
            }));
        }
    }

    fn project_message(&mut self, message: &Message) {
        if message.role == Role::User {
            if !self.messages.iter().any(|entry| entry.id == message.id) {
                self.push_message(message.clone());
            }
            return;
        }
        for part in &message.parts {
            let exists = self.messages.iter().rev().any(|entry| {
                entry
                    .parts
                    .iter()
                    .any(|existing| existing.id() == part.id())
            });
            if exists {
                self.handle_agent_event(AgentEvent::PartUpdated {
                    part_id: part.id(),
                    part: part.clone(),
                });
            } else {
                self.handle_agent_event(AgentEvent::Provider(ProviderEvent::PartStart {
                    part: part.clone(),
                }));
            }
        }
    }

    fn confirm_project(&mut self, cwd: Option<&str>) {
        let cwd = cwd.map(std::path::PathBuf::from);
        if self.session_cwd == cwd {
            return;
        }
        self.session_cwd = cwd;
        self.invalidate_file_catalog();
        let cwd = self.project_cwd();
        self.short_cwd = super::format_cwd(&cwd);
        self.git_branch = super::git_branch_in(&cwd);
        self.git_branch_resolved = true;
        self.skill_catalog = mew_skills::Loader::new(cwd.clone())
            .load()
            .unwrap_or_default();
        self.subagent_catalog = mew_subagents::Loader::new(cwd).load().unwrap_or_default();
    }
}
