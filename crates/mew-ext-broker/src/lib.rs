//! Extension broker types: capabilities, principals, manifests, audit.
//!
//! This crate is intentionally lightweight — no tokio, no process spawning.
//! It holds the data model that `mew-protocol` (wire format) and
//! `mew-hooks-runtime` (broker implementation) both depend on.

pub mod audit;
pub mod capabilities;
pub mod manifest;
pub mod principal;

pub use audit::{GateAuditEntry, GateOutcome};
pub use capabilities::{
    Capability, CapabilityDelta, CapabilitySet, EventContent, EventScope, RiskTier,
};
pub use manifest::{
    EventsConfig, ExtensionCapabilities, ExtensionEntry, ExtensionManifest, ExtensionMeta,
    ExtensionProvides, ExtensionSandbox, HooksConfig,
};
pub use principal::{Principal, PrincipalId, PrincipalKind};
