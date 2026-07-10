//! Plain-language descriptions for each capability, used in consent prompts.
//!
//! When a manifest-based extension requests capabilities, the consent resolver
//! builds a prompt listing each capability with a human-readable explanation.
//! Sensitive capabilities (High/Highest risk tier) are marked with a ⚠ marker.

use crate::capabilities::{Capability, EventContent, EventScope};
use crate::manifest::ExtensionManifest;

/// One-line plain-language explanation for each capability, used in consent prompts.
pub fn capability_description(cap: &Capability) -> &'static str {
    match cap {
        Capability::Storage => "Store data in a namespaced key-value store",
        Capability::ConfigRead => "Read its own configuration subtree",
        Capability::Ui => "Show notifications and input-area widgets",
        Capability::Register => "Register tools, slash commands, and providers",
        Capability::SessionsRead => "List sessions and read metadata/history",
        Capability::SessionsManage => "Create, attach, fork, and resume sessions",
        Capability::SessionsPrompt => "Send prompts and cancel turns",
        Capability::PermissionsResolve => "Answer permission prompts (remote approval)",
        Capability::Events { scope, content } => match (scope, content) {
            (EventScope::Session, EventContent::Meta) => {
                "Receive lifecycle events for this session"
            }
            (EventScope::Session, EventContent::Full) => {
                "Read full message/tool text for this session"
            }
            (EventScope::Global, EventContent::Meta) => "Receive lifecycle events for all sessions",
            (EventScope::Global, EventContent::Full) => {
                "Read full message/tool text for ALL sessions"
            }
        },
        Capability::HooksObserve => "Receive fire-and-forget hook notifications",
        Capability::HooksMutate => "Modify system prompts, user input, and chat messages",
        Capability::HooksMutateHeaders => "Rewrite provider request headers (auth injection)",
        Capability::HooksMutateShellEnv => "Modify shell environment variables",
        Capability::HooksMutateChatParams => "Override chat parameters (max_tokens, temperature)",
        Capability::HooksGate => "Approve or deny tool calls and permission prompts",
        Capability::HooksGateMutate => "Approve, deny, or REWRITE tool call inputs",
    }
}

/// Whether a capability is sensitive (requires individual confirmation).
pub fn is_sensitive(cap: &Capability) -> bool {
    cap.requires_individual_consent()
}

/// Build a consent prompt string listing all requested capabilities.
///
/// Splits capabilities into non-sensitive (batch-approved) and sensitive
/// (individually confirmed) sections. The prompt asks the user to grant
/// the non-sensitive batch; sensitive capabilities are listed for
/// information and prompted individually by the resolver.
///
/// Pure function — takes the extension name and manifest, returns the formatted
/// prompt text. Used by the consent resolver and testable independently.
pub fn build_consent_prompt(name: &str, manifest: &ExtensionManifest) -> String {
    let caps = manifest.requested_capabilities();
    let (non_sensitive, sensitive): (Vec<_>, Vec<_>) =
        caps.iter().cloned().partition(|c| !is_sensitive(c));

    let mut lines = vec![format!(
        "Extension '{}' v{} requests:",
        name, manifest.extension.version
    )];

    // Non-sensitive section.
    for cap in &non_sensitive {
        let desc = capability_description(cap);
        lines.push(format!("  • {} — {}", cap.id(), desc));
    }

    // Sensitive section (informational — individual prompts follow).
    if !sensitive.is_empty() {
        lines.push(String::new());
        lines.push("Also requests sensitive capabilities (requires individual approval):".into());
        for cap in &sensitive {
            let desc = capability_description(cap);
            lines.push(format!("  • {} — {} ⚠", cap.id(), desc));
        }
    }

    lines.push(String::new());
    lines.push("Grant non-sensitive capabilities?".into());
    lines.join("\n")
}

/// Build the individual prompt text for a single sensitive capability.
pub fn build_sensitive_cap_prompt(name: &str, version: &str, cap: &Capability) -> String {
    format!(
        "Extension '{}' v{} requests: {} — {} ⚠\nGrant?",
        name,
        version,
        cap.id(),
        capability_description(cap)
    )
}

/// Build a consent prompt for newly-added capabilities (manifest upgrade).
///
/// `added_caps` is the set of capabilities the manifest now requests
/// that were not in its previous version.
pub fn build_delta_prompt(
    name: &str,
    manifest: &ExtensionManifest,
    added_caps: &[Capability],
) -> String {
    let (non_sensitive, sensitive): (Vec<_>, Vec<_>) =
        added_caps.iter().cloned().partition(|c| !is_sensitive(c));

    let mut lines = vec![format!(
        "Extension '{}' v{} now requests NEW capabilities:",
        name, manifest.extension.version
    )];

    for cap in &non_sensitive {
        lines.push(format!(
            "  • {} — {}",
            cap.id(),
            capability_description(cap)
        ));
    }

    if !sensitive.is_empty() {
        lines.push(String::new());
        lines.push(
            "Also requests new sensitive capabilities (requires individual approval):".into(),
        );
        for cap in &sensitive {
            lines.push(format!(
                "  • {} — {} ⚠",
                cap.id(),
                capability_description(cap)
            ));
        }
    }

    if !non_sensitive.is_empty() {
        lines.push(String::new());
        lines.push("Grant new non-sensitive capabilities?".into());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Capability;

    #[test]
    fn test_capability_descriptions() {
        // Every variant must have a non-empty description.
        let all_caps = [
            Capability::Storage,
            Capability::ConfigRead,
            Capability::Ui,
            Capability::Register,
            Capability::SessionsRead,
            Capability::SessionsManage,
            Capability::SessionsPrompt,
            Capability::PermissionsResolve,
            Capability::Events {
                scope: EventScope::Session,
                content: EventContent::Meta,
            },
            Capability::Events {
                scope: EventScope::Session,
                content: EventContent::Full,
            },
            Capability::Events {
                scope: EventScope::Global,
                content: EventContent::Meta,
            },
            Capability::Events {
                scope: EventScope::Global,
                content: EventContent::Full,
            },
            Capability::HooksObserve,
            Capability::HooksMutate,
            Capability::HooksMutateHeaders,
            Capability::HooksMutateShellEnv,
            Capability::HooksMutateChatParams,
            Capability::HooksGate,
            Capability::HooksGateMutate,
        ];

        for cap in &all_caps {
            let desc = capability_description(cap);
            assert!(
                !desc.is_empty(),
                "capability {:?} has an empty description",
                cap
            );
        }
    }

    #[test]
    fn test_is_sensitive() {
        // Sensitive: High and Highest risk tiers.
        assert!(is_sensitive(&Capability::HooksGate));
        assert!(is_sensitive(&Capability::HooksGateMutate));
        assert!(is_sensitive(&Capability::PermissionsResolve));
        assert!(is_sensitive(&Capability::HooksMutateHeaders));
        assert!(is_sensitive(&Capability::HooksMutateShellEnv));
        assert!(is_sensitive(&Capability::HooksMutateChatParams));
        assert!(is_sensitive(&Capability::Events {
            scope: EventScope::Global,
            content: EventContent::Full
        }));

        // Not sensitive.
        assert!(!is_sensitive(&Capability::Storage));
        assert!(!is_sensitive(&Capability::Ui));
        assert!(!is_sensitive(&Capability::Register));
        assert!(!is_sensitive(&Capability::HooksObserve));
        assert!(!is_sensitive(&Capability::HooksMutate));
        assert!(!is_sensitive(&Capability::Events {
            scope: EventScope::Session,
            content: EventContent::Meta
        }));
    }

    #[test]
    fn test_build_consent_prompt_content() {
        use crate::manifest::{
            ExtensionCapabilities, ExtensionMeta, ExtensionProvides, ExtensionSandbox, HooksConfig,
        };

        let manifest = ExtensionManifest {
            extension: ExtensionMeta {
                name: "my-ext".into(),
                version: "1.2.3".into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities {
                    hooks: Some(HooksConfig {
                        observe: true,
                        gate: vec!["bash".into()],
                        gate_mutate: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        };

        let prompt = build_consent_prompt("my-ext", &manifest);

        // Contains the extension name and version.
        assert!(prompt.contains("Extension 'my-ext' v1.2.3 requests:"));

        // Non-sensitive capabilities are listed without ⚠.
        assert!(prompt.contains("hooks:observe — Receive fire-and-forget hook notifications"));
        // hooks:observe line must NOT have the ⚠ marker.
        assert!(!prompt.contains("hooks:observe — Receive fire-and-forget hook notifications ⚠"));

        // Sensitive capabilities are in a separate section.
        let sensitive_header =
            "Also requests sensitive capabilities (requires individual approval):";
        assert!(prompt.contains(sensitive_header));

        // The sensitive header must appear BEFORE the sensitive cap lines.
        let header_pos = prompt.find(sensitive_header).unwrap();
        let gate_mutate_pos = prompt.find("hooks:gate:mutate").unwrap();
        assert!(
            header_pos < gate_mutate_pos,
            "sensitive header must appear before sensitive cap lines"
        );

        // Sensitive caps have the ⚠ marker.
        assert!(prompt.contains("hooks:gate:mutate — Approve, deny, or REWRITE tool call inputs ⚠"));

        // The prompt asks for non-sensitive batch approval (not "Grant all").
        assert!(prompt.contains("Grant non-sensitive capabilities?"));
        assert!(!prompt.contains("Grant all listed capabilities?"));
    }

    #[test]
    fn test_build_delta_prompt() {
        use crate::manifest::{
            ExtensionCapabilities, ExtensionMeta, ExtensionProvides, ExtensionSandbox,
        };

        let manifest = ExtensionManifest {
            extension: ExtensionMeta {
                name: "upgraded-ext".into(),
                version: "2.0.0".into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities::default(),
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        };

        // Added caps: one non-sensitive (Ui), one sensitive (HooksGate).
        let added = vec![Capability::Ui, Capability::HooksGate];
        let prompt = build_delta_prompt("upgraded-ext", &manifest, &added);

        assert!(prompt.contains("Extension 'upgraded-ext' v2.0.0 now requests NEW capabilities:"));
        assert!(prompt.contains("ui — Show notifications and input-area widgets"));
        assert!(prompt.contains("hooks:gate — Approve or deny tool calls and permission prompts ⚠"));
        assert!(prompt.contains("Grant new non-sensitive capabilities?"));
    }

    #[test]
    fn test_build_delta_prompt_all_sensitive() {
        use crate::manifest::{
            ExtensionCapabilities, ExtensionMeta, ExtensionProvides, ExtensionSandbox,
        };

        let manifest = ExtensionManifest {
            extension: ExtensionMeta {
                name: "upgraded-ext".into(),
                version: "2.0.0".into(),
                description: String::new(),
                entry: None,
                capabilities: ExtensionCapabilities::default(),
            },
            sandbox: ExtensionSandbox::default(),
            provides: ExtensionProvides::default(),
        };

        // All added caps are sensitive → no "Grant ...?" line.
        let added = vec![Capability::HooksGate, Capability::HooksGateMutate];
        let prompt = build_delta_prompt("upgraded-ext", &manifest, &added);

        assert!(prompt.contains("Extension 'upgraded-ext' v2.0.0 now requests NEW capabilities:"));
        assert!(prompt.contains("hooks:gate — Approve or deny tool calls and permission prompts ⚠"));
        assert!(prompt.contains("hooks:gate:mutate — Approve, deny, or REWRITE tool call inputs ⚠"));
        // No batch prompt when all added caps are sensitive.
        assert!(!prompt.contains("Grant new non-sensitive capabilities?"));
    }

    #[test]
    fn test_build_sensitive_cap_prompt() {
        let cap = Capability::HooksGate;
        let prompt = build_sensitive_cap_prompt("my-ext", "1.0.0", &cap);
        assert!(prompt.contains("Extension 'my-ext' v1.0.0 requests:"));
        assert!(prompt.contains("hooks:gate"));
        assert!(prompt.contains("Approve or deny tool calls and permission prompts"));
        assert!(prompt.contains("⚠"));
        assert!(prompt.contains("Grant?"));
    }
}
