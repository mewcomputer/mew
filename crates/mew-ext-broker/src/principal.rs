//! Principal model: identifies a connection and its granted capabilities.

use crate::capabilities::CapabilitySet;

/// Unique identifier for a principal. ULID for sortability and uniqueness.
pub type PrincipalId = ulid::Ulid;

/// What kind of connection a principal represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// A frontend (TUI, web, iOS) driving sessions interactively.
    /// Gets the `client` profile: today's full `ClientMessage` surface.
    Client,
    /// An extension process (stdio-spawned or socket-attached).
    Extension,
}

/// A connection principal with granted capabilities.
///
/// Every connection to the daemon — frontend or extension — is bound
/// to a principal with a granted capability set. The daemon enforces
/// capabilities per method and per scope.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Unique identifier for this connection.
    pub id: PrincipalId,
    /// Human-readable name (extension name or "frontend").
    pub name: String,
    /// Whether this is a frontend or extension.
    pub kind: PrincipalKind,
    /// The capabilities granted to this principal.
    pub capabilities: CapabilitySet,
}

impl Principal {
    /// Create a new frontend principal with the client profile
    /// (all session permissions, no extension capabilities).
    pub fn frontend(name: impl Into<String>) -> Self {
        Self {
            id: PrincipalId::new(),
            name: name.into(),
            kind: PrincipalKind::Client,
            // Frontends get full session access — today's behavior.
            capabilities: CapabilitySet::from_iter([
                crate::capabilities::Capability::SessionsRead,
                crate::capabilities::Capability::SessionsManage,
                crate::capabilities::Capability::SessionsPrompt,
                crate::capabilities::Capability::PermissionsResolve,
            ]),
        }
    }

    /// Create a new extension principal with the given capabilities.
    pub fn extension(name: impl Into<String>, capabilities: CapabilitySet) -> Self {
        Self {
            id: PrincipalId::new(),
            name: name.into(),
            kind: PrincipalKind::Extension,
            capabilities,
        }
    }

    /// Check if this principal has a specific capability.
    pub fn has_capability(&self, cap: &crate::capabilities::Capability) -> bool {
        self.capabilities.satisfies(cap)
    }

    /// Whether this principal is an extension.
    pub fn is_extension(&self) -> bool {
        self.kind == PrincipalKind::Extension
    }

    /// Whether this principal is a frontend.
    pub fn is_frontend(&self) -> bool {
        self.kind == PrincipalKind::Client
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Capability;

    #[test]
    fn test_frontend_has_session_capabilities() {
        let p = Principal::frontend("tui");
        assert!(p.has_capability(&Capability::SessionsRead));
        assert!(p.has_capability(&Capability::SessionsManage));
        assert!(p.has_capability(&Capability::SessionsPrompt));
        assert!(p.has_capability(&Capability::PermissionsResolve));
        assert!(!p.has_capability(&Capability::HooksGate));
    }

    #[test]
    fn test_extension_with_gate() {
        let caps = CapabilitySet::from_iter([Capability::Storage, Capability::HooksGate]);
        let p = Principal::extension("bash-gate", caps);
        assert!(p.has_capability(&Capability::HooksGate));
        assert!(!p.has_capability(&Capability::HooksGateMutate));
    }

    #[test]
    fn test_principal_ids_are_unique() {
        let p1 = Principal::frontend("tui");
        let p2 = Principal::frontend("web");
        assert_ne!(p1.id, p2.id);
    }

    #[test]
    fn test_kind_checks() {
        let fe = Principal::frontend("tui");
        assert!(fe.is_frontend());
        assert!(!fe.is_extension());

        let ext = Principal::extension("ext", CapabilitySet::empty());
        assert!(ext.is_extension());
        assert!(!ext.is_frontend());
    }
}
