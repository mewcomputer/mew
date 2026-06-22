//! Classifier prompts for Auto mode (permission-decision via small LLM).
//!
//! Auto mode routes permission decisions through a small, cheap LLM instead
//! of the user. The classifier receives a formatted prompt and replies with
//! one of:
//!
//! - `"allow"` — the call is safe and within normal agent operation
//! - `"deny"` — the call would cause irreversible harm
//! - `"escalate"` — unsure; ask the user
//!
//! This module owns the prompt format. The classifier call itself lives in
//! the permission engine (or wherever Auto is wired up); the model response
//! parsing is its responsibility.
//!
//! As of this writing Auto isn't built yet — these functions are stubs the
//! future implementation can call without rewriting the prompt format.

/// What the classifier should report back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifierDecision {
    Allow,
    Deny,
    Escalate,
}

impl ClassifierDecision {
    /// Parse the classifier's text response. Tolerant of surrounding
    /// whitespace and common punctuation; case-insensitive on the verb.
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim().trim_end_matches('.').trim_end_matches('!').to_lowercase();
        match t.as_str() {
            "allow" | "approved" | "yes" => Some(Self::Allow),
            "deny" | "denied" | "no" | "block" => Some(Self::Deny),
            "escalate" | "ask" | "unsure" | "human" => Some(Self::Escalate),
            _ => None,
        }
    }
}

/// Build the permission-decision prompt for the classifier LLM. The prompt
/// is a single user-role message asking the classifier to render one of
/// `allow` / `deny` / `escalate` for the given tool call.
///
/// `cwd` and a short activity hint (`recent_action`) are optional context
/// the classifier can use to weigh the decision. Pass `None` to omit.
///
/// `sensitivity` is the tool's sensitivity tier as a string ("ReadOnly" /
/// "Mutating" / "Dangerous"). Passed as a string to keep `mew-prompts`
/// free of the `mew-tools` dependency (avoiding a workspace cycle); the
/// caller converts from `mew_tools::Sensitivity` if it has it.
pub fn permission_decision(
    tool_name: &str,
    input: &serde_json::Value,
    sensitivity: &str,
    cwd: Option<&str>,
    recent_action: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(
        "You are a permission classifier for an AI agent harness. Decide whether the \
         following tool call should be allowed, denied, or escalated to a human for review.\n\n",
    );
    out.push_str("Respond with EXACTLY one word on a single line:\n");
    out.push_str("  allow    — the call is safe and within normal agent operation\n");
    out.push_str("  deny     — the call would cause irreversible harm\n");
    out.push_str("  escalate — you are not sure; ask the user\n\n");
    out.push_str(&format!("Tool: {tool_name}\n"));
    out.push_str(&format!("Sensitivity: {sensitivity}\n"));
    out.push_str(&format!(
        "Input: {}\n",
        serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
    ));
    if let Some(c) = cwd {
        out.push_str(&format!("Working directory: {c}\n"));
    }
    if let Some(a) = recent_action {
        out.push_str(&format!("Recent activity: {a}\n"));
    }
    out.push_str("\nDecision:");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_classifier_decision_parse_allow() {
        assert_eq!(ClassifierDecision::parse("allow"), Some(ClassifierDecision::Allow));
        assert_eq!(ClassifierDecision::parse("Allow"), Some(ClassifierDecision::Allow));
        assert_eq!(ClassifierDecision::parse("allow."), Some(ClassifierDecision::Allow));
        assert_eq!(ClassifierDecision::parse("  approved"), Some(ClassifierDecision::Allow));
    }

    #[test]
    fn test_classifier_decision_parse_deny() {
        assert_eq!(ClassifierDecision::parse("deny"), Some(ClassifierDecision::Deny));
        assert_eq!(ClassifierDecision::parse("denied"), Some(ClassifierDecision::Deny));
        assert_eq!(ClassifierDecision::parse("block"), Some(ClassifierDecision::Deny));
    }

    #[test]
    fn test_classifier_decision_parse_escalate() {
        assert_eq!(
            ClassifierDecision::parse("escalate"),
            Some(ClassifierDecision::Escalate)
        );
        assert_eq!(ClassifierDecision::parse("ask"), Some(ClassifierDecision::Escalate));
        assert_eq!(ClassifierDecision::parse("unsure"), Some(ClassifierDecision::Escalate));
    }

    #[test]
    fn test_classifier_decision_parse_unknown_returns_none() {
        assert_eq!(ClassifierDecision::parse("maybe"), None);
        assert_eq!(ClassifierDecision::parse(""), None);
    }

    #[test]
    fn test_permission_decision_includes_tool_name_and_input() {
        let prompt = permission_decision(
            "bash",
            &json!({"command": "rm -rf /tmp/foo"}),
            "Dangerous",
            Some("/home/mew"),
            Some("compiling project"),
        );
        assert!(prompt.contains("Tool: bash"));
        assert!(prompt.contains("rm -rf /tmp/foo"));
        assert!(prompt.contains("Dangerous"));
        assert!(prompt.contains("/home/mew"));
        assert!(prompt.contains("compiling project"));
        assert!(prompt.contains("Decision:"));
    }

    #[test]
    fn test_permission_decision_without_optional_context() {
        let prompt = permission_decision(
            "read",
            &json!({"path": "README.md"}),
            "ReadOnly",
            None,
            None,
        );
        assert!(prompt.contains("Tool: read"));
        assert!(prompt.contains("README.md"));
        assert!(!prompt.contains("Working directory"));
        assert!(!prompt.contains("Recent activity"));
    }
}
