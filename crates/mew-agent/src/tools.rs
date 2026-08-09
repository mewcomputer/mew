use chrono::Utc;
use tokio::sync::{mpsc, oneshot};

use mew_hooks::{PermissionDecision, ToolCall as HookToolCall, ToolOutput};
use mew_message::{
    Message, Part, PartBase, PartId, ToolCallPart, ToolResultPart, ToolState, ToolStateCompleted,
    ToolStateError, ToolStateRunning, ToolTime,
};
use mew_tools::{Sensitivity, ToolCtx, ToolProgress};

use crate::agent::{format_subagent_result, Agent, ShellJobState, ToolInput};
use crate::{AgentEvent, GoalDecision, GoalState, GoalStatus};
use mew_subagents::SubagentRunOptions;

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

    /// Resolve the effective permission decision for a tool call.
    ///
    /// Runs the full permission pipeline: permission engine (including the
    /// workspace-escape tier and secret-file guard), dispatcher hooks,
    /// Auto/Auto+ classifier, and user modal. Returns the final decision and
    /// an optional human-readable deny reason. Callers must apply
    /// `AllowSession` themselves if they want to cache the grant.
    async fn resolve_permission_decision(
        &self,
        tool_name: &str,
        hook_call: &HookToolCall,
        sensitivity: Sensitivity,
        engine_cwd: &std::path::Path,
        ev_tx: &mpsc::Sender<AgentEvent>,
    ) -> (PermissionDecision, Option<String>) {
        let default_decision = if let Some(ref engine) = self.permission_engine {
            engine
                .check(tool_name, &hook_call.input, sensitivity, engine_cwd)
                .await
        } else {
            match sensitivity {
                Sensitivity::ReadOnly => PermissionDecision::AllowOnce,
                _ => PermissionDecision::Prompt,
            }
        };

        let mut deny_reason: Option<String> = None;
        let hook_outcome = self
            .dispatcher
            .on_permission_ask(hook_call, default_decision)
            .await;
        let decision = match hook_outcome {
            mew_hooks::HookOutcome::Proceed(d) => {
                if d == PermissionDecision::Deny {
                    deny_reason = Some("deny rule or workspace-escape tier".into());
                }
                d
            }
            mew_hooks::HookOutcome::Block(reason) => {
                tracing::info!(
                    tool = %hook_call.tool_name,
                    reason = %reason,
                    "permission hook blocked the call"
                );
                deny_reason = Some(format!("blocked by hook: {reason}"));
                PermissionDecision::Deny
            }
            mew_hooks::HookOutcome::Suppress => {
                deny_reason = Some("suppressed by hook".into());
                PermissionDecision::Deny
            }
        };

        let decision = if decision == PermissionDecision::Prompt
            && matches!(
                self.permission_mode(),
                mew_hooks::PermissionMode::Auto | mew_hooks::PermissionMode::AutoPlus
            ) {
            match self.classify_permission(hook_call).await {
                Some(mew_prompts::classifier::ClassifierDecision::Allow) => {
                    PermissionDecision::AllowOnce
                }
                Some(mew_prompts::classifier::ClassifierDecision::Deny) => {
                    deny_reason = Some("classifier denied".into());
                    PermissionDecision::Deny
                }
                Some(mew_prompts::classifier::ClassifierDecision::Escalate) => {
                    match self.permission_mode() {
                        mew_hooks::PermissionMode::AutoPlus => {
                            deny_reason = Some("classifier escalated (Auto+ fail-closed)".into());
                            PermissionDecision::Deny
                        }
                        _ => PermissionDecision::Prompt, // Auto → user modal
                    }
                }
                None => match self.permission_mode() {
                    mew_hooks::PermissionMode::AutoPlus => {
                        deny_reason = Some("classifier unavailable (Auto+ fail-closed)".into());
                        PermissionDecision::Deny
                    }
                    _ => PermissionDecision::Prompt, // Auto → user modal
                },
            }
        } else {
            decision
        };

        let decision = if decision == PermissionDecision::Prompt {
            let (perm_tx, perm_rx) = oneshot::channel();
            let _ = ev_tx
                .send(AgentEvent::PermissionRequest {
                    call: hook_call.clone(),
                    tx: perm_tx,
                })
                .await;
            match perm_rx.await {
                Ok(d) => {
                    if d == PermissionDecision::Deny {
                        deny_reason = Some("user denied".into());
                    }
                    d
                }
                Err(_) => {
                    deny_reason = Some("permission request channel closed".into());
                    PermissionDecision::Deny
                }
            }
        } else {
            decision
        };

        (decision, deny_reason)
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

            if tc.tool_name == "handoff_plan" {
                self.execute_handoff_plan(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if tc.tool_name == "propose_goal" {
                self.execute_propose_goal(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if tc.tool_name == "complete_goal" {
                self.execute_complete_goal(tc, assistant_msg, ev_tx, &mut result_parts)
                    .await;
                continue;
            }

            if tc.tool_name == "block_goal" {
                self.execute_block_goal(tc, assistant_msg, ev_tx, &mut result_parts)
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

            // Permission check. The escape tier inside the engine reads
            // the cwd to resolve relative path args. The agent layer's
            // `ToolCtx` is constructed a few lines below with
            // `std::env::current_dir()` as its cwd — we mirror that here
            // so the engine sees the same working directory the tool
            // itself will see.
            let sensitivity = self
                .tools
                .get(&tc.tool_name)
                .map(|t| t.sensitivity())
                .unwrap_or(Sensitivity::Dangerous);
            let engine_cwd =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (decision, deny_reason) = self
                .resolve_permission_decision(
                    &tc.tool_name,
                    &hook_call,
                    sensitivity,
                    &engine_cwd,
                    ev_tx,
                )
                .await;

            if decision == PermissionDecision::AllowSession {
                if let Some(ref engine) = self.permission_engine {
                    engine.add_session_allow(&tc.tool_name).await;
                }
            }

            if decision == PermissionDecision::Deny {
                let perm_error = match &deny_reason {
                    Some(reason) => format!("permission denied: {reason}"),
                    None => "permission denied".to_string(),
                };
                let error_state = ToolState::Error(ToolStateError {
                    input: hook_call.input.clone(),
                    error: perm_error,
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
            // Run the tool-execute-before hook. The hook can proceed with the
            // (possibly modified) input, force-block with a reason, or
            // silently suppress. Block/Suppress skip the tool invocation
            // entirely and produce a tool error result.
            let input = match self
                .dispatcher
                .on_tool_execute_before(&hook_call, input)
                .await
            {
                mew_hooks::HookOutcome::Proceed(v) => v,
                mew_hooks::HookOutcome::Block(reason) => {
                    tracing::info!(
                        tool = %hook_call.tool_name,
                        reason = %reason,
                        "tool-execute-before hook blocked the call"
                    );
                    let error_state = ToolState::Error(ToolStateError {
                        input: hook_call.input.clone(),
                        error: format!("blocked by hook: {reason}"),
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
                mew_hooks::HookOutcome::Suppress => {
                    tracing::debug!(
                        tool = %hook_call.tool_name,
                        "tool-execute-before hook suppressed the call"
                    );
                    // Suppress behaves like Block but at debug level — the
                    // model still needs to see a result, so produce an error
                    // state with a generic message.
                    let error_state = ToolState::Error(ToolStateError {
                        input: hook_call.input.clone(),
                        error: "tool call suppressed".into(),
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
            };

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

            let ctx = ToolCtx::new(
                std::sync::Arc::new(mew_tools::ToolCtxShared {
                    session_id: self.session_id,
                    cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                    dispatcher: Some(self.dispatcher.clone()),
                    secrets: self.secrets.clone(),
                    shell_session: self.shell_session.clone(),
                    snapshot_store: self.snapshot_store.clone(),
                    browser_enabled: self.browser_enabled,
                }),
                tc.call_id.clone(),
                self.cancel_token.child_token(),
                progress_tx,
            );

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
                        metadata: None,
                        file_delta: None,
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
                        metadata: output.metadata.clone(),
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
            // If the tool produced a file delta, emit it so the daemon can
            // accumulate per-session change stats.
            if let Some(delta) = &output.file_delta {
                let _ = ev_tx
                    .send(AgentEvent::FileDelta {
                        path: delta.path.clone(),
                        added: delta.added,
                        removed: delta.removed,
                    })
                    .await;
            }
            // If the flag_important tool ran, emit the current flagged-files set.
            if tc.tool_name == "flag_important" {
                let files: Vec<crate::FlaggedFileInfo> = self
                    .flagged_files
                    .lock()
                    .await
                    .iter()
                    .map(|f| crate::FlaggedFileInfo {
                        path: f.path.display().to_string(),
                        reason: Some(
                            mew_tools::tools::flag_important::flag_mode_label(f.mode).to_string(),
                        ),
                    })
                    .collect();
                let _ = ev_tx.send(AgentEvent::FlaggedFilesChanged { files }).await;
            }
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
        let model = input.get("model").and_then(|v| v.as_str());

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
        let model = model.map(|s| s.to_string());
        let call_id_clone = call_id.clone();
        let ev_tx_clone = ev_tx.clone();
        let cancel = self.cancel_token.child_token();
        let dispatcher = self.dispatcher.clone();

        let (sa_event_tx, mut sa_event_rx) = mpsc::channel(256);

        let parent_session_id = self.session_id;
        let runner_handle = tokio::spawn(async move {
            runner
                .run(SubagentRunOptions {
                    def: &def,
                    prompt,
                    parent_call_id: call_id_clone.clone(),
                    parent_session_id,
                    event_tx: sa_event_tx,
                    cancel,
                    model,
                })
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
                    manifests,
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
                                    manifests,
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
                            manifests,
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

        let (result, child_manifests) = match runner_handle.await {
            Ok(Ok(mew_subagents::SubagentResult::Complete {
                text,
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                session_unavailable,
                manifests,
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
                (out, manifests)
            }
            Ok(Ok(mew_subagents::SubagentResult::Cancelled)) => {
                tracing::info!(subagent = %name, "subagent cancelled");
                (
                    format!("subagent '{}' was cancelled before completion", name),
                    vec![],
                )
            }
            Ok(Ok(mew_subagents::SubagentResult::Error { reason })) => {
                tracing::warn!(subagent = %name, error = %reason, "subagent failed");
                (format!("subagent '{}' failed: {}", name, reason), vec![])
            }
            Ok(Err(e)) => {
                tracing::warn!(subagent = %name, error = %e, "subagent failed");
                (format!("subagent '{}' failed: {}", name, e), vec![])
            }
            Err(e) => {
                tracing::warn!(subagent = %name, error = %e, "subagent task panicked");
                (format!("subagent '{}' panicked: {}", name, e), vec![])
            }
        };

        let success = is_subagent_success(&result);
        let metadata = if !child_manifests.is_empty() {
            Some(serde_json::to_value(&child_manifests).expect("manifests serialize"))
        } else {
            None
        };
        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output: result,
                metadata,
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
        let model = input.get("model").and_then(|v| v.as_str());

        // Default (async=false): block until the subagent finishes, return the
        // result inline. This is the common case — the model doesn't have to
        // remember to call subagent_wait.
        // Async mode (async=true): return a task_id; the model calls
        // subagent_wait later. Use this for parallelism.
        let start_result = self.start_subagent(name, prompt, model, ev_tx).await;
        let (output, success, child_manifests) = match start_result {
            Ok(task_id) if async_mode => (task_id, true, vec![]),
            Ok(task_id) => {
                let wait_result = self.wait_subagent(&task_id).await;
                format_subagent_result(wait_result)
            }
            Err(e) => (format!("error: {}", e), false, vec![]),
        };

        let metadata = if !child_manifests.is_empty() {
            Some(serde_json::to_value(&child_manifests).expect("manifests serialize"))
        } else {
            None
        };
        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output,
                metadata,
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

        // Parse questions. Each must have a prompt and a 2-4 element options
        // array. The prompts and option labels are kept here for result
        // formatting; the full AskUserQuestion structs move into the event.
        let parsed: Vec<(String, Vec<String>)> = input
            .get("questions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|q| {
                        let prompt = q.get("prompt").and_then(|v| v.as_str())?.to_string();
                        let labels: Vec<String> = q
                            .get("options")
                            .and_then(|v| v.as_array())
                            .map(|opts| {
                                opts.iter()
                                    .filter_map(|o| {
                                        o.get("label").and_then(|v| v.as_str()).map(String::from)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        Some((prompt, labels))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let (output, success) = if parsed.is_empty() {
            ("ask_user_question received no questions".to_string(), false)
        } else {
            let questions: Vec<crate::AskUserQuestion> = parsed
                .iter()
                .map(|(prompt, labels)| crate::AskUserQuestion {
                    prompt: prompt.clone(),
                    options: labels
                        .iter()
                        .map(|l| crate::QuestionOption {
                            label: l.clone(),
                            description: String::new(),
                        })
                        .collect(),
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
                    for (i, (prompt, labels)) in parsed.iter().enumerate() {
                        let answer = answers.get(i).map(|s| s.as_str()).unwrap_or("(no answer)");
                        let picked = match answer {
                            a if labels.iter().any(|l| l == a) => "selected",
                            "(no answer)" => "no answer",
                            _ => "freeform",
                        };
                        if !text.is_empty() {
                            text.push_str("\n\n");
                        }
                        text.push_str(&format!("Q: {}\nA ({}): {}", prompt, picked, answer));
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

    /// Intercept a `handoff_plan` tool call: read the configured plan file,
    /// present it to the user for approval (blocking the tool), and on approval
    /// queue the target persona switch through `pending_persona_switch` so the
    /// end-of-turn drain emits `PersonaSwitchRequested`. On a change request the
    /// user's feedback becomes a successful tool result so the planner can
    /// revise and resubmit.
    async fn execute_handoff_plan(
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

        let persona = input
            .get("persona")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("builder")
            .to_string();

        let (output, success) = 'result: {
            // Validate the target persona exists.
            if !self.personas.iter().any(|p| p.name == persona) {
                let available: Vec<&str> = self.personas.iter().map(|p| p.name.as_str()).collect();
                break 'result (
                    format!(
                        "unknown persona '{persona}'. Available personas: {}",
                        if available.is_empty() {
                            "(none)".to_string()
                        } else {
                            available.join(", ")
                        }
                    ),
                    false,
                );
            }

            // Resolve and read the plan file.
            let plan_path = match self.resolved_plan_path() {
                Some(p) => p,
                None => break 'result ("no plan_path configured".to_string(), false),
            };
            let plan_markdown = match tokio::fs::read_to_string(&plan_path).await {
                Ok(text) if !text.trim().is_empty() => text,
                _ => {
                    break 'result (
                        format!(
                            "plan file {} is missing or empty — write it with write_plan first",
                            plan_path.display()
                        ),
                        false,
                    )
                }
            };

            let (tx, rx) = oneshot::channel();
            let _ = ev_tx
                .send(AgentEvent::PlanApprovalRequest {
                    call_id: call_id.clone(),
                    plan_path: plan_path.display().to_string(),
                    plan_markdown,
                    persona: persona.clone(),
                    tx,
                })
                .await;

            match rx.await {
                Ok(crate::PlanDecision::Approved) => {
                    *self.pending_persona_switch.lock().await = Some(persona.clone());
                    (
                        format!(
                            "Plan approved. Queued switch to '{persona}' at end of turn — \
                             wrap up now."
                        ),
                        true,
                    )
                }
                Ok(crate::PlanDecision::ChangesRequested(feedback)) => (
                    format!(
                        "The user requested changes to the plan:\n\n{feedback}\n\n\
                         Revise with edit_plan and call handoff_plan again."
                    ),
                    true,
                ),
                Err(_) => (
                    "handoff_plan cancelled (no response received)".to_string(),
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

    /// Intercept a `propose_goal` tool call: send the objective to the
    /// frontend as a `GoalProposed` event and block until the user accepts
    /// or rejects. On acceptance the goal becomes active. On rejection the
    /// tool returns an error so the agent knows the user declined.
    async fn execute_propose_goal(
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

        let objective = input
            .get("objective")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let (output, success) = if objective.is_empty() {
            (
                "propose_goal requires an \"objective\" field".to_string(),
                false,
            )
        } else {
            let (tx, rx) = oneshot::channel();
            let _ = ev_tx
                .send(AgentEvent::GoalProposed {
                    call_id: call_id.clone(),
                    objective: objective.clone(),
                    tx,
                })
                .await;

            match rx.await {
                Ok(GoalDecision::Accepted) => {
                    let now = chrono::Utc::now().timestamp_millis();
                    *self.goal.lock().await = Some(GoalState {
                        objective: objective.clone(),
                        status: GoalStatus::Active,
                        continuation_count: 0,
                        started_at: now,
                    });
                    (
                        format!(
                            "Goal accepted and activated. The agent will continue working \
                             across turns until the goal is complete. Call complete_goal \
                             when done, or block_goal if user input is needed.\n\n\
                             Objective: {objective}"
                        ),
                        true,
                    )
                }
                Ok(GoalDecision::Rejected) => ("Goal rejected by user.".to_string(), false),
                Err(_) => (
                    "propose_goal cancelled (no response received)".to_string(),
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

    /// Intercept a `complete_goal` tool call: mark the active goal as
    /// complete, stopping turn-loop continuation.
    async fn execute_complete_goal(
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

        let (output, success) = {
            let mut goal_guard = self.goal.lock().await;
            match &mut *goal_guard {
                Some(goal) => {
                    let reason = input
                        .get("terminal_reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("completed")
                        .to_string();
                    let objective = goal.objective.clone();
                    goal.status = GoalStatus::Complete;
                    (
                        format!("Goal marked complete: {objective}\nReason: {reason}"),
                        true,
                    )
                }
                None => ("No active goal to complete.".to_string(), false),
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

    /// Intercept a `block_goal` tool call: pause the active goal, stopping
    /// turn-loop continuation without clearing the goal.
    async fn execute_block_goal(
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

        let (output, success) = {
            let mut goal_guard = self.goal.lock().await;
            match &mut *goal_guard {
                Some(goal) => {
                    let reason = input
                        .get("terminal_reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("blocked")
                        .to_string();
                    let objective = goal.objective.clone();
                    goal.status = GoalStatus::Paused;
                    (
                        format!("Goal blocked: {objective}\nReason: {reason}\n\nThe user can resume with /goal resume."),
                        true,
                    )
                }
                None => ("No active goal to block.".to_string(), false),
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
                let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
                if command.is_empty() {
                    ("shell_background: 'command' is required".into(), false)
                } else {
                    let cwd_str = input.get("cwd").and_then(|v| v.as_str());
                    let cwd = cwd_str
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let hook_call = mew_hooks::ToolCall {
                        tool_name: tc.tool_name.clone(),
                        call_id: tc.call_id.clone(),
                        input: input.clone(),
                    };
                    let sensitivity = self
                        .tools
                        .get(&tc.tool_name)
                        .map(|t| t.sensitivity())
                        .unwrap_or(Sensitivity::Dangerous);
                    let (decision, deny_reason) = self
                        .resolve_permission_decision(
                            &tc.tool_name,
                            &hook_call,
                            sensitivity,
                            &cwd,
                            ev_tx,
                        )
                        .await;
                    if decision == PermissionDecision::AllowSession {
                        if let Some(ref engine) = self.permission_engine {
                            engine.add_session_allow(&tc.tool_name).await;
                        }
                    }
                    if decision == PermissionDecision::Deny {
                        let reason = deny_reason.unwrap_or_else(|| "permission denied".into());
                        (format!("permission denied: {reason}"), false)
                    } else {
                        let job_id = self.start_shell_job(command, &cwd).await;
                        let _ = ev_tx
                            .send(AgentEvent::JobUpdate {
                                job_id: job_id.clone(),
                                command: command.to_string(),
                                state: "running".into(),
                            })
                            .await;
                        (format!("started job: {}", job_id), true)
                    }
                }
            }
            "job_status" => {
                let job_id = input.get("job_id").and_then(|v| v.as_str()).unwrap_or("");
                match self.shell_job_status(job_id).await {
                    Some((state, output)) => {
                        let _ = ev_tx
                            .send(AgentEvent::JobUpdate {
                                job_id: job_id.to_string(),
                                command: String::new(),
                                state: state.as_str().into(),
                            })
                            .await;
                        let out = format!("state: {}\n\n{}", state.as_str(), output);
                        (out, true)
                    }
                    None => (format!("job '{}' not found", job_id), false),
                }
            }
            "job_block" => {
                let job_id = input.get("job_id").and_then(|v| v.as_str()).unwrap_or("");
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
                let job_id = input.get("job_id").and_then(|v| v.as_str()).unwrap_or("");
                if self.cancel_shell_job(job_id).await {
                    let _ = ev_tx
                        .send(AgentEvent::JobUpdate {
                            job_id: job_id.to_string(),
                            command: String::new(),
                            state: "cancelled".into(),
                        })
                        .await;
                    (format!("cancelled job: {}", job_id), true)
                } else {
                    (
                        format!("job '{}' not found or already finished", job_id),
                        false,
                    )
                }
            }
            "shell_monitor" => {
                let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let timeout = input
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60);
                if command.is_empty() {
                    ("shell_monitor: 'command' is required".into(), false)
                } else {
                    let cwd_str = input.get("cwd").and_then(|v| v.as_str());
                    let cwd = cwd_str
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let hook_call = mew_hooks::ToolCall {
                        tool_name: tc.tool_name.clone(),
                        call_id: tc.call_id.clone(),
                        input: input.clone(),
                    };
                    let sensitivity = self
                        .tools
                        .get(&tc.tool_name)
                        .map(|t| t.sensitivity())
                        .unwrap_or(Sensitivity::Dangerous);
                    let (decision, deny_reason) = self
                        .resolve_permission_decision(
                            &tc.tool_name,
                            &hook_call,
                            sensitivity,
                            &cwd,
                            ev_tx,
                        )
                        .await;
                    if decision == PermissionDecision::AllowSession {
                        if let Some(ref engine) = self.permission_engine {
                            engine.add_session_allow(&tc.tool_name).await;
                        }
                    }
                    if decision == PermissionDecision::Deny {
                        let reason = deny_reason.unwrap_or_else(|| "permission denied".into());
                        (format!("permission denied: {reason}"), false)
                    } else {
                        let job_id = self.start_shell_job(command, &cwd).await;
                        let result = self.shell_job_block(&job_id, timeout).await;
                        let _ = self.cancel_shell_job(&job_id).await;
                        match result {
                            Some((ShellJobState::Completed { exit_code: 0 }, output)) => {
                                let last = output
                                    .lines()
                                    .rev()
                                    .take(20)
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .rev()
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                (format!("ready (exit 0)\n\n{}", last), true)
                            }
                            Some((state, output)) => {
                                let last = output
                                    .lines()
                                    .rev()
                                    .take(20)
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .rev()
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                (
                                    format!(
                                        "not ready after {}s: {}\n\n{}",
                                        timeout,
                                        state.as_str(),
                                        last
                                    ),
                                    false,
                                )
                            }
                            None => (format!("shell_monitor: job '{}' not found", job_id), false),
                        }
                    }
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

        let single = input.get("task_id").and_then(|v| v.as_str());
        let batch = input.get("task_ids").and_then(|v| v.as_array());
        let all = input.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        let modes = single.is_some() as u8 + batch.is_some() as u8 + all as u8;
        let (output, success, child_manifests) = if modes != 1 {
            (
                "subagent_wait requires exactly one of \"task_id\", \"task_ids\", or \"all\""
                    .to_string(),
                false,
                vec![],
            )
        } else if let Some(task_id) = single {
            let result = self.wait_subagent(task_id).await;
            format_subagent_result(result)
        } else {
            // Batch mode: wait each task, collect results keyed by task_id.
            // A failed task does not fail the batch; its per-task status
            // carries the failure.
            let ids: Vec<String> = if all {
                self.subagent_task_ids().await
            } else {
                batch
                    .expect("batch checked above")
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            };
            if ids.is_empty() {
                ("no outstanding subagent tasks".to_string(), true, vec![])
            } else {
                (self.wait_subagents_batch(ids).await, true, vec![])
            }
        };

        let metadata = if !child_manifests.is_empty() {
            Some(serde_json::to_value(&child_manifests).expect("manifests serialize"))
        } else {
            None
        };
        let final_state = if success {
            ToolState::Completed(ToolStateCompleted {
                input: input.clone(),
                output,
                metadata,
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
