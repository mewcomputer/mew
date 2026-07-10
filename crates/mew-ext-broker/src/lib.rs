//! Extension broker types: capabilities, principals, manifests, audit.
//!
//! This crate is intentionally lightweight — no tokio, no process spawning.
//! It holds the data model that `mew-protocol` (wire format) and
//! `mew-hooks-runtime` (broker implementation) both depend on.

pub mod audit;
pub mod audit_log;
pub mod broker;
pub mod capabilities;
pub mod capability_descriptions;
pub mod consent;
pub mod discovery;
pub mod event_queue;
pub mod manifest;
pub mod principal;
pub mod sandbox;
pub mod tokens;

pub use audit::{GateAuditEntry, GateOutcome};
pub use audit_log::AuditLog;
pub use broker::ExtensionBroker;
pub use capabilities::{
    reconstruct_caps, Capability, CapabilityDelta, CapabilitySet, EventContent, EventScope,
    RiskTier,
};
pub use capability_descriptions::{
    build_consent_prompt, build_delta_prompt, build_sensitive_cap_prompt, capability_description,
    is_sensitive,
};
pub use consent::{
    is_legacy_full, ConsentDecision, ConsentResolver, ConsentState, LEGACY_FULL_SENTINEL,
};
pub use discovery::{
    discover_extensions, discover_extensions_from_dirs, DiscoveredExtension, ExtensionScope,
};
pub use manifest::{
    parse_manifest, EventsConfig, ExtensionCapabilities, ExtensionEntry, ExtensionManifest,
    ExtensionMeta, ExtensionProvides, ExtensionSandbox, HooksConfig,
};
pub use principal::{Principal, PrincipalId, PrincipalKind};
pub use sandbox::{build_sandbox_profile, sandbox_available, SandboxConfig};
pub use tokens::{mint_token, revoke_token, rotate_all_tokens, show_token, validate_token};
