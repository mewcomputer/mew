use chrono::Utc;
use futures::StreamExt;
use tokio::sync::mpsc;
use ulid::Ulid;

use mew_hooks::ChatParams;
use mew_message::{
    CompactionPart, ErrorKind, Finish, Message, MessageError, Part, PartBase, Role, TextPart, Time,
    ToolResultPart, ToolState, ToolStateError, ToolTime,
};
use mew_provider::{ProviderEvent, Request, ToolDef};
use mew_subagents::ChildGuidanceError;
use mew_tools::tools::flag_important::FlagMode;

use crate::agent::{AcceptedPrompt, Agent, PromptAdmissionError, TurnInput};
use crate::{AgentEvent, GoalStatus};

impl Agent {
    pub(crate) fn build_prompt_message(
        &self,
        prompt: String,
        attachments: Vec<Part>,
    ) -> Result<Message, PromptAdmissionError> {
        if !self.supports_vision
            && attachments
                .iter()
                .any(|part| matches!(part, Part::File(file) if file.mime.starts_with("image/")))
        {
            return Err(PromptAdmissionError::UnsupportedImage);
        }

        let msg_id = Ulid::new();
        let mut parts = Vec::with_capacity(attachments.len() + 1);
        parts.push(Part::Text(TextPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: msg_id,
                session_id: self.session_id,
            },
            text: prompt,
            synthetic: false,
        }));
        for mut attachment in attachments {
            if let Part::File(ref mut file) = attachment {
                file.base.message_id = msg_id;
                file.base.session_id = self.session_id;
            }
            parts.push(attachment);
        }

        Ok(Message {
            id: msg_id,
            session_id: self.session_id,
            role: Role::User,
            parts,
            time: Time {
                created: Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        })
    }

    pub(crate) async fn run_loop(
        &mut self,
        input: TurnInput,
        ev_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match input {
            TurnInput::Accepted(AcceptedPrompt(message)) => {
                self.messages.lock().await.push(*message);
            }
            TurnInput::New { text, attachments } => {
                match self.build_prompt_message(text, attachments) {
                    Ok(message) => self.append_message(message).await,
                    Err(error) => {
                        let _ = ev_tx.send(AgentEvent::Error(error.to_string())).await;
                        return Ok(());
                    }
                }
            }
        }
        self.inject_orphaned_subagent_tasks().await;
        self.turn_loop(ev_tx).await
    }

    /// On the first turn of a resumed session, surface any subagent tasks the
    /// previous run left uncollected: one synthetic message listing them, with
    /// each result recovered from the child transcript where possible. The
    /// registry file is cleared after the message is appended so it surfaces
    /// exactly once.
    async fn inject_orphaned_subagent_tasks(&mut self) {
        let Some(path) = self.subagent_registry_path.clone() else {
            return;
        };
        {
            let mut handled = self.subagent_registry_handled.lock().await;
            if *handled {
                return;
            }
            // Mark handled *before* the async work so a re-entrant turn can't
            // double-inject. If load fails below, the records are lost for
            // this process lifetime — acceptable tradeoff for surface-once.
            *handled = true;
        }
        let records = match crate::subagent_registry::load(&path).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load subagent task registry");
                return;
            }
        };
        if records.is_empty() {
            return;
        }

        let session_dir = path.parent().map(|p| p.to_path_buf());
        let mut lines = String::new();
        for r in &records {
            let todo_note = match r.todo_id {
                Some(id) => format!(", todo #{}", id),
                None => String::new(),
            };
            let recovered = match (&session_dir, &r.child_session_id) {
                (Some(dir), Some(child_id)) => {
                    crate::subagent_registry::recover_child_text(dir, child_id).await
                }
                _ => None,
            };
            let outcome = match recovered {
                Some(text) => {
                    let trimmed: String = text.chars().take(2000).collect();
                    format!("recovered result:\n{}", trimmed)
                }
                None => "result lost (transcript unavailable)".to_string(),
            };
            lines.push_str(&format!(
                "- {} ({}{}): {}\n",
                r.task_id, r.name, todo_note, outcome
            ));
        }
        let msg = Message {
            id: Ulid::new(),
            session_id: self.session_id,
            role: Role::User,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
                    session_id: self.session_id,
                },
                text: format!(
                    "<orphaned_subagent_tasks>\n\
                     The previous run of this session ended with {} subagent task(s) that \
                     were never collected:\n\
                     {lines}\
                     Use these results or disregard them as you see fit.\n\
                     </orphaned_subagent_tasks>",
                    records.len()
                ),
                synthetic: true,
            })],
            time: Time {
                created: Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        };
        self.append_message(msg).await;

        // Surface once: clear the registry only after the message is safely
        // appended, so a crash between here and the clear at worst re-surfaces
        // the orphans on the next resume instead of losing them.
        if let Err(e) = crate::subagent_registry::save(&path, &[]).await {
            tracing::warn!(error = %e, "failed to clear subagent task registry");
        }
    }

    pub(crate) async fn turn_loop(
        &mut self,
        ev_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut turn_count: u32 = 0;
        let mut continuing_for_guidance = false;
        loop {
            if self.cancel_token.is_cancelled() {
                let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                return Ok(());
            }
            if let Some(max) = self.max_turns {
                if turn_count >= max {
                    tracing::info!(turn_count, max, "max turns reached");
                    // Child runners report their turn limit after draining the
                    // final response and its tool results.
                    if self.child_guidance.is_none() {
                        let _ = ev_tx
                            .send(AgentEvent::Error(format!(
                                "max turns exceeded ({} turns)",
                                turn_count
                            )))
                            .await;
                    }
                    return Ok(());
                }
            }
            turn_count += 1;
            let mut tool_defs: Vec<ToolDef> = self
                .tools
                .values()
                .filter(|t| self.tool_available(t.name()))
                .map(|t| ToolDef {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    schema: t.schema().clone(),
                })
                .collect();
            sort_tool_defs(&mut tool_defs);
            let direct_tool_defs = tool_defs.clone();
            if self.uses_code_mode() {
                tool_defs = vec![self.python_tool_definition()];
            }

            // Ingest any user guidance queued since the last request so it
            // steers the next provider request (even a tool-call continuation).
            self.drain_guidance().await;

            // Check if compaction is needed (forced or auto).
            let force = {
                let mut flag = self.force_compact.lock().await;
                let was = *flag;
                *flag = false;
                was
            };
            // A configured retention window may make a deferred refresh safe
            // without compaction. Unknown retention remains conservative.
            self.apply_deferred_system_rebuild_if_safe();
            let system = self.request_system().await;

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

            let mut req = Request {
                model: String::new(),
                messages: self.prepare_messages().await,
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
                supports_vision: self.supports_vision,
            };

            let mut manifest = match self.fit_request_context(&mut req, force).await {
                Ok((manifest, _)) => manifest,
                Err(message) => {
                    if continuing_for_guidance {
                        if let Some((admission, _)) =
                            self.child_guidance.as_ref().and_then(|queue| queue.claim())
                        {
                            admission.finish(Err(ChildGuidanceError::Rejected(message)));
                        }
                    } else {
                        let _ = ev_tx.send(AgentEvent::Error(message)).await;
                    }
                    return Ok(());
                }
            };
            let guided = self.admit_child_guidance(&mut req, &mut manifest).await;
            if continuing_for_guidance && !guided {
                // The previous clean response already completed the original
                // task. A rejected guide needs no extra provider call.
                return Ok(());
            }
            if continuing_for_guidance {
                let _ = ev_tx.send(AgentEvent::ChildGuidanceContinuation).await;
                continuing_for_guidance = false;
            }
            render_compaction_summary(&mut req.messages);
            self.dispatcher
                .on_pre_model_turn(&req.messages, &req.system)
                .await;

            *self.pending_manifest.lock().unwrap() = Some(manifest);

            let cancel = self.cancel_token.clone();
            let opening = async {
                let fallback = self
                    .fallback_models
                    .as_ref()
                    .filter(|models| !models.is_empty())
                    .zip(self.provider_builder.as_ref())
                    .map(|(models, builder)| (models.clone(), builder.clone(), req.clone()));
                self.mark_prompt_request_sent();
                let primary_error = match self.provider.stream(req).await {
                    Ok(stream) => return Ok(stream),
                    Err(error) => error,
                };
                let Some((models, builder, request)) = fallback else {
                    return Err(format!("provider stream: {primary_error}"));
                };

                let mut last_error = primary_error.to_string();
                for (index, model) in models.iter().enumerate() {
                    tracing::warn!(error = %last_error, fallback = %model, "trying fallback provider");
                    let _ = ev_tx
                        .send(AgentEvent::Provider(ProviderEvent::RetryWait {
                            attempt: index as u32 + 1,
                            max_attempts: models.len() as u32,
                            delay_secs: 0,
                            reason: format!("trying fallback: {model}"),
                        }))
                        .await;
                    let (provider_id, model_id) = model.split_once('/').unwrap_or((model, ""));
                    let mut fallback_request = request.clone();
                    let code_mode = self.code_mode.for_model(provider_id, model_id)
                        == mew_config::CodeMode::Full;
                    fallback_request.system = self.request_system_for_mode(code_mode).await;
                    fallback_request.tools = if code_mode {
                        vec![self.python_tool_definition()]
                    } else {
                        direct_tool_defs.clone()
                    };
                    match (builder.0)(model) {
                        Ok(provider) => match provider.stream(fallback_request).await {
                            Ok(stream) => {
                                self.provider = provider;
                                self.set_model_info(model_id, provider_id);
                                return Ok(stream);
                            }
                            Err(error) => last_error = error.to_string(),
                        },
                        Err(error) => last_error = error,
                    }
                }
                Err(format!("all fallback models exhausted: {last_error}"))
            };
            let mut stream = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                    return Ok(());
                }
                result = opening => match result {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = ev_tx.send(AgentEvent::Error(error)).await;
                        return Ok(());
                    }
                },
            };

            let mut assistant_msg: Option<Message> = None;

            let finish = loop {
                tokio::select! {
                    biased;
                    _ = self.cancel_token.cancelled() => {
                        if let Some(ref mut msg) = assistant_msg {
                            msg.parts.retain(|part| !matches!(part, Part::ToolCall(_)));
                            msg.time.completed = Some(Utc::now().timestamp_millis());
                            if let Some(meta) = &mut msg.assistant {
                                meta.error = Some(MessageError {
                                    kind: ErrorKind::Aborted,
                                    message: "aborted".into(),
                                });
                            }
                            self.append_message(msg.clone()).await;
                        }
                        let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                        return Ok(());
                    }
                    ev = stream.next() => {
                        match ev {
                            None => {
                                let error = ProviderEvent::Error(MessageError {
                                    kind: ErrorKind::ProviderApi,
                                    message: "provider stream ended before completion".into(),
                                });
                                self.handle_provider_event(&error, &mut assistant_msg, &ev_tx).await;
                                self.dispatcher.on_provider_event(&error).await;
                                return Ok(());
                            }
                            Some(mut ev) => {
                                if let ProviderEvent::MessageEnd { usage, cost, .. } = &mut ev {
                                    if !cost.is_finite() || *cost <= 0.0 {
                                        *cost = (f64::from(usage.input) * self.input_price
                                            + f64::from(usage.output) * self.output_price
                                            + f64::from(usage.reasoning) * self.reasoning_price
                                            + f64::from(usage.cache_read) * self.cache_read_price
                                            + f64::from(usage.cache_write) * self.cache_write_price)
                                            / 1_000_000.0;
                                    }
                                }
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
                                    break *finish;
                                }
                                if matches!(ev, ProviderEvent::Error(_)) {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            };

            if finish == Finish::Error {
                let _ = ev_tx
                    .send(AgentEvent::Error(
                        "provider reported a failed response".into(),
                    ))
                    .await;
                return Ok(());
            }
            if assistant_msg.is_none() {
                let _ = ev_tx
                    .send(AgentEvent::Error("no assistant message received".into()))
                    .await;
                return Ok(());
            }

            // Reasoning-truncation hook: if any reasoning trace in this
            // turn exceeded the configured threshold, truncate the part
            // in place, append a short acknowledgement Text part to the
            // SAME assistant message, and flag the next request to force
            // a tool call. The ack is appended as a part (not a new
            // message) so the wire builder keeps pairing the assistant's
            // tool_calls with the user message's ToolResultParts that
            // follow — a forged assistant message between them would
            // break the alternation and cause providers to reject the
            // request ("insufficient tool messages following toolcalls").
            if self.reasoning_truncation_enabled {
                if let Some(msg) = assistant_msg.as_mut() {
                    if self.maybe_truncate_reasoning_in_place(msg) {
                        let ack_text_part = Part::Text(TextPart {
                            base: PartBase {
                                id: Ulid::new(),
                                message_id: msg.id,
                                session_id: self.session_id,
                            },
                            text: crate::reasoning_truncator::TRUNCATION_ACK_TEXT.to_string(),
                            synthetic: true,
                        });
                        msg.parts.push(ack_text_part);
                        tracing::info!(
                            threshold = self.reasoning_truncator.threshold,
                            "reasoning truncated; appended acknowledgement part + forcing tool_choice"
                        );
                        // Sync the modified assistant message back into
                        // the store so the next request sees the new part.
                        let updated_msg = msg.clone();
                        let mut messages = self.messages.lock().await;
                        for m in messages.iter_mut() {
                            if m.id == updated_msg.id {
                                *m = updated_msg;
                                break;
                            }
                        }
                        drop(messages);
                        self.reasoning_truncator.mark_truncated();
                    }
                }
            }

            let msg = assistant_msg.as_ref().ok_or_else(|| {
                Box::<dyn std::error::Error + Send + Sync>::from(
                    "assistant message missing after stream ended".to_string(),
                )
            })?;
            let pending = if finish == Finish::ToolUse {
                self.pending_tool_calls(msg)
            } else {
                Vec::new()
            };
            if pending.is_empty() {
                if self.cancel_token.is_cancelled() {
                    let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                    return Ok(());
                }
                let messages = self.messages.lock().await.clone();
                self.dispatcher.on_turn_end(&messages).await;

                // Goal continuation: if there's an active goal, inject a
                // continuation prompt as a user message and loop back
                // instead of ending the turn.
                let should_continue = {
                    let mut goal_guard = self.goal.lock().await;
                    if let Some(ref mut goal) = *goal_guard {
                        if goal.status == GoalStatus::Active {
                            let max = std::env::var("MEW_GOAL_MAX_CONTINUATIONS")
                                .ok()
                                .and_then(|v| v.parse::<u32>().ok())
                                .unwrap_or(500);
                            if goal.continuation_count >= max {
                                let _ = ev_tx
                                    .send(AgentEvent::Error(format!(
                                        "goal auto-continuation stopped after {max} continuations"
                                    )))
                                    .await;
                                goal.status = GoalStatus::Paused;
                                false
                            } else {
                                goal.continuation_count += 1;
                                let count = goal.continuation_count;
                                let objective = goal.objective.clone();
                                let continuation_msg = Message {
                                    id: Ulid::new(),
                                    session_id: self.session_id,
                                    role: Role::User,
                                    parts: vec![Part::Text(TextPart {
                                        base: PartBase {
                                            id: Ulid::new(),
                                            message_id: Ulid::new(),
                                            session_id: self.session_id,
                                        },
                                        text: format!(
                                            "{GOAL_CONTINUATION_TAG} count=\"{count}\">\n\
                                             Continue working toward the objective below. \
                                             Avoid repeating work that is already done. \
                                             Choose the next concrete action.\n\n\
                                             Objective: {objective}\n\
                                             </goal_continuation>"
                                        ),
                                        synthetic: true,
                                    })],
                                    time: Time {
                                        created: Utc::now().timestamp_millis(),
                                        completed: None,
                                    },
                                    assistant: None,
                                };
                                self.append_message(continuation_msg).await;
                                true
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if should_continue {
                    // Reset the per-turn counter so the max_turns guard
                    // applies to each continuation independently, not to
                    // the entire goal session. The goal continuation
                    // count is tracked separately on GoalState.
                    turn_count = 0;
                    continue;
                }

                // Leak reminder: the model is about to end its turn with
                // subagent tasks it never collected. Nudge it to collect or
                // explicitly abandon them, capped per user turn so a model
                // that keeps spawning instead of collecting can't loop.
                if self.leak_reminder && self.leak_reminder_count < self.leak_reminder_max {
                    let outstanding = self.list_subagents().await;
                    if !outstanding.is_empty() {
                        self.leak_reminder_count += 1;
                        let mut lines = String::new();
                        for (name, task_id, elapsed_ms, todo_id) in &outstanding {
                            let todo_note = match todo_id {
                                Some(id) => format!(", todo #{}", id),
                                None => String::new(),
                            };
                            lines.push_str(&format!(
                                "- {} ({}, {}s elapsed{})\n",
                                task_id,
                                name,
                                elapsed_ms / 1000,
                                todo_note
                            ));
                        }
                        let reminder_msg = Message {
                            id: Ulid::new(),
                            session_id: self.session_id,
                            role: Role::User,
                            parts: vec![Part::Text(TextPart {
                                base: PartBase {
                                    id: Ulid::new(),
                                    message_id: Ulid::new(),
                                    session_id: self.session_id,
                                },
                                text: format!(
                                    "<subagent_task_reminder>\n\
                                     You started subagent tasks that have not been collected:\n\
                                     {lines}\n\
                                     Collect their results with subagent_wait (pass task_ids, \
                                     or all: true). Note: uncollected tasks keep occupying a \
                                     concurrency slot, and the agent cannot discard them on its \
                                     own.\n\
                                     </subagent_task_reminder>"
                                ),
                                synthetic: true,
                            })],
                            time: Time {
                                created: Utc::now().timestamp_millis(),
                                completed: None,
                            },
                            assistant: None,
                        };
                        self.append_message(reminder_msg).await;
                        turn_count = 0;
                        continue;
                    }
                }
                if finish == Finish::Stop
                    && self.max_turns.is_none_or(|max| turn_count < max)
                    && self
                        .child_guidance
                        .as_ref()
                        .is_some_and(|queue| queue.has_waiting_guide())
                {
                    continuing_for_guidance = true;
                    continue;
                }

                return Ok(());
            }

            let mut result_parts = self
                .execute_pending_tool_calls(&pending, &mut assistant_msg, &ev_tx)
                .await;

            // If the turn was cancelled during tool execution, stop now
            // instead of looping back for another provider call. Make sure the
            // history we leave behind is still valid for the provider: an
            // assistant message with tool_calls must be followed by a tool
            // message for every call_id, so mark any unprocessed tool calls as
            // errored and append a matching result message.
            if self.cancel_token.is_cancelled() {
                if let Some(ref mut msg) = assistant_msg {
                    let now = Utc::now().timestamp_millis();
                    msg.time.completed = Some(now);
                    if let Some(meta) = &mut msg.assistant {
                        meta.error = Some(MessageError {
                            kind: ErrorKind::Aborted,
                            message: "aborted".into(),
                        });
                    }
                    let assistant_id = msg.id;
                    for tc in &pending {
                        let already_has_result = result_parts.iter().any(
                            |p| matches!(p, Part::ToolResult(ref tr) if tr.call_id == tc.call_id),
                        );
                        if !already_has_result {
                            self.update_tool_call(
                                msg,
                                tc.base.id,
                                ToolState::Error(ToolStateError {
                                    input: tc.state.input().clone(),
                                    error: "aborted".into(),
                                    time: ToolTime {
                                        start: now,
                                        end: Some(now),
                                    },
                                }),
                            );
                            result_parts.push(Part::ToolResult(ToolResultPart {
                                base: PartBase {
                                    id: Ulid::new(),
                                    message_id: assistant_id,
                                    session_id: self.session_id,
                                },
                                call_id: tc.call_id.clone(),
                            }));
                        }
                    }
                    // The assistant message was already appended at MessageEnd.
                    // Sync the updated state (aborted meta + terminal tool states)
                    // back into self.messages, then append the result message.
                    self.update_assistant_message(msg).await;
                    let result_msg = Message {
                        id: Ulid::new(),
                        session_id: self.session_id,
                        role: Role::User,
                        parts: result_parts,
                        time: Time {
                            created: now,
                            completed: None,
                        },
                        assistant: None,
                    };
                    self.append_message(result_msg).await;
                }
                let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                return Ok(());
            }

            // Sync updated assistant message (with tool state transitions)
            // back into self.messages so the next request has the correct state.
            if let Some(ref msg) = assistant_msg {
                tracing::debug!(msg_id = %msg.id, "syncing assistant message to store");
                self.update_assistant_message(msg).await;
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
            if std::mem::take(&mut *self.python_exit.lock().await)
                || assistant_msg.as_ref().is_some_and(|message| {
                    message.parts.iter().any(|part| {
                        matches!(part, Part::ToolCall(call) if call.tool_name == "exit_tool"
                        && matches!(call.state, ToolState::Completed(_)))
                    })
                })
            {
                return Ok(());
            }
        }
    }

    /// Record terminal tool state as a revision of the assistant message.
    async fn update_assistant_message(&self, message: &Message) {
        {
            let mut messages = self.messages.lock().await;
            if let Some(existing) = messages
                .iter_mut()
                .find(|existing| existing.id == message.id)
            {
                *existing = message.clone();
            }
        }
        if let Some(session) = &self.session {
            if let Err(error) = session.lock().await.write_message(message).await {
                tracing::error!(%error, "session write failed");
            }
        }
    }

    async fn admit_child_guidance(
        &self,
        request: &mut Request,
        manifest: &mut mew_message::TurnManifest,
    ) -> bool {
        use crate::child_guidance::CHILD_ENDED;

        let Some(guidance) = &self.child_guidance else {
            return false;
        };
        let Some((admission, text)) = guidance.claim() else {
            return false;
        };
        if self.cancel_token.is_cancelled() {
            admission.finish(Err(ChildGuidanceError::Rejected(CHILD_ENDED.into())));
            return false;
        }
        let message = match self.build_prompt_message(text, vec![]) {
            Ok(message) => message,
            Err(error) => {
                admission.finish(Err(ChildGuidanceError::Rejected(error.to_string())));
                return false;
            }
        };
        request
            .messages
            .push(self.dispatcher.on_chat_message(message.clone()).await);
        let guided_manifest = self.request_manifest(request);
        let result = if self.cancel_token.is_cancelled() || guidance.is_closed() {
            Err(ChildGuidanceError::Rejected(CHILD_ENDED.into()))
        } else if self.context_window > 0
            && estimated_request_tokens(request, &guided_manifest) > self.context_window
        {
            Err(ChildGuidanceError::Rejected(
                "Child guidance would exceed its model's context window.".into(),
            ))
        } else {
            self.persist_prompt(&message, &admission.request_id)
                .await
                .map_err(|error| match error {
                    PromptAdmissionError::Persistence(
                        mew_session::SessionError::TranscriptCommitUnknown { .. },
                    ) => ChildGuidanceError::Unknown(format!(
                        "Could not confirm whether the child guidance was saved: {error}"
                    )),
                    _ => ChildGuidanceError::Rejected(format!(
                        "Could not save child guidance: {error}"
                    )),
                })
        };
        let accepted = result.is_ok();
        if accepted {
            self.messages.lock().await.push(message);
            *manifest = guided_manifest;
        } else if let Some(rejected) = request.messages.pop() {
            self.token_count_cache.lock().unwrap().remove(&rejected.id);
        }
        admission.finish(result);
        accepted
    }

    async fn request_system(&self) -> String {
        self.request_system_for_mode(self.uses_code_mode()).await
    }

    async fn request_system_for_mode(&self, code_mode: bool) -> String {
        let mut system = match &self.persona_prompt {
            Some(persona) => format!("{}\n\n{}", persona, self.system),
            None => self.system.clone(),
        };
        if code_mode {
            system.push_str("\n\n## Python tool mode\n\nThe only direct tool is `python`. Call other available tools inside Python using `tools.name(arguments)`, with the schemas in the Python tool description. For example, use `result = tools.read({\"path\": \"file.txt\"}); text(result[\"output\"])` to read a file. Tool calls return dictionaries with `output` and `error`; check errors before using results. Python variables persist between calls. Use `text(value)` or `print` to show results. When Python is isolated, access the workspace and network through the available tools, not Python's direct file or network APIs.");
        }
        self.dispatcher.on_system_prompt(system).await
    }

    async fn prepare_messages(&self) -> Vec<Message> {
        let mut messages = self.messages.lock().await.clone();
        for message in &mut messages {
            *message = self.dispatcher.on_chat_message(message.clone()).await;
        }
        strip_empty_text_parts(&mut messages);
        messages
    }

    async fn fit_request_context(
        &mut self,
        request: &mut Request,
        force: bool,
    ) -> Result<(mew_message::TurnManifest, usize), String> {
        let manifest = self.request_manifest(request);
        let estimated = estimated_request_tokens(request, &manifest);
        let threshold = ((self.context_window as f64 * self.compaction_threshold) as u32)
            .min(self.context_window);
        let over_budget = |estimated| self.context_window > 0 && estimated > self.context_window;
        if !force && (self.context_window == 0 || estimated <= threshold) {
            return Ok((manifest, 0));
        }

        let before = &request.messages;
        let mut start = compaction_start(before, self.keep_turns);
        let latest_user = before.iter().rposition(|message| {
            message.role == Role::User
                && message.parts.iter().any(|part| match part {
                    Part::Text(text) => !text.synthetic && !text.text.trim().is_empty(),
                    Part::File(_) => true,
                    _ => false,
                })
        });
        if let Some(index) = latest_user.filter(|&index| {
            before[index]
                .parts
                .iter()
                .any(|part| matches!(part, Part::File(_)))
        }) {
            start = start.min(index);
        }
        if self
            .goal
            .lock()
            .await
            .as_ref()
            .is_some_and(|goal| goal.status == GoalStatus::Active)
        {
            if let Some(index) = before.iter().rposition(|message| {
                message.role == Role::User && message.parts.iter().any(|part| {
                    matches!(part, Part::Text(text) if text.synthetic && text.text.starts_with(GOAL_CONTINUATION_TAG))
                })
            }) {
                start = start.min(index);
            }
        }
        let dropped = before[..start]
            .iter()
            .filter(|message| !is_compaction_message(message))
            .count();
        if start == 0 || (dropped == 0 && !force) {
            return if over_budget(estimated) {
                Err(context_window_error(self.context_window))
            } else {
                Ok((manifest, 0))
            };
        }

        let summary = self.compaction_summary(&before[..start], latest_user);
        let compacted = self
            .compacted_messages(&before[start..], force, dropped, summary)
            .await;
        let previous = std::mem::replace(&mut request.messages, compacted);
        let mut candidate = self.request_manifest(request);
        let compacted_estimate = estimated_request_tokens(request, &candidate);
        if over_budget(compacted_estimate) || (!force && compacted_estimate >= estimated) {
            request.messages = previous;
            return if over_budget(estimated) {
                Err(context_window_error(self.context_window))
            } else {
                Ok((manifest, 0))
            };
        }

        let system = self.system.clone();
        self.apply_deferred_system_rebuild_after_compaction();
        if self.system != system {
            request.system = self.request_system().await;
            candidate = self.request_manifest(request);
            if self.context_window > 0
                && estimated_request_tokens(request, &candidate) > self.context_window
            {
                return Err(context_window_error(self.context_window));
            }
        }

        self.dispatcher.on_pre_compaction(&previous).await;
        self.commit_compaction(&request.messages).await?;
        self.dispatcher.on_post_compaction(&request.messages).await;
        tracing::info!(dropped, estimated, "compacted older context");
        Ok((candidate, dropped))
    }

    fn request_manifest(&self, request: &Request) -> mew_message::TurnManifest {
        crate::manifest::build_manifest(
            request,
            &self.model_id,
            self.context_window,
            &self.token_count_cache,
        )
    }

    async fn compacted_messages(
        &self,
        tail: &[Message],
        force: bool,
        removed: usize,
        summary: String,
    ) -> Vec<Message> {
        let tail_start_id = tail
            .iter()
            .find(|message| !is_compaction_message(message))
            .map(|message| message.id);
        let compact_id = Ulid::new();
        let compact_msg = Message {
            id: compact_id,
            session_id: self.session_id,
            role: Role::System,
            parts: vec![Part::Compaction(CompactionPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: compact_id,
                    session_id: self.session_id,
                },
                auto: !force,
                overflow: false,
                tail_start_id,
                summary: Some(summary),
                removed_count: Some(removed as u32),
            })],
            time: Time {
                created: Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        };
        let mut messages = vec![compact_msg];
        messages.extend(
            tail.iter()
                .filter(|message| !is_compaction_message(message))
                .cloned(),
        );

        // Re-inject flagged files so they survive compaction.
        let flagged = self.flagged_files.lock().await.clone();
        if !flagged.is_empty() {
            let now = Utc::now().timestamp_millis();
            for f in flagged.iter().rev() {
                let note_text = match f.mode {
                    FlagMode::Included => match tokio::fs::read_to_string(&f.path).await {
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
        messages
    }

    fn compaction_summary(&self, dropped: &[Message], latest_user: Option<usize>) -> String {
        let mut remaining = if self.context_window == 0 {
            2048
        } else {
            (self.context_window / 10).min(2048)
        };
        let mut excerpts = Vec::new();
        for (index, message) in dropped.iter().enumerate().rev() {
            let text_parts: Vec<_> = message
                .parts
                .iter()
                .filter_map(|part| match part {
                    Part::Text(text)
                        if message.role == Role::User
                            && !text.synthetic
                            && !text.text.trim().is_empty() =>
                    {
                        Some(text.text.as_str())
                    }
                    Part::Compaction(compaction) => compaction
                        .summary
                        .as_deref()?
                        .strip_prefix(CARRIED_USER_MESSAGES),
                    _ => None,
                })
                .filter(|text| !text.is_empty())
                .collect();
            if text_parts.is_empty() {
                continue;
            }
            let tokens = text_parts.iter().fold(0u32, |tokens, text| {
                tokens
                    .saturating_add(crate::manifest::count_tokens(text, &self.model_id))
                    .saturating_add(2)
            });
            if tokens <= remaining || Some(index) == latest_user {
                remaining = remaining.saturating_sub(tokens);
                excerpts.push(text_parts.join("\n\n"));
            }
        }

        // A marker's persisted summary contains verbatim user excerpts.
        // The transcript remains the source for older material.
        excerpts.reverse();
        format!("{CARRIED_USER_MESSAGES}{}", excerpts.join("\n\n---\n\n"))
    }

    async fn commit_compaction(&self, messages: &[Message]) -> Result<(), String> {
        if let Some(session) = &self.session {
            let marker = messages
                .iter()
                .find(|message| is_compaction_message(message))
                .expect("compacted history includes a marker");
            session
                .lock()
                .await
                .write_message(marker)
                .await
                .map_err(|error| format!("Could not save context compaction: {error}"))?;
        }
        *self.messages.lock().await = messages.to_vec();
        self.token_count_cache.lock().unwrap().clear();
        Ok(())
    }

    /// Compact the context immediately, regardless of token threshold.
    /// Returns the number of messages removed.
    pub async fn compact_context(&mut self) -> usize {
        let mut request = Request {
            messages: self.prepare_messages().await,
            system: self.request_system().await,
            ..Request::default()
        };
        self.fit_request_context(&mut request, true)
            .await
            .map_or(0, |(_, removed)| removed)
    }
}

fn estimated_request_tokens(request: &Request, manifest: &mew_message::TurnManifest) -> u32 {
    let output = request
        .params
        .as_ref()
        .and_then(|params| params.max_tokens)
        .unwrap_or_default()
        .max(0) as u32;
    // Account for per-message and per-tool framing outside the textual counts.
    let framing = (request.messages.len() + request.tools.len()).saturating_mul(4) as u32;
    manifest
        .segments
        .iter()
        .fold(output.saturating_add(framing), |total, segment| {
            total.saturating_add(segment.tokens)
        })
}

fn context_window_error(window: u32) -> String {
    format!(
        "The request and reserved response are estimated to exceed the {window}-token context window. Shorten the prompt or attachments, reduce the response limit, or switch to a larger model."
    )
}

pub(crate) const CONTEXT_CLEARED_TEXT: &str =
    "Context cleared. The model starts fresh; prior turns are no longer visible to it.";
const GOAL_CONTINUATION_TAG: &str = "<goal_continuation";
const CARRIED_USER_MESSAGES: &str = "Earlier context was compacted. This record contains a limited selection of earlier user messages. The full conversation remains in the transcript.\n\n";

fn render_compaction_summary(messages: &mut [Message]) {
    for message in messages {
        let Some(summary) = message.parts.iter().find_map(|part| match part {
            Part::Compaction(part) => part
                .summary
                .as_ref()
                .filter(|summary| summary.starts_with(CARRIED_USER_MESSAGES)),
            _ => None,
        }) else {
            continue;
        };
        // User excerpts keep their original instruction priority. The
        // transcript stores only the marker, so resumed requests render the
        // same text without exposing an extra chat row.
        message.role = Role::User;
        message.parts.push(Part::Text(TextPart {
            base: PartBase {
                id: message.id,
                message_id: message.id,
                session_id: message.session_id,
            },
            text: summary.clone(),
            synthetic: true,
        }));
    }
}

/// Keep the provider's tool array byte-stable when the registry was built
/// from a map. Providers include tool definitions in their cacheable prompt
/// prefix, so map iteration order must never become cache churn.
fn sort_tool_defs(defs: &mut [ToolDef]) {
    defs.sort_unstable_by(|left, right| left.name.cmp(&right.name));
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

/// Return the first message that should remain visible after compaction.
///
/// The count is a soft lower bound: if it would leave a tool result without
/// the assistant message that issued its call, the boundary moves backward to
/// keep that call/result pair together. Orphaned tool-result-only messages are
/// skipped when older compaction has already removed their call.
pub(crate) fn compaction_start(messages: &[Message], keep_count: usize) -> usize {
    let mut start = messages.len().saturating_sub(keep_count);

    loop {
        if start >= messages.len() {
            return start;
        }

        let mut rewind_to = None;
        let mut orphan_result = false;
        for message in &messages[start..] {
            for part in &message.parts {
                let Part::ToolResult(result) = part else {
                    continue;
                };

                if let Some(call_index) = messages[..start]
                    .iter()
                    .position(|candidate| has_tool_call(candidate, &result.call_id))
                {
                    rewind_to =
                        Some(rewind_to.map_or(call_index, |index: usize| index.min(call_index)));
                } else if !messages[start..]
                    .iter()
                    .any(|candidate| has_tool_call(candidate, &result.call_id))
                    && message
                        .parts
                        .iter()
                        .all(|part| matches!(part, Part::ToolResult(_)))
                {
                    orphan_result = true;
                }
            }
        }

        if let Some(index) = rewind_to {
            if index < start {
                start = index;
                continue;
            }
        }

        if orphan_result {
            start += 1;
            continue;
        }

        return start;
    }
}

fn has_tool_call(message: &Message, call_id: &str) -> bool {
    message
        .parts
        .iter()
        .any(|part| matches!(part, Part::ToolCall(call) if call.call_id == call_id))
}

fn is_compaction_message(message: &Message) -> bool {
    message
        .parts
        .iter()
        .any(|part| matches!(part, Part::Compaction(_)))
}

/// Repair orphaned Pending tool calls in the message history.
///
/// When a session is interrupted mid-turn (crash, kill, daemon restart),
/// the assistant message may carry `ToolCallPart`s in `Pending` state with
/// no corresponding `ToolResultPart` in any subsequent user message. Wire
/// builders skip Pending calls, but this function also transitions them to
/// `Error` state and appends matching `ToolResultPart`s so the session
/// history stays consistent and the model sees what happened.
pub(crate) fn repair_orphaned_tool_calls(
    messages: &mut Vec<Message>,
    session_id: mew_message::SessionId,
) {
    // Collect call_ids that already have a ToolResultPart somewhere in history.
    let mut resolved: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in messages.iter() {
        for part in &msg.parts {
            if let Part::ToolResult(tr) = part {
                resolved.insert(tr.call_id.clone());
            }
        }
    }

    // Find orphaned Pending tool calls and collect their call_ids + assistant
    // message indices so we can patch them in place. Running tool calls are
    // already skipped by the wire builders, so they don't need repair here.
    let now = chrono::Utc::now().timestamp();
    let mut to_repair: Vec<(usize, String)> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role != Role::Assistant {
            continue;
        }
        for part in &msg.parts {
            if let Part::ToolCall(tc) = part {
                if matches!(tc.state, ToolState::Pending(_)) && !resolved.contains(&tc.call_id) {
                    to_repair.push((i, tc.call_id.clone()));
                }
            }
        }
    }

    if to_repair.is_empty() {
        return;
    }

    tracing::info!(
        count = to_repair.len(),
        "repairing orphaned pending tool calls on session load"
    );

    // Transition each orphaned Pending call to Error state.
    for (msg_idx, call_id) in &to_repair {
        let msg = &mut messages[*msg_idx];
        for part in &mut msg.parts {
            if let Part::ToolCall(tc) = part {
                if tc.call_id == *call_id {
                    let input = tc.state.input().clone();
                    tc.state = ToolState::Error(ToolStateError {
                        input,
                        error: "interrupted: session was resumed before this tool call completed"
                            .to_string(),
                        time: ToolTime {
                            start: now,
                            end: Some(now),
                        },
                    });
                }
            }
        }
    }

    // Append a user message with the missing ToolResultParts so the
    // conversation alternation (assistant → user) is maintained and the
    // wire builders find a result for each call_id.
    let result_parts: Vec<Part> = to_repair
        .iter()
        .map(|(_, call_id)| {
            Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
                    session_id,
                },
                call_id: call_id.clone(),
            })
        })
        .collect();

    let result_msg = Message {
        id: Ulid::new(),
        session_id,
        role: Role::User,
        parts: result_parts,
        time: Time {
            created: now,
            completed: None,
        },
        assistant: None,
    };

    // Insert the result message after the last orphaned assistant message.
    let insert_after = to_repair.last().unwrap().0;
    messages.insert(insert_after + 1, result_msg);
}

/// Rebuild the model-visible context from the lossless session history.
///
/// Compaction markers are appended after the messages they replace. The
/// marker's `tail_start_id` identifies the first surviving message, so resume
/// can keep the complete JSONL log while presenting the same compacted prefix
/// that was used before shutdown.
pub(crate) fn context_from_history(mut history: Vec<Message>) -> Vec<Message> {
    if let Some(cleared) = history.iter().rposition(|message| {
        message.role == Role::User
            && matches!(message.parts.as_slice(), [Part::Text(text)]
                if text.synthetic && text.text == CONTEXT_CLEARED_TEXT)
    }) {
        history.drain(..=cleared);
    }
    let Some((marker_index, tail_start_id)) =
        history
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, message)| {
                message.parts.iter().find_map(|part| match part {
                    Part::Compaction(compaction) => Some((index, compaction.tail_start_id)),
                    _ => None,
                })
            })
    else {
        return history;
    };

    let marker = history[marker_index].clone();
    let mut context = vec![marker];

    if let Some(tail_start_id) = tail_start_id {
        if let Some(tail_index) = history
            .iter()
            .position(|message| message.id == tail_start_id)
        {
            if tail_index < marker_index {
                context.extend(
                    history[tail_index..marker_index]
                        .iter()
                        .filter(|message| !is_compaction_message(message))
                        .cloned(),
                );
                context.extend(history[marker_index + 1..].iter().cloned());
                return context;
            }

            context.extend(
                history[tail_index..]
                    .iter()
                    .filter(|message| !is_compaction_message(message))
                    .cloned(),
            );
            return context;
        }
    }

    context.extend(history[marker_index + 1..].iter().cloned());
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{
        CompactionPart, PartBase, TextPart, ToolCallPart, ToolResultPart, ToolState,
        ToolStateCompleted, ToolTime,
    };

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
                images: vec![],
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

    #[test]
    fn compaction_boundary_keeps_tool_call_with_result() {
        let tool_call = make_msg(Role::Assistant, vec![tool_call_part()]);
        let call_id = match &tool_call.parts[0] {
            Part::ToolCall(part) => part.call_id.clone(),
            _ => unreachable!(),
        };
        let tool_result = make_msg(
            Role::User,
            vec![Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                call_id,
            })],
        );
        let messages = vec![
            make_msg(Role::User, vec![text_part("old")]),
            tool_call,
            tool_result,
        ];

        assert_eq!(compaction_start(&messages, 1), 1);
    }

    #[test]
    fn rebuilds_context_from_persisted_compaction_marker() {
        let old = make_msg(Role::User, vec![text_part("old")]);
        let kept = make_msg(Role::User, vec![text_part("kept")]);
        // Newer sessions tag the compaction marker as a synthetic
        // System message (the wire still filters it out regardless of
        // role because the filter checks for the Compaction part, not
        // the role). Older sessions may serialize it as User — the
        // filter handles both, but the old role is no longer produced.
        let marker = make_msg(
            Role::System,
            vec![Part::Compaction(CompactionPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                auto: true,
                overflow: false,
                tail_start_id: Some(kept.id),
                summary: None,
                removed_count: Some(1),
            })],
        );
        let marker_id = marker.id;
        let old_id = old.id;
        let after = make_msg(Role::Assistant, vec![text_part("after")]);

        let context = context_from_history(vec![old, kept.clone(), marker, after.clone()]);

        assert_eq!(context.len(), 3);
        assert!(context.iter().all(|message| message.id != old_id));
        assert_eq!(context[0].id, marker_id);
        assert_eq!(context[1].id, kept.id);
        assert_eq!(context[2].id, after.id);
    }

    #[test]
    fn tool_definitions_are_sorted_by_name() {
        let mut defs = vec![
            ToolDef {
                name: "write".into(),
                description: String::new(),
                schema: serde_json::json!({}),
            },
            ToolDef {
                name: "bash".into(),
                description: String::new(),
                schema: serde_json::json!({}),
            },
            ToolDef {
                name: "read".into(),
                description: String::new(),
                schema: serde_json::json!({}),
            },
        ];

        sort_tool_defs(&mut defs);

        assert_eq!(
            defs.into_iter().map(|def| def.name).collect::<Vec<_>>(),
            vec!["bash", "read", "write"]
        );
    }

    fn pending_tool_call(call_id: &str) -> Part {
        Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            tool_name: "bash".to_string(),
            call_id: call_id.to_string(),
            state: ToolState::Pending(mew_message::ToolStatePending {
                input: serde_json::json!({"command": "ls"}),
                time: ToolTime {
                    start: 0,
                    end: None,
                },
            }),
            raw_input: "{}".to_string(),
        })
    }

    #[test]
    fn repair_orphaned_tool_calls_transitions_pending_to_error() {
        // An assistant message with a Pending tool call and no matching
        // ToolResultPart should be repaired: the call transitions to Error
        // and a user message with the matching ToolResultPart is appended.
        let mut messages = vec![make_msg(
            Role::Assistant,
            vec![text_part("let me check"), pending_tool_call("call_orphan")],
        )];
        let session_id = messages[0].session_id;

        repair_orphaned_tool_calls(&mut messages, session_id);

        // The assistant message now has an Error-state tool call.
        let Part::ToolCall(tc) = &messages[0].parts[1] else {
            panic!("expected tool call");
        };
        let ToolState::Error(ref e) = tc.state else {
            panic!("expected error state, got {:?}", tc.state);
        };
        assert!(e.error.contains("interrupted"));

        // A new user message was appended with the matching ToolResultPart.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, Role::User);
        let Part::ToolResult(tr) = &messages[1].parts[0] else {
            panic!("expected tool result");
        };
        assert_eq!(tr.call_id, "call_orphan");
    }

    #[test]
    fn repair_orphaned_tool_calls_skips_already_resolved() {
        // A Pending tool call that already has a matching ToolResultPart
        // should not be touched (the result is already in the conversation).
        let tool_result = make_msg(
            Role::User,
            vec![Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                call_id: "call_resolved".to_string(),
            })],
        );
        let mut messages = vec![
            make_msg(Role::Assistant, vec![pending_tool_call("call_resolved")]),
            tool_result,
        ];
        let session_id = messages[0].session_id;

        repair_orphaned_tool_calls(&mut messages, session_id);

        // Nothing was added — the call already had a result.
        assert_eq!(messages.len(), 2);
        // The tool call is still Pending (no Error transition).
        let Part::ToolCall(tc) = &messages[0].parts[0] else {
            panic!("expected tool call");
        };
        assert!(matches!(tc.state, ToolState::Pending(_)));
    }

    #[test]
    fn repair_orphaned_tool_calls_collects_multiple_orphans() {
        // Multiple Pending tool calls on the same assistant message should
        // all be repaired in a single user message.
        let messages = make_msg(
            Role::Assistant,
            vec![pending_tool_call("call_a"), pending_tool_call("call_b")],
        );
        let session_id = messages.session_id;
        let mut msgs = vec![messages];

        repair_orphaned_tool_calls(&mut msgs, session_id);

        assert_eq!(msgs.len(), 2);
        let Part::ToolCall(tc_a) = &msgs[0].parts[0] else {
            panic!();
        };
        let Part::ToolCall(tc_b) = &msgs[0].parts[1] else {
            panic!();
        };
        assert!(matches!(tc_a.state, ToolState::Error(_)));
        assert!(matches!(tc_b.state, ToolState::Error(_)));

        // Both results in the appended user message.
        assert_eq!(msgs[1].parts.len(), 2);
        let mut call_ids: Vec<&str> = msgs[1]
            .parts
            .iter()
            .filter_map(|p| {
                if let Part::ToolResult(tr) = p {
                    Some(tr.call_id.as_str())
                } else {
                    None
                }
            })
            .collect();
        call_ids.sort();
        assert_eq!(call_ids, vec!["call_a", "call_b"]);
    }
}
