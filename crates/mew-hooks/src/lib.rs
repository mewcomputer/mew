use async_trait::async_trait;
use mew_message::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// Type aliases for PluginHost callbacks to keep clippy's type_complexity happy.
type NotifyFn = Arc<dyn Fn(String) + Send + Sync>;
type ConfigReadFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
type LogFn = Arc<dyn Fn(String) + Send + Sync>;
type StorageReadFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
type StorageWriteFn = Arc<dyn Fn(&str, &str) + Send + Sync>;
type StorageDeleteFn = Arc<dyn Fn(&str) + Send + Sync>;
type SetUiFn = Arc<dyn Fn(&str, &str) + Send + Sync>;
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// All hook points in the dispatcher, as a single source of truth.
///
/// Eliminates the three places hook names used to live as separate string
/// literals (Rust method names, JSON-RPC wire names, and config keys).
/// Each variant knows its wire name and config name; the `as_config()`
/// value matches what plugins put in `disabled_hooks` and `matchers`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookId {
    PreModelTurn,
    Stop,
    PreCompaction,
    PostCompaction,
    TurnEnd,
    SystemPrompt,
    ChatMessage,
    ChatParams,
    ChatHeaders,
    ToolExecuteBefore,
    ToolExecuteAfter,
    PermissionAsk,
    ShellEnv,
    ProviderEvent,
    ToolError,
    SubagentStart,
    SubagentEnd,
    UserInput,
    PersonaChange,
    SessionSave,
    ModelFinish,
}

impl HookId {
    /// All variants, for iteration in validation.
    pub const ALL: &'static [HookId] = &[
        Self::PreModelTurn,
        Self::Stop,
        Self::PreCompaction,
        Self::PostCompaction,
        Self::TurnEnd,
        Self::SystemPrompt,
        Self::ChatMessage,
        Self::ChatParams,
        Self::ChatHeaders,
        Self::ToolExecuteBefore,
        Self::ToolExecuteAfter,
        Self::PermissionAsk,
        Self::ShellEnv,
        Self::ProviderEvent,
        Self::ToolError,
        Self::SubagentStart,
        Self::SubagentEnd,
    ];

    /// The JSON-RPC method name sent to subprocess plugins.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::PreModelTurn => "on-pre-model-turn",
            Self::Stop => "on-stop",
            Self::PreCompaction => "on-pre-compaction",
            Self::PostCompaction => "on-post-compaction",
            Self::TurnEnd => "on-turn-end",
            Self::SystemPrompt => "on-system-prompt",
            Self::ChatMessage => "on-chat-message",
            Self::ChatParams => "on-chat-params",
            Self::ChatHeaders => "on-chat-headers",
            Self::ToolExecuteBefore => "on-tool-execute-before",
            Self::ToolExecuteAfter => "on-tool-execute-after",
            Self::PermissionAsk => "on-permission-ask",
            Self::ShellEnv => "on-shell-env",
            Self::ProviderEvent => "on-provider-event",
            Self::ToolError => "on-tool-error",
            Self::SubagentStart => "on-subagent-start",
            Self::SubagentEnd => "on-subagent-end",
            Self::UserInput => "on-user-input",
            Self::PersonaChange => "on-persona-change",
            Self::SessionSave => "on-session-save",
            Self::ModelFinish => "on-model-finish",
        }
    }

    /// The config key used in `disabled_hooks` and `matchers`. Matches
    /// the Rust method name on the Dispatcher trait.
    pub fn as_config(self) -> &'static str {
        match self {
            Self::PreModelTurn => "on_pre_model_turn",
            Self::Stop => "on_stop",
            Self::PreCompaction => "on_pre_compaction",
            Self::PostCompaction => "on_post_compaction",
            Self::TurnEnd => "on_turn_end",
            Self::SystemPrompt => "on_system_prompt",
            Self::ChatMessage => "on_chat_message",
            Self::ChatParams => "on_chat_params",
            Self::ChatHeaders => "on_chat_headers",
            Self::ToolExecuteBefore => "on_tool_execute_before",
            Self::ToolExecuteAfter => "on_tool_execute_after",
            Self::PermissionAsk => "on_permission_ask",
            Self::ShellEnv => "on_shell_env",
            Self::ProviderEvent => "on_provider_event",
            Self::ToolError => "on_tool_error",
            Self::SubagentStart => "on_subagent_start",
            Self::SubagentEnd => "on_subagent_end",
            Self::UserInput => "on_user_input",
            Self::PersonaChange => "on_persona_change",
            Self::SessionSave => "on_session_save",
            Self::ModelFinish => "on_model_finish",
        }
    }
}

impl fmt::Display for HookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_config())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatParams {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub call_id: String,
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub output: String,
    pub error: String,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
    Prompt,
}

/// Outcome of a *blocking* hook — a hook that can not only mutate the
/// incoming value but also gate whether the action proceeds at all.
///
/// The transformation hooks (`on_chat_message`, `on_system_prompt`,
/// `on_tool_execute_after`, `on_shell_env`, `on_user_input`, etc.) still
/// return their transformed value directly — they're "see and rewrite"
/// hooks. The blocking hooks (`on_permission_ask`, `on_tool_execute_before`)
/// return `HookOutcome<T>` so plugins can veto the action entirely.
///
/// Variants:
/// - **`Proceed(value)`** — let the action run; `value` is the (possibly
///   modified) input the action will see.
/// - **`Block(reason)`** — don't run the action. `reason` is surfaced to
///   the user (permission modal) or to the model (tool error) and logged.
/// - **`Suppress`** — don't run, don't log, don't surface. Use this for
///   telemetry hooks that want to silently drop an action without revealing
///   that the action was attempted.
///
/// The `Retry` variant from the parity doc is intentionally not included —
/// there's no concrete use case yet and adding it would expose API surface
/// that isn't wired to anything. Add when there's a real retry path.
#[derive(Debug, Clone, PartialEq)]
pub enum HookOutcome<T> {
    Proceed(T),
    Block(String),
    Suppress,
}

impl<T> HookOutcome<T> {
    /// Convenience for the common case: proceed without modification.
    pub fn proceed(value: T) -> Self {
        HookOutcome::Proceed(value)
    }

    /// True if this outcome would let the action run.
    pub fn is_proceed(&self) -> bool {
        matches!(self, HookOutcome::Proceed(_))
    }

    /// True if this outcome blocks the action (Block or Suppress).
    pub fn is_blocked(&self) -> bool {
        !self.is_proceed()
    }

    /// Map the inner value when Proceed; pass Block/Suppress through.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> HookOutcome<U> {
        match self {
            HookOutcome::Proceed(v) => HookOutcome::Proceed(f(v)),
            HookOutcome::Block(r) => HookOutcome::Block(r),
            HookOutcome::Suppress => HookOutcome::Suppress,
        }
    }
}

#[cfg(test)]
mod hook_outcome_tests {
    use super::*;

    #[test]
    fn test_proceed_is_not_blocked() {
        let o: HookOutcome<i32> = HookOutcome::Proceed(42);
        assert!(o.is_proceed());
        assert!(!o.is_blocked());
    }

    #[test]
    fn test_block_is_blocked() {
        let o: HookOutcome<i32> = HookOutcome::Block("nope".into());
        assert!(!o.is_proceed());
        assert!(o.is_blocked());
    }

    #[test]
    fn test_suppress_is_blocked() {
        let o: HookOutcome<i32> = HookOutcome::Suppress;
        assert!(!o.is_proceed());
        assert!(o.is_blocked());
    }

    #[test]
    fn test_map_proceed_applies_fn() {
        let o: HookOutcome<i32> = HookOutcome::Proceed(5);
        let mapped = o.map(|x| x * 2);
        assert_eq!(mapped, HookOutcome::Proceed(10));
    }

    #[test]
    fn test_map_block_passes_through() {
        let o: HookOutcome<i32> = HookOutcome::Block("reason".into());
        let mapped = o.map(|x| x * 2);
        assert_eq!(mapped, HookOutcome::Block("reason".into()));
    }

    #[test]
    fn test_map_suppress_passes_through() {
        let o: HookOutcome<i32> = HookOutcome::Suppress;
        let mapped: HookOutcome<i32> = o.map(|x| x * 2);
        assert_eq!(mapped, HookOutcome::Suppress);
    }

    #[test]
    fn test_proceed_helper() {
        let o = HookOutcome::proceed("hello");
        assert_eq!(o, HookOutcome::Proceed("hello"));
    }
}

/// Runtime permission mode. Controls how `PermissionEngine::check` short-circuits
/// before reaching the normal rule cascade.
///
/// Five modes form a permission slider from most to least restrictive:
///
/// - **Standard** — default. Mutating/Dangerous tools prompt; deny rules,
///   ask rules, secret-file guards, and bash decomposition all fire.
/// - **Permissive** — Mutating tools (write/edit/switch_persona/job_cancel)
///   auto-allow; Dangerous tools (bash/shell_background/shell_monitor) still
///   prompt; user-configured deny rules, ask rules, secret-file guards, and
///   bash decomposition all still fire. "I trust the agent with file edits,
///   but bash and my safety rules still apply."
/// - **Auto** — every tool call is routed through a small/cheap LLM
///   classifier instead of the user. The classifier returns allow / deny /
///   escalate; escalate falls back to the user modal. Deny rules, ask rules,
///   secret-file guards, and bash decomposition are all skipped — the
///   classifier is the only gate. "Don't interrupt me; let the model decide."
///   Requires a classifier provider to be configured; without one, Auto
///   mode is a no-op and falls through to the user modal.
/// - **Auto+** — like Auto, but the classifier CANNOT escalate. If the
///   classifier returns "escalate" or the call fails (provider error /
///   timeout / malformed response), the call is **denied** — fail closed.
///   Use this when you want hands-off but you also don't want a provider
///   outage to silently run `rm -rf`. The classifier is the only gate, but
///   uncertainty means no.
/// - **Dangerous!** — every tool auto-runs; no prompts, no rule checks, no
///   secret-file guards, no bash decomposition. The user is explicitly
///   opting into "don't ask me anything, even the things I said don't do."
///   Secret redaction in tool output still applies (defense in depth).
///
/// Set via the `/permissions` slash command, the `--dangerously-skip-permissions`
/// / `--permissive` / `--auto` / `--auto-plus` CLI flags, or the
/// `MEW_DANGEROUS` / `MEW_PERMISSIVE` / `MEW_AUTO` / `MEW_AUTO_PLUS` env vars.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Default. Tools go through the normal cascade (deny → allow → ask →
    /// session-allow → sensitivity default). Mutating/Dangerous tools prompt.
    #[default]
    Standard,
    /// Auto-allow Mutating tools; still prompt on Dangerous sensitivity;
    /// respect deny/ask rules, secret-file guard, and bash decomposition.
    Permissive,
    /// Route every tool call through the classifier LLM. Classifier
    /// returns allow / deny / escalate; escalate falls back to user.
    /// All other permission tiers (rules, secret guard, bash decomp) are
    /// skipped — the classifier is the gate.
    Auto,
    /// Route every tool call through the classifier LLM, but the
    /// classifier CANNOT escalate. Escalate / failure → Deny (fail closed).
    AutoPlus,
    /// Override EVERYTHING. Every tool auto-runs; no prompts, no rule checks,
    /// no secret-file guards, no bash decomposition. The user has explicitly
    /// opted into "no holds barred." Output redaction still applies.
    Dangerous,
}

impl PermissionMode {
    /// Parse from the lowercase id used in slash-command / CLI serialization.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "standard" => Some(PermissionMode::Standard),
            "permissive" => Some(PermissionMode::Permissive),
            "auto" => Some(PermissionMode::Auto),
            "auto_plus" | "autoplus" => Some(PermissionMode::AutoPlus),
            "dangerous" => Some(PermissionMode::Dangerous),
            _ => None,
        }
    }

    /// Stable lowercase id for serialization and picker `id` fields.
    pub fn id(&self) -> &'static str {
        match self {
            PermissionMode::Standard => "standard",
            PermissionMode::Permissive => "permissive",
            PermissionMode::Auto => "auto",
            PermissionMode::AutoPlus => "auto_plus",
            PermissionMode::Dangerous => "dangerous",
        }
    }

    /// Human-readable label for the picker. Includes a short risk cue so the
    /// user knows what they're picking.
    pub fn picker_label(&self) -> &'static str {
        match self {
            PermissionMode::Standard => "Standard",
            PermissionMode::Permissive => "Permissive",
            PermissionMode::Auto => "Auto",
            PermissionMode::AutoPlus => "Auto+",
            PermissionMode::Dangerous => "Dangerous!",
        }
    }
}

/// Per-plugin hook configuration. Allows scoping which hooks fire, what
/// they match against, and how long they can run — without disabling the
/// plugin entirely.
///
/// In config.toml:
/// ```toml
/// [plugins.my-plugin]
/// disabled_hooks = ["on_turn_end"]
/// timeout_ms = 10000
///
/// [plugins.my-plugin.matchers]
/// on_tool_execute_before = "bash|write|edit"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginHookConfig {
    /// Hooks to skip for this plugin. Hook names use the snake_case form
    /// (`on_turn_end`, `on_system_prompt`, etc.) or `*` to disable all.
    #[serde(default)]
    pub disabled_hooks: Vec<String>,
    /// Per-plugin timeout override in milliseconds. Falls back to the
    /// global `MEW_PLUGIN_TIMEOUT_MS` or 5s default when unset.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Per-hook matchers. Keyed by hook name. The value is a pipe-separated
    /// list matched against the hook's subject:
    /// - Tool hooks (`on_tool_execute_before`, `on_tool_execute_after`,
    ///   `on_permission_ask`): matched against the tool name.
    /// - Other hooks: no subject; matcher is ignored.
    ///
    /// Example: `"bash|write|edit"` fires only for those three tools.
    #[serde(default)]
    pub matchers: HashMap<String, String>,
}

impl PluginHookConfig {
    /// True if `hook` is disabled for this plugin.
    pub fn is_hook_disabled(&self, hook: &str) -> bool {
        self.disabled_hooks.iter().any(|h| h == hook || h == "*")
    }

    /// True if `hook` should fire for the given `subject` (e.g. tool name).
    /// Returns `true` when no matcher is configured for the hook (fire by
    /// default) or the subject matches the pipe-separated pattern.
    ///
    /// Supports `!`-prefix negation:
    /// - `"bash"` → fire only for bash
    /// - `"bash|write"` → fire for bash or write
    /// - `"!bash"` → fire for everything except bash
    /// - `"!bash|!write"` → fire for everything except bash and write
    /// - `"bash|write|!rm"` → fire for bash or write, but never rm
    pub fn matches(&self, hook: &str, subject: &str) -> bool {
        match self.matchers.get(hook) {
            None => true,
            Some(pattern) => {
                let entries: Vec<&str> = pattern.split('|').map(|p| p.trim()).collect();

                // Check if any negative entry excludes the subject.
                if entries.iter().any(|&p| {
                    p.strip_prefix('!')
                        .is_some_and(|n| n.trim() == subject || n.trim() == "*")
                }) {
                    return false;
                }

                // If all entries are negative, subject is included by default.
                if entries.iter().all(|p| p.starts_with('!')) {
                    return true;
                }

                // Positive entries define the allowed set.
                entries
                    .iter()
                    .any(|&p| !p.starts_with('!') && (p == subject || p == "*"))
            }
        }
    }

    /// Warn about unknown hook names in the config. Called once at startup
    /// after loading config.toml to help users catch typos.
    pub fn validate(&self, plugin_name: &str) {
        let known: Vec<&str> = HookId::ALL.iter().map(|h| h.as_config()).collect();
        for h in &self.disabled_hooks {
            if h != "*" && !known.contains(&h.as_str()) {
                tracing::warn!(
                    plugin = plugin_name,
                    hook = %h,
                    "unknown hook in disabled_hooks — possible typo"
                );
            }
        }
        for h in self.matchers.keys() {
            if !known.contains(&h.as_str()) {
                tracing::warn!(
                    plugin = plugin_name,
                    hook = %h,
                    "unknown hook in matchers — possible typo"
                );
            }
        }
    }
}

/// A dynamically registered tool from a plugin.
pub struct ToolRegistration {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: Value,
    /// Called when the model invokes this tool. Returns the tool's output text.
    /// Async so both in-process plugins (wrap in async block) and subprocess
    /// plugins (JSON-RPC call) can implement it.
    pub execute: Box<dyn Fn(Value) -> BoxFuture<String> + Send + Sync>,
}

/// A dynamically registered slash command from a plugin (m10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommandDef {
    pub name: String,
    pub description: String,
    pub handler_id: String,
}

/// Host handle plugins receive during init. Provides session-context access.
#[derive(Clone)]
pub struct PluginHost {
    /// Pushes a non-modal alert to the TUI (supports CC Notification hook in m11).
    pub notify: NotifyFn,
    /// Read-only access to a safe subset of config values (known keys only).
    pub config_read: ConfigReadFn,
    /// Restricted log channel (prefixed with plugin name, rate-limited).
    pub log: LogFn,
    /// Read a per-plugin persistent value. Key is namespaced to the plugin.
    pub storage_read: StorageReadFn,
    /// Write a per-plugin persistent value (stored to disk).
    pub storage_write: StorageWriteFn,
    /// Delete a per-plugin persistent value.
    pub storage_delete: StorageDeleteFn,
    /// Push named text content to the TUI for rendering. The TUI exposes plugin
    /// data via a designated area beside the input prompt.
    /// Keys are plugin-namespaced: the format is `"plugin-name/key"`.
    /// The TUI reads `app.plugin_ui` during render.
    pub set_ui: SetUiFn,
}

#[async_trait]
pub trait Dispatcher: Send + Sync {
    /// Lifecycle: called at session startup with the host handle.
    async fn init(&self, host: &PluginHost);

    /// Lifecycle: called at session shutdown.
    async fn shutdown(&self);

    /// Dynamic tool registration: called at startup after init and on plugin
    /// reload. Returns additional tools to merge into the agent's tool registry.
    async fn on_register_tools(&self) -> Vec<ToolRegistration>;

    /// Dynamic slash command registration (m10): returns slash commands to
    /// merge into the TUI's command registry.
    async fn on_register_slash_commands(&self) -> Vec<SlashCommandDef>;

    /// Execute a previously registered slash command (m10). Called by the TUI
    /// when the user invokes a plugin-registered slash command. Returns the
    /// result text to display, or None to fall through to the model.
    async fn execute_slash_command(&self, command: &str, args: &str) -> Option<String>;

    /// Forward a provider stream event to plugins. Replaces the old
    /// `on_event(&dyn Any)` (broken for subprocess dispatch). Plugins can
    /// observe the stream — useful for logging, metrics, or "what is the
    /// model doing right now" UIs.
    async fn on_provider_event(&self, ev: &mew_provider::ProviderEvent);

    /// Called when a tool call terminates with an error. Distinct from
    /// `on_tool_execute_after` (which fires for both success and failure)
    /// so plugins can react specifically to failures — log them, update a
    /// metrics counter, or surface a notification.
    async fn on_tool_error(&self, call: &ToolCall, error: &str);

    /// Called when a subagent starts. `display_name` is the human-friendly
    /// name picked by the runner (e.g. "Curie (researcher)"), if any.
    async fn on_subagent_start(&self, name: &str, parent_call_id: &str, display_name: Option<&str>);

    /// Called when a subagent finishes. `outcome` is a short string
    /// describing the result ("completed", "failed: <reason>",
    /// "cancelled").
    async fn on_subagent_end(&self, name: &str, parent_call_id: &str, outcome: &str);

    /// Turn-grain observer: fires after each assistant turn completes
    /// (tool results pushed, about to loop back or terminate). Errors logged,
    /// never propagated. Fire-and-forget — must not block the turn loop.
    async fn on_turn_end(&self, messages: &[Message]);

    /// Called before each model turn (LLM request). Fire-and-forget —
    /// plugins can observe what's about to be sent to the model but must
    /// not block the turn loop. Maps to polytoken's `pre_model_turn`.
    async fn on_pre_model_turn(&self, messages: &[Message], system: &str);

    /// Called when the session is about to stop (user quit, max turns
    /// reached, or the agent loop exits). Fire-and-forget. Maps to
    /// polytoken's `stop`.
    async fn on_stop(&self);

    /// Called before context compaction runs. Receives the full message
    /// list that's about to be compacted. Fire-and-forget — lets plugins
    /// save important context or prepare for the compaction. Maps to
    /// polytoken's `pre_compaction`.
    async fn on_pre_compaction(&self, messages: &[Message]);

    /// Called after context compaction completes. Receives the compacted
    /// messages. Fire-and-forget. Maps to polytoken's `post_compaction`.
    async fn on_post_compaction(&self, messages: &[Message]);

    /// Mutation hooks. Each returns the (possibly modified) value.
    /// Errors fall back to the input unchanged and are logged.
    async fn on_chat_message(&self, msg: Message) -> Message;
    async fn on_chat_params(&self, p: ChatParams) -> ChatParams;
    async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap;
    /// Called when the system prompt is assembled, before it's sent to the
    /// model. Plugins may prepend, append, or replace sections. Called every
    /// turn (system prompt is rebuilt from scratch each turn).
    async fn on_system_prompt(&self, prompt: String) -> String;
    async fn on_tool_execute_before(&self, call: &ToolCall, input: Value) -> HookOutcome<Value>;
    async fn on_tool_execute_after(&self, call: &ToolCall, output: ToolOutput) -> ToolOutput;
    async fn on_permission_ask(
        &self,
        call: &ToolCall,
        current: PermissionDecision,
    ) -> HookOutcome<PermissionDecision>;
    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String>;

    /// Called when the user submits a prompt, before it reaches the agent.
    /// Mutation hook: plugins can rewrite or annotate the input (e.g.
    /// prepend context, expand @mentions, inject git status). The
    /// returned string is what the agent receives.
    async fn on_user_input(&self, prompt: String) -> String;

    /// Called when the active persona changes. `old_persona` is the
    /// previous persona name (or `None` if there was none). `new_persona`
    /// is the name of the persona being activated (or `"default"` /
    /// `"none"` when clearing). Fire-and-forget.
    async fn on_persona_change(&self, old_persona: Option<&str>, new_persona: &str);

    /// Called when the session is being saved (on quit or manual save).
    /// Plugins can flush any in-memory state to their persistent storage
    /// (via the PluginHost storage callbacks) before the session file is
    /// finalized. Fire-and-forget.
    async fn on_session_save(&self);

    /// Called when the model finishes a response. More specific than
    /// `on_provider_event` (which fires for every stream delta) — this
    /// fires once per response with the finish reason, token usage, and
    /// cost. Perfect for metrics/telemetry plugins (OpenTelemetry,
    /// Prometheus, etc.). Fire-and-forget.
    async fn on_model_finish(&self, finish: &str, input_tokens: u32, output_tokens: u32, cost: f64);
}

pub struct NopDispatcher;

#[async_trait]
impl Dispatcher for NopDispatcher {
    async fn init(&self, _: &PluginHost) {}
    async fn shutdown(&self) {}
    async fn on_register_tools(&self) -> Vec<ToolRegistration> {
        vec![]
    }
    async fn on_register_slash_commands(&self) -> Vec<SlashCommandDef> {
        vec![]
    }
    async fn execute_slash_command(&self, _command: &str, _args: &str) -> Option<String> {
        None
    }
    async fn on_provider_event(&self, _ev: &mew_provider::ProviderEvent) {}
    async fn on_tool_error(&self, _call: &ToolCall, _error: &str) {}
    async fn on_subagent_start(
        &self,
        _name: &str,
        _parent_call_id: &str,
        _display_name: Option<&str>,
    ) {
    }
    async fn on_subagent_end(&self, _name: &str, _parent_call_id: &str, _outcome: &str) {}
    async fn on_turn_end(&self, _messages: &[Message]) {}
    async fn on_pre_model_turn(&self, _messages: &[Message], _system: &str) {}
    async fn on_stop(&self) {}
    async fn on_pre_compaction(&self, _messages: &[Message]) {}
    async fn on_post_compaction(&self, _messages: &[Message]) {}
    async fn on_chat_message(&self, msg: Message) -> Message {
        msg
    }
    async fn on_chat_params(&self, p: ChatParams) -> ChatParams {
        p
    }
    async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap {
        h
    }
    async fn on_system_prompt(&self, prompt: String) -> String {
        prompt
    }
    async fn on_tool_execute_before(&self, _call: &ToolCall, input: Value) -> HookOutcome<Value> {
        HookOutcome::Proceed(input)
    }
    async fn on_tool_execute_after(&self, _call: &ToolCall, output: ToolOutput) -> ToolOutput {
        output
    }
    async fn on_permission_ask(
        &self,
        _call: &ToolCall,
        current: PermissionDecision,
    ) -> HookOutcome<PermissionDecision> {
        HookOutcome::Proceed(current)
    }
    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String> {
        env
    }
    async fn on_user_input(&self, prompt: String) -> String {
        prompt
    }
    async fn on_persona_change(&self, _old_persona: Option<&str>, _new_persona: &str) {}
    async fn on_session_save(&self) {}
    async fn on_model_finish(
        &self,
        _finish: &str,
        _input_tokens: u32,
        _output_tokens: u32,
        _cost: f64,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_nop_init_shutdown() {
        let host = PluginHost {
            notify: Arc::new(|_| {}),
            config_read: Arc::new(|_| None),
            log: Arc::new(|_| {}),
            storage_read: Arc::new(|_| None),
            storage_write: Arc::new(|_, _| {}),
            storage_delete: Arc::new(|_| {}),
            set_ui: Arc::new(|_, _| {}),
        };
        let nop = NopDispatcher;
        nop.init(&host).await;
        nop.shutdown().await;
    }

    #[tokio::test]
    async fn test_nop_on_register_slash_commands_returns_empty() {
        assert!(NopDispatcher.on_register_slash_commands().await.is_empty());
    }

    #[tokio::test]
    async fn test_nop_on_turn_end_does_not_panic() {
        NopDispatcher.on_turn_end(&[]).await;
    }

    #[tokio::test]
    async fn test_nop_on_system_prompt_passthrough() {
        let prompt = "system instructions".to_string();
        let result = NopDispatcher.on_system_prompt(prompt.clone()).await;
        assert_eq!(result, prompt);
    }

    #[tokio::test]
    async fn test_nop_execute_slash_command_returns_none() {
        assert!(NopDispatcher
            .execute_slash_command("/buddy", "pet")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_nop_on_pre_model_turn_passthrough() {
        // Fire-and-forget; just verify it doesn't panic.
        NopDispatcher.on_pre_model_turn(&[], "system prompt").await;
    }

    #[tokio::test]
    async fn test_nop_on_stop_does_not_panic() {
        NopDispatcher.on_stop().await;
    }

    #[tokio::test]
    async fn test_nop_on_compaction_hooks_do_not_panic() {
        NopDispatcher.on_pre_compaction(&[]).await;
        NopDispatcher.on_post_compaction(&[]).await;
    }

    #[test]
    fn test_plugin_hook_config_disabled() {
        let cfg = PluginHookConfig {
            disabled_hooks: vec!["on_turn_end".into()],
            ..Default::default()
        };
        assert!(cfg.is_hook_disabled("on_turn_end"));
        assert!(!cfg.is_hook_disabled("on_system_prompt"));
    }

    #[test]
    fn test_plugin_hook_config_disable_all() {
        let cfg = PluginHookConfig {
            disabled_hooks: vec!["*".into()],
            ..Default::default()
        };
        assert!(cfg.is_hook_disabled("on_turn_end"));
        assert!(cfg.is_hook_disabled("on_system_prompt"));
        assert!(cfg.is_hook_disabled("anything"));
    }

    #[test]
    fn test_plugin_hook_config_matcher_fires_by_default() {
        let cfg = PluginHookConfig::default();
        // No matcher → fires for everything.
        assert!(cfg.matches("on_tool_execute_before", "bash"));
        assert!(cfg.matches("on_tool_execute_before", "read"));
    }

    #[test]
    fn test_plugin_hook_config_matcher_filters() {
        let mut matchers = HashMap::new();
        matchers.insert("on_tool_execute_before".into(), "bash|write|edit".into());
        let cfg = PluginHookConfig {
            matchers,
            ..Default::default()
        };
        assert!(cfg.matches("on_tool_execute_before", "bash"));
        assert!(cfg.matches("on_tool_execute_before", "write"));
        assert!(!cfg.matches("on_tool_execute_before", "read"));
        // Other hooks are unaffected.
        assert!(cfg.matches("on_system_prompt", "anything"));
    }

    #[test]
    fn test_plugin_hook_config_matcher_wildcard() {
        let mut matchers = HashMap::new();
        matchers.insert("on_permission_ask".into(), "*".into());
        let cfg = PluginHookConfig {
            matchers,
            ..Default::default()
        };
        assert!(cfg.matches("on_permission_ask", "bash"));
        assert!(cfg.matches("on_permission_ask", "anything"));
    }

    #[test]
    fn test_matcher_negation_single_exclude() {
        let mut matchers = HashMap::new();
        matchers.insert("on_tool_execute_before".into(), "!bash".into());
        let cfg = PluginHookConfig {
            matchers,
            ..Default::default()
        };
        assert!(!cfg.matches("on_tool_execute_before", "bash"), "!bash must exclude bash");
        assert!(cfg.matches("on_tool_execute_before", "read"), "!bash must include read");
        assert!(cfg.matches("on_tool_execute_before", "write"), "!bash must include write");
    }

    #[test]
    fn test_matcher_negation_multiple_excludes() {
        let mut matchers = HashMap::new();
        matchers.insert("on_tool_execute_before".into(), "!bash|!write".into());
        let cfg = PluginHookConfig {
            matchers,
            ..Default::default()
        };
        assert!(!cfg.matches("on_tool_execute_before", "bash"));
        assert!(!cfg.matches("on_tool_execute_before", "write"));
        assert!(cfg.matches("on_tool_execute_before", "read"));
        assert!(cfg.matches("on_tool_execute_before", "edit"));
    }

    #[test]
    fn test_matcher_mixed_positive_and_negative() {
        // "bash|write|!rm" = fire for bash or write, but never rm.
        let mut matchers = HashMap::new();
        matchers.insert("on_tool_execute_before".into(), "bash|write|!rm".into());
        let cfg = PluginHookConfig {
            matchers,
            ..Default::default()
        };
        assert!(cfg.matches("on_tool_execute_before", "bash"));
        assert!(cfg.matches("on_tool_execute_before", "write"));
        assert!(!cfg.matches("on_tool_execute_before", "rm"));
        assert!(!cfg.matches("on_tool_execute_before", "read"), "read not in positives");
    }

    #[test]
    fn test_matcher_negation_wildcard() {
        // "!*" = exclude everything.
        let mut matchers = HashMap::new();
        matchers.insert("on_tool_execute_before".into(), "!*".into());
        let cfg = PluginHookConfig {
            matchers,
            ..Default::default()
        };
        assert!(!cfg.matches("on_tool_execute_before", "bash"));
        assert!(!cfg.matches("on_tool_execute_before", "anything"));
    }

    #[test]
    fn test_plugin_hook_config_serde() {
        let json = serde_json::json!({
            "disabled_hooks": ["on_turn_end", "on_event"],
            "timeout_ms": 15000,
            "matchers": {
                "on_tool_execute_before": "bash|write"
            }
        });
        let cfg: PluginHookConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.disabled_hooks, vec!["on_turn_end", "on_event"]);
        assert_eq!(cfg.timeout_ms, Some(15000));
        assert!(cfg.matches("on_tool_execute_before", "bash"));
        assert!(!cfg.matches("on_tool_execute_before", "read"));
        assert!(cfg.is_hook_disabled("on_turn_end"));
    }

    #[tokio::test]
    async fn test_slash_command_def_serialization() {
        let cmd = SlashCommandDef {
            name: "/buddy".into(),
            description: "pet companion".into(),
            handler_id: "buddy-handler".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: SlashCommandDef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "/buddy");
        assert_eq!(parsed.description, "pet companion");
        assert_eq!(parsed.handler_id, "buddy-handler");
    }
}
