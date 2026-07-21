//! Unified path resolution using `etcetera`.
//!
//! All mew crates should use these functions instead of rolling their own
//! platform-specific path logic. The paths follow XDG conventions on Unix
//! (macOS and Linux) for consistency with mew's existing file layout:
//!
//! - **config_dir**: `$XDG_CONFIG_HOME/mew` or `~/.config/mew`
//! - **data_dir**: `$XDG_DATA_HOME/mew` or `~/.local/share/mew`
//! - **cache_dir**: `$XDG_CACHE_HOME/mew` or `~/.cache/mew`
//!
//! On Windows, etcetera's default `Windows` strategy is used, which maps
//! to `%APPDATA%`.
//!
//! All can be overridden with `MEW_CONFIG_DIR`, `MEW_DATA_DIR`, and
//! `MEW_CACHE_DIR` environment variables respectively.

use std::path::PathBuf;

#[cfg(unix)]
use etcetera::base_strategy::{BaseStrategy, Xdg};

/// Build the XDG strategy on Unix, panicking only if HOME is truly unset.
#[cfg(unix)]
fn xdg() -> Xdg {
    Xdg::new().expect("etcetera::Xdg requires HOME to be set")
}

/// The config directory: where `config.toml` and `state.toml` live.
///
/// Override with `MEW_CONFIG_DIR`.
pub fn config_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("MEW_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    #[cfg(unix)]
    {
        xdg().config_dir().join("mew")
    }
    #[cfg(not(unix))]
    {
        etcetera::choose_base_strategy()
            .map(|s| s.config_dir().join("mew"))
            .unwrap_or_else(|_| PathBuf::from(".").join("config").join("mew"))
    }
}

/// The data directory: where persistent user data lives (sessions, themes,
/// plugin storage, extension consent).
///
/// Override with `MEW_DATA_DIR`.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("MEW_DATA_DIR") {
        return PathBuf::from(p);
    }
    #[cfg(unix)]
    {
        xdg().data_dir().join("mew")
    }
    #[cfg(not(unix))]
    {
        etcetera::choose_base_strategy()
            .map(|s| s.data_dir().join("mew"))
            .unwrap_or_else(|_| config_dir())
    }
}

/// The cache directory: where transient cache files live (catalog cache,
/// ETags).
///
/// Override with `MEW_CACHE_DIR`.
pub fn cache_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("MEW_CACHE_DIR") {
        return PathBuf::from(p);
    }
    #[cfg(unix)]
    {
        xdg().cache_dir().join("mew")
    }
    #[cfg(not(unix))]
    {
        etcetera::choose_base_strategy()
            .map(|s| s.cache_dir().join("mew"))
            .unwrap_or_else(|_| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".cache").join("mew"))
                    .unwrap_or_else(|| PathBuf::from(".").join(".cache").join("mew"))
            })
    }
}

/// Path to `state.toml`.
pub fn state_path() -> PathBuf {
    config_dir().join("state.toml")
}
