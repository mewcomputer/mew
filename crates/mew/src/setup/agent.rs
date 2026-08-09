//! Agent, tool, dispatcher, and MCP construction functions.
//!
//! Extracted from `main.rs` as pure code motion. These build the tool registry,
//! permission engine, plugin dispatcher, MCP server connections, context file
//! rendering, and the full session agent used by the daemon and TUI.

use std::sync::Arc;

use anyhow::{Context, Result};

use mew_agent::{Agent, PromptCacheRetention};
use mew_catalog::Catalog;
use mew_config::Config;
use mew_ext_broker::{
    build_consent_prompt, build_delta_prompt, build_sensitive_cap_prompt, is_legacy_full,
    reconstruct_caps, Capability, CapabilitySet, ConsentDecision, ExtensionManifest,
};
use mew_hooks::Dispatcher;

/// Apply catalog pricing for `model_id` onto `agent`.
///
/// Sets catalog-derived pricing and prompt-cache retention on the agent. An
/// absent catalog entry resets retention to `Unknown` while leaving pricing
/// unchanged, preserving the existing pricing fallback behavior.
pub(crate) fn apply_catalog_pricing(agent: &mut Agent, cat: Option<&Catalog>, model_id: &str) {
    let retention = cat
        .and_then(|catalog| catalog.lookup(model_id))
        .and_then(|model| model.prompt_cache_retention_secs);
    agent.set_prompt_cache_retention(PromptCacheRetention::from_secs(retention));

    if let Some(c) = cat {
        if let Some(m) = c.lookup(model_id) {
            agent.input_price = m.pricing.input;
            agent.output_price = m.pricing.output;
            agent.cache_read_price = m.pricing.cache_read;
            agent.cache_write_price = m.pricing.cache_write;
            agent.reasoning_price = m.pricing.reasoning;
        }
    }
}
use mew_message::SessionId;
use mew_session::Writer as SessionWriter;
use mew_tools::tools::ask_user::AskUser;
use mew_tools::tools::bash::Bash;
use mew_tools::tools::echo::Echo;
use mew_tools::tools::edit_hashline::EditHashline;
use mew_tools::tools::edit_plan::EditPlan;
use mew_tools::tools::edit_str_replace::EditStrReplace;
use mew_tools::tools::exit_tool::ExitTool;
use mew_tools::tools::flag_important::{FlagImportant, FlaggedFile};
use mew_tools::tools::glob::Glob;
use mew_tools::tools::goal::{BlockGoal, CompleteGoal, ProposeGoal};
use mew_tools::tools::grep::Grep;
use mew_tools::tools::handoff_plan::HandoffPlan;
use mew_tools::tools::jobs::{JobBlock, JobCancel, JobStatus, ShellBackground, ShellMonitor};
use mew_tools::tools::progress_update::ProgressUpdate;
use mew_tools::tools::read::Read;
use mew_tools::tools::skill::Skill;
use mew_tools::tools::switch_persona::SwitchPersona as SwitchPersonaTool;
use mew_tools::tools::todo::{TodoComplete, TodoCreate, TodoDelete, TodoListTool, TodoUpdate};
use mew_tools::tools::web_fetch::WebFetch;
use mew_tools::tools::write::Write;
use mew_tools::tools::write_plan::WritePlan;
use mew_tools::SecretSet;

use crate::setup::providers::{
    build_provider, find_router_provider, make_provider_builder, maybe_set_classifier_provider,
    MainModelResolver,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_tools(
    skills: Arc<Vec<mew_skills::Skill>>,
    skill_filter: Arc<tokio::sync::RwLock<Option<std::collections::HashSet<String>>>>,
    template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>>,
    personas: Arc<Vec<mew_personas::Persona>>,
    pending_persona_switch: Arc<tokio::sync::Mutex<Option<String>>>,
    current_persona_name: Arc<tokio::sync::RwLock<Option<String>>>,
    hashline_enabled: bool,
    plan_path: String,
) -> Vec<Arc<dyn mew_tools::Tool>> {
    let mut tools: Vec<Arc<dyn mew_tools::Tool>> = vec![
        Arc::new(Read),
        Arc::new(Write),
        Arc::new(EditStrReplace),
        Arc::new(Bash),
        Arc::new(Glob),
        Arc::new(Grep),
        Arc::new(Echo),
        Arc::new(ExitTool),
        Arc::new(ProgressUpdate),
        Arc::new(AskUser),
        Arc::new(ShellBackground),
        Arc::new(ShellMonitor),
        Arc::new(JobStatus),
        Arc::new(JobBlock),
        Arc::new(JobCancel),
        Arc::new(TodoCreate),
        Arc::new(TodoUpdate),
        Arc::new(TodoComplete),
        Arc::new(TodoDelete),
        Arc::new(TodoListTool),
        Arc::new(WebFetch),
        // Plan-workflow tools. Registered unconditionally; nothing enforces
        // planner-only use — persona allowlists gate access when set.
        Arc::new(WritePlan::new(plan_path.clone())),
        Arc::new(EditPlan::new(plan_path)),
        Arc::new(HandoffPlan),
        Arc::new(ProposeGoal),
        Arc::new(CompleteGoal),
        Arc::new(BlockGoal),
    ];
    if hashline_enabled {
        tools.insert(3, Arc::new(EditHashline));
    }
    if !skills.is_empty() {
        tools.push(Arc::new(Skill::new(skills, skill_filter, template_ctx)));
    }
    // The switch_persona tool is only useful when there's at least one
    // persona to switch to. With zero discovered personas the tool would
    // be a permanent dead-end for the model.
    if !personas.is_empty() {
        tools.push(Arc::new(SwitchPersonaTool::new(
            personas,
            pending_persona_switch,
            current_persona_name,
        )));
    }
    tools
}

pub(crate) fn build_permission_engine(
    cfg: &Config,
    mode: mew_hooks::PermissionMode,
) -> Arc<mew_config::permissions::PermissionEngine> {
    let secret_globs: Vec<String> = cfg
        .secrets
        .files
        .iter()
        .flat_map(|f| f.paths.iter().cloned())
        .collect();
    // Default cwd for the escape tier: the process's current directory.
    // The 4 call sites all hand the same `cfg` to this helper, so the
    // escape tier gets the same default cwd everywhere.
    let default_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    Arc::new(
        mew_config::permissions::PermissionEngine::new(cfg.permissions.rules.clone())
            .with_secret_files(secret_globs)
            .with_mode(mode)
            .with_workspace_roots(cfg.workspace.roots.clone(), default_cwd),
    )
}

/// Build the `SecretSet` shared with every tool call: words to redact from
/// output, and file globs whose results get dropped from search tools.
pub(crate) fn build_secret_set(cfg: &Config) -> Arc<SecretSet> {
    Arc::new(SecretSet {
        words: cfg
            .secrets
            .words
            .iter()
            .flat_map(|w| w.values.iter().cloned())
            .collect(),
        globs: cfg
            .secrets
            .files
            .iter()
            .flat_map(|f| f.paths.iter().cloned())
            .collect(),
    })
}

/// Prompt function type — takes a question string, returns y/n/None.
type PromptFn = Box<dyn Fn(&str) -> Option<bool> + Send + Sync>;

/// Build a consent resolver for extensions and legacy plugins.
///
/// The resolver checks persisted consent state first. If no decision
/// exists, it prompts the user (if interactive) or auto-restricts
/// (if non-interactive). Decisions are persisted.
///
/// For manifest-based extensions, the resolver shows a capability-delta
/// prompt listing each requested capability. For bare-executable plugins,
/// it shows a simple approved/restricted prompt.
///
/// `is_interactive` is injected (from `stdin().is_terminal()`) so
/// this function can be unit-tested with `false`.
/// `prompt_fn` is injected so tests can use a mock instead of the
/// real `prompt_yn`.
#[allow(dead_code)] // kept for behavior tests; the local dispatcher is sunset
pub(crate) fn build_consent_resolver(
    is_interactive: bool,
    prompt_fn: PromptFn,
    state: mew_ext_broker::ConsentState,
) -> mew_ext_broker::ConsentResolver {
    Box::new(move |name: &str, manifest: Option<&ExtensionManifest>| {
        // ── Bare-plugin path (no manifest) ──
        let persisted = state.get_granted_caps(name);
        if let Some(ref granted) = persisted {
            match manifest {
                None => {
                    return if is_legacy_full(granted) {
                        ConsentDecision::Approved
                    } else {
                        ConsentDecision::Restricted
                    };
                }
                Some(_) if is_legacy_full(granted) => {
                    // Stale sentinel — fall through to manifest first-run.
                }
                Some(_) => {} // Handled below.
            }
        }

        // ── Manifest path ──
        if let Some(m) = manifest {
            let manifest_caps = m.requested_capabilities();

            // Check for persisted consent with valid (non-sentinel) stored IDs.
            if let Some(ref granted) = persisted {
                if !is_legacy_full(granted) {
                    let granted_caps = reconstruct_caps(granted);

                    // Determine last-requested for delta detection.
                    // Empty (migration) → treat as current manifest (no delta).
                    let last_requested_ids = state
                        .get_last_requested(name)
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| manifest_caps.to_ids());
                    let last_requested = reconstruct_caps(&last_requested_ids);

                    // Delta: what's new in the manifest?
                    let delta = manifest_caps.difference(&last_requested);

                    if delta.added.is_empty() {
                        // No new capabilities — clamp and return.
                        return ConsentDecision::ApprovedWithCaps(
                            granted_caps.intersect(&manifest_caps),
                        );
                    }

                    // Manifest grew — re-prompt for new capabilities only.
                    let added_caps = reconstruct_caps(&delta.added);
                    let base = granted_caps.intersect(&manifest_caps);

                    if !is_interactive {
                        // Non-interactive: keep existing granted caps, deny new ones.
                        tracing::warn!(
                            "extension '{}' new caps auto-denied (non-interactive)",
                            name
                        );
                        let ids = base.to_ids();
                        state.set_consent(name, ids, manifest_caps.to_ids());
                        state.save().ok();
                        return ConsentDecision::ApprovedWithCaps(base);
                    }

                    // Interactive: prompt for new caps with individual consent.
                    let added_vec: Vec<Capability> = added_caps.iter().cloned().collect();
                    let batch_prompt = build_delta_prompt(name, m, &added_vec);
                    let newly_approved = prompt_and_consent_caps(
                        name,
                        &m.extension.version,
                        &added_caps,
                        &batch_prompt,
                        &prompt_fn,
                    );

                    // Combine: existing granted (clamped) + newly approved.
                    let mut final_caps = base;
                    for cap in newly_approved.iter() {
                        final_caps.grant(cap.clone());
                    }

                    let ids = final_caps.to_ids();
                    state.set_consent(name, ids, manifest_caps.to_ids());
                    state.save().ok();
                    return ConsentDecision::ApprovedWithCaps(final_caps);
                }
            }

            // First run (no persisted consent, or stale sentinel).
            if !is_interactive {
                // Non-interactive: auto-restrict (same as current behavior).
                // Restricted → broker maps to observe_only(), preserving HooksObserve.
                tracing::warn!("extension '{}' auto-restricted (non-interactive)", name);
                state.set_consent(name, vec![], manifest_caps.to_ids());
                state.save().ok();
                return ConsentDecision::Restricted;
            }
            let batch_prompt = build_consent_prompt(name, m);
            let approved = prompt_and_consent_caps(
                name,
                &m.extension.version,
                &manifest_caps,
                &batch_prompt,
                &prompt_fn,
            );
            let ids = approved.to_ids();
            state.set_consent(name, ids, manifest_caps.to_ids());
            state.save().ok();
            return ConsentDecision::ApprovedWithCaps(approved);
        }

        // ── Bare-plugin first run (no persisted, no manifest) ──
        let question = format!(
            "Plugin '{}' has been running with full access. Grant full access?",
            name
        );
        let decision = if is_interactive {
            match prompt_fn(&question) {
                Some(true) => ConsentDecision::Approved,
                _ => ConsentDecision::Restricted,
            }
        } else {
            tracing::warn!("plugin '{}' auto-restricted (non-interactive)", name);
            ConsentDecision::Restricted
        };
        let ids = decision.to_granted_ids();
        state.set_granted_caps(name, ids);
        state.save().ok();
        decision
    })
}

/// Prompt for a set of capabilities using two-phase consent.
///
/// Phase 1: batch prompt for non-sensitive caps (excluding always-granted).
/// Phase 2: individual prompts for each sensitive cap.
///
/// Phase 1 and Phase 2 are independent — the user can deny the batch
/// but still approve individual sensitive caps, and vice versa.
///
/// Returns the approved `CapabilitySet` (always-granted + approved).
#[allow(dead_code)] // kept for behavior tests; the local dispatcher is sunset
fn prompt_and_consent_caps(
    name: &str,
    version: &str,
    caps: &CapabilitySet,
    batch_prompt: &str,
    prompt_fn: &PromptFn,
) -> CapabilitySet {
    use mew_ext_broker::is_sensitive;

    let mut approved = CapabilitySet::always_granted();

    let (non_sensitive, sensitive): (Vec<_>, Vec<_>) =
        caps.iter().cloned().partition(|c| !is_sensitive(c));

    // Phase 1: batch prompt for non-sensitive (excluding always-granted).
    let promptable: Vec<_> = non_sensitive
        .into_iter()
        .filter(|c| !approved.has(c))
        .collect();

    if !promptable.is_empty() && prompt_fn(batch_prompt) == Some(true) {
        for cap in &promptable {
            approved.grant(cap.clone());
        }
        // If user denied, approved stays at always-granted.
    }

    // Phase 2: individual prompts for sensitive caps.
    for cap in &sensitive {
        let p = build_sensitive_cap_prompt(name, version, cap);
        if prompt_fn(&p) == Some(true) {
            approved.grant(cap.clone());
        }
    }

    approved
}

/// Render any context files marked with `template: true` through minijinja
/// using the agent's template context. Non-templated files are left as-is.
/// Returns a new Vec with rendered content.
pub(crate) fn render_templated_context_files(
    files: &[mew_context::File],
    agent: &mew_agent::Agent,
) -> Vec<mew_context::File> {
    let has_templated = files.iter().any(|f| f.template);
    if !has_templated {
        return files.to_vec();
    }

    // Build a template context from the agent's current state.
    let mut tool_names: Vec<String> = agent.tools.keys().cloned().collect();
    tool_names.sort_unstable();
    let ctx = mew_prompts::template::TemplateContext {
        model_id: agent.model_id.clone(),
        provider_id: agent.provider_id.clone(),
        session_id: agent.session_id.to_string(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        current_date: mew_prompts::template::TemplateContext::today(),
        tools: tool_names,
        skills: agent.skills.iter().map(|s| s.name.clone()).collect(),
        project_vars: agent.project_vars.clone(),
        ..Default::default()
    };

    files
        .iter()
        .map(|f| {
            if f.template {
                mew_context::File {
                    path: f.path.clone(),
                    content: mew_prompts::template::render(&f.content, &ctx),
                    template: false,
                }
            } else {
                f.clone()
            }
        })
        .collect()
}

/// Wire subagent infrastructure (defs, runner, tools) onto an agent.
/// Called inside build_session_agent (for daemon path) and again by
/// run_tui/build_and_run after register_plugin_tools to refresh the
/// runner's tool collection with plugin tools included.
pub(crate) fn wire_subagents(
    agent: &mut Agent,
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    raw: bool,
    dispatcher: Arc<dyn Dispatcher>,
    discovered_extensions: &[mew_ext_broker::DiscoveredExtension],
) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let subagents_extra: Vec<std::path::PathBuf> = discovered_extensions
        .iter()
        .filter_map(|ext| ext.provides_subagents())
        .collect();
    let subagent_defs = {
        let loader = if subagents_extra.is_empty() {
            mew_subagents::Loader::new(cwd)
        } else {
            mew_subagents::Loader::with_extra_dirs(cwd, subagents_extra)
        };
        Arc::new(loader.load().unwrap_or_default())
    };
    if !subagent_defs.is_empty() {
        let resolver = Arc::new(MainModelResolver {
            cfg: Arc::new(cfg.clone()),
            cat: cat.cloned().map(Arc::new),
            default_provider_id: provider_id.to_string(),
            router_provider_id: find_router_provider(cfg).map(|(id, _)| id),
            raw,
        });
        let runner = mew_agent::runner::SimpleRunner::new(
            agent.provider.clone(),
            agent.tools.values().cloned().collect(),
            dispatcher,
        )
        .with_model_resolver(resolver);
        agent.subagent_runner = Some(Arc::new(runner));
        agent.subagent_defs = subagent_defs.to_vec();
        agent.tools.insert(
            "subagent_start".into(),
            Arc::new(mew_tools::tools::subagent_start::SubagentStart::new(
                subagent_defs.clone(),
            )),
        );
        agent.tools.insert(
            "subagent_wait".into(),
            Arc::new(mew_tools::tools::subagent_wait::SubagentWait::new()),
        );
    }
}

/// Build a full agent for a session. Used by `run_daemon` (and the TUI's
/// `--connect` daemon-client mode goes through the daemon side). Sets up
/// the provider, tools, MCP, personas, skills, subagents, context files,
/// and pricing.
///
/// `writer` / `session_id` come from the daemon's `SessionManager`, which
/// owns the session directory. The agent is wired to append to that writer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_session_agent(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    model_id: &str,
    raw: bool,
    mode: mew_hooks::PermissionMode,
    writer: Option<SessionWriter>,
    session_id: Option<SessionId>,
    session_cwd: Option<std::path::PathBuf>,
    browser_enabled: bool,
    dispatcher: Arc<dyn Dispatcher>,
    todos_path: Option<std::path::PathBuf>,
    discovered_extensions: &[mew_ext_broker::DiscoveredExtension],
) -> Result<Agent> {
    let provider =
        build_provider(cfg, cat, provider_id, model_id, raw).context("build provider")?;
    let cwd = session_cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Collect [provides] paths from discovered extensions.
    let skills_extra: Vec<std::path::PathBuf> = discovered_extensions
        .iter()
        .filter_map(|ext| ext.provides_skills())
        .collect();
    let personas_extra: Vec<std::path::PathBuf> = discovered_extensions
        .iter()
        .filter_map(|ext| ext.provides_personas())
        .collect();

    let skills_loader = if skills_extra.is_empty() {
        mew_skills::Loader::new(cwd.clone())
    } else {
        mew_skills::Loader::with_extra_dirs(cwd.clone(), skills_extra)
    };
    let skills = Arc::new(skills_loader.load().unwrap_or_default());
    let skill_filter = Arc::new(tokio::sync::RwLock::new(None));
    let persona_loader = if personas_extra.is_empty() {
        mew_personas::Loader::new(cwd.clone())
    } else {
        mew_personas::Loader::with_extra_dirs(cwd.clone(), personas_extra)
    };
    let personas_arc = Arc::new(persona_loader.load().unwrap_or_default());
    let pending_persona_switch = Arc::new(tokio::sync::Mutex::new(None));
    let current_persona_name = Arc::new(tokio::sync::RwLock::new(None));
    let template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>> =
        Arc::new(tokio::sync::RwLock::new(None));
    let mut tools = build_tools(
        skills.clone(),
        skill_filter.clone(),
        template_ctx.clone(),
        personas_arc.clone(),
        pending_persona_switch.clone(),
        current_persona_name.clone(),
        crate::commands::config::hashline_enabled_for(cfg, provider_id),
        cfg.plan_path.clone(),
    );
    tools.extend(mew_tools::tools::browser::tools());

    let flagged_files: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let mut tools = tools;
    tools.push(Arc::new(FlagImportant::new(flagged_files.clone())));

    let permission_engine = build_permission_engine(cfg, mode);

    let mut agent = Agent::new(provider, dispatcher.clone(), writer, tools, session_id);
    agent.set_browser_enabled(browser_enabled);
    agent.set_model_info(model_id, provider_id);
    agent.template_ctx = template_ctx;
    agent.flagged_files = flagged_files;
    agent.secrets = build_secret_set(cfg);
    agent.todos_path = todos_path;
    agent.leak_reminder = cfg.orchestration.leak_reminder;
    agent.leak_reminder_max = cfg.orchestration.leak_reminder_max;
    agent.set_permission_engine(permission_engine);
    maybe_set_classifier_provider(&mut agent, cfg, cat, raw, provider_id, model_id);
    agent.set_plan_path(&cfg.plan_path);
    agent.set_personas((*personas_arc).clone());
    agent.set_pending_persona_switch(pending_persona_switch.clone());
    agent.set_current_persona_name(current_persona_name.clone());

    // Enable a persistent shell session so `cd`, `export`, and other
    // state survive across bash tool calls.
    let shell_session = mew_tools::tools::shell_session::shared_session(
        std::env::current_dir().unwrap_or_default(),
    );
    agent.set_shell_session(shell_session);

    // Wire the fallback-model provider builder.
    agent.set_provider_builder(make_provider_builder(cfg.clone(), cat.cloned(), raw));
    // Plugin tools: register_plugin_tools is async but we're in a sync
    // builder. The daemon's agent builder closure must be sync. Plugin tool
    // registration is a no-op for NopDispatcher (the default), so skipping
    // the call is safe. When a real dispatcher is wired, this will need to
    // become async — at which point the AgentBuilder type should change.
    // agent.register_plugin_tools().await;
    // Apply the default persona on startup (non-interactive path — no TUI app).
    if cfg.default_persona != "none" && cfg.default_persona != "default" {
        if let Some(persona) = personas_arc.iter().find(|p| p.name == cfg.default_persona) {
            agent.apply_persona(persona);
            tracing::info!(persona = %persona.name, "applied default persona on startup");
        }
    }
    if cfg.workspace.roots.is_empty() {
        agent.workspace_roots = vec![cwd.clone()];
    } else {
        agent.workspace_roots = cfg.workspace.roots.clone();
    }

    // Wire up subagent infrastructure.
    wire_subagents(
        &mut agent,
        cfg,
        cat,
        provider_id,
        raw,
        dispatcher.clone(),
        discovered_extensions,
    );

    // Load project context and skills for system prompt.
    let ctx_loader = mew_context::Loader::new(&cwd);
    let ctx_files = ctx_loader.load().unwrap_or_default();
    let project_vars = mew_context::load_project_vars(&cwd);
    agent.project_vars = project_vars;

    // Build the stable system scaffold by rendering base.md through minijinja.
    // Runtime capability names and schemas are carried by the provider request
    // instead of being repeated in this cacheable instruction block.
    let base_prompt = {
        let mut tool_names: Vec<String> = agent
            .tools
            .keys()
            .filter(|name| browser_enabled || !name.starts_with("browser_"))
            .cloned()
            .collect();
        tool_names.sort_unstable();
        let mut mcp_names: Vec<String> = agent
            .tools
            .values()
            .filter_map(|t| {
                let name = t.name();
                if name.starts_with("mcp__") {
                    Some(
                        name.strip_prefix("mcp__")
                            .and_then(|s| s.split("__").next())
                            .unwrap_or(name)
                            .to_string(),
                    )
                } else {
                    None
                }
            })
            .collect();
        mcp_names.sort_unstable();
        let mut subagent_infos: Vec<mew_prompts::template::SubagentInfo> = agent
            .subagent_defs
            .iter()
            .map(|d| mew_prompts::template::SubagentInfo {
                name: d.name.clone(),
                description: d.description.clone(),
            })
            .collect();
        subagent_infos.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        let ctx = mew_prompts::template::TemplateContext {
            supports_vision: agent.supports_vision,
            model_id: agent.model_id.clone(),
            provider_id: agent.provider_id.clone(),
            session_id: agent.session_id.to_string(),
            cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            current_date: mew_prompts::template::TemplateContext::today(),
            tools: tool_names,
            skills: agent.skills.iter().map(|s| s.name.clone()).collect(),
            mcp_servers: mcp_names,
            project_vars: agent.project_vars.clone(),
            available_subagents: subagent_infos,
            ..Default::default()
        };
        let base_body = mew_prompts::vfs::read_builtin("system_prompts/base").unwrap_or("");
        mew_prompts::template::render(base_body, &ctx)
    };

    // Assemble: base prompt → context files → (skills added by rebuild_system)
    let mut system = base_prompt;
    if !ctx_files.is_empty() {
        let rendered_ctx = render_templated_context_files(&ctx_files, &agent);
        system.push_str(&mew_context::build_system_prompt(&rendered_ctx));
    }
    agent.set_system(system);
    if !skills.is_empty() {
        agent.set_skills((*skills).clone());
    }

    if let Some(c) = cat {
        agent.supports_vision = c.supports_vision(model_id);
        agent.context_window = c.context_window(model_id).max(0) as u32;
        // Default `max_output_tokens` from the catalog, capped at 32K so
        // models with very large total context (e.g. GPT-5-Codex at 400K
        // with 128K max output) leave more room for input. 0 means
        // "unknown" — the agent keeps its existing default of 0 (no
        // override) so the provider's own default applies.
        if let Some(raw_max_output) = c.max_output(model_id) {
            agent.default_max_output_tokens = raw_max_output.min(32_768);
        }
        apply_catalog_pricing(&mut agent, cat, model_id);
    }

    Ok(agent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_catalog::{Catalog, Model, Pricing};
    use mew_provider_fake::FakeProvider;

    fn make_agent() -> mew_agent::Agent {
        let provider = Arc::new(FakeProvider::new(vec![]));
        let dispatcher: Arc<dyn mew_hooks::Dispatcher> = Arc::new(mew_hooks::NopDispatcher);
        mew_agent::Agent::new(provider, dispatcher, None, vec![], None)
    }

    #[test]
    fn apply_catalog_pricing_sets_all_fields() {
        let mut agent = make_agent();
        let mut cat = Catalog::empty();
        cat.models.insert(
            "test-model".into(),
            Model {
                id: "test-model".into(),
                pricing: Pricing {
                    input: 1.5,
                    output: 6.0,
                    cache_read: 0.15,
                    cache_write: 2.25,
                    reasoning: 9.0,
                },
                prompt_cache_retention_secs: Some(14_400),
                ..Default::default()
            },
        );
        apply_catalog_pricing(&mut agent, Some(&cat), "test-model");
        assert_eq!(agent.input_price, 1.5);
        assert_eq!(agent.output_price, 6.0);
        assert_eq!(agent.cache_read_price, 0.15);
        assert_eq!(agent.cache_write_price, 2.25);
        assert_eq!(agent.reasoning_price, 9.0);
        assert_eq!(
            agent.prompt_cache_retention(),
            mew_agent::PromptCacheRetention::Known(std::time::Duration::from_secs(14_400))
        );
    }

    #[test]
    fn apply_catalog_pricing_none_catalog_no_panic() {
        let mut agent = make_agent();
        apply_catalog_pricing(&mut agent, None, "test-model");
        // Prices should remain at their default 0.0
        assert_eq!(agent.input_price, 0.0);
        assert_eq!(agent.output_price, 0.0);
        assert_eq!(
            agent.prompt_cache_retention(),
            mew_agent::PromptCacheRetention::Unknown
        );
    }

    #[test]
    fn apply_catalog_pricing_model_not_in_catalog() {
        let mut agent = make_agent();
        let cat = Catalog::empty();
        apply_catalog_pricing(&mut agent, Some(&cat), "nonexistent-model");
        assert_eq!(agent.input_price, 0.0);
        assert_eq!(agent.output_price, 0.0);
    }

    // ── Consent resolver tests ──────────────────────────────────────────

    /// Build a test manifest with specific hooks config.
    fn test_manifest(
        name: &str,
        version: &str,
        observe: bool,
        gate: Vec<String>,
        gate_mutate: bool,
        mutate_headers: bool,
    ) -> mew_ext_broker::ExtensionManifest {
        use mew_ext_broker::{
            ExtensionCapabilities, ExtensionMeta, ExtensionProvides, ExtensionSandbox, HooksConfig,
        };
        ExtensionManifest {
            extension: ExtensionMeta {
                name: name.into(),
                version: version.into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities {
                    hooks: Some(HooksConfig {
                        observe,
                        gate,
                        gate_mutate,
                        mutate_headers,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        }
    }

    #[test]
    fn test_first_run_individual_consent() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        // Manifest requests hooks:observe (non-sensitive), hooks:gate (sensitive),
        // hooks:mutate:headers (sensitive).
        let manifest = test_manifest("ext", "1.0.0", true, vec!["bash".into()], false, true);

        let dir = tempfile::tempdir().unwrap();
        let state = mew_ext_broker::ConsentState::with_path(dir.path().join("consent.json"));
        let call_count = Arc::new(AtomicU32::new(0));

        // prompt_fn: returns true for batch prompt, true for hooks:gate,
        // false for hooks:mutate:headers.
        let count_clone = call_count.clone();
        let prompt_fn: PromptFn = Box::new(move |q: &str| -> Option<bool> {
            count_clone.fetch_add(1, Ordering::Relaxed);
            if q.contains("hooks:mutate:headers") && q.contains("Grant?") {
                Some(false) // Deny hooks:mutate:headers
            } else {
                Some(true) // Approve batch + hooks:gate
            }
        });

        let resolver = build_consent_resolver(true, prompt_fn, state);
        let decision = resolver("ext", Some(&manifest));

        match decision {
            ConsentDecision::ApprovedWithCaps(caps) => {
                // hooks:observe should be granted (non-sensitive, batch approved).
                assert!(caps.has(&mew_ext_broker::Capability::HooksObserve));
                // hooks:gate should be granted (sensitive, individually approved).
                assert!(caps.has(&mew_ext_broker::Capability::HooksGate));
                // hooks:mutate:headers should NOT be granted (sensitive, individually denied).
                assert!(!caps.has(&mew_ext_broker::Capability::HooksMutateHeaders));
            }
            other => panic!("expected ApprovedWithCaps, got {:?}", other),
        }

        // Prompt called: 1 (batch) + 1 (hooks:gate) + 1 (hooks:mutate:headers) = 3.
        assert_eq!(
            call_count.load(Ordering::Relaxed),
            3,
            "expected 3 prompt calls (batch + 2 sensitive)"
        );
    }

    #[test]
    fn test_first_run_noninteractive_denies_sensitive() {
        let manifest = test_manifest("ext", "1.0.0", true, vec!["bash".into()], false, true);

        let dir = tempfile::tempdir().unwrap();
        let state = mew_ext_broker::ConsentState::with_path(dir.path().join("consent.json"));
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = call_count.clone();
        let prompt_fn: PromptFn = Box::new(move |_q: &str| -> Option<bool> {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(true)
        });

        let resolver = build_consent_resolver(false, prompt_fn, state);
        let decision = resolver("ext", Some(&manifest));

        // Non-interactive first run → Restricted.
        assert_eq!(decision, ConsentDecision::Restricted);
        // Prompt_fn NOT called.
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "non-interactive should not prompt"
        );
    }

    #[test]
    fn test_upgrade_noninteractive_keeps_existing() {
        // Store consent with observe + gate, last_requested = observe + gate.
        let manifest_v1 = test_manifest("ext", "1.0.0", true, vec!["bash".into()], false, false);
        let dir = tempfile::tempdir().unwrap();
        let state = mew_ext_broker::ConsentState::with_path(dir.path().join("consent.json"));

        let v1_caps = manifest_v1.requested_capabilities();
        state.set_consent("ext", v1_caps.to_ids(), v1_caps.to_ids());
        state.save().unwrap();

        // Now call resolver non-interactive with manifest v2 (adds mutate_headers).
        let manifest_v2 = test_manifest("ext", "2.0.0", true, vec!["bash".into()], false, true);
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = call_count.clone();
        let prompt_fn: PromptFn = Box::new(move |_q: &str| -> Option<bool> {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(true)
        });

        let resolver = build_consent_resolver(false, prompt_fn, state);
        let decision = resolver("ext", Some(&manifest_v2));

        match decision {
            ConsentDecision::ApprovedWithCaps(caps) => {
                // Existing caps preserved.
                assert!(caps.has(&mew_ext_broker::Capability::HooksObserve));
                assert!(caps.has(&mew_ext_broker::Capability::HooksGate));
                // New cap NOT granted (non-interactive → auto-denied).
                assert!(!caps.has(&mew_ext_broker::Capability::HooksMutateHeaders));
            }
            other => panic!("expected ApprovedWithCaps, got {:?}", other),
        }

        // Prompt_fn NOT called (non-interactive).
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "non-interactive upgrade should not prompt"
        );
    }

    #[test]
    fn test_upgrade_delta_reprompts() {
        // Store consent with observe + gate, last_requested = observe + gate.
        let manifest_v1 = test_manifest("ext", "1.0.0", true, vec!["bash".into()], false, false);
        let dir = tempfile::tempdir().unwrap();
        let state = mew_ext_broker::ConsentState::with_path(dir.path().join("consent.json"));

        let v1_caps = manifest_v1.requested_capabilities();
        state.set_consent("ext", v1_caps.to_ids(), v1_caps.to_ids());
        state.save().unwrap();

        // Manifest v2 adds mutate_headers (sensitive).
        let manifest_v2 = test_manifest("ext", "2.0.0", true, vec!["bash".into()], false, true);

        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = call_count.clone();
        let prompt_fn: PromptFn = Box::new(move |q: &str| -> Option<bool> {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Approve everything (the delta has only a sensitive cap,
            // so we get just the individual sensitive prompt).
            let _ = q;
            Some(true)
        });

        let resolver = build_consent_resolver(true, prompt_fn, state);
        let decision = resolver("ext", Some(&manifest_v2));

        match decision {
            ConsentDecision::ApprovedWithCaps(caps) => {
                // Existing caps preserved.
                assert!(caps.has(&mew_ext_broker::Capability::HooksObserve));
                assert!(caps.has(&mew_ext_broker::Capability::HooksGate));
                // New cap granted (interactive, approved).
                assert!(caps.has(&mew_ext_broker::Capability::HooksMutateHeaders));
            }
            other => panic!("expected ApprovedWithCaps, got {:?}", other),
        }

        // Prompt was called at least once (for the new sensitive cap).
        assert!(
            call_count.load(std::sync::atomic::Ordering::Relaxed) >= 1,
            "upgrade delta should prompt"
        );
    }

    #[test]
    fn test_upgrade_no_change_no_reprompt() {
        // Store consent with observe + gate, last_requested = observe + gate.
        let manifest = test_manifest("ext", "1.0.0", true, vec!["bash".into()], false, false);
        let dir = tempfile::tempdir().unwrap();
        let state = mew_ext_broker::ConsentState::with_path(dir.path().join("consent.json"));

        let caps = manifest.requested_capabilities();
        state.set_consent("ext", caps.to_ids(), caps.to_ids());
        state.save().unwrap();

        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = call_count.clone();
        let prompt_fn: PromptFn = Box::new(move |_q: &str| -> Option<bool> {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(true)
        });

        let resolver = build_consent_resolver(true, prompt_fn, state);
        let decision = resolver("ext", Some(&manifest));

        match decision {
            ConsentDecision::ApprovedWithCaps(caps) => {
                // Clamped persisted caps.
                assert!(caps.has(&mew_ext_broker::Capability::HooksObserve));
                assert!(caps.has(&mew_ext_broker::Capability::HooksGate));
            }
            other => panic!("expected ApprovedWithCaps, got {:?}", other),
        }

        // No prompt when manifest hasn't changed.
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "no reprompt when manifest unchanged"
        );
    }

    #[test]
    fn test_upgrade_backward_compat() {
        // Simulate an old entry: granted = observe only (subset), no last_requested.
        // The resolver should treat empty last_requested as "current manifest"
        // (no delta → no spurious re-prompt).
        let manifest = test_manifest("ext", "1.0.0", true, vec!["bash".into()], false, false);
        let dir = tempfile::tempdir().unwrap();
        let state = mew_ext_broker::ConsentState::with_path(dir.path().join("consent.json"));

        // Store using set_granted_caps (which preserves last_requested as empty
        // for a new entry — simulating old data).
        state.set_granted_caps(
            "ext",
            vec![
                "storage".into(),
                "config:read".into(),
                "ui".into(),
                "register".into(),
                "hooks:observe".into(),
            ],
        );
        state.save().unwrap();

        // Verify last_requested is empty (old entry migration).
        assert!(state.get_last_requested("ext").unwrap().is_empty());

        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = call_count.clone();
        let prompt_fn: PromptFn = Box::new(move |_q: &str| -> Option<bool> {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(true)
        });

        let resolver = build_consent_resolver(true, prompt_fn, state);
        let decision = resolver("ext", Some(&manifest));

        match decision {
            ConsentDecision::ApprovedWithCaps(caps) => {
                // Clamped to manifest caps. The granted subset is observe only
                // (no gate), so the result should have observe but NOT gate.
                assert!(caps.has(&mew_ext_broker::Capability::HooksObserve));
                assert!(!caps.has(&mew_ext_broker::Capability::HooksGate));
            }
            other => panic!("expected ApprovedWithCaps, got {:?}", other),
        }

        // No spurious re-prompt (migration: empty last_requested → no delta).
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "backward-compat: no spurious reprompt"
        );
    }
}
