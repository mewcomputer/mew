//! Extension manifest types — the parsed `mew-ext.toml` structure.
//!
//! This module defines the types; the actual parser/loader is Phase 2.

use std::path::PathBuf;

use crate::capabilities::{Capability, EventContent, EventScope};

// ── Top-level manifest ─────────────────────────────────────────────

/// Parsed `mew-ext.toml` — one installable unit, one manifest.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtensionManifest {
    pub extension: ExtensionMeta,
    #[serde(default)]
    pub sandbox: ExtensionSandbox,
    #[serde(default)]
    pub provides: ExtensionProvides,
}

/// `[extension]` section.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtensionMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entry: Option<ExtensionEntry>,
    #[serde(default)]
    pub capabilities: ExtensionCapabilities,
}

/// `[extension.entry]` — how to spawn the extension process.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtensionEntry {
    /// Command + args, e.g. `["node", "dist/index.js"]`.
    pub run: Vec<String>,
}

/// `[extension.capabilities]` — requested capabilities.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ExtensionCapabilities {
    #[serde(default)]
    pub sessions: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub events: Option<EventsConfig>,
    #[serde(default)]
    pub hooks: Option<HooksConfig>,
}

/// `[extension.capabilities.events]` — event subscription config.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventsConfig {
    pub scope: EventScope,
    pub content: EventContent,
    /// Event types to subscribe to (e.g. "MessageEnd", "ToolEnd").
    /// Empty means all types.
    #[serde(default)]
    pub types: Vec<String>,
}

/// `[extension.capabilities.hooks]` — hook subscription config.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub observe: bool,
    /// `"mutate"` for benign mutations, or individual sub-scopes.
    #[serde(default)]
    pub mutate: bool,
    #[serde(default)]
    pub mutate_headers: bool,
    #[serde(default)]
    pub mutate_shell_env: bool,
    #[serde(default)]
    pub mutate_chat_params: bool,
    /// `["bash", "write"]` or `["*"]` — per-tool gate scope.
    #[serde(default)]
    pub gate: Vec<String>,
    /// Whether gate can also mutate tool input.
    #[serde(default)]
    pub gate_mutate: bool,
}

/// `[extension.sandbox]` — OS sandbox configuration.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ExtensionSandbox {
    #[serde(default)]
    pub net: bool,
    /// Extra readable paths beyond package dir + storage.
    #[serde(default)]
    pub fs_read: Vec<String>,
    /// Extra writable paths beyond package dir + storage.
    #[serde(default)]
    pub fs_write: Vec<String>,
}

/// `[provides]` — paths relative to package root for the five discovery
/// loaders (skills, commands, personas, subagents, MCP). All optional.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ExtensionProvides {
    #[serde(default)]
    pub skills: Option<PathBuf>,
    #[serde(default)]
    pub commands: Option<PathBuf>,
    #[serde(default)]
    pub personas: Option<PathBuf>,
    #[serde(default)]
    pub subagents: Option<PathBuf>,
    #[serde(default)]
    pub mcp: Option<PathBuf>,
}

// ── Manifest → CapabilitySet conversion ───────────────────────────

impl ExtensionManifest {
    /// Resolve the manifest's requested capabilities into a typed
    /// `CapabilitySet`. Always-granted capabilities (storage, config:read)
    /// are included automatically.
    pub fn requested_capabilities(&self) -> crate::capabilities::CapabilitySet {
        let mut caps = crate::capabilities::CapabilitySet::always_granted();

        // UI is implied by any hook/event capability (extensions need to
        // communicate), but we keep it explicit: grant it if any
        // capability is requested beyond always-granted.
        let caps_meta = &self.extension.capabilities;

        // UI and Register are granted if the extension has any
        // non-always-granted capability (they need to communicate
        // and register tools/commands to be useful).
        let mut has_non_granted = false;

        // Sessions
        for s in &caps_meta.sessions {
            match s.as_str() {
                "read" => {
                    caps.grant(Capability::SessionsRead);
                    has_non_granted = true;
                }
                "manage" => {
                    caps.grant(Capability::SessionsManage);
                    has_non_granted = true;
                }
                "prompt" => {
                    caps.grant(Capability::SessionsPrompt);
                    has_non_granted = true;
                }
                _ => {}
            }
        }

        // Permissions
        if caps_meta.permissions.contains(&"resolve".to_string()) {
            caps.grant(Capability::PermissionsResolve);
            has_non_granted = true;
        }

        // Events
        if let Some(events) = &caps_meta.events {
            caps.grant(Capability::Events {
                scope: events.scope,
                content: events.content,
            });
            has_non_granted = true;
        }

        // Hooks
        if let Some(hooks) = &caps_meta.hooks {
            if hooks.observe {
                caps.grant(Capability::HooksObserve);
                has_non_granted = true;
            }
            if hooks.mutate {
                caps.grant(Capability::HooksMutate);
                has_non_granted = true;
            }
            if hooks.mutate_headers {
                caps.grant(Capability::HooksMutateHeaders);
                has_non_granted = true;
            }
            if hooks.mutate_shell_env {
                caps.grant(Capability::HooksMutateShellEnv);
                has_non_granted = true;
            }
            if hooks.mutate_chat_params {
                caps.grant(Capability::HooksMutateChatParams);
                has_non_granted = true;
            }
            if !hooks.gate.is_empty() {
                if hooks.gate_mutate {
                    caps.grant(Capability::HooksGateMutate);
                } else {
                    caps.grant(Capability::HooksGate);
                }
                has_non_granted = true;
            }
        }

        if has_non_granted {
            caps.grant(Capability::Ui);
            caps.grant(Capability::Register);
        }

        caps
    }

    /// The tools the gate is scoped to. Empty means no gate.
    /// `["*"]` means all tools.
    pub fn gate_tools(&self) -> &[String] {
        self.extension
            .capabilities
            .hooks
            .as_ref()
            .map(|h| h.gate.as_slice())
            .unwrap_or(&[])
    }

    /// Whether the extension has a network sandbox widening.
    pub fn needs_network(&self) -> bool {
        self.sandbox.net
    }
}

// ── Sensitive path denylist ────────────────────────────────────────

/// Paths that are refused in `fs.read`/`fs.write` regardless of consent.
/// Extensions cannot read these even with sandbox widenings.
pub const SENSITIVE_PATH_DENYLIST: &[&str] = &[
    "~/.ssh",
    "~/.aws",
    "~/.gnupg",
    "~/.config/mew/credentials.json",
];

/// Check if a path is in the sensitive denylist.
///
/// Normalizes `..` components before checking to prevent traversal
/// bypasses like `~/.ssh/../etc/passwd`. Also checks each parent
/// path component, so `~/.ssh/id_rsa` matches `~/.ssh`.
pub fn is_sensitive_path(path: &str) -> bool {
    let normalized = normalize_path(path);
    SENSITIVE_PATH_DENYLIST.iter().any(|p| {
        let np = normalize_path(p);
        normalized == np || normalized.starts_with(&format!("{}/", np))
    })
}

/// Normalize `..` and `.` components in a path string.
/// Does NOT resolve symlinks (the OS sandbox handles that), but
/// eliminates traversal via `..` in the path string itself.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    parts.join("/")
}

/// Validate sandbox fs paths against the denylist.
/// Returns the list of denied paths (empty = all clean).
pub fn validate_fs_paths(sandbox: &ExtensionSandbox) -> Vec<String> {
    let mut denied = Vec::new();
    for path in sandbox.fs_read.iter().chain(sandbox.fs_write.iter()) {
        if is_sensitive_path(path) {
            denied.push(path.clone());
        }
    }
    denied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimal_manifest_capabilities() {
        let manifest = ExtensionManifest {
            extension: ExtensionMeta {
                name: "test".into(),
                version: "0.1.0".into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities::default(),
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        };
        let caps = manifest.requested_capabilities();
        // Only always-granted (storage + config:read)
        assert!(caps.has(&Capability::Storage));
        assert!(caps.has(&Capability::ConfigRead));
        assert!(!caps.has(&Capability::Ui));
    }

    #[test]
    fn test_manifest_with_hooks_observe() {
        let manifest = ExtensionManifest {
            extension: ExtensionMeta {
                name: "observer".into(),
                version: "0.1.0".into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities {
                    hooks: Some(HooksConfig {
                        observe: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        };
        let caps = manifest.requested_capabilities();
        assert!(caps.has(&Capability::HooksObserve));
        // Has non-always-granted → gets ui + register
        assert!(caps.has(&Capability::Ui));
        assert!(caps.has(&Capability::Register));
    }

    #[test]
    fn test_manifest_with_gate() {
        let manifest = ExtensionManifest {
            extension: ExtensionMeta {
                name: "gate".into(),
                version: "0.1.0".into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities {
                    hooks: Some(HooksConfig {
                        gate: vec!["bash".into()],
                        gate_mutate: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        };
        let caps = manifest.requested_capabilities();
        assert!(caps.has(&Capability::HooksGate));
        assert!(!caps.has(&Capability::HooksGateMutate));
        assert_eq!(manifest.gate_tools(), &["bash"]);
    }

    #[test]
    fn test_manifest_with_gate_mutate() {
        let manifest = ExtensionManifest {
            extension: ExtensionMeta {
                name: "gate-mutate".into(),
                version: "0.1.0".into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities {
                    hooks: Some(HooksConfig {
                        gate: vec!["*".into()],
                        gate_mutate: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        };
        let caps = manifest.requested_capabilities();
        assert!(caps.has(&Capability::HooksGateMutate));
        // gate:mutate satisfies gate
        assert!(caps.satisfies(&Capability::HooksGate));
    }

    #[test]
    fn test_sensitive_path_denylist() {
        assert!(is_sensitive_path("~/.ssh/id_rsa"));
        assert!(is_sensitive_path("~/.aws/credentials"));
        assert!(is_sensitive_path("~/.gnupg/secring.gpg"));
        assert!(is_sensitive_path("~/.config/mew/credentials.json"));
        assert!(!is_sensitive_path("~/.local/share/mew"));
        assert!(!is_sensitive_path("./src/main.rs"));
    }

    #[test]
    fn test_sensitive_path_traversal_bypass() {
        // Paths that normalize INTO a sensitive directory must be caught.
        // Old code used starts_with — these would have bypassed it.
        assert!(is_sensitive_path("~/.local/../.ssh/id_rsa"));
        assert!(is_sensitive_path("~/.local/../.aws/credentials"));
        assert!(is_sensitive_path("~/.local/../.gnupg/secring.gpg"));
        assert!(is_sensitive_path(
            "~/.local/../.config/mew/credentials.json"
        ));

        // Paths that normalize OUT of a sensitive directory are NOT sensitive.
        // Old code false-positive matched these (starts_with "~/.ssh").
        assert!(!is_sensitive_path("~/.ssh/../etc/passwd"));
        assert!(!is_sensitive_path("~/.aws/../aws/credentials"));

        // A path that normalizes to something NOT in the denylist
        assert!(!is_sensitive_path("~/.local/../local/share/mew"));
    }

    #[test]
    fn test_validate_fs_paths_catches_denylisted() {
        let sandbox = ExtensionSandbox {
            net: false,
            fs_read: vec!["~/.local/share/mew".into(), "~/.ssh/id_rsa".into()],
            fs_write: vec![],
        };
        let denied = validate_fs_paths(&sandbox);
        assert_eq!(denied, vec!["~/.ssh/id_rsa"]);
    }
}
