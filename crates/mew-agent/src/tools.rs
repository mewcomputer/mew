use chrono::Utc;
use tokio::sync::{mpsc, oneshot};

use mew_hooks::{PermissionDecision, ToolCall as HookToolCall, ToolOutput};
use mew_message::{
    Message, Part, PartBase, PartId, ToolCallPart, ToolResultPart, ToolState, ToolStateCompleted,
    ToolStateError, ToolStateRunning, ToolTime,
};
use mew_tools::{Sensitivity, ToolCtx, ToolProgress};

use crate::agent::{Agent, ToolInput};
use crate::AgentEvent;

// Track whether the subagent tool returned an error, not whether the
// output text contains specific substrings.
fn is_subagent_success(result: &str) -> bool {
    !result.starts_with("subagent '")
}

impl Agent {
    pub(crate) fn pending_tool_calls(&self, msg: &Message) -> Vec<ToolCallPart> {
        msg.parts
            .iter()
            .filter_map(|p| {
                if let Part::ToolCall(tc) = p {
                    match &tc.state {
                        ToolState::Pending(_) | ToolState::Running(_) => Some(tc.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    pub(crate) fn update_tool_call(&self, msg: &mut Message, part_id: PartId, state: ToolState) {
        for part in &mut msg.parts {
            if let Part::ToolCall(ref mut tc) = part {
                if tc.base.id == part_id {
                    tc.state = state;
                    return;
                }
            }
        }
    }

    pub(crate) async fn execute_pending_tool_calls(
        &self,
        pending: &[ToolCallPart],
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
    ) -> Vec<Part> {
        let mut result_parts: Vec<Part> = Vec::with_capacity(pending.len());

        // Capture the assistant message id for result parts; if None
        // (shouldn't happen once the stream guard in turn_loop passes),
        // return early with no result parts rather than panicking.
        let assistant_id = match assistant_msg {
            Some(ref msg) => msg.id,
            None => {
                tracing::error!("execute_pending_tool_calls called with no assistant message");
                return result_parts;
            }
        };
        for tc in pending {
            let call_id = tc.call_id.clone();
            let part_id = tc.base.id;

            if tc.tool_name == "subagent" && self.subagent_runner.is_some() {
                self.execute_subagent_call(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if tc.tool_name == "subagent_start" {
                self.execute_subagent_start(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if tc.tool_name == "subagent_wait" {
                self.execute_subagent_wait(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if tc.tool_name == "ask_user_question" {
                self.execute_ask_user(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if matches!(
                tc.tool_name.as_str(),
                "shell_background" | "job_status" | "job_block" | "job_cancel"
            ) {
                self.execute_job_tool(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if matches!(
                tc.tool_name.as_str(),
                "todo_create" | "todo_update" | "todo_complete" | "todo_delete" | "todo_list"
            ) {
                self.execute_todo(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            // Mark as running.
            let running_state = ToolState::Running(ToolStateRunning {
                input: tc.input().clone(),
                output: String::new(),
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: None,
                },
            });

            if let Some(ref mut msg) = assistant_msg {
                self.update_tool_call(msg, part_id, running_state.clone());
            }
            let _ = ev_tx
                .send(AgentEvent::PartUpdated {
                    part_id,
                    part: Part::ToolCall(ToolCallPart {
                        base: tc.base.clone(),
                        tool_name: tc.tool_name.clone(),
                        call_id: tc.call_id.clone(),
                        state: running_state,
                        raw_input: tc.raw_input.clone(),
                    }),
                })
                .await;
            let _ = ev_tx
                .send(AgentEvent::ToolStart {
                    call_id: call_id.clone(),
                })
                .await;

            let hook_call = HookToolCall {
                tool_name: tc.tool_name.clone(),
                call_id: tc.call_id.clone(),
                input: tc.input().clone(),
            };

            // Permission check.
            let sensitivity = self
                .tools
                .get(&tc.tool_name)
                .map(|t| t.sensitivity())
                .unwrap_or(Sensitivity::Dangerous);
            let default_decision = if let Some(ref engine) = self.permission_engine {
                engine
                    .check(&tc.tool_name, &hook_call.input, sensitivity)
                    .await
            } else {
                match sensitivity {
                    Sensitivity::ReadOnly => PermissionDecision::AllowOnce,
                    _ => PermissionDecision::Prompt,
                }
            };
            let decision = self
                .dispatcher
                .on_permission_ask(&hook_call, default_decision)
                .await;

            let decision = if decision == PermissionDecision::Prompt {
                let (perm_tx, perm_rx) = oneshot::channel();
                let _ = ev_tx
                    .send(AgentEvent::PermissionRequest {
                        call: hook_call.clone(),
                        tx: perm_tx,
                    })
                    .await;
                match perm_rx.await {
                    Ok(d) => d,
                    Err(_) => PermissionDecision::Deny,
                }
            } else {
                decision
            };

            if decision == PermissionDecision::AllowSession {
                if let Some(ref engine) = self.permission_engine {
                    engine.add_session_allow(&tc.tool_name).await;
                }
            }

            if decision == PermissionDecision::Deny {
                let error_state = ToolState::Error(ToolStateError {
                    input: hook_call.input.clone(),
                    error: "permission denied".into(),
                    time: ToolTime {
                        start: Utc::now().timestamp_millis(),
                        end: Some(Utc::now().timestamp_millis()),
                    },
                });
                if let Some(ref mut msg) = assistant_msg {
                    self.update_tool_call(msg, part_id, error_state.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::PartUpdated {
                        part_id,
                        part: Part::ToolCall(ToolCallPart {
                            base: tc.base.clone(),
                            tool_name: tc.tool_name.clone(),
                            call_id: tc.call_id.clone(),
                            state: error_state,
                            raw_input: tc.raw_input.clone(),
                        }),
                    })
                    .await;
                let _ = ev_tx
                    .send(AgentEvent::ToolEnd {
                        call_id: call_id.clone(),
                        success: false,
                    })
                    .await;
                result_parts.push(Part::ToolResult(ToolResultPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: assistant_id,
                        session_id: self.session_id,
                    },
                    call_id: tc.call_id.clone(),
                }));
                continue;
            }

            let tool = match self.tools.get(&tc.tool_name) {
                Some(t) => t,
                None => {
                    let error_state = ToolState::Error(ToolStateError {
                        input: hook_call.input.clone(),
                        error: format!("unknown tool {:?}", tc.tool_name),
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    });
                    if let Some(ref mut msg) = assistant_msg {
                        self.update_tool_call(msg, part_id, error_state.clone());
                    }
                    let _ = ev_tx
                        .send(AgentEvent::PartUpdated {
                            part_id,
                            part: Part::ToolCall(ToolCallPart {
                                base: tc.base.clone(),
                                tool_name: tc.tool_name.clone(),
                                call_id: tc.call_id.clone(),
                                state: error_state,
                                raw_input: tc.raw_input.clone(),
                            }),
                        })
                        .await;
                    let _ = ev_tx
                        .send(AgentEvent::ToolEnd {
                            call_id: call_id.clone(),
                            success: false,
                        })
                        .await;
                    self.dispatcher
                        .on_tool_error(&hook_call, "unknown tool")
                        .await;
                    result_parts.push(Part::ToolResult(ToolResultPart {
                        base: PartBase {
                            id: ulid::Ulid::new(),
                            message_id: assistant_id,
                            session_id: self.session_id,
                        },
                        call_id: tc.call_id.clone(),
                    }));
                    continue;
                }
            };

            let input = if hook_call.input.is_null() && !tc.raw_input.is_empty() {
                serde_json::from_str(&tc.raw_input).unwrap_or_else(|_| hook_call.input.clone())
            } else {
                hook_call.input.clone()
            };
            let input = self
                .dispatcher
                .on_tool_execute_before(&hook_call, input)
                .await;

            let (progress_tx, mut progress_rx) = mpsc::channel::<ToolProgress>(16);
            // Forward progress chunks to the TUI with 50ms debounce.
            let ev_tx2 = ev_tx.clone();
            let call_id2 = call_id.clone();
            tokio::spawn(async move {
                let mut buf = String::new();
                let mut tick = tokio::time::interval(tokio::time::Duration::from_millis(50));
                // Skip immediate first tick so first chunk isn't delayed.
                tick.tick().await;
                loop {
                    tokio::select! {
                        progress = progress_rx.recv() => {
                            match progress {
                                Some(ToolProgress::OutputChunk(chunk)) => {
                                    buf.push_str(&chunk);
                                }
                                Some(ToolProgress::Metadata(_)) => {}
                                None => {
                                    // Channel closed; flush remaining.
                                    if !buf.is_empty() {
                                        let _ = ev_tx2
                                            .send(AgentEvent::ToolProgress {
                                                call_id: call_id2.clone(),
                                                chunk: std::mem::take(&mut buf),
                                            })
                                            .await;
                                    }
                                    break;
                                }
                            }
                        }
                        _ = tick.tick() => {
                            if !buf.is_empty() {
                                let _ = ev_tx2
                                    .send(AgentEvent::ToolProgress {
                                        call_id: call_id2.clone(),
                                        chunk: std::mem::take(&mut buf),
                                    })
                                    .await;
                            }
                        }
                    }
                }
            });

            let ctx = ToolCtx {
                session_id: self.session_id,
                call_id: tc.call_id.clone(),
                cancel: self.cancel_token.child_token(),
                progress_tx,
                cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                dispatcher: Some(self.dispatcher.clone()),
                secrets: self.secrets.clone(),
            };

            // Workspace sandbox check for path-based tools (skip bash/echo).
            let tool_cwd = ctx.cwd.clone();
            if let Some(arg_path) = self.workspace_path_for_tool(&tc.tool_name, &input) {
                let resolved = tool_cwd.join(&arg_path);
                if let Err(msg) = self.ensure_workspace_path(&resolved, ev_tx).await {
                    let error_msg = msg.clone();
                    let error_state = ToolState::Error(ToolStateError {
                        input: input.clone(),
                        error: msg,
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    });
                    if let Some(ref mut msg) = assistant_msg {
                        self.update_tool_call(msg, part_id, error_state.clone());
                    }
                    let _ = ev_tx
                        .send(AgentEvent::PartUpdated {
                            part_id,
                            part: Part::ToolCall(ToolCallPart {
                                base: tc.base.clone(),
                                tool_name: tc.tool_name.clone(),
                                call_id: tc.call_id.clone(),
                                state: error_state,
                                raw_input: tc.raw_input.clone(),
                            }),
                        })
                        .await;
                    let _ = ev_tx
                        .send(AgentEvent::ToolEnd {
                            call_id: call_id.clone(),
                            success: false,
                        })
                        .await;
                    self.dispatcher.on_tool_error(&hook_call, &error_msg).await;
                    result_parts.push(Part::ToolResult(ToolResultPart {
                        base: PartBase {
                            id: ulid::Ulid::new(),
                            message_id: assistant_id,
                            session_id: self.session_id,
                        },
                        call_id: tc.call_id.clone(),
                    }));
                    continue;
                }
            }

            tracing::info!(tool = %tc.tool_name, call_id = %call_id, input = %input, "executing tool");
            let exec_result = tool.execute(ctx, input.clone()).await;
            let tool_output = match exec_result {
                Ok(out) => {
                    tracing::info!(tool = %tc.tool_name, call_id = %call_id, output = %out.output, error = %out.error, "tool executed successfully");
                    out
                }
                Err(e) => {
                    tracing::warn!(tool = %tc.tool_name, call_id = %call_id, error = %e, "tool execution failed");
                    ToolOutput {
                        output: String::new(),
                        error: e.to_string(),
                        diff: None,
                    }
                }
            };

            // Update hook_call with parsed input so hooks and final state are correct.
            let hook_call = HookToolCall {
                tool_name: hook_call.tool_name,
                call_id: hook_call.call_id,
                input: input.clone(),
            };

            let output = self
                .dispatcher
                .on_tool_execute_after(&hook_call, tool_output)
                .await;

            tracing::info!(tool = %tc.tool_name, call_id = %call_id, success = %output.error.is_empty(), "tool finished");
            let (success, final_state) = if !output.error.is_empty() {
                (
                    false,
                    ToolState::Error(ToolStateError {
                        input: input.clone(),
                        error: output.error.clone(),
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    }),
                )
            } else {
                (
                    true,
                    ToolState::Completed(ToolStateCompleted {
                        input: input.clone(),
                        output: output.output.clone(),
                        metadata: None,
                        diff: output.diff.clone(),
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    }),
                )
            };

            if let Some(ref mut msg) = assistant_msg {
                self.update_tool_call(msg, part_id, final_state.clone());
            }
            let _ = ev_tx
                .send(AgentEvent::PartUpdated {
                    part_id,
                    part: Part::ToolCall(ToolCallPart {
                        base: tc.base.clone(),
                        tool_name: tc.tool_name.clone(),
                        call_id: tc.call_id.clone(),
                        state: final_state,
                        raw_input: tc.raw_input.clone(),
                    }),
                })
                .await;
            let _ = ev_tx
                .send(AgentEvent::ToolEnd {
                    call_id: call_id.clone(),
                    success,
                })
                .await;
            result_parts.push(Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: assistant_id,
                    session_id: self.session_id,
                },
                call_id: tc.call_id.clone(),
            }));
        }
        result_parts
    }

    async fn execute_subagent_call(
        &self,
        tc: &ToolCallPart,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
        result_parts: &mut Vec<Part>,
    ) {
        let call_id = tc.call_id.clone();
        let part_id = tc.base.id;

        let assistant_id = match assistant_msg {
            Some(ref msg) => msg.id,
            None => {
                tracing::error!("execute_subagent_call called with no assistant message");
                return;
            }
        };

        let input = tc.input().clone();
        let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let prompt = input.get("prompt").and_then(|v| v.as_str()).unwrap_or("");

        let def = match self.subagent_defs.iter().find(|d| d.name == name) {
            Some(d) => d,
            None => {
                let error_state = ToolState::Error(ToolStateError {
                    input: input.clone(),
                    error: format!("unknown subagent: {name}"),
                    time: ToolTime {
                        start: Utc::now().timestamp_millis(),
                        end: Some(Utc::now().timestamp_millis()),
                    },
                });
                if let Some(ref mut msg) = assistant_msg {
                    self.update_tool_call(msg, part_id, error_state.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::PartUpdated {
                        part_id,
                        part: Part::ToolCall(ToolCallPart {
                            base: tc.base.clone(),
                            tool_name: tc.tool_name.clone(),
                            call_id: tc.call_id.clone(),
                            state: error_state,
                            raw_input: tc.raw_input.clone(),
                        }),
                    })
                    .await;
                let _ = ev_tx
                    .send(AgentEvent::ToolEnd {
                        call_id: call_id.clone(),
                        success: false,
                    })
                    .await;
                result_parts.push(Part::ToolResult(ToolResultPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: assistant_id,
                        session_id: self.session_id,
                    },
                    call_id: tc.call_id.clone(),
                }));
                return;
            }
        };

        let running_state = ToolState::Running(ToolStateRunning {
            input: input.clone(),
            output: String::new(),
            time: ToolTime {
                start: Utc::now().timestamp_millis(),
                end: None,
            },
        });
        if let Some(ref mut msg) = assistant_msg {
            self.update_tool_call(msg, part_id, running_state.clone());
        }
        let _ = ev_tx
            .send(AgentEvent::PartUpdated {
                part_id,
                part: Part::ToolCall(ToolCallPart {
                    base: tc.base.clone(),
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    state: running_state,
                    raw_input: tc.raw_input.clone(),
                }),
            })
            .await;
        let _ = ev_tx
            .send(AgentEvent::ToolStart {
                call_id: call_id.clone(),
            })
            .await;

        let runner = match self.subagent_runner.as_ref() {
            Some(r) => r.clone(),
            None => {
                tracing::error!("subagent_runner missing despite earlier is_some check");
                return;
            }
        };
        let def = def.clone();
        let def_name = def.name.clone();
        let prompt = prompt.to_string();
        let call_id_clone = call_id.clone();
        let ev_tx_clone = ev_tx.clone();
        let cancel = self.cancel_token.child_token();
        let dispatcher = self.dispatcher.clone();

        let (sa_event_tx, mut sa_event_rx) = mpsc::channel(256);

        let parent_session_id = self.session_id;
        let runner_handle = tokio::spawn(async move {
            runner
                .run(
                    &def,
                    prompt,
                    call_id_clone.clone(),
                    parent_session_id,
                    sa_event_tx,
                    cancel,
                )
                .await
        });

        while let Some(event) = sa_event_rx.recv().await {
            match event {
                mew_subagents::SubagentEvent::Started {
                    child_session_id,
                    display_name,
                } => {
                    dispatcher
                        .on_subagent_start(&def_name, &call_id, display_name.as_deref())
                        .await;
                    let _ = ev_tx_clone
                        .send(AgentEvent::SubagentStart {
                            parent_call_id: call_id.clone(),
                            name: def_name.clone(),
                            child_session_id,
                            display_name,
                        })
                        .await;
                }
                mew_subagents::SubagentEvent::Finished {
                    child_session_id,
                    outcome,
                } => {
                    let outcome_str = match &outcome {
                        mew_subagents::SubagentOutcome::Completed => "completed",
                        mew_subagents::SubagentOutcome::Failed { reason } => {
                            // Need to own the string for the hook call.
                            // We call the hook before sending the event,
                            // while we still have the outcome.
                            let r = reason.clone();
                            let r_one_line = r.lines().next().unwrap_or("unknown").to_string();
                            let _ = dispatcher
                                .on_subagent_end(
                                    &def_name,
                                    &call_id,
                                    &format!("failed: {}", r_one_line),
                                )
                                .await;
                            let _ = ev_tx_clone
                                .send(AgentEvent::SubagentEnd {
                                    parent_call_id: call_id.clone(),
                                    child_session_id,
                                    outcome,
                                })
                                .await;
                            continue;
                        }
                        mew_subagents::SubagentOutcome::Cancelled => "cancelled",
                    };
                    let _ = dispatcher
                        .on_subagent_end(&def_name, &call_id, outcome_str)
                        .await;
                    let _ = ev_tx_clone
                        .send(AgentEvent::SubagentEnd {
                            parent_call_id: call_id.clone(),
                            child_session_id,
                            outcome,
                        })
                        .await;
                }
                mew_subagents::SubagentEvent::TextDelta { text } => {
                    let _ = ev_tx_clone
                        .send(AgentEvent::SubagentProgress {
                            parent_call_id: call_id.clone(),
                            child_event: Box::new(AgentEvent::Provider(
                                mew_provider::ProviderEvent::PartDelta {
                                    part_id: ulid::Ulid::new(),
                                    field: "text",
                                    delta: text,
                                },
                            )),
                        })
                        .await;
                }
                mew_subagents::SubagentEvent::ToolStart {
                    call_id: tool_call_id,
                    tool_name: _,
                } => {
                    let _ = ev_tx_clone
                        .send(AgentEvent::SubagentProgress {
                            parent_call_id: call_id.clone(),
                            child_event: Box::new(AgentEvent::ToolStart {
                                call_id: tool_call_id,
                            }),
                        })
                        .await;
                }
                mew_subagents::SubagentEvent::ToolEnd {
                    call_id: tool_call_id,
                    success,
                } => {
                    let _ = ev_tx_clone
                        .send(AgentEvent::SubagentProgress {
                            parent_call_id: call_id.clone(),
                            child_event: Box::new(AgentEvent::ToolEnd {
                                call_id: tool_call_id,
                                success,
                            }),
                        })
                        .await;
                }
                mew_subagents::SubagentEvent::Progress {
                    tool_name: progress_tool_name,
                    message,
                    ..
                } => {
                    let _ = ev_tx_clone
                        .send(AgentEvent::SubagentStatus {
                            parent_call_id: call_id.clone(),
                            tool_name: progress_tool_name,
                            message,
                        })
                        .await;
                }
                mew_subagents::SubagentEvent::PermissionRequest {
                    tool_name: req_tool_name,
                    call_id: req_call_id,
                    input: req_input,
                    tx,
                } => {
                    let hook_call = mew_hooks::ToolCall {
                        tool_name: req_tool_name,
                        call_id: req_call_id,
                        input: req_input,
                    };
                    let _ = ev_tx_clone
                        .send(AgentEvent::SubagentPermissionRequest {
                            parent_call_id: call_id.clone(),
                            call: hook_call,
                            tx,
                        })
                        .await;
                }
            }
        }

        let result = match runner_handle.await {
            Ok(Ok(mew_subagents::SubagentResult::Complete {
                text,
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                session_unavailable,
            })) => {
                tracing::info!(subagent = %name, output_len = text.len(), turns_used, hit_turn_limit, hit_time_limit, session_unavailable, "subagent completed");
                let mut out = text.trim_end_matches('\n').to_string();
                if hit_turn_limit {
                    out.insert_str(
                        0,
                        &format!(
                            "warning: subagent hit max_turns limit ({} turns); result may be incomplete\n\n",
                            turns_used
                        ),
                    );
                }
                if hit_time_limit {
                    out.insert_str(
                        0,
                        "warning: subagent hit max_duration limit; result may be incomplete\n\n",
                    );
                }
                if session_unavailable {
                    out.insert_str(
                        0,
                        "warning: subagent transcript could not be written; result is unrecorded\n\n",
                    );
                }
                out
            }
            Ok(Ok(mew_subagents::SubagentResult::Cancelled)) => {
                tracing::info!(subagent = %name, "subagent cancelled");
                format!("subagent '{}' was cancelled before completion", name)
            }
            Ok(Ok(mew_subagents::SubagentResult::Error { reason })) => {
                tracing::warn!(subagent = %name, error = %reason, "subagent failed");
                format!("subagent '{}' failed: {}", name, reason)
            }
            Ok(Err(e)) => {
                tracing::warn!(subagent = %name, error = %e, "subagent failed");
                format!("subagent '{}' failed: {}", name, e)
            }
            Err(e) => {
                tracing::warn!(subagent = %name, error = %e, "subagent task panicked");
                format!("subagent '{}' panicked: {}", name, e)
            }
        };

        let success = is_subagent_success(&result);
        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output: result,
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        } else {
            ToolState::Error(ToolStateError {
                input: input.clone(),
                error: result,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        };

        if let Some(ref mut msg) = assistant_msg {
            self.update_tool_call(msg, part_id, final_state.clone());
        }
        let _ = ev_tx
            .send(AgentEvent::PartUpdated {
                part_id,
                part: Part::ToolCall(ToolCallPart {
                    base: tc.base.clone(),
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    state: final_state,
                    raw_input: tc.raw_input.clone(),
                }),
            })
            .await;
        let _ = ev_tx
            .send(AgentEvent::ToolEnd {
                call_id: call_id.clone(),
                success,
            })
            .await;

        result_parts.push(Part::ToolResult(ToolResultPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: assistant_id,
                session_id: self.session_id,
            },
            call_id: tc.call_id.clone(),
        }));
    }

    async fn execute_subagent_start(
        &self,
        tc: &ToolCallPart,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
        result_parts: &mut Vec<Part>,
    ) {
        let call_id = tc.call_id.clone();
        let part_id = tc.base.id;
        let input = tc.input().clone();

        let assistant_id = match assistant_msg {
            Some(ref msg) => msg.id,
            None => return,
        };

        let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let prompt = input.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        let async_mode = input
            .get("async")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Default (async=false): block until the subagent finishes, return the
        // result inline. This is the common case — the model doesn't have to
        // remember to call subagent_wait.
        // Async mode (async=true): return a task_id; the model calls
        // subagent_wait later. Use this for parallelism.
        let start_result = self.start_subagent(name, prompt, ev_tx).await;
        let (output, success) = match start_result {
            Ok(task_id) if async_mode => (task_id, true),
            Ok(task_id) => {
                let wait_result = self.wait_subagent(&task_id).await;
                match wait_result {
                    Ok(mew_subagents::SubagentResult::Complete {
                        text,
                        turns_used,
                        hit_turn_limit,
                        hit_time_limit,
                        session_unavailable,
                    }) => {
                        let mut out = text.trim_end_matches('\n').to_string();
                        if hit_turn_limit {
                            out.insert_str(
                                0,
                                &format!(
                                    "warning: subagent hit max_turns limit ({} turns); result may be incomplete\n\n",
                                    turns_used
                                ),
                            );
                        }
                        if hit_time_limit {
                            out.insert_str(
                                0,
                                "warning: subagent hit max_duration limit; result may be incomplete\n\n",
                            );
                        }
                        if session_unavailable {
                            out.insert_str(
                                0,
                                "warning: subagent transcript could not be written; result is unrecorded\n\n",
                            );
                        }
                        (out, true)
                    }
                    Ok(mew_subagents::SubagentResult::Cancelled) => (
                        "subagent was cancelled before completion".to_string(),
                        false,
                    ),
                    Ok(mew_subagents::SubagentResult::Error { reason }) => {
                        (format!("subagent failed: {}", reason), false)
                    }
                    Err(e) => (format!("error: {}", e), false),
                }
            }
            Err(e) => (format!("error: {}", e), false),
        };

        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output,
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        } else {
            ToolState::Error(ToolStateError {
                input: input.clone(),
                error: output,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        };

        if let Some(ref mut msg) = assistant_msg {
            self.update_tool_call(msg, part_id, final_state.clone());
        }
        let _ = ev_tx
            .send(AgentEvent::PartUpdated {
                part_id,
                part: Part::ToolCall(ToolCallPart {
                    base: tc.base.clone(),
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    state: final_state,
                    raw_input: tc.raw_input.clone(),
                }),
            })
            .await;
        let _ = ev_tx
            .send(AgentEvent::ToolEnd {
                call_id: call_id.clone(),
                success,
            })
            .await;

        result_parts.push(Part::ToolResult(ToolResultPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: assistant_id,
                session_id: self.session_id,
            },
            call_id: tc.call_id.clone(),
        }));
    }

    async fn execute_ask_user(
        &self,
        tc: &ToolCallPart,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
        result_parts: &mut Vec<Part>,
    ) {
        let call_id = tc.call_id.clone();
        let part_id = tc.base.id;
        let input = tc.input().clone();

        let assistant_id = match assistant_msg {
            Some(ref msg) => msg.id,
            None => return,
        };

        // Parse questions. The prompts stay here for result formatting; the
        // AskUserQuestion structs move into the event.
        let parsed: Vec<String> = input
            .get("questions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|q| q.get("prompt").and_then(|v| v.as_str()).map(String::from))
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let (output, success) = if parsed.is_empty() {
            ("ask_user_question received no questions".to_string(), false)
        } else {
            let questions: Vec<crate::AskUserQuestion> = parsed
                .iter()
                .map(|prompt| crate::AskUserQuestion {
                    prompt: prompt.clone(),
                    default: None,
                })
                .collect();
            let (tx, rx) = oneshot::channel();
            let _ = ev_tx
                .send(AgentEvent::AskUser {
                    call_id: call_id.clone(),
                    questions,
                    tx,
                })
                .await;
            match rx.await {
                Ok(answers) => {
                    let mut text = String::new();
                    for (i, prompt) in parsed.iter().enumerate() {
                        let answer = answers.get(i).map(|s| s.as_str()).unwrap_or("(no answer)");
                        if !text.is_empty() {
                            text.push_str("\n\n");
                        }
                        text.push_str(&format!("Q: {}\nA: {}", prompt, answer));
                    }
                    (text, true)
                }
                Err(_) => (
                    "ask_user_question cancelled (no response received)".to_string(),
                    false,
                ),
            }
        };

        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output,
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        } else {
            ToolState::Error(ToolStateError {
                input: input.clone(),
                error: output,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        };

        if let Some(ref mut msg) = assistant_msg {
            self.update_tool_call(msg, part_id, final_state.clone());
        }
        let _ = ev_tx
            .send(AgentEvent::PartUpdated {
                part_id,
                part: Part::ToolCall(ToolCallPart {
                    base: tc.base.clone(),
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    state: final_state,
                    raw_input: tc.raw_input.clone(),
                }),
            })
            .await;
        let _ = ev_tx
            .send(AgentEvent::ToolEnd {
                call_id: call_id.clone(),
                success,
            })
            .await;

        result_parts.push(Part::ToolResult(ToolResultPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: assistant_id,
                session_id: self.session_id,
            },
            call_id: tc.call_id.clone(),
        }));
    }

    async fn execute_job_tool(
        &self,
        tc: &ToolCallPart,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
        result_parts: &mut Vec<Part>,
    ) {
        let call_id = tc.call_id.clone();
        let part_id = tc.base.id;
        let input = tc.input().clone();

        let assistant_id = match assistant_msg {
            Some(ref msg) => msg.id,
            None => return,
        };

        let (output, success) = match tc.tool_name.as_str() {
            "shell_background" => {
                let command = input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if command.is_empty() {
                    ("shell_background: 'command' is required".into(), false)
                } else {
                    let cwd_str = input.get("cwd").and_then(|v| v.as_str());
                    let cwd = cwd_str
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let job_id = self.start_shell_job(command, &cwd).await;
                    (format!("started job: {}", job_id), true)
                }
            }
            "job_status" => {
                let job_id = input
                    .get("job_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match self.shell_job_status(job_id).await {
                    Some((state, output)) => {
                        let out = format!("state: {}\n\n{}", state.as_str(), output);
                        (out, true)
                    }
                    None => (format!("job '{}' not found", job_id), false),
                }
            }
            "job_block" => {
                let job_id = input
                    .get("job_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let timeout = input
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(300);
                match self.shell_job_block(job_id, timeout).await {
                    Some((state, output)) => {
                        let out = format!("state: {}\n\n{}", state.as_str(), output);
                        (out, state.as_str() != "running" || state.is_terminal())
                    }
                    None => (format!("job '{}' not found", job_id), false),
                }
            }
            "job_cancel" => {
                let job_id = input
                    .get("job_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if self.cancel_shell_job(job_id).await {
                    (format!("cancelled job: {}", job_id), true)
                } else {
                    (
                        format!("job '{}' not found or already finished", job_id),
                        false,
                    )
                }
            }
            _ => (format!("unknown job tool: {}", tc.tool_name), false),
        };

        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output: output.clone(),
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        } else {
            ToolState::Error(ToolStateError {
                input: input.clone(),
                error: output.clone(),
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        };
        let error_for_hook = if success { String::new() } else { output };

        if let Some(ref mut msg) = assistant_msg {
            self.update_tool_call(msg, part_id, final_state.clone());
        }
        let _ = ev_tx
            .send(AgentEvent::PartUpdated {
                part_id,
                part: Part::ToolCall(ToolCallPart {
                    base: tc.base.clone(),
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    state: final_state,
                    raw_input: tc.raw_input.clone(),
                }),
            })
            .await;
        let _ = ev_tx
            .send(AgentEvent::ToolEnd {
                call_id: call_id.clone(),
                success,
            })
            .await;
        self.dispatcher
            .on_tool_error(
                &mew_hooks::ToolCall {
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    input: input.clone(),
                },
                if success { "" } else { &error_for_hook },
            )
            .await;
        result_parts.push(Part::ToolResult(ToolResultPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: assistant_id,
                session_id: self.session_id,
            },
            call_id: tc.call_id.clone(),
        }));
    }

    async fn execute_todo(
        &self,
        tc: &ToolCallPart,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
        result_parts: &mut Vec<Part>,
    ) {
        let call_id = tc.call_id.clone();
        let part_id = tc.base.id;
        let input = tc.input().clone();

        let assistant_id = match assistant_msg {
            Some(ref msg) => msg.id,
            None => return,
        };

        // Mutate under the lock, then render + snapshot for persistence.
        let (output, success, snapshot) = {
            let mut list = self.todos.lock().await;
            let op = apply_todo_op(&tc.tool_name, &input, &mut list);
            let snapshot = list.clone();
            let (output, success) = match op {
                Ok(note) => {
                    let mut text = if note.is_empty() {
                        String::new()
                    } else {
                        format!("{}\n", note)
                    };
                    text.push_str(&snapshot.render());
                    (text, true)
                }
                Err(e) => (e, false),
            };
            (output, success, snapshot)
        };

        // Persist on success only — failures don't change state.
        if success {
            if let Some(path) = &self.todos_path {
                if let Err(e) = snapshot.save(path).await {
                    tracing::warn!(error = %e, "failed to persist todos");
                }
            }
            // Push the new snapshot to the TUI so the sidebar pane updates.
            let _ = ev_tx
                .send(AgentEvent::TodosUpdated {
                    todos: snapshot.items.clone(),
                })
                .await;
        }

        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input,
                output,
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        } else {
            ToolState::Error(ToolStateError {
                input,
                error: output,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        };

        if let Some(ref mut msg) = assistant_msg {
            self.update_tool_call(msg, part_id, final_state.clone());
        }
        let _ = ev_tx
            .send(AgentEvent::PartUpdated {
                part_id,
                part: Part::ToolCall(ToolCallPart {
                    base: tc.base.clone(),
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    state: final_state,
                    raw_input: tc.raw_input.clone(),
                }),
            })
            .await;
        let _ = ev_tx.send(AgentEvent::ToolEnd { call_id, success }).await;

        result_parts.push(Part::ToolResult(ToolResultPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: assistant_id,
                session_id: self.session_id,
            },
            call_id: tc.call_id.clone(),
        }));
    }

    async fn execute_subagent_wait(
        &self,
        tc: &ToolCallPart,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
        result_parts: &mut Vec<Part>,
    ) {
        let call_id = tc.call_id.clone();
        let part_id = tc.base.id;
        let input = tc.input().clone();

        let assistant_id = match assistant_msg {
            Some(ref msg) => msg.id,
            None => return,
        };

        let task_id = input.get("task_id").and_then(|v| v.as_str()).unwrap_or("");

        let result = self.wait_subagent(task_id).await;

        let (output, success) = match result {
            Ok(mew_subagents::SubagentResult::Complete {
                text,
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                session_unavailable,
            }) => {
                let mut out = text.trim_end_matches('\n').to_string();
                if hit_turn_limit {
                    out.insert_str(
                        0,
                        &format!(
                            "warning: subagent hit max_turns limit ({} turns); result may be incomplete\n\n",
                            turns_used
                        ),
                    );
                }
                if hit_time_limit {
                    out.insert_str(
                        0,
                        "warning: subagent hit max_duration limit; result may be incomplete\n\n",
                    );
                }
                if session_unavailable {
                    out.insert_str(
                        0,
                        "warning: subagent transcript could not be written; result is unrecorded\n\n",
                    );
                }
                (out, true)
            }
            Ok(mew_subagents::SubagentResult::Cancelled) => (
                "subagent was cancelled before completion".to_string(),
                false,
            ),
            Ok(mew_subagents::SubagentResult::Error { reason }) => {
                (format!("subagent failed: {}", reason), false)
            }
            Err(e) => (format!("error: {}", e), false),
        };

        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output,
                metadata: None,
                diff: None,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        } else {
            ToolState::Error(ToolStateError {
                input: input.clone(),
                error: output,
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: Some(Utc::now().timestamp_millis()),
                },
            })
        };

        if let Some(ref mut msg) = assistant_msg {
            self.update_tool_call(msg, part_id, final_state.clone());
        }
        let _ = ev_tx
            .send(AgentEvent::PartUpdated {
                part_id,
                part: Part::ToolCall(ToolCallPart {
                    base: tc.base.clone(),
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    state: final_state,
                    raw_input: tc.raw_input.clone(),
                }),
            })
            .await;
        let _ = ev_tx
            .send(AgentEvent::ToolEnd {
                call_id: call_id.clone(),
                success,
            })
            .await;

        result_parts.push(Part::ToolResult(ToolResultPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: assistant_id,
                session_id: self.session_id,
            },
            call_id: tc.call_id.clone(),
        }));
    }
}

/// Apply a `todo_*` tool operation against a list. Pure (no agent state, no
/// I/O): parses the JSON input, dispatches to the matching `TodoList` method,
/// returns a short human-readable note on success or an error message. Kept as
/// a free function so it unit-tests without an `Agent`.
pub(crate) fn apply_todo_op(
    tool_name: &str,
    input: &serde_json::Value,
    list: &mut crate::TodoList,
) -> Result<String, String> {
    match tool_name {
        "todo_create" => {
            let arr = input
                .get("todos")
                .and_then(|v| v.as_array())
                .ok_or_else(|| "missing or non-array `todos`".to_string())?;
            if arr.is_empty() {
                return Err("`todos` must not be empty".to_string());
            }
            let items: Vec<(String, Vec<usize>)> = arr
                .iter()
                .filter_map(|t| {
                    let content = t.get("content")?.as_str()?.to_string();
                    let depends_on = t
                        .get("depends_on")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_u64().map(|n| n as usize))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some((content, depends_on))
                })
                .collect();
            if items.is_empty() {
                return Err("no valid items (each needs a `content` string)".to_string());
            }
            let (created, dropped) = list.create(items);
            let ids: Vec<String> = created.iter().map(|t| format!("#{}", t.id)).collect();
            let total_dropped: usize = dropped.iter().map(|d| d.len()).sum();
            let mut note = format!("created {}", ids.join(", "));
            if total_dropped > 0 {
                note.push_str(&format!(
                    " ({} dependency reference(s) dropped — no such todo)",
                    total_dropped
                ));
            }
            Ok(note)
        }
        "todo_complete" => {
            let id = parse_todo_id(input)?;
            list.complete(id)?;
            Ok(format!("completed #{}", id))
        }
        "todo_delete" => {
            let id = parse_todo_id(input)?;
            let removed = list.delete(id)?;
            Ok(format!("deleted #{} ({})", id, removed.content))
        }
        "todo_update" => {
            let id = parse_todo_id(input)?;
            let content = input
                .get("content")
                .and_then(|v| v.as_str())
                .map(String::from);
            let status = input
                .get("status")
                .and_then(|v| v.as_str())
                .and_then(crate::TodoStatus::parse);
            if content.is_none() && status.is_none() {
                return Err("nothing to update: pass `content` and/or `status`".to_string());
            }
            list.update(id, content, status)?;
            Ok(format!("updated #{}", id))
        }
        "todo_list" => Ok(String::new()),
        other => Err(format!("unknown todo tool: {}", other)),
    }
}

fn parse_todo_id(input: &serde_json::Value) -> Result<usize, String> {
    input
        .get("id")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .ok_or_else(|| "missing or non-integer `id`".to_string())
}
