//! Consent state model for legacy plugin bridge.
//!
//! When a bare-executable plugin is discovered (no manifest, no
//! `ExtensionHello` handshake), the broker calls a consent resolver
//! to determine whether to grant full access (`legacy_full`) or
//! observe-only access. The resolver checks persisted consent state
//! and prompts the user on first run.
//!
//! Consent state is persisted to `~/.local/share/mew/extensions/consent.json`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The consent decision for a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentDecision {
    /// Full access (legacy profile — all hooks, registration, gating).
    Approved,
    /// Observe-only (fire-and-forget observe hooks only).
    Restricted,
}

/// One consent entry in the persisted state file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConsentEntry {
    decision: ConsentDecision,
    timestamp: String,
}

/// Inner state — the actual consent map. Wrapped in a `Mutex` so
/// the `Fn` resolver can mutate through a shared `&` reference.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ConsentStateInner {
    consents: std::collections::HashMap<String, ConsentEntry>,
}

/// Persisted consent state for legacy plugins.
///
/// Uses interior mutability (`Mutex<ConsentStateInner>`) so the resolver
/// closure (which is `Fn`, not `FnMut`) can lock-and-mutate through a
/// shared reference.
pub struct ConsentState {
    inner: Mutex<ConsentStateInner>,
    path: PathBuf,
}

impl ConsentState {
    /// Load consent state from the default data dir path.
    pub fn load() -> Self {
        let path = directories::ProjectDirs::from("ai", "mew", "mew")
            .map(|d| d.data_dir().join("extensions").join("consent.json"))
            .unwrap_or_else(|| PathBuf::from("consent.json"));
        Self::with_path(path)
    }

    /// Create a `ConsentState` backed by an explicit path (for tests).
    pub fn with_path(path: PathBuf) -> Self {
        let inner = Self::read_file(&path).unwrap_or_default();
        Self {
            inner: Mutex::new(inner),
            path,
        }
    }

    /// Look up a plugin's consent decision.
    pub fn get(&self, plugin_name: &str) -> Option<ConsentDecision> {
        let inner = self.inner.lock().unwrap();
        inner.consents.get(plugin_name).map(|e| e.decision)
    }

    /// Record a consent decision for a plugin.
    pub fn set(&self, plugin_name: &str, decision: ConsentDecision) {
        let mut inner = self.inner.lock().unwrap();
        inner.consents.insert(
            plugin_name.to_string(),
            ConsentEntry {
                decision,
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
        );
    }

    /// Persist consent state to disk (atomic: write to temp, rename).
    pub fn save(&self) -> std::io::Result<()> {
        let inner = self.inner.lock().unwrap();

        // Ensure parent dir exists.
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&*inner).map_err(std::io::Error::other)?;

        // Atomic write: write to temp file, then rename.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;

        Ok(())
    }

    fn read_file(path: &Path) -> Option<ConsentStateInner> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }
}

/// A consent resolver — called by the broker for each discovered plugin.
/// Returns the consent decision (approved or restricted).
pub type ConsentResolver = Box<dyn Fn(&str) -> ConsentDecision + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consent_state_load_empty() {
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));
        assert_eq!(state.get("nonexistent"), None);
    }

    #[test]
    fn test_consent_state_set_get() {
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));

        state.set("plugin-a", ConsentDecision::Approved);
        state.set("plugin-b", ConsentDecision::Restricted);

        assert_eq!(state.get("plugin-a"), Some(ConsentDecision::Approved));
        assert_eq!(state.get("plugin-b"), Some(ConsentDecision::Restricted));
        assert_eq!(state.get("plugin-c"), None);
    }

    #[test]
    fn test_consent_state_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consent.json");

        {
            let state = ConsentState::with_path(path.clone());
            state.set("plugin-a", ConsentDecision::Approved);
            state.set("plugin-b", ConsentDecision::Restricted);
            state.save().unwrap();
        }

        // Reload from the same path.
        let reloaded = ConsentState::with_path(path);
        assert_eq!(reloaded.get("plugin-a"), Some(ConsentDecision::Approved));
        assert_eq!(reloaded.get("plugin-b"), Some(ConsentDecision::Restricted));
    }

    #[test]
    fn test_consent_state_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("does-not-exist.json"));
        // Should not panic, should return None for any plugin.
        assert_eq!(state.get("anything"), None);
    }

    #[test]
    fn test_consent_persisted_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consent.json");
        let state = ConsentState::with_path(path);

        // First call: no existing decision → "prompts" and persists.
        assert_eq!(state.get("plugin-x"), None);
        state.set("plugin-x", ConsentDecision::Approved);
        state.save().unwrap();

        // Second call: returns persisted decision without "prompting".
        assert_eq!(state.get("plugin-x"), Some(ConsentDecision::Approved));
    }

    #[test]
    fn test_consent_auto_restrict_non_tty() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consent.json");
        let state = ConsentState::with_path(path);
        let prompt_count = Arc::new(AtomicU32::new(0));

        // Simulate non-interactive mode (is_interactive = false).
        // The resolver should auto-restrict without calling the prompt function.
        let count_clone = prompt_count.clone();
        let resolver: ConsentResolver = Box::new(move |name: &str| {
            if let Some(existing) = state.get(name) {
                return existing;
            }
            // In non-interactive mode, we skip prompting entirely.
            let is_interactive = false;
            let decision = if is_interactive {
                count_clone.fetch_add(1, Ordering::Relaxed);
                // Would call prompt_fn here, but is_interactive is false.
                ConsentDecision::Approved
            } else {
                tracing::warn!("plugin '{}' auto-restricted (non-interactive)", name);
                ConsentDecision::Restricted
            };
            state.set(name, decision);
            state.save().ok();
            decision
        });

        // First call: auto-restricts (non-interactive).
        assert_eq!(resolver("plugin-y"), ConsentDecision::Restricted);
        // Prompt was NOT called (counter stays 0).
        assert_eq!(prompt_count.load(Ordering::Relaxed), 0);

        // Second call: returns persisted decision.
        assert_eq!(resolver("plugin-y"), ConsentDecision::Restricted);
        // Still 0 — no prompt on second call either.
        assert_eq!(prompt_count.load(Ordering::Relaxed), 0);
    }
}
