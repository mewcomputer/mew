//! Agent, tool, dispatcher, and MCP construction functions.
//!
//! Extracted from `main.rs` as pure code motion. These build the tool registry,
//! permission engine, plugin dispatcher, MCP server connections, context file
//! rendering, and the full session agent used by the daemon and TUI.

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use mew_agent::Agent;
use mew_catalog::Catalog;
use mew_config::Config;
use mew_ext_broker::ExtensionBroker;
use mew_hooks::{Dispatcher, NopDispatcher, PluginHost};

/// Apply catalog pricing for `model_id` onto `agent`.
///
/// Sets `input_price`, `output_price`, `cache_read_price`, `cache_write_price`,
/// and `reasoning_price` from the catalog entry, if found. Does nothing when
/// the catalog or model is absent.
pub(crate) fn apply_catalog_pricing(agent: &mut Agent, cat: Option<&Catalog>, model_id: &str) {
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
use mew_mcp::McpClient;
use mew_message::SessionId;
use mew_session::Writer as SessionWriter;
use mew_tools::tools::ask_user::AskUser;
use mew_tools::tools::bash::Bash;
use mew_tools::tools::echo::Echo;
use mew_tools::tools::edit_hashline::EditHashline;
use mew_tools::tools::edit_str_replace::EditStrReplace;
use mew_tools::tools::exit_tool::ExitTool;
use mew_tools::tools::flag_important::{FlagImportant, FlaggedFile};
use mew_tools::tools::glob::Glob;
use mew_tools::tools::grep::Grep;
use mew_tools::tools::jobs::{JobBlock, JobCancel, JobStatus, ShellBackground, ShellMonitor};
use mew_tools::tools::progress_update::ProgressUpdate;
use mew_tools::tools::read::Read;
use mew_tools::tools::skill::Skill;
use mew_tools::tools::switch_persona::SwitchPersona as SwitchPersonaTool;
use mew_tools::tools::todo::{TodoComplete, TodoCreate, TodoDelete, TodoListTool, TodoUpdate};
use mew_tools::tools::web_fetch::WebFetch;
use mew_tools::tools::write::Write;
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

pub(crate) fn plugin_storage_map() -> std::collections::HashMap<String, String> {
    let dir = plugin_storage_dir();
    if !dir.exists() {
        return std::collections::HashMap::new();
    }
    let mut map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(json_map) =
                        serde_json::from_str::<std::collections::HashMap<String, String>>(&content)
                    {
                        for (k, v) in json_map {
                            map.insert(format!("{}/{}", name, k), v);
                        }
                    }
                }
            }
        }
    }
    map
}

pub(crate) fn plugin_storage_dir() -> std::path::PathBuf {
    mew_config::config_dir().join("plugin-storage")
}

pub(crate) fn write_plugin_storage(
    map: &mut std::collections::HashMap<String, String>,
    key: &str,
    value: &str,
) {
    map.insert(key.to_string(), value.to_string());
    let dir = plugin_storage_dir();
    let _ = std::fs::create_dir_all(&dir);
    // Group by plugin name (first segment of key before /)
    let mut per_plugin: std::collections::HashMap<
        String,
        std::collections::HashMap<String, String>,
    > = std::collections::HashMap::new();
    for (k, v) in map.iter() {
        if let Some((plugin, subkey)) = k.split_once('/') {
            per_plugin
                .entry(plugin.to_string())
                .or_default()
                .insert(subkey.to_string(), v.clone());
        }
    }
    for (plugin, data) in &per_plugin {
        let path = dir.join(format!("{}.json", plugin));
        let tmp = dir.join(format!("{}.tmp", plugin));
        if let Ok(json) = serde_json::to_string(data) {
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_dispatcher(
    notify: impl Fn(String) + Send + Sync + 'static,
    config_read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    log_fn: impl Fn(String) + Send + Sync + 'static,
    storage_read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    storage_write: impl Fn(&str, &str) + Send + Sync + 'static,
    storage_delete: impl Fn(&str) + Send + Sync + 'static,
    set_ui: impl Fn(&str, &str) + Send + Sync + 'static,
    disabled_plugins: &[String],
    plugin_configs: std::collections::HashMap<String, mew_hooks::PluginHookConfig>,
) -> Arc<dyn Dispatcher> {
    let host = PluginHost {
        notify: Arc::new(notify),
        config_read: Arc::new(config_read),
        log: Arc::new(log_fn),
        storage_read: Arc::new(storage_read),
        storage_write: Arc::new(storage_write),
        storage_delete: Arc::new(storage_delete),
        set_ui: Arc::new(set_ui),
    };

    // Build consent resolver for legacy plugins.
    let consent_state = mew_ext_broker::ConsentState::load();
    let is_interactive = std::io::stdin().is_terminal();
    use std::io::IsTerminal as _;
    let resolver =
        build_consent_resolver(is_interactive, Box::new(crate::prompt_yn), consent_state);

    let dirs = mew_hooks_runtime::PluginLoader::default_dirs();
    match ExtensionBroker::from_dirs_filtered_with_config(
        dirs,
        host.clone(),
        disabled_plugins,
        plugin_configs,
        ExtensionBroker::default_timeout(),
        Some(resolver),
    )
    .await
    {
        Ok(d) => {
            tracing::info!("plugin runtime loaded");
            Arc::new(d)
        }
        Err(e) => {
            tracing::warn!("plugin runtime unavailable, using no-op: {}", e);
            Arc::new(NopDispatcher)
        }
    }
}

/// Prompt function type — takes a question string, returns y/n/None.
type PromptFn = Box<dyn Fn(&str) -> Option<bool> + Send + Sync>;

/// Build a consent resolver for legacy plugins.
///
/// The resolver checks persisted consent state first. If no decision
/// exists, it prompts the user (if interactive) or auto-restricts
/// (if non-interactive). Decisions are persisted.
///
/// `is_interactive` is injected (from `stdin().is_terminal()`) so
/// this function can be unit-tested with `false`.
/// `prompt_fn` is injected so tests can use a mock instead of the
/// real `prompt_yn`.
pub(crate) fn build_consent_resolver(
    is_interactive: bool,
    prompt_fn: PromptFn,
    state: mew_ext_broker::ConsentState,
) -> mew_ext_broker::ConsentResolver {
    Box::new(move |name: &str| {
        if let Some(existing) = state.get(name) {
            return existing;
        }
        let decision = if is_interactive {
            match prompt_fn(name) {
                Some(true) => mew_ext_broker::ConsentDecision::Approved,
                Some(false) => mew_ext_broker::ConsentDecision::Restricted,
                None => mew_ext_broker::ConsentDecision::Restricted,
            }
        } else {
            tracing::warn!("plugin '{}' auto-restricted (non-interactive)", name);
            mew_ext_broker::ConsentDecision::Restricted
        };
        state.set(name, decision);
        state.save().ok();
        decision
    })
}

/// Load MCP server configs from standard locations.
///
/// Searches (in order):
///   cwd/mcp.json, cwd/.mcp.json, cwd/.mew/mcp.json, cwd/.mew/.mcp.json
/// Each file uses Claude-Code-compatible format:
///   { "mcpServers": { "name": { "command": "...", "args": [...], "type": "stdio"|"http" } } }
pub(crate) fn load_mcp_configs() -> Vec<mew_mcp::McpServerConfig> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let paths = [
        cwd.join("mcp.json"),
        cwd.join(".mcp.json"),
        cwd.join(".mew").join("mcp.json"),
        cwd.join(".mew").join(".mcp.json"),
    ];

    let mut configs = Vec::new();

    for path in &paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!("failed to read {}: {}", path.display(), e);
                continue;
            }
        };

        let v: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to parse {}: {}", path.display(), e);
                continue;
            }
        };

        let Some(servers) = v.get("mcpServers").and_then(|s| s.as_object()) else {
            continue;
        };

        for (name, cfg) in servers {
            let command = cfg
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from);
            let url = cfg.get("url").and_then(|v| v.as_str()).map(String::from);
            let type_ = cfg.get("type").and_then(|v| v.as_str()).map(String::from);
            let args = cfg
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v: &serde_json::Value| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            configs.push(mew_mcp::McpServerConfig {
                name: name.clone(),
                type_,
                url,
                command,
                args,
            });
        }
    }

    configs
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
    let tool_names: Vec<String> = agent.tools.keys().cloned().collect();
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
) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let subagent_defs = {
        let loader = mew_subagents::Loader::new(cwd);
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
    dispatcher: Arc<dyn Dispatcher>,
    todos_path: Option<std::path::PathBuf>,
) -> Result<Agent> {
    let provider =
        build_provider(cfg, cat, provider_id, model_id, raw).context("build provider")?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let skills_loader = mew_skills::Loader::new(cwd.clone());
    let skills = Arc::new(skills_loader.load().unwrap_or_default());
    let skill_filter = Arc::new(tokio::sync::RwLock::new(None));
    let persona_loader = mew_personas::Loader::new(cwd.clone());
    let personas_arc = Arc::new(persona_loader.load().unwrap_or_default());
    let pending_persona_switch = Arc::new(tokio::sync::Mutex::new(None));
    let current_persona_name = Arc::new(tokio::sync::RwLock::new(None));
    let template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>> =
        Arc::new(tokio::sync::RwLock::new(None));
    let tools = build_tools(
        skills.clone(),
        skill_filter.clone(),
        template_ctx.clone(),
        personas_arc.clone(),
        pending_persona_switch.clone(),
        current_persona_name.clone(),
        crate::commands::config::hashline_enabled_for(cfg, provider_id),
    );

    let flagged_files: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let mut tools = tools;
    tools.push(Arc::new(FlagImportant::new(flagged_files.clone())));

    let permission_engine = build_permission_engine(cfg, mode);

    let mut agent = Agent::new(provider, dispatcher.clone(), writer, tools, session_id);
    agent.set_model_info(model_id, provider_id);
    agent.template_ctx = template_ctx;
    agent.flagged_files = flagged_files;
    agent.secrets = build_secret_set(cfg);
    agent.todos_path = todos_path;
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
    wire_subagents(&mut agent, cfg, cat, provider_id, raw, dispatcher.clone());

    // Load project context and skills for system prompt.
    let ctx_loader = mew_context::Loader::new(&cwd);
    let ctx_files = ctx_loader.load().unwrap_or_default();
    let project_vars = mew_context::load_project_vars(&cwd);
    agent.project_vars = project_vars;
    if !ctx_files.is_empty() {
        let rendered_ctx = render_templated_context_files(&ctx_files, &agent);
        agent.set_system(mew_context::build_system_prompt(&rendered_ctx));
    }
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

pub(crate) async fn connect_mcp_servers(
    configs: &[mew_mcp::McpServerConfig],
) -> (
    Vec<Arc<dyn mew_tools::Tool>>,
    Vec<Arc<McpClient>>,
    Vec<String>,
) {
    let mut tools: Vec<Arc<dyn mew_tools::Tool>> = Vec::new();
    let mut clients: Vec<Arc<McpClient>> = Vec::new();
    let mut status: Vec<String> = Vec::new();

    for cfg in configs {
        info!(server = %cfg.name, "connecting MCP server");

        let client = if let Some(ref url) = cfg.url {
            match McpClient::connect_http(&cfg.name, url).await {
                Ok(client) => {
                    status.push(format!("{} connected (http)", cfg.name));
                    client
                }
                Err(e) => {
                    status.push(format!("{} connection failed", cfg.name));
                    warn!(server = %cfg.name, error = %e, "MCP connect failed");
                    continue;
                }
            }
        } else if let Some(ref command) = cfg.command {
            match McpClient::connect_stdio(&cfg.name, command, &cfg.args).await {
                Ok(client) => {
                    status.push(format!("{} connected (stdio)", cfg.name));
                    client
                }
                Err(e) => {
                    status.push(format!("{} connection failed", cfg.name));
                    warn!(server = %cfg.name, error = %e, "MCP connect failed");
                    continue;
                }
            }
        } else {
            status.push(format!("{} skipped (no url or command)", cfg.name));
            warn!(server = %cfg.name, "MCP server has no url or command");
            continue;
        };

        match client.list_tools().await {
            Ok(mcp_tools) => {
                let count = mcp_tools.len();
                let client = Arc::new(client);
                for def in &mcp_tools {
                    tools.push(Arc::new(mew_mcp::McpTool::new(
                        &cfg.name,
                        def,
                        client.clone(),
                    )));
                }
                clients.push(client);
                status.push(format!("{} ready ({} tools)", cfg.name, count));
                info!(server = %cfg.name, tool_count = count, "MCP tools registered");
            }
            Err(e) => {
                status.push(format!("{} tool listing failed", cfg.name));
                warn!(server = %cfg.name, error = %e, "failed to list MCP tools");
            }
        }
    }

    (tools, clients, status)
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
                ..Default::default()
            },
        );
        apply_catalog_pricing(&mut agent, Some(&cat), "test-model");
        assert_eq!(agent.input_price, 1.5);
        assert_eq!(agent.output_price, 6.0);
        assert_eq!(agent.cache_read_price, 0.15);
        assert_eq!(agent.cache_write_price, 2.25);
        assert_eq!(agent.reasoning_price, 9.0);
    }

    #[test]
    fn apply_catalog_pricing_none_catalog_no_panic() {
        let mut agent = make_agent();
        apply_catalog_pricing(&mut agent, None, "test-model");
        // Prices should remain at their default 0.0
        assert_eq!(agent.input_price, 0.0);
        assert_eq!(agent.output_price, 0.0);
    }

    #[test]
    fn apply_catalog_pricing_model_not_in_catalog() {
        let mut agent = make_agent();
        let cat = Catalog::empty();
        apply_catalog_pricing(&mut agent, Some(&cat), "nonexistent-model");
        assert_eq!(agent.input_price, 0.0);
        assert_eq!(agent.output_price, 0.0);
    }
}
