//! Typed daemon commands over the TUI's persistent shared-core client.

use crate::runtime::client::TuiClient;
use crate::runtime::target::{
    CommandTarget, GoalAction, PersonaApplied, SwitchedModel, Unsupported,
};
use mew_hooks::PermissionMode;
use mew_message::Part;
use mew_personas::Persona;
use mew_protocol::{ClientKind, ClientMessage};
use std::sync::Arc;

pub struct DaemonTarget {
    pub client: Arc<TuiClient>,
}

impl DaemonTarget {
    pub fn new(client: Arc<TuiClient>) -> Self {
        Self { client }
    }

    async fn command(
        &self,
        message: ClientMessage,
        reason: &'static str,
    ) -> Result<(), Unsupported> {
        self.client
            .send(message)
            .await
            .map_err(|_| Unsupported(reason))
    }

    async fn refresh_sessions(&self) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::ListSessions {},
            "failed to refresh daemon sessions",
        )
        .await
    }
}

#[async_trait::async_trait]
impl CommandTarget for DaemonTarget {
    async fn prompt(
        &mut self,
        session_id: &str,
        request_id: &str,
        enriched: String,
        parts: Vec<Part>,
    ) -> anyhow::Result<()> {
        let attachments = parts
            .into_iter()
            .filter_map(|part| match part {
                Part::File(file) => Some(mew_protocol::Attachment {
                    path: file.url,
                    mime: Some(file.mime),
                }),
                _ => None,
            })
            .collect();
        self.client
            .submit(session_id, request_id, enriched, attachments)
            .await
    }

    async fn cancel(&mut self, session_id: &str) -> anyhow::Result<()> {
        self.client
            .send_to_session(session_id, ClientMessage::Cancel)
            .await
    }
    async fn close_question(&mut self, session_id: &str, request_id: &str) -> anyhow::Result<()> {
        self.client
            .send_to_session(
                session_id,
                ClientMessage::AskUserResponse {
                    request_id: request_id.into(),
                    answers: Vec::new(),
                    cancelled: true,
                },
            )
            .await
    }
    async fn guide(&mut self, session_id: &str, text: String) -> anyhow::Result<()> {
        self.client
            .send_to_session(session_id, ClientMessage::Guide { text })
            .await
    }

    async fn clear(&mut self) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::SlashCommand {
                command: "/clear".into(),
            },
            "daemon connection failed during clear",
        )
        .await
    }
    async fn compact(&mut self) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::SlashCommand {
                command: "/compact".into(),
            },
            "daemon connection failed during compact",
        )
        .await
    }
    async fn read_subagent_history(
        &mut self,
        session_id: &str,
        child_session_id: &str,
        request_id: &str,
    ) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::ReadSubagentHistory {
                session_id: session_id.into(),
                child_session_id: child_session_id.into(),
                request_id: request_id.into(),
            },
            "could not request subagent trace",
        )
        .await
    }
    async fn update_todo(
        &mut self,
        session_id: &str,
        edit: mew_protocol::TodoEdit,
    ) -> anyhow::Result<Vec<mew_agent::Todo>> {
        self.client.update_todo(session_id, edit).await
    }
    async fn switch_model(&mut self, spec: &str) -> Result<SwitchedModel, Unsupported> {
        let (provider_id, model_id) = crate::setup::providers::split_provider_model(spec, "");
        self.command(
            ClientMessage::SwitchModel {
                provider: provider_id.clone(),
                model: model_id.clone(),
            },
            "daemon connection failed during model switch",
        )
        .await?;
        Ok(SwitchedModel {
            provider_id,
            model_id,
            display: spec.into(),
        })
    }
    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::SetPermissionMode {
                mode: mode.id().into(),
            },
            "daemon connection failed during permission mode change",
        )
        .await
    }
    async fn set_thinking(&mut self, variant: &str) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::SetThinkingVariant {
                variant: variant.into(),
            },
            "daemon connection failed during thinking variant change",
        )
        .await
    }
    async fn attach_session(&mut self, id: &str) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::AttachSession {
                session_id: id.into(),
                client_kind: ClientKind::Tui,
            },
            "failed to attach to daemon session",
        )
        .await?;
        self.refresh_sessions().await
    }
    async fn resume(&mut self, id: &str) -> Result<(), Unsupported> {
        self.attach_session(id).await
    }
    async fn rewind(&mut self, _n: usize) -> Result<(), Unsupported> {
        Err(Unsupported("rewind not available in daemon mode"))
    }
    async fn switch_persona(
        &mut self,
        name: &str,
        _personas: &[Persona],
    ) -> Result<PersonaApplied, Unsupported> {
        self.command(
            ClientMessage::SwitchPersona { name: name.into() },
            "daemon connection failed during persona switch",
        )
        .await?;
        Ok(PersonaApplied {
            pinned_model: None,
            display: format!("switched to persona: {name}"),
        })
    }
    async fn plugin_command(&mut self, _name: &str, _args: &str) -> Result<String, Unsupported> {
        Err(Unsupported("plugin commands not available in daemon mode"))
    }
    async fn cancel_subagent(&mut self, _task_id: &str) -> Result<bool, Unsupported> {
        Err(Unsupported(
            "subagent cancellation not available in daemon mode",
        ))
    }
    async fn manage_goal(&mut self, action: GoalAction) -> Result<String, Unsupported> {
        let text = match action {
            GoalAction::Set(text) => format!("/goal {text}"),
            GoalAction::Status => "/goal status".into(),
            GoalAction::Pause => "/goal pause".into(),
            GoalAction::Resume => "/goal resume".into(),
            GoalAction::Clear => "/goal clear".into(),
            GoalAction::Complete => "/goal complete".into(),
        };
        self.command(
            ClientMessage::SlashCommand { command: text },
            "daemon goal command failed",
        )
        .await?;
        Ok("goal command sent".into())
    }
    async fn set_auto_title(&mut self, enabled: bool) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::SetAutoTitle { enabled },
            "failed to update daemon auto-title setting",
        )
        .await
    }
    async fn set_auto_summary(&mut self, enabled: bool) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::SetAutoSummary { enabled },
            "failed to update daemon auto-summary setting",
        )
        .await
    }
    async fn yield_control(&mut self) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::YieldControl {},
            "daemon connection failed during yield",
        )
        .await
    }
    async fn unflag_file(&mut self, path: &str) -> Result<(), Unsupported> {
        let id = self
            .client
            .session_id()
            .await
            .ok_or(Unsupported("no active daemon session"))?;
        self.command(
            ClientMessage::UnflagFile {
                session_id: id,
                path: path.into(),
            },
            "failed to unflag daemon file",
        )
        .await
    }
    async fn list_projects(&mut self) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::ListProjects {},
            "failed to list daemon projects",
        )
        .await
    }
    async fn new_session_in(&mut self, path: &str) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::NewSession {
                cwd: Some(path.into()),
                client_kind: ClientKind::Tui,
            },
            "failed to create daemon session",
        )
        .await?;
        self.refresh_sessions().await
    }
    async fn archive_session(&mut self, id: &str, archived: bool) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::ArchiveSession {
                session_id: id.into(),
                archived,
            },
            "failed to archive daemon session",
        )
        .await?;
        self.refresh_sessions().await
    }
    async fn pin_session(&mut self, id: &str, pinned: bool) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::PinSession {
                session_id: id.into(),
                pinned,
            },
            "failed to pin daemon session",
        )
        .await?;
        self.refresh_sessions().await
    }
    async fn rename_session(&mut self, id: &str, title: &str) -> Result<(), Unsupported> {
        self.command(
            ClientMessage::RenameSession {
                session_id: id.into(),
                title: title.into(),
            },
            "failed to rename daemon session",
        )
        .await?;
        self.refresh_sessions().await
    }
}
