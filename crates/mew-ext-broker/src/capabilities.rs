//! Capability model for extension principals.
//!
//! Capabilities are granted at install/upgrade time and enforced
//! daemon-side per method. See the plan's capability table for the
//! full rationale.

use std::collections::HashSet;

// ── Risk tier ───────────────────────────────────────────────────────

/// Risk classification used in consent prompts. Higher = scarier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    /// Granted automatically (storage, config:read).
    AlwaysGranted,
    /// Low risk, batched in consent (ui, register, hooks:observe).
    Low,
    /// Medium risk (sessions:*, hooks:mutate, events session/meta).
    Medium,
    /// High risk — individually confirmed (hooks:mutate:*, hooks:gate,
    /// permissions:resolve, events global/full).
    High,
    /// Highest risk — hooks:gate:mutate.
    Highest,
}

// ── Event scope/content ────────────────────────────────────────────

/// Whether events cover only the extension's own session or all sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventScope {
    Session,
    Global,
}

/// Whether events carry only lifecycle metadata or full message/tool text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventContent {
    /// Lifecycle events only (session created, turn ended, tool ran — no bodies).
    Meta,
    /// Full message/tool text. The scariest read grant.
    Full,
}

// ── Capability ─────────────────────────────────────────────────────

/// A capability granted to an extension principal.
///
/// Granularity is proportional to risk: coarse grants for low-risk
/// surfaces (`Register`, `Ui`), fine sub-scopes for the sharp ones
/// (`Gate`, `GateMutate`, `MutateHeaders`, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Capability {
    /// Namespaced key-value storage. Always granted.
    Storage,
    /// Read own config.toml subtree only. Always granted.
    ConfigRead,
    /// UI affordances: notify, input-area widget, modal prompts.
    Ui,
    /// Dynamic registration: tools, commands, providers.
    Register,
    /// List sessions, read metadata/history.
    SessionsRead,
    /// Create, attach, fork, resume sessions.
    SessionsManage,
    /// Send prompts, cancel turns.
    SessionsPrompt,
    /// Answer user-facing permission prompts (zedra remote-approval).
    PermissionsResolve,
    /// Subscribe to events with a scope and content level.
    Events {
        scope: EventScope,
        content: EventContent,
    },
    /// Fire-and-forget observe hooks.
    HooksObserve,
    /// Benign mutation hooks: system_prompt, user_input, chat_message.
    HooksMutate,
    /// Rewrite provider request headers (auth injection for gateways/proxies).
    HooksMutateHeaders,
    /// Rewrite shell environment variables for bash/shell tools.
    HooksMutateShellEnv,
    /// Override chat params (max_tokens, temperature, tool_choice).
    HooksMutateChatParams,
    /// Gate: approve/deny only. on_permission_ask and
    /// on_tool_execute_before restricted to Proceed-unchanged/Block/Suppress.
    HooksGate,
    /// Gate with input mutation: on_tool_execute_before may rewrite tool input.
    HooksGateMutate,
}

impl Capability {
    /// Human-readable identifier for consent prompts and audit logs.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::ConfigRead => "config:read",
            Self::Ui => "ui",
            Self::Register => "register",
            Self::SessionsRead => "sessions:read",
            Self::SessionsManage => "sessions:manage",
            Self::SessionsPrompt => "sessions:prompt",
            Self::PermissionsResolve => "permissions:resolve",
            Self::Events { scope, content } => match (scope, content) {
                (EventScope::Session, EventContent::Meta) => "events:session:meta",
                (EventScope::Session, EventContent::Full) => "events:session:full",
                (EventScope::Global, EventContent::Meta) => "events:global:meta",
                (EventScope::Global, EventContent::Full) => "events:global:full",
            },
            Self::HooksObserve => "hooks:observe",
            Self::HooksMutate => "hooks:mutate",
            Self::HooksMutateHeaders => "hooks:mutate:headers",
            Self::HooksMutateShellEnv => "hooks:mutate:shell_env",
            Self::HooksMutateChatParams => "hooks:mutate:chat_params",
            Self::HooksGate => "hooks:gate",
            Self::HooksGateMutate => "hooks:gate:mutate",
        }
    }

    /// Risk tier for consent prompt display.
    pub fn risk_tier(&self) -> RiskTier {
        match self {
            Self::Storage | Self::ConfigRead => RiskTier::AlwaysGranted,
            Self::Ui | Self::Register | Self::HooksObserve => RiskTier::Low,
            Self::SessionsRead | Self::SessionsManage | Self::SessionsPrompt => RiskTier::Medium,
            Self::HooksMutate => RiskTier::Medium,
            Self::Events { scope, content } => match (scope, content) {
                (EventScope::Session, EventContent::Meta) => RiskTier::Low,
                (EventScope::Session, EventContent::Full) => RiskTier::Medium,
                (EventScope::Global, EventContent::Meta) => RiskTier::Medium,
                (EventScope::Global, EventContent::Full) => RiskTier::High,
            },
            Self::HooksMutateHeaders | Self::HooksMutateShellEnv | Self::HooksMutateChatParams => {
                RiskTier::High
            }
            Self::HooksGate => RiskTier::High,
            Self::HooksGateMutate => RiskTier::Highest,
            Self::PermissionsResolve => RiskTier::High,
        }
    }

    /// Whether this capability requires individual confirmation in consent.
    pub fn requires_individual_consent(&self) -> bool {
        matches!(self.risk_tier(), RiskTier::High | RiskTier::Highest)
    }

    /// Reverse-map an ID string back to a `Capability`.
    ///
    /// Returns `None` for unknown IDs. Used to reconstruct a `CapabilitySet`
    /// from persisted consent state (which stores capability ID strings).
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "storage" => Some(Self::Storage),
            "config:read" => Some(Self::ConfigRead),
            "ui" => Some(Self::Ui),
            "register" => Some(Self::Register),
            "sessions:read" => Some(Self::SessionsRead),
            "sessions:manage" => Some(Self::SessionsManage),
            "sessions:prompt" => Some(Self::SessionsPrompt),
            "permissions:resolve" => Some(Self::PermissionsResolve),
            "hooks:observe" => Some(Self::HooksObserve),
            "hooks:mutate" => Some(Self::HooksMutate),
            "hooks:mutate:headers" => Some(Self::HooksMutateHeaders),
            "hooks:mutate:shell_env" => Some(Self::HooksMutateShellEnv),
            "hooks:mutate:chat_params" => Some(Self::HooksMutateChatParams),
            "hooks:gate" => Some(Self::HooksGate),
            "hooks:gate:mutate" => Some(Self::HooksGateMutate),
            // Events: format is "events:<scope>:<content>".
            s if s.starts_with("events:") => {
                let parts: Vec<&str> = s.split(':').collect();
                if parts.len() != 3 {
                    return None;
                }
                let scope = match parts[1] {
                    "session" => EventScope::Session,
                    "global" => EventScope::Global,
                    _ => return None,
                };
                let content = match parts[2] {
                    "meta" => EventContent::Meta,
                    "full" => EventContent::Full,
                    _ => return None,
                };
                Some(Self::Events { scope, content })
            }
            _ => None,
        }
    }

    /// Whether this capability is a gate capability (gate or gate:mutate).
    pub fn is_gate(&self) -> bool {
        matches!(self, Self::HooksGate | Self::HooksGateMutate)
    }

    /// Whether this capability is any mutation capability.
    pub fn is_mutate(&self) -> bool {
        matches!(
            self,
            Self::HooksMutate
                | Self::HooksMutateHeaders
                | Self::HooksMutateShellEnv
                | Self::HooksMutateChatParams
                | Self::HooksGateMutate
        )
    }
}

// ── Capability set ─────────────────────────────────────────────────

/// A set of granted capabilities. Owned by a [`super::Principal`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    caps: HashSet<Capability>,
}

impl CapabilitySet {
    /// Create an empty set.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create a set with the always-granted capabilities (storage, config:read).
    pub fn always_granted() -> Self {
        let mut set = Self::empty();
        set.grant(Capability::Storage);
        set.grant(Capability::ConfigRead);
        set
    }

    /// Create a set with ALL capabilities granted — the legacy profile for
    /// bare executables that have no manifest. This preserves current
    /// behavior exactly: every hook fires for every extension.
    pub fn legacy_full() -> Self {
        let mut set = Self::always_granted();
        set.grant(Capability::Ui);
        set.grant(Capability::Register);
        set.grant(Capability::SessionsRead);
        set.grant(Capability::SessionsManage);
        set.grant(Capability::SessionsPrompt);
        set.grant(Capability::PermissionsResolve);
        set.grant(Capability::Events {
            scope: EventScope::Global,
            content: EventContent::Full,
        });
        set.grant(Capability::HooksObserve);
        set.grant(Capability::HooksMutate);
        set.grant(Capability::HooksMutateHeaders);
        set.grant(Capability::HooksMutateShellEnv);
        set.grant(Capability::HooksMutateChatParams);
        set.grant(Capability::HooksGate);
        set.grant(Capability::HooksGateMutate);
        set
    }

    /// Create a set with only observe hooks — for restricted legacy plugins.
    /// Grants Storage + ConfigRead (always-granted) + HooksObserve.
    /// No mutations, no gates, no registration, no events, no sessions.
    pub fn observe_only() -> Self {
        let mut set = Self::always_granted();
        set.grant(Capability::HooksObserve);
        set
    }

    /// Grant a capability.
    pub fn grant(&mut self, cap: Capability) {
        self.caps.insert(cap);
    }

    /// Revoke a capability.
    pub fn revoke(&mut self, cap: &Capability) {
        self.caps.remove(cap);
    }

    /// Check if a specific capability is granted.
    pub fn has(&self, cap: &Capability) -> bool {
        self.caps.contains(cap)
    }

    /// Check if the set satisfies a required capability.
    ///
    /// For `Events`, a granted capability with broader scope/content
    /// satisfies a narrower requirement:
    /// - `global` scope satisfies `session` scope
    /// - `full` content satisfies `meta` content
    pub fn satisfies(&self, required: &Capability) -> bool {
        match required {
            Capability::Events { scope, content } => {
                // Check for exact match first
                if self.caps.contains(required) {
                    return true;
                }
                // Check for broader grants
                let broader = match (scope, content) {
                    (EventScope::Session, EventContent::Meta) => &[
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
                    ][..],
                    (EventScope::Session, EventContent::Full) => &[Capability::Events {
                        scope: EventScope::Global,
                        content: EventContent::Full,
                    }][..],
                    (EventScope::Global, EventContent::Meta) => &[Capability::Events {
                        scope: EventScope::Global,
                        content: EventContent::Full,
                    }][..],
                    (EventScope::Global, EventContent::Full) => &[][..],
                };
                broader.iter().any(|c| self.caps.contains(c))
            }
            // Gate:mutate satisfies gate (if you can mutate, you can gate)
            Capability::HooksGate => {
                self.caps.contains(&Capability::HooksGate)
                    || self.caps.contains(&Capability::HooksGateMutate)
            }
            // Gate:mutate satisfies mutate (highest mutation tier
            // encompasses benign mutations).
            Capability::HooksMutate => {
                self.caps.contains(&Capability::HooksMutate)
                    || self.caps.contains(&Capability::HooksGateMutate)
            }
            // Session capability hierarchy: manage → prompt → read.
            // If you can manage sessions, you can also prompt and read.
            Capability::SessionsRead => {
                self.caps.contains(&Capability::SessionsRead)
                    || self.caps.contains(&Capability::SessionsManage)
                    || self.caps.contains(&Capability::SessionsPrompt)
            }
            Capability::SessionsPrompt => {
                self.caps.contains(&Capability::SessionsPrompt)
                    || self.caps.contains(&Capability::SessionsManage)
            }
            _ => self.caps.contains(required),
        }
    }

    /// Compute the delta between two capability sets.
    /// Returns (added, removed) — capabilities in `self` but not `other`,
    /// and vice versa.
    pub fn difference(&self, other: &CapabilitySet) -> CapabilityDelta {
        let added: Vec<Capability> = self.caps.difference(&other.caps).cloned().collect();
        let removed: Vec<Capability> = other.caps.difference(&self.caps).cloned().collect();
        CapabilityDelta {
            added: added.into_iter().map(|c| c.id().to_string()).collect(),
            removed: removed.into_iter().map(|c| c.id().to_string()).collect(),
        }
    }

    /// Return only capabilities present in both sets (exact match).
    ///
    /// Unlike `satisfies()`, this does NOT apply the capability hierarchy
    /// (e.g., `events:global:full` does not intersect with a set that only
    /// has `events:session:meta`). This is intentional for security clamping:
    /// we want to drop any stored capability the manifest didn't explicitly request.
    pub fn intersect(&self, other: &CapabilitySet) -> CapabilitySet {
        let mut result = CapabilitySet::empty();
        for cap in &self.caps {
            if other.caps.contains(cap) {
                result.caps.insert(cap.clone());
            }
        }
        result
    }

    /// Iterate over granted capabilities.
    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.caps.iter()
    }

    /// Collect capability ID strings from this set.
    pub fn to_ids(&self) -> Vec<String> {
        self.caps.iter().map(|c| c.id().to_string()).collect()
    }

    /// Number of granted capabilities.
    pub fn len(&self) -> usize {
        self.caps.len()
    }

    /// Whether the set is empty (not counting always-granted).
    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        Self {
            caps: iter.into_iter().collect(),
        }
    }
}

/// Reconstruct a `CapabilitySet` from a list of capability ID strings.
///
/// Used to rebuild the capability set from persisted consent state.
/// Unknown IDs are skipped with a `tracing::warn`.
pub fn reconstruct_caps(ids: &[String]) -> CapabilitySet {
    let mut caps = CapabilitySet::empty();
    for id in ids {
        match Capability::from_id(id) {
            Some(cap) => caps.grant(cap),
            None => tracing::warn!("unknown capability ID in consent state: {}", id),
        }
    }
    caps
}

// ── Capability delta ───────────────────────────────────────────────

/// The difference between two capability sets, used in consent prompts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityDelta {
    /// Capability IDs that are newly requested (in `self`, not in `other`).
    pub added: Vec<String>,
    /// Capability IDs that are being removed (in `other`, not in `self`).
    pub removed: Vec<String>,
}

impl CapabilityDelta {
    /// Whether there are any changes at all.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// Whether any newly-added capability requires individual consent.
    pub fn has_sensitive_additions(&self) -> bool {
        // We check by string prefix since the Capability enum's
        // requires_individual_consent needs the typed value. The IDs
        // are stable strings from Capability::id().
        self.added.iter().any(|id| {
            id.starts_with("hooks:gate")
                || id.starts_with("hooks:mutate:")
                || id == "permissions:resolve"
                || id == "events:global:full"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_always_granted() {
        let set = CapabilitySet::always_granted();
        assert!(set.has(&Capability::Storage));
        assert!(set.has(&Capability::ConfigRead));
        assert!(!set.has(&Capability::Ui));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_observe_only_satisfies() {
        let set = CapabilitySet::observe_only();
        // Observe hooks are satisfied.
        assert!(set.satisfies(&Capability::HooksObserve));
        // Always-granted capabilities.
        assert!(set.has(&Capability::Storage));
        assert!(set.has(&Capability::ConfigRead));
        // Mutation/gate/registration capabilities are NOT satisfied.
        assert!(!set.satisfies(&Capability::HooksMutate));
        assert!(!set.satisfies(&Capability::HooksMutateHeaders));
        assert!(!set.satisfies(&Capability::HooksMutateShellEnv));
        assert!(!set.satisfies(&Capability::HooksMutateChatParams));
        assert!(!set.satisfies(&Capability::HooksGate));
        assert!(!set.satisfies(&Capability::HooksGateMutate));
        assert!(!set.has(&Capability::Register));
        assert!(!set.has(&Capability::Ui));
        // Sessions/events are not granted.
        assert!(!set.satisfies(&Capability::SessionsRead));
        assert!(!set.satisfies(&Capability::PermissionsResolve));
    }

    #[test]
    fn test_legacy_full_satisfies_all_hooks() {
        let set = CapabilitySet::legacy_full();
        // Every hook capability must be satisfied
        assert!(set.satisfies(&Capability::HooksObserve));
        assert!(set.satisfies(&Capability::HooksMutate));
        assert!(set.satisfies(&Capability::HooksMutateHeaders));
        assert!(set.satisfies(&Capability::HooksMutateShellEnv));
        assert!(set.satisfies(&Capability::HooksMutateChatParams));
        assert!(set.satisfies(&Capability::HooksGate));
        assert!(set.satisfies(&Capability::HooksGateMutate));
        // All host function capabilities
        assert!(set.has(&Capability::Ui));
        assert!(set.has(&Capability::Register));
        assert!(set.has(&Capability::PermissionsResolve));
        // All session capabilities (hierarchy: manage → prompt → read)
        assert!(set.satisfies(&Capability::SessionsRead));
        assert!(set.satisfies(&Capability::SessionsManage));
        assert!(set.satisfies(&Capability::SessionsPrompt));
        // Events: global+full satisfies everything
        assert!(set.satisfies(&Capability::Events {
            scope: EventScope::Session,
            content: EventContent::Meta
        }));
    }

    #[test]
    fn test_grant_revoke() {
        let mut set = CapabilitySet::empty();
        set.grant(Capability::HooksObserve);
        assert!(set.has(&Capability::HooksObserve));
        set.revoke(&Capability::HooksObserve);
        assert!(!set.has(&Capability::HooksObserve));
    }

    #[test]
    fn test_satisfies_exact() {
        let set = CapabilitySet::from_iter([Capability::HooksGate]);
        assert!(set.satisfies(&Capability::HooksGate));
        assert!(!set.satisfies(&Capability::HooksGateMutate));
    }

    #[test]
    fn test_satisfies_gate_mutate_implies_gate() {
        let set = CapabilitySet::from_iter([Capability::HooksGateMutate]);
        assert!(set.satisfies(&Capability::HooksGate));
        assert!(set.satisfies(&Capability::HooksGateMutate));
    }

    #[test]
    fn test_satisfies_events_broader_scope() {
        let set = CapabilitySet::from_iter([Capability::Events {
            scope: EventScope::Global,
            content: EventContent::Full,
        }]);
        // global+full satisfies everything
        assert!(set.satisfies(&Capability::Events {
            scope: EventScope::Session,
            content: EventContent::Meta
        }));
        assert!(set.satisfies(&Capability::Events {
            scope: EventScope::Session,
            content: EventContent::Full
        }));
        assert!(set.satisfies(&Capability::Events {
            scope: EventScope::Global,
            content: EventContent::Meta
        }));
    }

    #[test]
    fn test_satisfies_events_narrower_does_not_satisfy_broader() {
        let set = CapabilitySet::from_iter([Capability::Events {
            scope: EventScope::Session,
            content: EventContent::Meta,
        }]);
        // session+meta does NOT satisfy session+full or global
        assert!(!set.satisfies(&Capability::Events {
            scope: EventScope::Session,
            content: EventContent::Full
        }));
        assert!(!set.satisfies(&Capability::Events {
            scope: EventScope::Global,
            content: EventContent::Meta
        }));
    }

    #[test]
    fn test_satisfies_gate_mutate_implies_mutate() {
        // HooksGateMutate (highest tier) should satisfy HooksMutate (medium)
        let set = CapabilitySet::from_iter([Capability::HooksGateMutate]);
        assert!(set.satisfies(&Capability::HooksMutate));
        // But not the reverse
        let set2 = CapabilitySet::from_iter([Capability::HooksMutate]);
        assert!(!set2.satisfies(&Capability::HooksGateMutate));
    }

    #[test]
    fn test_satisfies_sessions_hierarchy() {
        // SessionsManage implies SessionsPrompt and SessionsRead
        let set = CapabilitySet::from_iter([Capability::SessionsManage]);
        assert!(set.satisfies(&Capability::SessionsPrompt));
        assert!(set.satisfies(&Capability::SessionsRead));

        // SessionsPrompt implies SessionsRead
        let set2 = CapabilitySet::from_iter([Capability::SessionsPrompt]);
        assert!(set2.satisfies(&Capability::SessionsRead));
        // But not the reverse
        assert!(!set2.satisfies(&Capability::SessionsManage));

        // SessionsRead does not imply higher
        let set3 = CapabilitySet::from_iter([Capability::SessionsRead]);
        assert!(!set3.satisfies(&Capability::SessionsPrompt));
        assert!(!set3.satisfies(&Capability::SessionsManage));
    }

    #[test]
    fn test_difference_added_and_removed() {
        let old = CapabilitySet::from_iter([Capability::HooksObserve, Capability::HooksMutate]);
        let new = CapabilitySet::from_iter([Capability::HooksObserve, Capability::HooksGate]);
        let delta = new.difference(&old);
        assert_eq!(delta.added, vec!["hooks:gate"]);
        assert_eq!(delta.removed, vec!["hooks:mutate"]);
    }

    #[test]
    fn test_difference_empty() {
        let set = CapabilitySet::from_iter([Capability::Ui, Capability::Register]);
        let delta = set.difference(&set);
        assert!(delta.is_empty());
    }

    #[test]
    fn test_intersect() {
        // Overlapping sets → result is the overlap.
        let a = CapabilitySet::from_iter([Capability::HooksObserve, Capability::HooksGate]);
        let b = CapabilitySet::from_iter([Capability::HooksGate, Capability::HooksMutate]);
        let overlap = a.intersect(&b);
        assert!(overlap.has(&Capability::HooksGate));
        assert!(!overlap.has(&Capability::HooksObserve));
        assert!(!overlap.has(&Capability::HooksMutate));
        assert_eq!(overlap.len(), 1);

        // Disjoint sets → result is empty.
        let c = CapabilitySet::from_iter([Capability::Ui, Capability::Register]);
        let d = CapabilitySet::from_iter([Capability::HooksGate, Capability::HooksObserve]);
        let empty = c.intersect(&d);
        assert!(empty.is_empty());

        // Self-intersect → returns a copy (all caps present in both).
        let e = CapabilitySet::from_iter([Capability::HooksObserve, Capability::HooksGate]);
        let self_overlap = e.intersect(&e);
        assert_eq!(self_overlap, e);
    }

    #[test]
    fn test_delta_has_sensitive_additions() {
        let old = CapabilitySet::empty();
        let new = CapabilitySet::from_iter([Capability::HooksGate, Capability::Ui]);
        let delta = new.difference(&old);
        assert!(delta.has_sensitive_additions());

        let new2 = CapabilitySet::from_iter([Capability::Ui]);
        let delta2 = new2.difference(&old);
        assert!(!delta2.has_sensitive_additions());
    }

    #[test]
    fn test_capability_id_stability() {
        // IDs are used in consent prompts and audit logs — they must be stable.
        assert_eq!(Capability::Storage.id(), "storage");
        assert_eq!(Capability::HooksGate.id(), "hooks:gate");
        assert_eq!(Capability::HooksGateMutate.id(), "hooks:gate:mutate");
        assert_eq!(
            Capability::Events {
                scope: EventScope::Global,
                content: EventContent::Full
            }
            .id(),
            "events:global:full"
        );
    }

    #[test]
    fn test_risk_tier_classification() {
        assert_eq!(Capability::Storage.risk_tier(), RiskTier::AlwaysGranted);
        assert_eq!(Capability::Ui.risk_tier(), RiskTier::Low);
        assert_eq!(Capability::SessionsRead.risk_tier(), RiskTier::Medium);
        assert_eq!(Capability::HooksGate.risk_tier(), RiskTier::High);
        assert_eq!(Capability::HooksGateMutate.risk_tier(), RiskTier::Highest);
        assert_eq!(
            Capability::Events {
                scope: EventScope::Global,
                content: EventContent::Full
            }
            .risk_tier(),
            RiskTier::High
        );
    }

    #[test]
    fn test_requires_individual_consent() {
        assert!(!Capability::Storage.requires_individual_consent());
        assert!(!Capability::Ui.requires_individual_consent());
        assert!(Capability::HooksGate.requires_individual_consent());
        assert!(Capability::HooksGateMutate.requires_individual_consent());
        assert!(Capability::PermissionsResolve.requires_individual_consent());
        assert!(Capability::Events {
            scope: EventScope::Global,
            content: EventContent::Full
        }
        .requires_individual_consent());
        // session+meta events don't need individual consent
        assert!(!Capability::Events {
            scope: EventScope::Session,
            content: EventContent::Meta
        }
        .requires_individual_consent());
    }

    #[test]
    fn test_is_gate_is_mutate() {
        assert!(Capability::HooksGate.is_gate());
        assert!(Capability::HooksGateMutate.is_gate());
        assert!(!Capability::HooksObserve.is_gate());

        assert!(Capability::HooksMutate.is_mutate());
        assert!(Capability::HooksMutateHeaders.is_mutate());
        assert!(Capability::HooksGateMutate.is_mutate());
        assert!(!Capability::HooksGate.is_mutate());
    }

    #[test]
    fn test_from_id_round_trip() {
        // Every capability should round-trip: id() → from_id() → original.
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
            let id = cap.id();
            let reconstructed = Capability::from_id(id);
            assert_eq!(
                reconstructed.as_ref(),
                Some(cap),
                "round-trip failed for id {:?}",
                id
            );
        }
    }

    #[test]
    fn test_from_id_unknown() {
        assert!(Capability::from_id("nonexistent").is_none());
        assert!(Capability::from_id(crate::consent::LEGACY_FULL_SENTINEL).is_none());
        assert!(Capability::from_id("events:bogus").is_none());
        assert!(Capability::from_id("events:session:unknown").is_none());
        assert!(Capability::from_id("events:unknown:meta").is_none());
    }

    #[test]
    fn test_reconstruct_caps() {
        let ids = vec![
            "storage".to_string(),
            "hooks:observe".to_string(),
            "hooks:gate".to_string(),
            "nonexistent".to_string(), // skipped
        ];
        let caps = reconstruct_caps(&ids);
        assert!(caps.has(&Capability::Storage));
        assert!(caps.has(&Capability::HooksObserve));
        assert!(caps.has(&Capability::HooksGate));
        assert_eq!(caps.len(), 3); // nonexistent skipped
    }

    #[test]
    fn test_to_ids() {
        let set = CapabilitySet::from_iter([
            Capability::Storage,
            Capability::HooksObserve,
            Capability::HooksGate,
        ]);
        let ids = set.to_ids();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"storage".to_string()));
        assert!(ids.contains(&"hooks:observe".to_string()));
        assert!(ids.contains(&"hooks:gate".to_string()));
    }
}
