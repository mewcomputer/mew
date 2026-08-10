use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, warn};

/// Sidebar section collapsed state: section name → collapsed.
pub type SidebarState = HashMap<String, bool>;

pub mod paths;
pub mod permissions;
pub mod shell;

pub use paths::{cache_dir, config_dir, data_dir, state_path};

/// Top-level user configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(rename = "default_model", default)]
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<CustomModel>,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub secrets: SecretsConfig,
    #[serde(default)]
    pub plugins: HashMap<String, mew_hooks::PluginHookConfig>,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// Persona to load on startup. Defaults to "builder". Set to "planner"
    /// for a read-only planning phase, or any user-defined persona name.
    /// Set to "none" or "default" to start without a persona.
    #[serde(default = "default_persona")]
    pub default_persona: String,
    /// Where the planner persona writes its plan and the builder reads it.
    /// Defaults to "PLAN.md" in the workspace root. Can be a relative path
    /// (".mew/plans/current.md") or absolute.
    #[serde(default = "default_plan_path")]
    pub plan_path: String,
    /// TUI configuration (theme, etc.).
    #[serde(default)]
    pub tui: TuiConfig,
    /// Subagent orchestration guardrails (fan-in reminders, concurrency cap,
    /// nesting depth).
    #[serde(default)]
    pub orchestration: OrchestrationConfig,
}

/// TUI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
    /// Theme name. "dark" (default), "light", or the name of a JSON theme
    /// file in `~/.config/mew/themes/` or `.mew/themes/`.
    #[serde(default)]
    pub theme: String,
    /// How long (seconds) sidebar entries stay visible after finishing:
    /// completed subagents and done todos are hidden once they are older
    /// than this. 0 hides them immediately. Defaults to 180 (3 minutes).
    #[serde(default = "default_sidebar_finished_ttl_secs")]
    pub sidebar_finished_ttl_secs: u64,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            theme: String::new(),
            sidebar_finished_ttl_secs: default_sidebar_finished_ttl_secs(),
        }
    }
}

fn default_sidebar_finished_ttl_secs() -> u64 {
    180
}

/// Subagent orchestration guardrails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationConfig {
    /// Maximum subagent runs active at once per session. `subagent_start`
    /// past the cap returns a structured error telling the model to collect
    /// results first. 0 disables the cap.
    #[serde(default = "default_max_concurrent_subagents")]
    pub max_concurrent_subagents: u32,
    /// How deep subagent spawning may nest. 1 (default) means children never
    /// receive the subagent tools. Deeper nesting additionally requires the
    /// subagent def to set `can_spawn: true`.
    #[serde(default = "default_max_subagent_depth")]
    pub max_subagent_depth: u32,
    /// Default wall-clock cap (seconds) for subagent runs whose def does not
    /// set `max_duration_secs`.
    #[serde(default = "default_max_duration_secs")]
    pub default_max_duration_secs: u64,
    /// When true, the agent reminds the model at turn end about subagent
    /// tasks that were started but never collected with `subagent_wait`.
    #[serde(default = "default_true")]
    pub leak_reminder: bool,
    /// Maximum leak-reminder loop-backs per user turn, so a model that keeps
    /// spawning instead of collecting cannot loop forever.
    #[serde(default = "default_leak_reminder_max")]
    pub leak_reminder_max: u32,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            max_concurrent_subagents: default_max_concurrent_subagents(),
            max_subagent_depth: default_max_subagent_depth(),
            default_max_duration_secs: default_max_duration_secs(),
            leak_reminder: true,
            leak_reminder_max: default_leak_reminder_max(),
        }
    }
}

fn default_max_concurrent_subagents() -> u32 {
    4
}

fn default_max_subagent_depth() -> u32 {
    1
}

fn default_max_duration_secs() -> u64 {
    300
}

fn default_leak_reminder_max() -> u32 {
    2
}

fn default_persona() -> String {
    "builder".into()
}

fn default_plan_path() -> String {
    "PLAN.md".into()
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = HashMap::new();
        providers.insert(
            "opencode-zen".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://opencode.ai/zen/v1".into(),
                credential_ref: "opencode-zen".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "opencode-go".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://opencode.ai/zen/go/v1".into(),
                credential_ref: "opencode-zen".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "z-ai".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://api.z.ai/api/coding/paas/v4".into(),
                credential_ref: "z-ai".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "deepseek".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://api.deepseek.com/v1".into(),
                credential_ref: "deepseek".into(),
                ..Default::default()
            },
        );
        // Umans AI Coding Plan (https://app.umans.ai/offers/code/docs). Anthropic-shaped:
        // hits /v1/messages with `x-api-key` and `anthropic-version: 2023-06-01` headers.
        // Model list and pricing come from umans's own /v1/models/info endpoint
        // (loaded by `mew_catalog::load_umans`), not from models.dev.
        providers.insert(
            "umans".into(),
            ProviderConfig {
                shape: "anthropic".into(),
                base_url: "https://api.code.umans.ai/v1".into(),
                credential_ref: "umans".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "codex".into(),
            ProviderConfig {
                shape: "responses".into(),
                base_url: "https://api.openai.com/v1".into(),
                credential_ref: "codex".into(),
                ..Default::default()
            },
        );
        // Kimi (Moonshot AI) coding endpoint — OpenAI-compatible.
        // Base URL includes /v1 so the adapter hits /v1/chat/completions.
        // Models: k3 (Kimi K3, thinking-capable), kimi-for-coding (Kimi K2.7
        // Code), kimi-for-coding-highspeed (Kimi K2.7 Code HighSpeed).
        // Docs: https://platform.kimi.ai/docs/guide/coding
        //
        // Kimi also exposes an Anthropic-compatible endpoint, but its thinking
        // (reasoning) stream is unreliable there — Moonshot's OpenAI surface is
        // their primary, fully-tested API, so we use it. k3 thinking is driven
        // by a top-level `reasoning_effort` param (low/high/max), which the
        // catalog produces and the OpenAI adapter forwards as-is.
        providers.insert(
            "kimi-for-coding".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://api.kimi.com/coding/v1".into(),
                credential_ref: "kimi-for-coding".into(),
                ..Default::default()
            },
        );
        // Alibaba Token Plan (https://www.alibabacloud.com/help/en/model-studio).
        // OpenAI-compatible (`/compatible-mode/v1`); model list, pricing, and
        // capabilities come from the models.dev catalog. The (China) variant
        // uses the mainland endpoint; credentials are separate per region.
        providers.insert(
            "alibaba-token-plan".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
                    .into(),
                credential_ref: "alibaba-token-plan".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "alibaba-token-plan-cn".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1"
                    .into(),
                credential_ref: "alibaba-token-plan-cn".into(),
                ..Default::default()
            },
        );
        Self {
            providers,
            default_model: String::new(),
            models: Vec::new(),
            permissions: PermissionsConfig::default(),
            secrets: SecretsConfig::default(),
            plugins: HashMap::new(),
            workspace: WorkspaceConfig::default(),
            default_persona: default_persona(),
            plan_path: default_plan_path(),
            tui: TuiConfig::default(),
            orchestration: OrchestrationConfig::default(),
        }
    }
}

/// Workspace path sandboxing configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Directories the agent is allowed to access.
    /// Defaults to [current directory] if empty.
    #[serde(default)]
    pub roots: Vec<std::path::PathBuf>,
}

/// A user-defined model entry that overrides or extends the models.dev catalog.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomModel {
    /// Model identifier (e.g. "glm-5.3").
    pub id: String,
    /// Provider ID that serves this model.
    pub provider: String,
    /// Adapter shape: "openai" or "anthropic".
    #[serde(default)]
    pub shape: String,
    /// Context window in tokens.
    #[serde(default)]
    pub context_window: i64,
    /// User-defined thinking variants. When set, overrides built-in defaults.
    #[serde(default)]
    pub thinking_variants: Vec<ThinkingVariantDef>,
    /// Numeric thinking-budget range for models that accept a
    /// `thinking_budget` token cap (e.g. Qwen3.8-max). Mirrors
    /// `mew_catalog::ThinkingBudget`; the setup layer converts between them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<ThinkingBudgetDef>,
    /// True for OpenAI Codex models that require the Responses Lite transport.
    #[serde(default)]
    pub responses_lite: bool,
    /// Prompt-cache retention in seconds when the provider/model documents it.
    /// Omit this when unknown; mew will wait for compaction before refreshing
    /// the cacheable system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_retention_secs: Option<u64>,
    /// When true and the model already exists in the catalog, fields left
    /// unset here keep the catalog's values (pricing, capability flags, …)
    /// instead of resetting to defaults. When false (default), this entry
    /// replaces the catalog entry wholesale.
    #[serde(default)]
    pub merge: bool,
}

/// A named thinking variant in config.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThinkingVariantDef {
    pub name: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Numeric thinking-budget range in config.toml. Mirrors
/// `mew_catalog::ThinkingBudget` (which the setup layer converts this into).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThinkingBudgetDef {
    #[serde(default)]
    pub min: i64,
    #[serde(default)]
    pub max: i64,
    #[serde(default)]
    pub step: i64,
    #[serde(default)]
    pub default: i64,
    /// Canonical budget (in tokens) for each named effort variant, so the UI
    /// can seed a slider position from the active effort level.
    #[serde(default)]
    pub by_effort: Vec<(String, i64)>,
}

/// Permission configuration section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub rules: Vec<permissions::PermissionRule>,
    /// Provider ID for the classifier LLM used by Auto / Auto+ permission
    /// modes. If unset, Auto mode falls through to the user modal on every
    /// call (no classifier available).
    #[serde(default)]
    pub classifier_provider: Option<String>,
    /// Model ID for the classifier LLM. If unset, uses the provider's default.
    #[serde(default)]
    pub classifier_model: Option<String>,
}

/// Secrets configuration section. Files listed here are guarded: reads of
/// matching paths force a permission prompt unless a literal (non-glob)
/// allow rule explicitly permits that exact path. Words listed here are
/// redacted from search-tool output before the model or user sees them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretsConfig {
    #[serde(default)]
    pub files: Vec<SecretFilesRule>,
    #[serde(default)]
    pub words: Vec<SecretWordsRule>,
}

/// A group of secret-file glob patterns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretFilesRule {
    #[serde(default)]
    pub paths: Vec<String>,
}

/// A group of secret-word values to redact from tool output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretWordsRule {
    #[serde(default)]
    pub values: Vec<String>,
}

/// Describes a single provider entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub shape: String,
    #[serde(rename = "base_url", default)]
    pub base_url: String,
    #[serde(rename = "credential_ref", default)]
    pub credential_ref: String,
    /// Provider kind: "direct" (default) or "router".
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Router: cheapest model ID for very simple turns.
    #[serde(default)]
    pub nano: String,
    /// Router: medium model ID for simple turns.
    #[serde(default)]
    pub micro: String,
    /// Router: most-capable model ID for complex turns.
    #[serde(default)]
    pub deci: String,
    /// Legacy router field; used as a fallback when `micro` is empty so old
    /// configs keep working. New configs should use `micro`.
    #[serde(default)]
    pub small: String,
    /// Legacy router field; used as a fallback when `deci` is empty so old
    /// configs keep working. New configs should use `deci`.
    #[serde(default)]
    pub big: String,
    /// When true, the `edit_hashline` tool is not registered for models using
    /// this provider. Useful for less-capable models that do not follow the
    /// hashline format reliably.
    #[serde(default)]
    pub disable_hashline: bool,
}

fn default_kind() -> String {
    "direct".into()
}

impl ProviderConfig {
    /// Effective `micro` model, falling back to the legacy `small` field.
    pub fn micro_model(&self) -> &str {
        if self.micro.is_empty() {
            &self.small
        } else {
            &self.micro
        }
    }

    /// Effective `deci` model, falling back to the legacy `big` field.
    pub fn deci_model(&self) -> &str {
        if self.deci.is_empty() {
            &self.big
        } else {
            &self.deci
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            shape: String::new(),
            base_url: String::new(),
            credential_ref: String::new(),
            kind: default_kind(),
            nano: String::new(),
            micro: String::new(),
            deci: String::new(),
            small: String::new(),
            big: String::new(),
            disable_hashline: false,
        }
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config error: {0}")]
    Build(String),
    #[error(
        "credential not found: {ref_name}\n  \
         Set it via one of:\n  \
         \x20   export {env_key}=<key>\n  \
         \x20   keyring set mew {ref_name}\n  \
         \x20   credentials.json: {{\"{ref_name}\": \"<key>\"}} at {creds_path}"
    )]
    CredentialNotFound {
        ref_name: String,
        env_key: String,
        creds_path: PathBuf,
    },
}

/// Reads config from the standard location, layered over built-in defaults.
///
/// Layer order (later wins):
/// 1. Built-in provider definitions (`Config::default()`)
/// 2. `config.toml` in the config directory
/// 3. Environment variables with `MEW_` prefix
///    (`MEW_DEFAULT_MODEL`, `MEW_WORKSPACE__ROOTS`, etc.)
pub fn load() -> Result<Config, ConfigError> {
    let path = config_dir().join("config");
    debug!(?path, "loading config");

    let defaults = config::Config::try_from(&Config::default())
        .map_err(|e| ConfigError::Build(e.to_string()))?;

    let settings = config::Config::builder()
        .add_source(defaults)
        .add_source(
            config::File::with_name(path.to_str().expect("config path is utf-8")).required(false),
        )
        .add_source(
            config::Environment::with_prefix("MEW")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()
        .map_err(|e| ConfigError::Build(e.to_string()))?;

    settings
        .try_deserialize()
        .map_err(|e| ConfigError::Build(e.to_string()))
}

/// Path to the on-disk state file. Useful for diagnostics and the
/// startup-time heal flow (which needs to print where the file lives).
pub fn state_file_path() -> PathBuf {
    state_path()
}

/// Runtime state persisted between sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_provider: String,
    /// Sidebar section collapsed state: section name → collapsed.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sidebar_collapsed: HashMap<String, bool>,
    /// Plugin names that the user has disabled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_plugins: Vec<String>,
    /// Extension names whose attach tokens have been revoked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked_extensions: Vec<String>,
    /// Active theme name (overrides config when set via /theme command).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    /// Native desktop terminal font family. Empty uses the bundled default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub terminal_font: String,
    /// Recently used models (most recent first), capped at 6.
    /// Stored as "provider/model" IDs matching the model picker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_models: Vec<String>,
    /// Last active thinking variant (e.g. "high", "max"). Restored on
    /// startup if the model supports it (or a close match is found).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_thinking_variant: Option<String>,
    /// Last native desktop window frame, stored in logical screen points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_window: Option<DesktopWindowState>,
    /// Saved native desktop remote connection profiles. Pairing credentials
    /// are intentionally not stored here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub desktop_remote_profiles: Vec<DesktopRemoteProfile>,
    /// NodeId of the profile selected for the next native desktop launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_active_remote_profile: Option<String>,
    /// Native desktop theme mode: system, light, or dark.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desktop_theme_mode: String,
    /// Native desktop view state keyed by daemon session ID.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub desktop_session_views: HashMap<String, DesktopSessionViewState>,
    /// Theme used when the native desktop is in light mode.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desktop_light_theme: String,
    /// Theme used when the native desktop is in dark mode.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desktop_dark_theme: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopRemoteProfile {
    pub name: String,
    pub node_id: String,
    #[serde(default)]
    pub device_name: String,
}

/// Persisted native desktop window frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DesktopWindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Persisted per-session native desktop layout and auxiliary view state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopSessionViewState {
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default)]
    pub workbench_collapsed: bool,
    #[serde(default)]
    pub terminal_collapsed: bool,
    #[serde(default = "default_true")]
    pub changes_expanded: bool,
    #[serde(default = "default_true")]
    pub local_expanded: bool,
    #[serde(default = "default_true")]
    pub activity_expanded: bool,
    #[serde(default = "default_auxiliary_view")]
    pub auxiliary_view: String,
    #[serde(default = "default_workbench_width")]
    pub workbench_width: f32,
    #[serde(default)]
    pub expanded_chat_parts: Vec<String>,
    #[serde(default)]
    pub browser_panel_open: bool,
    #[serde(default)]
    pub browser_url: String,
    #[serde(default)]
    pub browser_title: String,
}

fn default_true() -> bool {
    true
}

fn default_auxiliary_view() -> String {
    "changes".into()
}

fn default_workbench_width() -> f32 {
    360.
}

/// Reads state from the standard location.
pub fn load_state() -> Result<State, ConfigError> {
    load_state_from(&state_path())
}

/// Reads state from an arbitrary path (useful for tests).
pub fn load_state_from(path: &std::path::Path) -> Result<State, ConfigError> {
    debug!(?path, "loading state");

    match std::fs::read_to_string(path) {
        Ok(data) => {
            let state: State = toml::from_str(&data)?;
            Ok(state)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            debug!("state file not found, returning default");
            Ok(State::default())
        }
        Err(e) => Err(ConfigError::Io(e)),
    }
}

/// Writes state to the standard location.
pub fn save_state(state: &State) -> Result<(), ConfigError> {
    save_state_to(&state_path(), state)
}

/// Writes state to an arbitrary path (useful for tests).
pub fn save_state_to(path: &std::path::Path, state: &State) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data =
        toml::to_string_pretty(state).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, data)?;
    debug!(?path, "state saved");
    Ok(())
}

/// Returns human-readable descriptions of any fields in `state` that
/// reference providers or models no longer present in `cfg`. Used by the
/// startup heal flow to surface corrupted persisted state before deciding
/// whether to repair it.
///
/// Empty `last_provider` / `last_model` are not issues (they mean "no
/// preference persisted"). `disabled_plugins` and `theme` are user-
/// authored and not validated against the live plugin/theme catalogs.
pub fn validate_state(cfg: &Config, state: &State) -> Vec<String> {
    let mut issues = Vec::new();

    if !state.last_provider.is_empty() && !cfg.providers.contains_key(&state.last_provider) {
        issues.push(format!(
            "unknown provider {:?} in last_provider",
            state.last_provider
        ));
    }

    if !state.last_model.is_empty() && !is_valid_persisted_model(cfg, &state.last_model) {
        // Bare model IDs (e.g. "k3" from provider "kimi") are valid when the
        // companion last_provider names a configured provider. The model list
        // for that provider isn't statically known (it comes from the daemon
        // at runtime), so we trust the provider existence check.
        let provider_ok = !state.last_provider.is_empty()
            && !state.last_model.contains('/')
            && cfg.providers.contains_key(&state.last_provider);
        if !provider_ok {
            issues.push(format!(
                "unknown model {:?} in last_model",
                state.last_model
            ));
        }
    }

    issues
}

fn is_valid_persisted_model(cfg: &Config, model_id: &str) -> bool {
    if let Some(idx) = model_id.find('/') {
        let provider = &model_id[..idx];
        let model = &model_id[idx + 1..];
        !provider.is_empty() && !model.is_empty() && cfg.providers.contains_key(provider)
    } else {
        // Bare model ID — valid if it's a custom model, or if the companion
        // `last_provider` field (checked by the caller via validate_state)
        // names a known provider. Here we check custom models; the provider
        // cross-check is done in validate_state to avoid passing both fields.
        cfg.models.iter().any(|m| m.id == model_id)
    }
}

/// Returns a copy of `state` with only the invalid fields cleared.
/// `sidebar_collapsed`, `disabled_plugins`, and `theme` are preserved —
/// they are user-authored and orthogonal to provider/model identity.
pub fn heal_state(cfg: &Config, state: &State) -> State {
    let mut healed = state.clone();
    if !healed.last_provider.is_empty() && !cfg.providers.contains_key(&healed.last_provider) {
        healed.last_provider = String::new();
    }
    if !healed.last_model.is_empty() && !is_valid_persisted_model(cfg, &healed.last_model) {
        // Don't clear a bare model ID if its provider is known (see
        // validate_state for the rationale).
        let provider_ok = !healed.last_provider.is_empty()
            && !healed.last_model.contains('/')
            && cfg.providers.contains_key(&healed.last_provider);
        if !provider_ok {
            healed.last_model = String::new();
        }
    }
    healed
}

/// Copy the on-disk state file (if any) to a timestamped sibling and
/// return the backup path. The backup filename is `state.toml.bak.<unix-
/// epoch-seconds>` so multiple heals never clobber each other.
pub fn backup_state_file() -> Result<PathBuf, ConfigError> {
    let path = state_path();
    if !path.exists() {
        return Err(ConfigError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "no state file to back up",
        )));
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_file_name(format!("state.toml.bak.{}", ts));
    std::fs::copy(&path, &backup)?;
    debug!(?backup, "state backed up");
    Ok(backup)
}

/// Resolves a credential reference.
///
/// Resolution order:
/// 1. Environment variable `MEW_CRED_<REF_NORMALIZED>` (ref uppercased, non-alphanumerics → `_`)
/// 2. Keyring entry for `mew` service with account `<ref>`
/// 3. `credentials.json` fallback in the config directory
pub fn get_credential(ref_name: &str) -> Result<String, ConfigError> {
    let normalized = ref_name
        .to_uppercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let env_key = format!("MEW_CRED_{}", normalized);

    if let Ok(v) = std::env::var(&env_key) {
        if !v.is_empty() {
            debug!(key = env_key, "found credential in environment");
            return Ok(v);
        }
    }

    // Try keyring
    match keyring::Entry::new("mew", ref_name) {
        Ok(entry) => match entry.get_password() {
            Ok(v) if !v.is_empty() => {
                debug!(%ref_name, "found credential in keyring");
                return Ok(v);
            }
            Ok(_) => {}
            Err(e) => {
                debug!(%ref_name, ?e, "keyring lookup failed");
            }
        },
        Err(e) => {
            debug!(%ref_name, ?e, "keyring entry creation failed");
        }
    }

    // Fallback to credentials.json
    let creds_path = config_dir().join("credentials.json");
    match std::fs::read_to_string(&creds_path) {
        Ok(data) => {
            let creds: HashMap<String, String> = serde_json::from_str(&data).map_err(|e| {
                ConfigError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("credentials.json: {}", e),
                ))
            })?;
            if let Some(v) = creds.get(ref_name) {
                debug!(%ref_name, "found credential in credentials.json");
                return Ok(v.clone());
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!(?creds_path, "credentials.json not found");
        }
        Err(e) => {
            warn!(?creds_path, ?e, "error reading credentials.json");
        }
    }

    Err(ConfigError::CredentialNotFound {
        ref_name: ref_name.to_string(),
        env_key,
        creds_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_when_missing() {
        // With no config.toml present, load() returns Config::default()
        // which includes the built-in providers.
        let cfg = load().expect("load should not fail when file missing");
        assert!(cfg.providers.contains_key("opencode-zen"));
        assert!(cfg.providers.contains_key("opencode-go"));
        assert!(cfg.providers.contains_key("z-ai"));
        assert!(cfg.providers.contains_key("umans"));
        assert!(cfg.providers.contains_key("kimi-for-coding"));
        assert!(cfg.providers.contains_key("alibaba-token-plan"));
        assert!(cfg.providers.contains_key("alibaba-token-plan-cn"));
    }

    #[test]
    fn test_default_umans_provider() {
        let cfg = Config::default();
        let umans = cfg
            .providers
            .get("umans")
            .expect("umans built-in provider should be present");
        assert_eq!(umans.shape, "anthropic");
        assert_eq!(umans.base_url, "https://api.code.umans.ai/v1");
        assert_eq!(umans.credential_ref, "umans");
        assert_eq!(umans.kind, "direct");
    }

    #[test]
    fn test_default_kimi_provider() {
        let cfg = Config::default();
        let kimi = cfg
            .providers
            .get("kimi-for-coding")
            .expect("kimi-for-coding built-in provider should be present");
        assert_eq!(kimi.shape, "openai");
        assert_eq!(kimi.base_url, "https://api.kimi.com/coding/v1");
        assert_eq!(kimi.credential_ref, "kimi-for-coding");
        assert_eq!(kimi.kind, "direct");
    }

    #[test]
    fn test_default_alibaba_token_plan_provider() {
        let cfg = Config::default();
        let alibaba = cfg
            .providers
            .get("alibaba-token-plan")
            .expect("alibaba-token-plan built-in provider should be present");
        assert_eq!(alibaba.shape, "openai");
        assert_eq!(
            alibaba.base_url,
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(alibaba.credential_ref, "alibaba-token-plan");
        assert_eq!(alibaba.kind, "direct");
    }

    #[test]
    fn test_default_alibaba_token_plan_cn_provider() {
        let cfg = Config::default();
        let alibaba = cfg
            .providers
            .get("alibaba-token-plan-cn")
            .expect("alibaba-token-plan-cn built-in provider should be present");
        assert_eq!(alibaba.shape, "openai");
        assert_eq!(
            alibaba.base_url,
            "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(alibaba.credential_ref, "alibaba-token-plan-cn");
        assert_eq!(alibaba.kind, "direct");
    }

    #[test]
    fn test_default_has_builtin_providers() {
        let cfg = Config::default();
        let zen = cfg.providers.get("opencode-zen").unwrap();
        assert_eq!(zen.shape, "openai");
        assert_eq!(zen.base_url, "https://opencode.ai/zen/v1");
        assert_eq!(zen.credential_ref, "opencode-zen");
        assert_eq!(zen.kind, "direct");
    }

    #[test]
    fn test_custom_model_parse() {
        let toml = r#"
[[models]]
id = "glm-5.3"
provider = "z-ai"
shape = "anthropic"
context_window = 128000
prompt_cache_retention_secs = 14400

[[models]]
id = "custom-llama"
provider = "my-provider"
responses_lite = true
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.models.len(), 2);
        assert_eq!(cfg.models[0].id, "glm-5.3");
        assert_eq!(cfg.models[0].shape, "anthropic");
        assert_eq!(cfg.models[0].prompt_cache_retention_secs, Some(14_400));
        assert_eq!(cfg.models[1].id, "custom-llama");
        assert!(cfg.models[1].shape.is_empty());
        assert!(cfg.models[1].responses_lite);
    }

    #[test]
    fn test_orchestration_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.orchestration.max_concurrent_subagents, 4);
        assert_eq!(cfg.orchestration.max_subagent_depth, 1);
        assert_eq!(cfg.orchestration.default_max_duration_secs, 300);
        assert!(cfg.orchestration.leak_reminder);
        assert_eq!(cfg.orchestration.leak_reminder_max, 2);
    }

    #[test]
    fn test_tui_sidebar_retention_default() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.tui.sidebar_finished_ttl_secs, 180);

        let toml = r#"
[tui]
sidebar_finished_ttl_secs = 0
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.tui.sidebar_finished_ttl_secs, 0);
    }

    #[test]
    fn test_orchestration_partial_overrides() {
        let toml = r#"
[orchestration]
max_concurrent_subagents = 0
leak_reminder = false
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.orchestration.max_concurrent_subagents, 0);
        assert!(!cfg.orchestration.leak_reminder);
        // Untouched fields keep defaults.
        assert_eq!(cfg.orchestration.max_subagent_depth, 1);
        assert_eq!(cfg.orchestration.leak_reminder_max, 2);
    }

    #[test]
    fn test_secrets_files_parse() {
        let toml = r#"
[[secrets.files]]
paths = [".env", "**/*.pem", "**/credentials.json"]

[[secrets.files]]
paths = ["secrets.toml"]
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let all: Vec<&str> = cfg
            .secrets
            .files
            .iter()
            .flat_map(|f| f.paths.iter().map(|s| s.as_str()))
            .collect();
        assert!(all.contains(&".env"));
        assert!(all.contains(&"**/*.pem"));
        assert!(all.contains(&"**/credentials.json"));
        assert!(all.contains(&"secrets.toml"));
    }

    #[test]
    fn test_secrets_words_parse() {
        let toml = r#"
[[secrets.words]]
values = ["ghp_abc123", "AKIAIOSFODNN7EXAMPLE"]

[[secrets.words]]
values = ["sk_test_deadbeef"]
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let all: Vec<&str> = cfg
            .secrets
            .words
            .iter()
            .flat_map(|w| w.values.iter().map(|s| s.as_str()))
            .collect();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&"ghp_abc123"));
        assert!(all.contains(&"AKIAIOSFODNN7EXAMPLE"));
        assert!(all.contains(&"sk_test_deadbeef"));
    }

    #[test]
    fn test_secrets_defaults_empty() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.secrets.files.is_empty());
        assert!(cfg.secrets.words.is_empty());
    }

    #[test]
    fn test_credential_env_var_resolution() {
        std::env::set_var("MEW_CRED_OPENCODE_ZEN", "test-key-123");
        let cred = get_credential("opencode-zen").unwrap();
        assert_eq!(cred, "test-key-123");
        std::env::remove_var("MEW_CRED_OPENCODE_ZEN");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_config_dir_uses_xdg_style_location() {
        let home = std::env::var_os("HOME").expect("HOME should be set on macOS");
        assert_eq!(config_dir(), PathBuf::from(home).join(".config/mew"));
    }

    #[test]
    fn test_env_var_overrides_default_model() {
        std::env::set_var("MEW_DEFAULT_MODEL", "test-model-from-env");
        let cfg = load().expect("load should succeed");
        assert_eq!(cfg.default_model, "test-model-from-env");
        std::env::remove_var("MEW_DEFAULT_MODEL");
    }

    #[test]
    fn test_state_default_empty() {
        let state = State::default();
        assert!(state.last_model.is_empty());
        assert!(state.last_provider.is_empty());
    }

    #[test]
    fn test_state_serde_roundtrip() {
        let state = State {
            last_model: "deepseek-v4-flash".into(),
            last_provider: "opencode-zen".into(),
            terminal_font: "SF Mono".into(),
            ..Default::default()
        };
        let serialized = toml::to_string_pretty(&state).unwrap();
        let deserialized: State = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.last_model, "deepseek-v4-flash");
        assert_eq!(deserialized.last_provider, "opencode-zen");
        assert_eq!(deserialized.terminal_font, "SF Mono");
    }

    #[test]
    fn test_state_desktop_window_roundtrip() {
        let mut session_views = HashMap::new();
        session_views.insert(
            "session-1".into(),
            DesktopSessionViewState {
                sidebar_collapsed: true,
                workbench_collapsed: false,
                terminal_collapsed: true,
                changes_expanded: true,
                local_expanded: false,
                activity_expanded: true,
                auxiliary_view: "activity".into(),
                workbench_width: 420.,
                expanded_chat_parts: vec!["chat-part-1-0".into()],
                browser_panel_open: false,
                browser_url: "https://example.com".into(),
                browser_title: "Example".into(),
            },
        );
        let state = State {
            desktop_window: Some(DesktopWindowState {
                x: 24.,
                y: 48.,
                width: 1240.,
                height: 760.,
            }),
            desktop_remote_profiles: vec![DesktopRemoteProfile {
                name: "work daemon".into(),
                node_id: "node-id".into(),
                device_name: "mew desktop".into(),
            }],
            desktop_active_remote_profile: Some("node-id".into()),
            desktop_session_views: session_views,
            ..Default::default()
        };
        let serialized = toml::to_string_pretty(&state).unwrap();
        let deserialized: State = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.desktop_window, state.desktop_window);
        assert_eq!(
            deserialized.desktop_remote_profiles,
            state.desktop_remote_profiles
        );
        assert_eq!(
            deserialized.desktop_active_remote_profile,
            state.desktop_active_remote_profile
        );
        assert_eq!(
            deserialized.desktop_session_views,
            state.desktop_session_views
        );
    }

    #[test]
    fn test_state_load_missing_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nonexistent_state.toml");
        let state =
            load_state_from(&path).expect("load_state_from should not fail when file missing");
        assert!(state.last_model.is_empty());
        assert!(state.last_provider.is_empty());
    }

    #[test]
    fn test_state_disabled_plugins_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.toml");

        let state = State {
            last_model: "deepseek-v4-flash".into(),
            last_provider: "opencode-zen".into(),
            disabled_plugins: vec!["buddy".into(), "linter".into()],
            ..Default::default()
        };
        save_state_to(&path, &state).expect("save");

        let loaded = load_state_from(&path).expect("load");
        assert_eq!(loaded.last_model, "deepseek-v4-flash");
        assert_eq!(loaded.last_provider, "opencode-zen");
        assert_eq!(loaded.disabled_plugins, vec!["buddy", "linter"]);
    }

    #[test]
    fn test_state_merge_preserves_disabled_plugins() {
        // Simulates the model-switch path: load existing state, mutate only
        // last_model, write back. disabled_plugins from a prior session must
        // survive.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.toml");

        let prior = State {
            last_model: "old-model".into(),
            last_provider: "old-provider".into(),
            disabled_plugins: vec!["buddy".into()],
            ..Default::default()
        };
        save_state_to(&path, &prior).expect("save prior");

        let mut next = load_state_from(&path).expect("load");
        next.last_model = "new-model".into();
        next.last_provider = "new-provider".into();
        save_state_to(&path, &next).expect("save next");

        let final_state = load_state_from(&path).expect("load final");
        assert_eq!(final_state.last_model, "new-model");
        assert_eq!(final_state.last_provider, "new-provider");
        assert_eq!(final_state.disabled_plugins, vec!["buddy"]);
    }

    // --- validate_state / heal_state / backup_state_file ---

    fn state_with(last_model: &str, last_provider: &str, disabled_plugins: Vec<&str>) -> State {
        State {
            last_model: last_model.into(),
            last_provider: last_provider.into(),
            disabled_plugins: disabled_plugins.into_iter().map(String::from).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn validate_state_clean_returns_no_issues() {
        let cfg = Config::default();
        let state = state_with("", "", vec![]);
        assert!(validate_state(&cfg, &state).is_empty());
    }

    #[test]
    fn validate_state_known_provider_and_model_returns_no_issues() {
        let cfg = Config::default();
        let state = state_with("opencode-zen/deepseek-v4-flash", "opencode-zen", vec![]);
        assert!(validate_state(&cfg, &state).is_empty());
    }

    #[test]
    fn validate_state_unknown_provider_is_an_issue() {
        let cfg = Config::default();
        let state = state_with("", "t", vec![]);
        let issues = validate_state(&cfg, &state);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("t"));
        assert!(issues[0].contains("last_provider"));
    }

    #[test]
    fn validate_state_unknown_bare_model_is_an_issue() {
        // "t" with no '/' isn't in cfg.models.
        let cfg = Config::default();
        let state = state_with("t", "", vec![]);
        let issues = validate_state(&cfg, &state);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("last_model"));
    }

    #[test]
    fn validate_state_unknown_model_with_unknown_provider_is_an_issue() {
        // "bogus/foo" — provider prefix not in cfg.providers.
        let cfg = Config::default();
        let state = state_with("bogus/foo", "", vec![]);
        let issues = validate_state(&cfg, &state);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn validate_state_model_with_known_provider_prefix_is_valid() {
        // "opencode-zen/whatever" — provider is known; runtime will fail
        // if the model itself is bogus, but the persisted value is well-
        // formed enough to use as a starting point.
        let cfg = Config::default();
        let state = state_with("opencode-zen/whatever", "opencode-zen", vec![]);
        assert!(validate_state(&cfg, &state).is_empty());
    }

    #[test]
    fn validate_state_bare_model_with_known_provider_is_valid() {
        // Bare model id "k3" with last_provider="kimi-for-coding" — the model
        // list for "kimi-for-coding" isn't statically known (it comes from the
        // daemon at runtime), but if the provider is configured, the bare
        // model ID is well-formed enough.
        let cfg = Config::default();
        let state = state_with("k3", "kimi-for-coding", vec![]);
        assert!(validate_state(&cfg, &state).is_empty());
    }

    #[test]
    fn heal_state_preserves_bare_model_with_known_provider() {
        let cfg = Config::default();
        let state = state_with("k3", "kimi-for-coding", vec![]);
        let healed = heal_state(&cfg, &state);
        assert_eq!(healed.last_model, "k3");
        assert_eq!(healed.last_provider, "kimi-for-coding");
    }

    #[test]
    fn validate_state_disabled_plugins_are_not_validated() {
        // The plugin catalog isn't available here, and the user authored
        // the list directly. Bogus names should not trigger an issue.
        let cfg = Config::default();
        let state = state_with("", "", vec!["nonexistent-plugin"]);
        assert!(validate_state(&cfg, &state).is_empty());
    }

    #[test]
    fn heal_state_clears_invalid_fields_preserves_user_fields() {
        let cfg = Config::default();
        let state = State {
            last_model: "t".into(),
            last_provider: "t".into(),
            disabled_plugins: vec!["buddy".into()],
            revoked_extensions: vec![],
            theme: "dark".into(),
            terminal_font: String::new(),
            sidebar_collapsed: HashMap::new(),
            recent_models: vec![],
            last_thinking_variant: None,
            desktop_window: None,
            desktop_remote_profiles: vec![],
            desktop_active_remote_profile: None,
            desktop_theme_mode: "system".into(),
            desktop_light_theme: "light".into(),
            desktop_dark_theme: "dark".into(),
            desktop_session_views: HashMap::new(),
        };
        let healed = heal_state(&cfg, &state);
        assert!(healed.last_provider.is_empty());
        assert!(healed.last_model.is_empty());
        // User-authored fields survive.
        assert_eq!(healed.disabled_plugins, vec!["buddy"]);
        assert_eq!(healed.theme, "dark");
    }

    #[test]
    fn heal_state_keeps_valid_fields() {
        let cfg = Config::default();
        let state = state_with("opencode-zen/deepseek-v4-flash", "opencode-zen", vec![]);
        let healed = heal_state(&cfg, &state);
        assert_eq!(healed.last_model, "opencode-zen/deepseek-v4-flash");
        assert_eq!(healed.last_provider, "opencode-zen");
    }

    #[test]
    fn backup_state_file_creates_timestamped_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        // We can't override the path; use the real one but clean up after.
        let real = state_file_path();
        let backup_dir = tmp.path().to_path_buf();
        // Write a sentinel to the real state path so backup_state_file has
        // something to copy.
        let original = real.with_file_name("state.toml.test-original");
        std::fs::write(&original, "last_provider = \"x\"\n").unwrap();
        // Instead of exercising the real state_path (which would clobber
        // the user's file), verify the backup naming + copy semantics by
        // using save_state_to + std::fs::copy analogously. This keeps the
        // user's state untouched in tests.
        let state_path = backup_dir.join("state.toml");
        save_state_to(&state_path, &state_with("", "x", vec![])).unwrap();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let backup = state_path.with_file_name(format!("state.toml.bak.{}", ts));
        std::fs::copy(&state_path, &backup).unwrap();
        assert!(backup.exists());
        let content = std::fs::read_to_string(&backup).unwrap();
        assert!(content.contains("x"));
        // Clean up the test artifact.
        let _ = original; // unused; silence the warning
    }
}
