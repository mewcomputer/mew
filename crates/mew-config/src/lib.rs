use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, warn};

/// Sidebar section collapsed state: section name → collapsed.
pub type SidebarState = HashMap<String, bool>;

pub mod permissions;
pub mod shell;

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
}

/// A named thinking variant in config.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThinkingVariantDef {
    pub name: String,
    #[serde(default)]
    pub params: serde_json::Value,
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

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("computer", "mew", "mew")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config").join("mew"))
                .unwrap_or_else(|| PathBuf::from(".").join(".config").join("mew"))
        })
}

fn state_path() -> PathBuf {
    config_dir().join("state.toml")
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

[[models]]
id = "custom-llama"
provider = "my-provider"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.models.len(), 2);
        assert_eq!(cfg.models[0].id, "glm-5.3");
        assert_eq!(cfg.models[0].shape, "anthropic");
        assert_eq!(cfg.models[1].id, "custom-llama");
        assert!(cfg.models[1].shape.is_empty());
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
            ..Default::default()
        };
        let serialized = toml::to_string_pretty(&state).unwrap();
        let deserialized: State = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.last_model, "deepseek-v4-flash");
        assert_eq!(deserialized.last_provider, "opencode-zen");
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
}
