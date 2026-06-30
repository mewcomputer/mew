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
        &mut self,
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
        &mut self,
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
                .filter(|t| {
                    let name = t.name();
                    let allowed = self
                        .active_tool_names
                        .as_ref()
                        .is_none_or(|names| names.contains(name));
                    let not_denied = !self.denied_tool_names.contains(name);
                    allowed && not_denied
                })
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

            strip_empty_text_parts(&mut messages);

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
                // Notify plugins before compaction so they can capture any
                // context they want to preserve.
                self.dispatcher.on_pre_compaction(&messages).await;
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

                // Notify plugins that compaction finished with the
                // resulting (compacted) message list.
                self.dispatcher.on_post_compaction(&messages).await;
            }

            let base_system = match &self.persona_prompt {
                Some(persona) => format!("{}\n\n{}", persona, self.system),
                None => self.system.clone(),
            };
            let system = self.dispatcher.on_system_prompt(base_system).await;

            // Notify plugins that a model turn is about to start.
            self.dispatcher.on_pre_model_turn(&messages, &system).await;

            // Chat params / headers: give plugins a chance to modify them
            // before the request goes out. Defaults are "use the provider's
            // own defaults" — plugins can set specific values when they
            // know what works.
            //
            // If the reasoning truncator flagged the previous turn's
            // reasoning as over-length, force this request to call a
            // tool. This breaks the "But.. Wait.. Actually.." loop that
            // open models fall into without external pressure.
            let force_tool_choice = self.take_force_tool_choice();
            // Default `max_output_tokens` from the agent's configured
            // value. Saturation to i32 is safe — 32K is well within range
            // and i32::MAX is 2.1B, so the only thing saturation affects
            // is pathologically-large values (which is fine — those
            // saturate to "lots of output" intent).
            let default_max_tokens: Option<i32> = if self.default_max_output_tokens > 0 {
                Some(self.default_max_output_tokens.clamp(1, i32::MAX as i64) as i32)
            } else {
                None
            };
            let params = self
                .dispatcher
                .on_chat_params(ChatParams {
                    temperature: None,
                    top_p: None,
                    max_tokens: default_max_tokens,
                    tool_choice: if force_tool_choice {
                        Some(mew_hooks::ToolChoice::Required)
                    } else {
                        None
                    },
                })
                .await;
            let headers = self
                .dispatcher
                .on_chat_headers(http::HeaderMap::new())
                .await;

            let req = Request {
                model: String::new(),
                messages,
                tools: tool_defs,
                system,
                reasoning: self.reasoning.clone(),
                params: Some(mew_provider::ChatParams {
                    temperature: params.temperature,
                    top_p: params.top_p,
                    max_tokens: params.max_tokens,
                    tool_choice: params.tool_choice.map(|c| match c {
                        mew_hooks::ToolChoice::Auto => mew_provider::ToolChoice::Auto,
                        mew_hooks::ToolChoice::Required => mew_provider::ToolChoice::Required,
                        mew_hooks::ToolChoice::None_ => mew_provider::ToolChoice::None_,
                    }),
                }),
                headers,
            };

            let req_for_fallback = req.clone();
            let mut stream = match self.provider.stream(req).await {
                Ok(s) => s,
                Err(e) => {
                    // Try fallback models before giving up. Each fallback
                    // is a "provider/model" string; the provider_builder
                    // callback (set by the main loop) constructs a new
                    // provider for it. If any fallback succeeds, we swap
                    // it in as the active provider and retry the request.
                    let mut resolved_stream: Option<_> = None;
                    if let Some(ref fb) = self.fallback_models {
                        if !fb.is_empty() {
                            if let Some(ref builder) = self.provider_builder {
                                let mut last_err = e.to_string();
                                for model_str in fb.iter() {
                                    tracing::warn!(
                                        primary_error = %last_err,
                                        fallback = %model_str,
                                        "primary provider failed; trying fallback model"
                                    );
                                    let _ = ev_tx
                                        .send(AgentEvent::Error(format!(
                                            "provider error ({}); trying fallback: {model_str}",
                                            last_err
                                        )))
                                        .await;
                                    match (builder.0)(model_str) {
                                        Ok(new_provider) => {
                                            self.provider = new_provider;
                                            match self
                                                .provider
                                                .stream(req_for_fallback.clone())
                                                .await
                                            {
                                                Ok(s) => {
                                                    resolved_stream = Some(s);
                                                    break;
                                                }
                                                Err(e2) => {
                                                    last_err = e2.to_string();
                                                    tracing::warn!(
                                                        fallback = %model_str,
                                                        error = %last_err,
                                                        "fallback provider also failed"
                                                    );
                                                }
                                            }
                                        }
                                        Err(build_err) => {
                                            tracing::warn!(
                                                fallback = %model_str,
                                                error = %build_err,
                                                "could not build fallback provider"
                                            );
                                            last_err = build_err;
                                        }
                                    }
                                }
                                if resolved_stream.is_none() {
                                    let _ = ev_tx
                                        .send(AgentEvent::Error(format!(
                                            "all fallback models exhausted: {}",
                                            last_err
                                        )))
                                        .await;
                                    return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                                        last_err,
                                    ));
                                }
                            }
                        }
                    }
                    match resolved_stream {
                        Some(s) => s,
                        None => {
                            let _ = ev_tx
                                .send(AgentEvent::Error(format!("provider stream: {}", e)))
                                .await;
                            return Err(Box::new(e));
                        }
                    }
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
                                // Specific typed hook for subprocess plugins.
                                self.dispatcher.on_provider_event(&ev).await;
                                // Dedicated telemetry hook for metrics/OTEL
                                // plugins that only care about the final
                                // response, not every stream delta.
                                if let ProviderEvent::MessageEnd {
                                    ref finish,
                                    ref usage,
                                    cost,
                                } = ev
                                {
                                    let finish_str = format!("{:?}", finish);
                                    self.dispatcher
                                        .on_model_finish(
                                            &finish_str,
                                            usage.input,
                                            usage.output,
                                            cost,
                                        )
                                        .await;
                                }
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

            // Reasoning-truncation hook: if any reasoning trace in this
            // turn exceeded the configured threshold, truncate the part
            // in place, forge a short acknowledgement assistant message
            // into history, and flag the next request to force a tool
            // call. Provider-agnostic — no stream mutation needed.
            if self.reasoning_truncation_enabled {
                if let Some(msg) = assistant_msg.as_mut() {
                    if self.maybe_truncate_reasoning_in_place(msg) {
                        let ack_msg = Message {
                            id: Ulid::new(),
                            session_id: self.session_id,
                            role: Role::Assistant,
                            parts: vec![Part::Text(TextPart {
                                base: PartBase {
                                    id: Ulid::new(),
                                    message_id: Ulid::new(),
                                    session_id: self.session_id,
                                },
                                text: crate::reasoning_truncator::TRUNCATION_ACK_TEXT.to_string(),
                                synthetic: true,
                            })],
                            time: Time {
                                created: Utc::now().timestamp_millis(),
                                completed: Some(Utc::now().timestamp_millis()),
                            },
                            assistant: None,
                        };
                        tracing::info!(
                            threshold = self.reasoning_truncator.threshold,
                            "reasoning truncated; forging acknowledgement + forcing tool_choice"
                        );
                        self.append_message(ack_msg).await;
                        self.reasoning_truncator.mark_truncated();
                    }
                }
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
                // End of turn. If the model called `switch_persona` during
                // this turn, drain the pending slot and emit the event the
                // main loop uses to apply the change. Done at end of turn
                // (not mid-turn) so the user sees the full response before
                // the model swap happens, and so the model can chain
                // switch_persona with one final text response.
                if let Some(name) = self.pending_persona_switch.lock().await.take() {
                    if self.personas.iter().any(|p| p.name == name) {
                        let _ = ev_tx
                            .send(AgentEvent::PersonaSwitchRequested { name })
                            .await;
                    } else {
                        tracing::warn!(
                            name = %name,
                            "switch_persona queued an unknown persona; dropping"
                        );
                    }
                }
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

/// Strip empty text parts from assistant messages so providers don't choke on
/// spurious empty content blocks before tool calls.
pub(crate) fn strip_empty_text_parts(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        if msg.role == Role::Assistant {
            msg.parts.retain(|p| match p {
                Part::Text(pt) => !pt.text.is_empty(),
                _ => true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{PartBase, TextPart, ToolCallPart, ToolState, ToolStateCompleted, ToolTime};

    fn text_part(text: &str) -> Part {
        Part::Text(TextPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: text.to_string(),
            synthetic: false,
        })
    }

    fn tool_call_part() -> Part {
        Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            tool_name: "bash".to_string(),
            call_id: "call_1".to_string(),
            state: ToolState::Completed(ToolStateCompleted {
                input: serde_json::json!({"command": "ls"}),
                output: "ok".to_string(),
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: 0,
                    end: Some(0),
                },
            }),
            raw_input: "{}".to_string(),
        })
    }

    fn make_msg(role: Role, parts: Vec<Part>) -> Message {
        Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role,
            parts,
            time: Time {
                created: 0,
                completed: Some(0),
            },
            assistant: None,
        }
    }

    #[test]
    fn strips_empty_text_from_assistant_messages() {
        let mut messages = vec![make_msg(
            Role::Assistant,
            vec![text_part(""), tool_call_part()],
        )];
        strip_empty_text_parts(&mut messages);
        assert_eq!(messages[0].parts.len(), 1);
        assert!(matches!(&messages[0].parts[0], Part::ToolCall(_)));
    }

    #[test]
    fn keeps_nonempty_text_in_assistant_messages() {
        let mut messages = vec![make_msg(
            Role::Assistant,
            vec![text_part("let me check"), tool_call_part()],
        )];
        strip_empty_text_parts(&mut messages);
        assert_eq!(messages[0].parts.len(), 2);
    }

    #[test]
    fn does_not_touch_user_messages() {
        let mut messages = vec![make_msg(Role::User, vec![text_part("")])];
        strip_empty_text_parts(&mut messages);
        assert_eq!(messages[0].parts.len(), 1);
    }

    #[test]
    fn handles_empty_parts_vec() {
        let mut messages = vec![make_msg(Role::Assistant, vec![])];
        strip_empty_text_parts(&mut messages);
        assert!(messages[0].parts.is_empty());
    }
}
