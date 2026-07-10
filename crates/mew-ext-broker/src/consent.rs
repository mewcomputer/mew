//! Consent state model for extensions and legacy plugins.
//!
//! When a plugin or extension is discovered, the broker calls a consent resolver
//! to determine what capabilities to grant. The resolver checks persisted consent
//! state and prompts the user on first run.
//!
//! For manifest-based extensions, the resolver shows a capability-delta prompt
//! listing each requested capability with a plain-language explanation.
//! For legacy bare-executable plugins (no manifest), the resolver shows a simple
//! approved/restricted prompt.
//!
//! Consent state is persisted to `~/.local/share/mew/extensions/consent.json`
//! as capability ID strings (not the enum itself — see Risk #2 in the plan).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::capabilities::CapabilitySet;
use crate::manifest::ExtensionManifest;

/// Sentinel stored in `granted_capabilities` for legacy plugins that were
/// granted full access (`legacy_full()`). Not a real capability ID.
pub const LEGACY_FULL_SENTINEL: &str = "__legacy_full__";

/// The consent decision for a plugin or extension.
///
/// `ConsentDecision` is NOT serialized directly — the consent state persists
/// capability ID strings via `granted_capabilities`. This type is reconstructed
/// at load time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentDecision {
    /// Full access (legacy profile — all hooks, registration, gating).
    ///
    /// Returned for bare-executable plugins that the user approved.
    /// The broker maps this to `CapabilitySet::legacy_full()`.
    Approved,
    /// Observe-only (fire-and-forget observe hooks only).
    ///
    /// Returned for plugins/extensions that the user declined.
    /// The broker maps this to `CapabilitySet::observe_only()`.
    Restricted,
    /// Approved with specific capabilities (manifest-based extensions).
    ///
    /// The broker uses the `CapabilitySet` directly.
    ApprovedWithCaps(CapabilitySet),
}

impl ConsentDecision {
    /// Serialize this decision into capability ID strings for persistence.
    ///
    /// - `Approved` → `["__legacy_full__"]` (bare-plugin sentinel)
    /// - `Restricted` → `[]` (empty)
    /// - `ApprovedWithCaps(caps)` → the actual capability IDs
    pub fn to_granted_ids(&self) -> Vec<String> {
        match self {
            ConsentDecision::Approved => vec![LEGACY_FULL_SENTINEL.to_string()],
            ConsentDecision::Restricted => vec![],
            ConsentDecision::ApprovedWithCaps(caps) => {
                caps.iter().map(|cap| cap.id().to_string()).collect()
            }
        }
    }

    /// Map this decision to a `CapabilitySet`, using `approved_fallback`
    /// for the `Approved` variant. Pass `legacy_full()` for bare plugins,
    /// `observe_only()` for manifest extensions (fail closed).
    pub fn to_caps(&self, approved_fallback: CapabilitySet) -> CapabilitySet {
        match self {
            ConsentDecision::Approved => approved_fallback,
            ConsentDecision::Restricted => CapabilitySet::observe_only(),
            ConsentDecision::ApprovedWithCaps(c) => c.clone(),
        }
    }
}

/// One consent entry in the persisted state file.
///
/// Stores `granted_capabilities` as a list of capability ID strings
/// (from `Capability::id()`). For legacy bare plugins:
/// - `["__legacy_full__"]` = Approved (full access)
/// - `[]` = Restricted (observe-only)
///
/// For manifest-based extensions: the actual capability IDs granted.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConsentEntry {
    granted_capabilities: Vec<String>,
    #[serde(default)]
    last_requested: Vec<String>,
    timestamp: String,
}

/// Inner state — the actual consent map. Wrapped in a `Mutex` so
/// the `Fn` resolver can mutate through a shared `&` reference.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ConsentStateInner {
    consents: std::collections::HashMap<String, ConsentEntry>,
}

/// Persisted consent state for extensions and legacy plugins.
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

    /// Look up a plugin/extension's granted capability IDs.
    ///
    /// Returns `None` if no consent has been recorded for this name.
    /// Returns `Some(vec![])` for restricted plugins (empty caps).
    /// Returns `Some(["__legacy_full__"])` for legacy-approved plugins.
    /// Returns `Some([cap_ids...])` for manifest-approved extensions.
    pub fn get_granted_caps(&self, name: &str) -> Option<Vec<String>> {
        let inner = self.inner.lock().unwrap();
        inner
            .consents
            .get(name)
            .map(|e| e.granted_capabilities.clone())
    }

    /// Record granted capability IDs for a plugin/extension.
    ///
    /// For legacy bare plugins: use `vec![LEGACY_FULL_SENTINEL.to_string()]`
    /// for Approved, `vec![]` for Restricted.
    /// For manifest extensions: use the actual capability IDs from
    /// `Capability::id()`.
    pub fn set_granted_caps(&self, name: &str, ids: Vec<String>) {
        let mut inner = self.inner.lock().unwrap();
        // Preserve existing last_requested if updating, else empty.
        let last_requested = inner
            .consents
            .get(name)
            .map(|e| e.last_requested.clone())
            .unwrap_or_default();
        inner.consents.insert(
            name.to_string(),
            ConsentEntry {
                granted_capabilities: ids,
                last_requested,
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
        );
    }

    /// Look up the last-requested capability IDs recorded for this name.
    /// Returns `None` if no entry exists; `Some(vec![])` for old entries
    /// (pre-last_requested migration).
    pub fn get_last_requested(&self, name: &str) -> Option<Vec<String>> {
        let inner = self.inner.lock().unwrap();
        inner.consents.get(name).map(|e| e.last_requested.clone())
    }

    /// Record both granted and last-requested IDs atomically.
    /// Used by the manifest-extension consent path.
    pub fn set_consent(&self, name: &str, granted_ids: Vec<String>, requested_ids: Vec<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.consents.insert(
            name.to_string(),
            ConsentEntry {
                granted_capabilities: granted_ids,
                last_requested: requested_ids,
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
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!(
                    "failed to read consent state at {}: {} — treating as empty",
                    path.display(),
                    e
                );
                return None;
            }
        };
        match serde_json::from_str(&content) {
            Ok(state) => Some(state),
            Err(e) => {
                tracing::warn!(
                    "consent state at {} is corrupt ({}); treating as empty",
                    path.display(),
                    e
                );
                None
            }
        }
    }
}

/// Check whether granted capability IDs contain the legacy-full sentinel.
///
/// Used by the consent resolver to distinguish legacy-approved bare plugins
/// from restricted ones.
pub fn is_legacy_full(granted: &[String]) -> bool {
    granted.iter().any(|id| id == LEGACY_FULL_SENTINEL)
}

/// A consent resolver — called by the broker for each discovered plugin/extension.
///
/// The resolver receives the extension name and an optional manifest reference.
/// For bare-executable plugins, `manifest` is `None` and the resolver returns
/// `Approved` or `Restricted`. For manifest-based extensions, `manifest` is
/// `Some(...)` and the resolver returns `ApprovedWithCaps(...)` or `Restricted`.
pub type ConsentResolver =
    Box<dyn Fn(&str, Option<&ExtensionManifest>) -> ConsentDecision + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consent_state_load_empty() {
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));
        assert_eq!(state.get_granted_caps("nonexistent"), None);
    }

    #[test]
    fn test_consent_state_set_get() {
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));

        state.set_granted_caps("plugin-a", vec![LEGACY_FULL_SENTINEL.into()]);
        state.set_granted_caps("plugin-b", vec![]);

        assert_eq!(
            state.get_granted_caps("plugin-a"),
            Some(vec![LEGACY_FULL_SENTINEL.into()])
        );
        assert_eq!(state.get_granted_caps("plugin-b"), Some(vec![]));
        assert_eq!(state.get_granted_caps("plugin-c"), None);
    }

    #[test]
    fn test_consent_state_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consent.json");

        {
            let state = ConsentState::with_path(path.clone());
            state.set_granted_caps("plugin-a", vec![LEGACY_FULL_SENTINEL.into()]);
            state.set_granted_caps("plugin-b", vec![]);
            state.save().unwrap();
        }

        // Reload from the same path.
        let reloaded = ConsentState::with_path(path);
        assert_eq!(
            reloaded.get_granted_caps("plugin-a"),
            Some(vec![LEGACY_FULL_SENTINEL.into()])
        );
        assert_eq!(reloaded.get_granted_caps("plugin-b"), Some(vec![]));
    }

    #[test]
    fn test_consent_state_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("does-not-exist.json"));
        // Should not panic, should return None for any plugin.
        assert_eq!(state.get_granted_caps("anything"), None);
    }

    #[test]
    fn test_consent_persisted_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consent.json");
        let state = ConsentState::with_path(path);

        // First call: no existing decision → persists.
        assert_eq!(state.get_granted_caps("plugin-x"), None);
        state.set_granted_caps("plugin-x", vec![LEGACY_FULL_SENTINEL.into()]);
        state.save().unwrap();

        // Second call: returns persisted decision without "prompting".
        assert_eq!(
            state.get_granted_caps("plugin-x"),
            Some(vec![LEGACY_FULL_SENTINEL.into()])
        );
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
        let resolver: ConsentResolver =
            Box::new(move |name: &str, _manifest: Option<&ExtensionManifest>| {
                if let Some(existing) = state.get_granted_caps(name) {
                    if is_legacy_full(&existing) {
                        return ConsentDecision::Approved;
                    }
                    return ConsentDecision::Restricted;
                }
                // In non-interactive mode, we skip prompting entirely.
                let is_interactive = false;
                let decision = if is_interactive {
                    count_clone.fetch_add(1, Ordering::Relaxed);
                    ConsentDecision::Approved
                } else {
                    tracing::warn!("plugin '{}' auto-restricted (non-interactive)", name);
                    ConsentDecision::Restricted
                };
                let ids = match decision {
                    ConsentDecision::Approved => vec![LEGACY_FULL_SENTINEL.into()],
                    _ => vec![],
                };
                state.set_granted_caps(name, ids);
                state.save().ok();
                decision
            });

        // First call: auto-restricts (non-interactive).
        assert_eq!(resolver("plugin-y", None), ConsentDecision::Restricted);
        // Prompt was NOT called (counter stays 0).
        assert_eq!(prompt_count.load(Ordering::Relaxed), 0);

        // Second call: returns persisted decision.
        assert_eq!(resolver("plugin-y", None), ConsentDecision::Restricted);
        // Still 0 — no prompt on second call either.
        assert_eq!(prompt_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_consent_entry_serialization() {
        // Round-trip: serialize → deserialize a ConsentEntry with granted_capabilities.
        let entry = ConsentEntry {
            granted_capabilities: vec![
                "storage".into(),
                "config:read".into(),
                "hooks:observe".into(),
                "hooks:gate".into(),
            ],
            last_requested: vec![],
            timestamp: "2025-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: ConsentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.granted_capabilities,
            entry.granted_capabilities
        );
        assert_eq!(deserialized.timestamp, entry.timestamp);
    }

    #[test]
    fn test_consent_entry_legacy_sentinel_serialization() {
        // Legacy full sentinel should serialize/deserialize correctly.
        let entry = ConsentEntry {
            granted_capabilities: vec![LEGACY_FULL_SENTINEL.into()],
            last_requested: vec![],
            timestamp: "2025-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: ConsentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.granted_capabilities,
            vec![LEGACY_FULL_SENTINEL]
        );
        assert!(is_legacy_full(&deserialized.granted_capabilities));
    }

    #[test]
    fn test_consent_approved_with_caps() {
        use crate::capabilities::{reconstruct_caps, Capability};

        // Simulate a manifest extension with hooks:observe + hooks:gate.
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));

        // Store granted caps as if the user approved a manifest extension.
        let granted_ids = vec![
            "storage".into(),
            "config:read".into(),
            "hooks:observe".into(),
            "hooks:gate".into(),
        ];
        state.set_granted_caps("manifest-ext", granted_ids.clone());
        state.save().unwrap();

        // Reload and reconstruct.
        let reloaded = ConsentState::with_path(dir.path().join("consent.json"));
        let granted = reloaded.get_granted_caps("manifest-ext").unwrap();
        let caps = reconstruct_caps(&granted);

        assert!(caps.has(&Capability::Storage));
        assert!(caps.has(&Capability::HooksObserve));
        assert!(caps.has(&Capability::HooksGate));
        assert!(!caps.has(&Capability::HooksMutate));

        // The ConsentDecision would be ApprovedWithCaps(caps).
        let decision = ConsentDecision::ApprovedWithCaps(caps);
        assert_eq!(
            decision,
            ConsentDecision::ApprovedWithCaps(reconstruct_caps(&granted_ids))
        );
    }

    #[test]
    fn test_is_legacy_full_helper() {
        assert!(is_legacy_full(&[LEGACY_FULL_SENTINEL.into()]));
        assert!(is_legacy_full(&[
            "storage".into(),
            LEGACY_FULL_SENTINEL.into()
        ]));
        assert!(!is_legacy_full(&["storage".into(), "hooks:observe".into()]));
        assert!(!is_legacy_full(&[]));
    }

    #[test]
    fn test_to_granted_ids() {
        use crate::capabilities::{Capability, EventContent, EventScope};

        // Approved → legacy sentinel.
        let ids = ConsentDecision::Approved.to_granted_ids();
        assert_eq!(ids, vec![LEGACY_FULL_SENTINEL]);

        // Restricted → empty.
        let ids = ConsentDecision::Restricted.to_granted_ids();
        assert!(ids.is_empty());

        // ApprovedWithCaps → actual capability IDs.
        let caps = CapabilitySet::from_iter([
            Capability::Storage,
            Capability::HooksObserve,
            Capability::Events {
                scope: EventScope::Session,
                content: EventContent::Meta,
            },
        ]);
        let ids = ConsentDecision::ApprovedWithCaps(caps).to_granted_ids();
        assert!(ids.contains(&"storage".to_string()));
        assert!(ids.contains(&"hooks:observe".to_string()));
        assert!(ids.contains(&"events:session:meta".to_string()));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn test_to_caps_bare() {
        use crate::capabilities::Capability;

        // Approved + legacy_full fallback → legacy_full.
        let caps = ConsentDecision::Approved.to_caps(CapabilitySet::legacy_full());
        assert!(caps.satisfies(&Capability::HooksMutate));
        assert!(caps.satisfies(&Capability::HooksGate));

        // Restricted → observe_only.
        let caps = ConsentDecision::Restricted.to_caps(CapabilitySet::legacy_full());
        assert!(caps.satisfies(&Capability::HooksObserve));
        assert!(!caps.satisfies(&Capability::HooksMutate));
    }

    #[test]
    fn test_to_caps_manifest() {
        use crate::capabilities::Capability;

        // Approved + observe_only fallback → observe_only (fail closed).
        let caps = ConsentDecision::Approved.to_caps(CapabilitySet::observe_only());
        assert!(caps.satisfies(&Capability::HooksObserve));
        assert!(
            !caps.satisfies(&Capability::HooksMutate),
            "manifest Approved fallback must NOT grant HooksMutate"
        );

        // ApprovedWithCaps → passthrough.
        let requested = CapabilitySet::from_iter([Capability::HooksMutate, Capability::HooksGate]);
        let caps = ConsentDecision::ApprovedWithCaps(requested.clone())
            .to_caps(CapabilitySet::observe_only());
        assert!(caps.satisfies(&Capability::HooksMutate));
        assert!(caps.satisfies(&Capability::HooksGate));
    }

    #[test]
    fn test_persisted_caps_clamped() {
        // Simulate what the resolver does: store broad caps, then clamp
        // against a narrower manifest's requested set via intersect.
        use crate::capabilities::{reconstruct_caps, Capability};

        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));

        // Store broad caps as if previously approved.
        state.set_granted_caps("ext", vec!["hooks:observe".into(), "hooks:gate".into()]);
        state.save().unwrap();

        // Reload and simulate the clamp.
        let reloaded = ConsentState::with_path(dir.path().join("consent.json"));
        let granted = reloaded.get_granted_caps("ext").unwrap();
        let stored = reconstruct_caps(&granted);

        // Simulate a manifest that only requests hooks:observe.
        let manifest_caps = CapabilitySet::from_iter([Capability::HooksObserve]);
        let clamped = stored.intersect(&manifest_caps);

        assert!(clamped.has(&Capability::HooksObserve));
        assert!(
            !clamped.has(&Capability::HooksGate),
            "intersect must drop caps not in the manifest"
        );
    }

    #[test]
    fn test_stale_sentinel_detected() {
        // When a bare plugin was approved (stored sentinel), the same name
        // used by a manifest extension must detect the stale sentinel.
        use crate::capabilities::reconstruct_caps;

        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));

        // Store legacy sentinel as if a bare plugin was approved.
        state.set_granted_caps("name", vec![LEGACY_FULL_SENTINEL.into()]);
        state.save().unwrap();

        // Reload and check: is_legacy_full detects the sentinel.
        let reloaded = ConsentState::with_path(dir.path().join("consent.json"));
        let granted = reloaded.get_granted_caps("name").unwrap();
        assert!(
            is_legacy_full(&granted),
            "stale sentinel must be detected so the resolver can fall through to re-prompt"
        );

        // reconstruct_caps on the sentinel yields empty (not a real capability).
        let caps = reconstruct_caps(&granted);
        assert!(
            caps.is_empty(),
            "sentinel must not produce real capabilities"
        );
    }

    #[test]
    fn test_read_file_corrupt() {
        // Corrupt JSON in consent.json → state loads as empty (no panic).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consent.json");
        std::fs::write(&path, "this is not valid json {{{{").unwrap();

        let state = ConsentState::with_path(path);
        // Should return None for all names — corrupt state treated as empty.
        assert_eq!(state.get_granted_caps("anything"), None);
    }

    #[test]
    fn test_set_consent_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let state = ConsentState::with_path(dir.path().join("consent.json"));

        let granted = vec!["storage".into(), "hooks:observe".into()];
        let requested = vec![
            "storage".into(),
            "hooks:observe".into(),
            "hooks:gate".into(),
        ];
        state.set_consent("ext-a", granted.clone(), requested.clone());
        state.save().unwrap();

        // Reload from disk.
        let reloaded = ConsentState::with_path(dir.path().join("consent.json"));
        assert_eq!(reloaded.get_granted_caps("ext-a"), Some(granted));
        assert_eq!(reloaded.get_last_requested("ext-a"), Some(requested));
    }

    #[test]
    fn test_backward_compat_last_requested_default() {
        // Simulate old JSON: a ConsentEntry without `last_requested`.
        // The field has `#[serde(default)]`, so it should deserialize to `vec![]`.
        let old_json = r#"{
            "granted_capabilities": ["storage", "hooks:observe"],
            "timestamp": "2025-01-01T00:00:00Z"
        }"#;
        let entry: ConsentEntry = serde_json::from_str(old_json).unwrap();
        assert_eq!(entry.granted_capabilities, vec!["storage", "hooks:observe"]);
        assert!(
            entry.last_requested.is_empty(),
            "old entries without last_requested should default to empty"
        );
    }
}
