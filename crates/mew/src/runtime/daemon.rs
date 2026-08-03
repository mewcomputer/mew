//! `DaemonTarget` — `CommandTarget` impl for daemon-connected TUI mode.
//!
//! Wraps a `DaemonClient` and forwards operations to the daemon over the
//! WebSocket protocol. Operations that aren't meaningful in daemon mode
//! (rewind, plugin commands) return `Unsupported`.

use std::sync::Arc;

use mew_agent::AgentEvent;
use mew_hooks::PermissionMode;
use mew_message::Part;
use mew_personas::Persona;
use tokio::sync::mpsc::Receiver;

use crate::runtime::target::{
    CommandTarget, GoalAction, PersonaApplied, SwitchedModel, Unsupported,
};

/// The daemon-connected command target. Wraps an `Arc<DaemonClient>` and
/// forwards operations over the WebSocket protocol.
pub struct DaemonTarget {
    pub client: Arc<mew_daemon::DaemonClient>,
}

impl DaemonTarget {
    pub fn new(client: Arc<mew_daemon::DaemonClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl CommandTarget for DaemonTarget {
    fn prompt(&mut self, enriched: String, parts: Vec<Part>) -> Receiver<AgentEvent> {
        // Convert Part::File attachments to protocol Attachment structs.
        // Other part types (Text, Reasoning, etc.) are inlined into the
        // prompt text by process_mentions and don't need separate attachment.
        let attachments: Vec<mew_protocol::Attachment> = parts
            .iter()
            .filter_map(|p| match p {
                Part::File(fp) => Some(mew_protocol::Attachment {
                    path: fp.url.clone(),
                    mime: Some(fp.mime.clone()),
                }),
                _ => None,
            })
            .collect();

        // Bridge the async client.prompt() into the sync fn prompt() signature
        // by spawning a forwarding task that drains the client's receiver into
        // a channel.
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut recv = client.prompt(enriched, attachments).await;
            while let Some(event) = recv.recv().await {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        rx
    }

    async fn cancel(&mut self) {
        self.client.cancel().await;
    }

    async fn clear(&mut self) -> Result<(), Unsupported> {
        let client = self.client.clone();
        client
            .slash_command("/clear".into())
            .await
            .map_err(|_| Unsupported("daemon connection failed during clear"))
    }

    async fn compact(&mut self) -> Result<(), Unsupported> {
        let client = self.client.clone();
        client
            .slash_command("/compact".into())
            .await
            .map_err(|_| Unsupported("daemon connection failed during compact"))
    }

    async fn todos(&mut self) -> Result<String, Unsupported> {
        // Todos come via TodosUpdated events from the daemon, not as a
        // direct response. Return a placeholder — the app has the snapshot.
        Err(Unsupported("todos are pushed by the daemon as events"))
    }

    async fn switch_model(&mut self, spec: &str) -> Result<SwitchedModel, Unsupported> {
        let (provider_id, model_id) = crate::setup::providers::split_provider_model(spec, "");
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SwitchModel {
            provider: provider_id.clone(),
            model: model_id.clone(),
        };
        match mew_protocol::encode_json(&msg) {
            Ok(json) => match client.send_raw(&json).await {
                Ok(()) => Ok(SwitchedModel {
                    provider_id,
                    model_id,
                    display: spec.to_string(),
                }),
                Err(_) => Err(Unsupported("daemon connection failed during model switch")),
            },
            Err(_) => Err(Unsupported("failed to encode model switch message")),
        }
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), Unsupported> {
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SetPermissionMode {
            mode: mode.id().to_string(),
        };
        match mew_protocol::encode_json(&msg) {
            Ok(json) => match client.send_raw(&json).await {
                Ok(()) => Ok(()),
                Err(_) => Err(Unsupported(
                    "daemon connection failed during permission mode change",
                )),
            },
            Err(_) => Err(Unsupported("failed to encode permission mode message")),
        }
    }

    async fn set_thinking(&mut self, variant: &str) -> Result<(), Unsupported> {
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SetThinkingVariant {
            variant: variant.to_string(),
        };
        match mew_protocol::encode_json(&msg) {
            Ok(json) => match client.send_raw(&json).await {
                Ok(()) => Ok(()),
                Err(_) => Err(Unsupported(
                    "daemon connection failed during thinking variant change",
                )),
            },
            Err(_) => Err(Unsupported("failed to encode thinking variant message")),
        }
    }

    async fn attach_session(&mut self, id: &str) -> Result<(), Unsupported> {
        let client = self.client.clone();
        match client.attach_session(id).await {
            Ok(()) => {
                client
                    .list_sessions()
                    .await
                    .map_err(|_| Unsupported("failed to refresh daemon sessions"))?;
                Ok(())
            }
            Err(_) => Err(Unsupported("failed to attach to daemon session")),
        }
    }

    async fn resume(&mut self, id: &str) -> Result<(), Unsupported> {
        // In daemon mode, resume is the same as attach.
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
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SwitchPersona {
            name: name.to_string(),
        };
        match mew_protocol::encode_json(&msg) {
            Ok(json) => match client.send_raw(&json).await {
                Ok(()) => Ok(PersonaApplied {
                    pinned_model: None,
                    display: format!("switched to persona: {}", name),
                }),
                Err(_) => Err(Unsupported(
                    "daemon connection failed during persona switch",
                )),
            },
            Err(_) => Err(Unsupported("failed to encode persona switch message")),
        }
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
        // Send the goal command to the daemon as a slash command. The
        // daemon's slash-command handler recognizes /goal and manages the
        // agent's goal state server-side, returning a status string.
        let cmd = match action {
            GoalAction::Set(text) => format!("/goal {text}"),
            GoalAction::Status => "/goal status".to_string(),
            GoalAction::Pause => "/goal pause".to_string(),
            GoalAction::Resume => "/goal resume".to_string(),
            GoalAction::Clear => "/goal clear".to_string(),
            GoalAction::Complete => "/goal complete".to_string(),
        };
        match self.client.slash_command(cmd).await {
            Ok(()) => Ok("goal command sent".to_string()),
            Err(_) => Err(Unsupported("daemon goal command failed")),
        }
    }

    async fn set_auto_title(&mut self, enabled: bool) -> Result<(), Unsupported> {
        self.client
            .set_auto_title(enabled)
            .await
            .map_err(|_| Unsupported("failed to update daemon auto-title setting"))
    }

    async fn set_auto_summary(&mut self, enabled: bool) -> Result<(), Unsupported> {
        self.client
            .set_auto_summary(enabled)
            .await
            .map_err(|_| Unsupported("failed to update daemon auto-summary setting"))
    }

    async fn yield_control(&mut self) -> Result<(), Unsupported> {
        let msg = mew_protocol::ClientMessage::YieldControl {};
        match mew_protocol::encode_json(&msg) {
            Ok(json) => match self.client.send_raw(&json).await {
                Ok(()) => Ok(()),
                Err(_) => Err(Unsupported("daemon connection failed during yield")),
            },
            Err(_) => Err(Unsupported("failed to encode yield message")),
        }
    }

    async fn unflag_file(&mut self, path: &str) -> Result<(), Unsupported> {
        match self.client.session_id().await {
            Some(id) => self
                .client
                .unflag_file(&id, path)
                .await
                .map_err(|_| Unsupported("failed to unflag daemon file")),
            None => Err(Unsupported("no active daemon session")),
        }
    }

    async fn list_projects(&mut self) -> Result<(), Unsupported> {
        self.client
            .list_projects()
            .await
            .map_err(|_| Unsupported("failed to list daemon projects"))
    }

    async fn new_session_in(&mut self, path: &str) -> Result<(), Unsupported> {
        self.client
            .new_session_in(path)
            .await
            .map_err(|_| Unsupported("failed to create daemon session"))?;
        self.client
            .list_sessions()
            .await
            .map_err(|_| Unsupported("failed to refresh daemon sessions"))
    }

    async fn archive_session(&mut self, id: &str, archived: bool) -> Result<(), Unsupported> {
        self.client
            .archive_session(id, archived)
            .await
            .map_err(|_| Unsupported("failed to archive daemon session"))?;
        self.client
            .list_sessions()
            .await
            .map_err(|_| Unsupported("failed to refresh daemon sessions"))
    }

    async fn pin_session(&mut self, id: &str, pinned: bool) -> Result<(), Unsupported> {
        self.client
            .pin_session(id, pinned)
            .await
            .map_err(|_| Unsupported("failed to pin daemon session"))?;
        self.client
            .list_sessions()
            .await
            .map_err(|_| Unsupported("failed to refresh daemon sessions"))
    }

    async fn rename_session(&mut self, id: &str, title: &str) -> Result<(), Unsupported> {
        self.client
            .rename_session(id, title)
            .await
            .map_err(|_| Unsupported("failed to rename daemon session"))?;
        self.client
            .list_sessions()
            .await
            .map_err(|_| Unsupported("failed to refresh daemon sessions"))
    }
}
