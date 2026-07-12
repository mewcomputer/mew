//! Non-interactive run command implementations extracted from `main.rs`.
//!
//! These functions handle running a single prompt without a TUI: resolving
//! the permission mode from CLI flags (`resolve_mode`), the `run_cmd` entry
//! point, and the core `build_and_run` agent execution loop.

use anyhow::{Context, Result};
use std::io::Write;
use std::sync::Arc;
use tracing::info;

use mew_catalog::Catalog;
use mew_config::Config;
use mew_hooks::NopDispatcher;
use mew_message::{Finish, Part, PartId};
use mew_session::Writer as SessionWriter;

use crate::setup::agent::{build_session_agent, wire_subagents};
use crate::setup::providers::{load_catalog, resolve_model, resolve_reasoning};

/// Resolve the initial permission mode from CLI flags. Precedence:
/// `-D` (Dangerous) > `--auto-plus` (Auto+) > `-A` (Auto) >
/// `-P` (Permissive) > Standard. Dangerous and the Auto family both
/// bypass all gates but Dangerous is the stronger "trust me" signal.
pub(crate) fn resolve_mode(
    permissive: bool,
    auto: bool,
    auto_plus: bool,
    dangerous: bool,
) -> mew_hooks::PermissionMode {
    if dangerous {
        mew_hooks::PermissionMode::Dangerous
    } else if auto_plus {
        mew_hooks::PermissionMode::AutoPlus
    } else if auto {
        mew_hooks::PermissionMode::Auto
    } else if permissive {
        mew_hooks::PermissionMode::Permissive
    } else {
        mew_hooks::PermissionMode::Standard
    }
}

pub(crate) async fn run_cmd(
    provider_flag: String,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let prompt = prompt_parts.join(" ");
    if prompt.is_empty() {
        anyhow::bail!("missing prompt");
    }

    let cfg = mew_config::load().context("load config")?;

    let cat = load_catalog(&cfg).await;

    build_and_run(
        &cfg,
        cat.as_ref(),
        &provider_flag,
        model_flag,
        variant_flag,
        raw,
        mode,
        prompt,
    )
    .await
}

/// Resolve @mentions in `text`. Text files are inlined into the model-facing
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_and_run(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
    prompt: String,
) -> Result<()> {
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let session_id = ulid::Ulid::new().to_string();
    let session_writer = SessionWriter::open(&session_id)
        .await
        .context("open session")?;
    let todos_path = session_writer.path().parent().map(|p| p.join("todos.json"));

    let dispatcher = Arc::new(NopDispatcher);

    // Discover manifest-based extension packages (shared with build_session_agent
    // for [provides] paths). The run command uses NopDispatcher, so manifest
    // extensions are not spawned here — only their declarative [provides]
    // directories are collected.
    let cwd = std::env::current_dir().unwrap_or_default();
    let discovered = mew_ext_broker::discover_extensions(&cwd);

    // Build the full session agent (provider, tools, personas, skills,
    // subagents, context files, pricing, etc.) via the shared builder.
    let mut agent = build_session_agent(
        cfg,
        cat,
        &provider_id,
        &model_id,
        raw,
        mode,
        Some(session_writer),
        None,
        dispatcher.clone(),
        todos_path.clone(),
        &discovered,
    )?;

    // Register plugin-discovered tools (no-op for NopDispatcher).
    agent.register_plugin_tools().await;

    // Refresh subagent wiring now that plugin tools are registered.
    wire_subagents(
        &mut agent,
        cfg,
        cat,
        &provider_id,
        raw,
        dispatcher.clone(),
        &discovered,
    );

    // Load the saved todo list (if any) into the agent.
    if let Some(ref tp) = todos_path {
        if let Ok(list) = mew_agent::TodoList::load(tp).await {
            *agent.todos.lock().await = list;
        }
    }

    // Apply reasoning variant from catalog or CLI flag.
    let reasoning = resolve_reasoning(cat, &model_id, variant_flag.as_deref());
    if let Some(r) = reasoning {
        agent.set_reasoning(Some(r));
        info!(variant = ?variant_flag, model = %model_id, "enabled thinking variant");
    }

    let mut rx = agent.run(prompt);

    let mut part_types: std::collections::HashMap<PartId, &'static str> =
        std::collections::HashMap::new();

    while let Some(event) = rx.recv().await {
        match event {
            mew_agent::AgentEvent::Provider(ev) => match ev {
                mew_provider::ProviderEvent::PartStart { part } => {
                    let id = part.id();
                    match &part {
                        Part::Text(_) => {
                            part_types.insert(id, "text");
                        }
                        Part::Reasoning(_) => {
                            part_types.insert(id, "reasoning");
                            eprintln!("\n[thinking]");
                        }
                        Part::ToolCall(_) => {
                            part_types.insert(id, "tool");
                        }
                        _ => {}
                    }
                }
                mew_provider::ProviderEvent::PartDelta {
                    part_id,
                    field: _,
                    delta,
                } => match part_types.get(&part_id) {
                    Some(&"reasoning") => {
                        eprint!("{}", delta);
                        let _ = std::io::stderr().flush();
                    }
                    Some(&"text") => {
                        print!("{}", delta);
                        let _ = std::io::stdout().flush();
                    }
                    Some(&"tool") => {}
                    _ => {}
                },
                mew_provider::ProviderEvent::PartEnd { part_id } => {
                    match part_types.get(&part_id) {
                        Some(&"reasoning") => eprintln!("\n[/thinking]"),
                        Some(&"tool") => eprintln!(),
                        _ => {}
                    }
                    part_types.remove(&part_id);
                }
                mew_provider::ProviderEvent::MessageEnd { finish, .. } => {
                    if finish == Finish::Stop {
                        println!();
                    }
                }
                _ => {}
            },
            mew_agent::AgentEvent::PermissionRequest { call, tx } => {
                eprintln!("\n[permission] {}: {:?}", call.tool_name, call.input);
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            mew_agent::AgentEvent::ToolStart { call_id } => {
                eprintln!("\n[tool start: {}]", call_id);
            }
            mew_agent::AgentEvent::ToolEnd { call_id, success } => {
                eprintln!("[tool end: {}] success={}", call_id, success);
            }
            mew_agent::AgentEvent::PartUpdated { part_id: _, part } => {
                if let Part::ToolCall(tc) = &part {
                    if let mew_message::ToolState::Completed(c) = &tc.state {
                        if let Some(ref diff) = c.diff {
                            eprintln!("[diff] {}", diff);
                        }
                    }
                }
            }
            mew_agent::AgentEvent::ToolProgress { .. } => {}
            mew_agent::AgentEvent::Error(msg) => {
                anyhow::bail!("agent error: {}", msg);
            }
            mew_agent::AgentEvent::SubagentStart {
                name: _,
                child_session_id: _,
                ..
            } => {}
            mew_agent::AgentEvent::SubagentProgress { .. } => {}
            mew_agent::AgentEvent::SubagentStatus { .. } => {}
            mew_agent::AgentEvent::SubagentEnd {
                child_session_id: _,
                ..
            } => {}
            mew_agent::AgentEvent::SubagentPermissionRequest { call, tx, .. } => {
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
                let _ = call;
            }
            mew_agent::AgentEvent::WorkspacePermissionRequest { tx, .. } => {
                // Non-interactive mode: auto-allow workspace access.
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            mew_agent::AgentEvent::AskUser { tx, .. } => {
                // Non-interactive mode: no TUI to answer. Dropping `tx`
                // cancels the call so the model gets a clear "cancelled"
                // result instead of hanging.
                eprintln!("\n[ask_user_question: cancelled — no TUI in non-interactive mode]");
                drop(tx);
            }
            mew_agent::AgentEvent::PlanApprovalRequest { tx, .. } => {
                // Non-interactive mode: no TUI to approve the plan. Dropping
                // `tx` cancels the call so the model gets a clear "cancelled"
                // result instead of hanging.
                eprintln!("\n[handoff_plan: cancelled — no TUI in non-interactive mode]");
                drop(tx);
            }
            mew_agent::AgentEvent::TodosUpdated { .. } => {
                // No sidebar in non-interactive mode; nothing to update.
            }
            mew_agent::AgentEvent::PersonaSwitchRequested { .. } => {
                // Non-interactive mode: no TUI to confirm. The tool layer
                // already gates switch_persona via the permission engine,
                // and the switch is harmless on its own (no model pin
                // means just a system prompt + tool allowlist change), so
                // we silently drop the apply.
            }
            mew_agent::AgentEvent::JobUpdate { .. } => {
                // No sidebar in non-interactive mode; the job's output is
                // surfaced through its own tool result when it finishes.
            }
            mew_agent::AgentEvent::FileDelta { .. } => {
                // Diff stats are accumulated daemon-side; no-op in CLI mode.
            }
            mew_agent::AgentEvent::FlaggedFilesChanged { .. } => {
                // Flagged files visibility is web-UI only.
            }
        }
    }

    agent.dispatcher.on_stop().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_mode_all_false_is_standard() {
        assert_eq!(
            resolve_mode(false, false, false, false),
            mew_hooks::PermissionMode::Standard
        );
    }

    #[test]
    fn resolve_mode_permissive() {
        assert_eq!(
            resolve_mode(true, false, false, false),
            mew_hooks::PermissionMode::Permissive
        );
    }

    #[test]
    fn resolve_mode_auto() {
        assert_eq!(
            resolve_mode(false, true, false, false),
            mew_hooks::PermissionMode::Auto
        );
    }

    #[test]
    fn resolve_mode_auto_plus() {
        assert_eq!(
            resolve_mode(false, false, true, false),
            mew_hooks::PermissionMode::AutoPlus
        );
    }

    #[test]
    fn resolve_mode_dangerous() {
        assert_eq!(
            resolve_mode(false, false, false, true),
            mew_hooks::PermissionMode::Dangerous
        );
    }

    #[test]
    fn resolve_mode_dangerous_beats_auto_plus() {
        assert_eq!(
            resolve_mode(false, false, true, true),
            mew_hooks::PermissionMode::Dangerous
        );
    }
}
