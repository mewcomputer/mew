use chrono::Utc;
use futures::StreamExt;
use tokio::sync::mpsc;
use ulid::Ulid;

use mew_hooks::ChatParams;
use mew_message::{
    AssistantMeta, ErrorKind, Message, MessageError, Part, PartBase, Role, TextPart, Time, Tokens,
};
use mew_provider::{ProviderEvent, Request, ToolDef};
use mew_tools::tools::flag_important::FlagMode;

use crate::agent::Agent;
use crate::AgentEvent;

impl Agent {
    pub(crate) async fn run_loop(
        &self,
        prompt: String,
        attachments: Vec<Part>,
        ev_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let msg_id = Ulid::new();
        let mut parts = vec![Part::Text(TextPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: msg_id,
                session_id: self.session_id,
            },
            text: prompt,
            synthetic: false,
        })];
        // Fix attachment IDs to reference this message, then append.
        let mut has_image = false;
        for mut attachment in attachments {
            if let Part::File(ref mut fp) = attachment {
                fp.base.message_id = msg_id;
                fp.base.session_id = self.session_id;
                if fp.mime.starts_with("image/") {
                    has_image = true;
                }
            }
            parts.push(attachment);
        }

        // Reject image attachments if the model doesn't support vision.
        if has_image && !self.supports_vision {
            let _ = ev_tx
                .send(AgentEvent::Error(
                    "image attachments not supported by the current model".into(),
                ))
                .await;
            return Ok(());
        }
        let user_msg = Message {
            id: msg_id,
            session_id: self.session_id,
            role: Role::User,
            parts,
            time: Time {
                created: Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        };

        self.append_message(user_msg).await;
        self.turn_loop(ev_tx).await
    }

    pub(crate) async fn turn_loop(
        &self,
        ev_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut turn_count: u32 = 0;
        loop {
            if let Some(max) = self.max_turns {
                if turn_count >= max {
                    tracing::info!(turn_count, max, "max turns reached");
                    let _ = ev_tx
                        .send(AgentEvent::Error(format!(
                            "max turns exceeded ({} turns)",
                            turn_count
                        )))
                        .await;
                    return Ok(());
                }
            }
            turn_count += 1;
            let tool_defs: Vec<ToolDef> = self
                .tools
                .values()
                .map(|t| ToolDef {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    schema: t.schema().clone(),
                })
                .collect();

            let mut messages = self.messages.lock().await.clone();

            // Apply on_chat_message hook to each message.
            for msg in &mut messages {
                *msg = self.dispatcher.on_chat_message(msg.clone()).await;
            }

            // Check if compaction is needed (forced or auto).
            let force = {
                let mut flag = self.force_compact.lock().await;
                let was = *flag;
                *flag = false;
                was
            };
            let estimated = self.estimated_tokens(&messages);
            let threshold = (self.context_window as f64 * self.compaction_threshold) as u32;
            let should_compact = force || (self.context_window > 0 && estimated > threshold);
            if should_compact {
                tracing::info!(
                    estimated,
                    threshold,
                    context_window = self.context_window,
                    force,
                    "compacting context"
                );
                let keep_count = self.keep_turns.min(messages.len());
                let compacted = messages.split_off(messages.len() - keep_count);
                let compact_msg = Message {
                    id: Ulid::new(),
                    session_id: self.session_id,
                    role: Role::User,
                    parts: vec![Part::Text(TextPart {
                        base: PartBase {
                            id: Ulid::new(),
                            message_id: Ulid::new(),
                            session_id: self.session_id,
                        },
                        text: "Previous conversation has been compacted to stay within the context window. Recent turns are preserved below."
                            .into(),
                        synthetic: true,
                    })],
                    time: Time {
                        created: chrono::Utc::now().timestamp_millis(),
                        completed: None,
                    },
                    assistant: None,
                };
                let len_before = messages.len();
                messages = compacted;
                // Prepend the summary note.
                messages.insert(0, compact_msg);

                // Re-inject flagged files so they survive compaction. Included
                // files are inlined as text; referenced files get a pointer.
                // Iterate in reverse so insert(0, ...) preserves flag order.
                let flagged = self.flagged_files.lock().await;
                if !flagged.is_empty() {
                    let now = chrono::Utc::now().timestamp_millis();
                    for f in flagged.iter().rev() {
                        let note_text = match f.mode {
                            FlagMode::Included => match std::fs::read_to_string(&f.path) {
                                Ok(content) => format!(
                                    "Flagged file (preserved across compaction) — {}:\n\n{}",
                                    f.path.display(),
                                    content,
                                ),
                                Err(e) => {
                                    tracing::warn!(
                                        path = %f.path.display(),
                                        error = %e,
                                        "could not read flagged file for compaction re-injection",
                                    );
                                    continue;
                                }
                            },
                            FlagMode::Referenced => format!(
                                "Flagged file (referenced — re-read with the read tool when needed): {}",
                                f.path.display(),
                            ),
                        };
                        let id = Ulid::new();
                        messages.insert(
                            0,
                            Message {
                                id,
                                session_id: self.session_id,
                                role: Role::User,
                                parts: vec![Part::Text(TextPart {
                                    base: PartBase {
                                        id: Ulid::new(),
                                        message_id: id,
                                        session_id: self.session_id,
                                    },
                                    text: note_text,
                                    synthetic: true,
                                })],
                                time: Time {
                                    created: now,
                                    completed: None,
                                },
                                assistant: None,
                            },
                        );
                    }
                }
                drop(flagged);

                let _ = ev_tx
                    .send(AgentEvent::Error(format!(
                        "context compacted: {} turns removed ({} estimated tokens)",
                        len_before, estimated
                    )))
                    .await;
            }

            let system = self.dispatcher.on_system_prompt(self.system.clone()).await;

            let req = Request {
                model: String::new(),
                messages,
                tools: tool_defs,
                system,
                reasoning: self.reasoning.clone(),
            };

            let _ = self
                .dispatcher
                .on_chat_params(ChatParams {
                    temperature: None,
                    top_p: None,
                    max_tokens: None,
                })
                .await;
            let _ = self
                .dispatcher
                .on_chat_headers(http::HeaderMap::new())
                .await;

            let mut stream = match self.provider.stream(req).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = ev_tx
                        .send(AgentEvent::Error(format!("provider stream: {}", e)))
                        .await;
                    return Err(Box::new(e));
                }
            };

            let mut assistant_msg: Option<Message> = None;

            // Stream provider events until the stream ends or we are cancelled.
            loop {
                tokio::select! {
                    biased;
                    _ = self.cancel_token.cancelled() => {
                        if let Some(ref mut msg) = assistant_msg {
                            let now = Utc::now().timestamp_millis();
                            msg.time.completed = Some(now);
                            msg.assistant = Some(AssistantMeta {
                                provider_id: String::new(),
                                model_id: String::new(),
                                cost: 0.0,
                                tokens: Tokens::default(),
                                finish: None,
                                error: Some(MessageError {
                                    kind: ErrorKind::Aborted,
                                    message: "aborted".into(),
                                }),
                            });
                            self.append_message(msg.clone()).await;
                        }
                        let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                        return Ok(());
                    }
                    ev = stream.next() => {
                        match ev {
                            None => break,
                            Some(ev) => {
                                self.handle_provider_event(
                                    &ev,
                                    &mut assistant_msg,
                                    &ev_tx,
                                ).await;
                                let agent_ev = AgentEvent::Provider(ev.clone());
                                self.dispatcher.on_event(&agent_ev).await;
                                if matches!(ev, ProviderEvent::Error(_)) {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }

            // Stream ended naturally.
            if assistant_msg.is_none() {
                let _ = ev_tx
                    .send(AgentEvent::Error("no assistant message received".into()))
                    .await;
                return Ok(());
            }

            if self.cancel_token.is_cancelled() {
                if let Some(ref mut msg) = assistant_msg {
                    let now = Utc::now().timestamp_millis();
                    msg.time.completed = Some(now);
                    msg.assistant = Some(AssistantMeta {
                        provider_id: String::new(),
                        model_id: String::new(),
                        cost: 0.0,
                        tokens: Tokens::default(),
                        finish: None,
                        error: Some(MessageError {
                            kind: ErrorKind::Aborted,
                            message: "aborted".into(),
                        }),
                    });
                    self.append_message(msg.clone()).await;
                }
                let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                return Ok(());
            }

            let msg = assistant_msg.as_ref().ok_or_else(|| {
                Box::<dyn std::error::Error + Send + Sync>::from(
                    "assistant message missing after stream ended".to_string(),
                )
            })?;
            let pending = self.pending_tool_calls(msg);
            if pending.is_empty() {
                let messages = self.messages.lock().await.clone();
                self.dispatcher.on_turn_end(&messages).await;
                return Ok(());
            }

            let result_parts = self
                .execute_pending_tool_calls(&pending, &mut assistant_msg, &ev_tx)
                .await;

            // Sync updated assistant message (with tool state transitions)
            // back into self.messages so the next request has the correct state.
            if let Some(ref msg) = assistant_msg {
                tracing::debug!(msg_id = %msg.id, "syncing assistant message to store");
                let mut messages = self.messages.lock().await;
                for m in messages.iter_mut() {
                    if m.id == msg.id {
                        *m = msg.clone();
                        break;
                    }
                }
            }

            let result_msg = Message {
                id: Ulid::new(),
                session_id: self.session_id,
                role: Role::User,
                parts: result_parts,
                time: Time {
                    created: Utc::now().timestamp_millis(),
                    completed: None,
                },
                assistant: None,
            };
            self.append_message(result_msg).await;

            let messages = self.messages.lock().await.clone();
            let dispatcher = self.dispatcher.clone();
            tokio::spawn(async move {
                dispatcher.on_turn_end(&messages).await;
            });
        }
    }
}
