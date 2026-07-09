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

use crate::runtime::target::{CommandTarget, PersonaApplied, SwitchedModel, Unsupported};

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
                client.list_sessions().await;
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
}
