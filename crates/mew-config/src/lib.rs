use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, warn};

/// Top-level user configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(rename = "default_model", default)]
    pub default_model: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            providers: HashMap::new(),
            default_model: String::new(),
        }
    }
}

/// Describes a single provider entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub shape: String,
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(rename = "credential_ref")]
    pub credential_ref: String,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("credential not found: {0}")]
    CredentialNotFound(String),
}

/// Reads config from the standard location.
pub fn load() -> Result<Config, ConfigError> {
    let path = config_path();
    debug!(?path, "loading config");

    match std::fs::read_to_string(&path) {
        Ok(data) => {
            let mut cfg: Config = toml::from_str(&data)?;
            if cfg.providers.is_empty() {
                cfg.providers = HashMap::new();
            }
            Ok(cfg)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            debug!("config file not found, returning default");
            Ok(Config::default())
        }
        Err(e) => Err(ConfigError::Io(e)),
    }
}

fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("ai", "mew", "mew")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config").join("mew"))
                .unwrap_or_else(|| PathBuf::from(".").join(".config").join("mew"))
        })
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
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
            let creds: HashMap<String, String> =
                serde_json::from_str(&data).map_err(|e| ConfigError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("credentials.json: {}", e),
                )))?;
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

    Err(ConfigError::CredentialNotFound(ref_name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_when_missing() {
        // This test assumes no config.toml is present in the default location,
        // which is true in CI / clean environments.
        let cfg = load().expect("load should not fail when file missing");
        assert!(cfg.providers.is_empty());
    }

    #[test]
    fn test_credential_env_var_resolution() {
        std::env::set_var("MEW_CRED_OPENCODE_ZEN", "test-key-123");
        let cred = get_credential("opencode-zen").unwrap();
        assert_eq!(cred, "test-key-123");
        std::env::remove_var("MEW_CRED_OPENCODE_ZEN");
    }
}
