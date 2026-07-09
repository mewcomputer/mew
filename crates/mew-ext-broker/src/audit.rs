//! Gate audit log types.
//!
//! Every gate decision (approve/deny/mutate) is written to an append-only
//! audit log under the daemon's data dir. `mew ext audit <name>` prints it.

/// The outcome of a gate hook decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateOutcome {
    /// Hook returned `Proceed` (unchanged input).
    Proceed,
    /// Hook returned `Block`.
    Block,
    /// Hook returned `Suppress`.
    Suppress,
    /// Hook timed out and failed open (default decision used).
    Timeout,
    /// Hook errored and failed open.
    Error,
    /// Hook returned `Proceed` with modified input (`hooks:gate:mutate`).
    Mutated,
}

impl GateOutcome {
    /// Whether the gate allowed the action to proceed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Proceed | Self::Mutated)
    }

    /// Whether the gate blocked the action.
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block | Self::Suppress)
    }

    /// Whether the gate failed open (timeout or error).
    pub fn is_fail_open(&self) -> bool {
        matches!(self, Self::Timeout | Self::Error)
    }
}

/// One entry in the gate audit log.
///
/// Written for every `on_permission_ask` and `on_tool_execute_before`
/// decision made by a gate extension. Append-only; never updated.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GateAuditEntry {
    /// The extension that made the decision.
    pub extension: String,
    /// The session the decision was for.
    pub session_id: String,
    /// The tool being gated (e.g. "bash", "write").
    pub tool: String,
    /// Hash of the tool input (for detecting repeat decisions without
    /// logging the full input, which may contain secrets).
    pub input_hash: String,
    /// The gate's decision.
    pub outcome: GateOutcome,
    /// When the decision was made (ISO 8601).
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl GateAuditEntry {
    /// Create a new audit entry with the current timestamp.
    pub fn new(
        extension: impl Into<String>,
        session_id: impl Into<String>,
        tool: impl Into<String>,
        input_hash: impl Into<String>,
        outcome: GateOutcome,
    ) -> Self {
        Self {
            extension: extension.into(),
            session_id: session_id.into(),
            tool: tool.into(),
            input_hash: input_hash.into(),
            outcome,
            timestamp: chrono::Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_classification() {
        assert!(GateOutcome::Proceed.is_allowed());
        assert!(GateOutcome::Mutated.is_allowed());
        assert!(!GateOutcome::Block.is_allowed());

        assert!(GateOutcome::Block.is_blocked());
        assert!(GateOutcome::Suppress.is_blocked());
        assert!(!GateOutcome::Proceed.is_blocked());

        assert!(GateOutcome::Timeout.is_fail_open());
        assert!(GateOutcome::Error.is_fail_open());
        assert!(!GateOutcome::Proceed.is_fail_open());
    }

    #[test]
    fn test_audit_entry_creation() {
        let entry = GateAuditEntry::new(
            "bash-gate",
            "01HXYZ...",
            "bash",
            "sha256:abc123",
            GateOutcome::Block,
        );
        assert_eq!(entry.extension, "bash-gate");
        assert_eq!(entry.tool, "bash");
        assert_eq!(entry.outcome, GateOutcome::Block);
    }

    #[test]
    fn test_audit_entry_serialization() {
        let entry = GateAuditEntry::new(
            "gate-ext",
            "session-1",
            "write",
            "sha256:deadbeef",
            GateOutcome::Mutated,
        );
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: GateAuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.extension, "gate-ext");
        assert_eq!(parsed.outcome, GateOutcome::Mutated);
    }
}
