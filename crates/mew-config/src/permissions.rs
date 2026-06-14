use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use tokio::sync::Mutex;

/// Decision specified in a config rule.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleDecision {
    Allow,
    Deny,
    Ask,
}

/// A single permission rule from config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    pub decision: RuleDecision,
    #[serde(default)]
    pub r#match: MatchConditions,
}

/// Match conditions for a rule. All fields are optional; at least one must be
/// present for the rule to match anything.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MatchConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_glob: Option<String>,
}

/// Evaluates permission rules and tracks session-level allowances.
pub struct PermissionEngine {
    rules: Vec<PermissionRule>,
    session_allows: Mutex<HashSet<String>>,
}

impl PermissionEngine {
    pub fn new(rules: Vec<PermissionRule>) -> Self {
        Self {
            rules,
            session_allows: Mutex::new(HashSet::new()),
        }
    }

    /// Evaluate rules for a tool call and return the runtime decision.
    ///
    /// Evaluation order:
    /// 1. Deny rules (first match wins)
    /// 2. Allow rules (first match wins)
    /// 3. Session-allow cache
    /// 4. Default based on sensitivity
    pub async fn check(
        &self,
        tool_name: &str,
        input: &Value,
        sensitivity: mew_tools::Sensitivity,
    ) -> mew_hooks::PermissionDecision {
        // 1. Deny rules
        for rule in &self.rules {
            if rule.decision != RuleDecision::Deny {
                continue;
            }
            if !self.rule_applies(rule, tool_name) {
                continue;
            }
            if self.matches(&rule.r#match, input) {
                return mew_hooks::PermissionDecision::Deny;
            }
        }

        // 2. Allow rules
        for rule in &self.rules {
            if rule.decision != RuleDecision::Allow {
                continue;
            }
            if !self.rule_applies(rule, tool_name) {
                continue;
            }
            if self.matches(&rule.r#match, input) {
                return mew_hooks::PermissionDecision::AllowOnce;
            }
        }

        // 2.5 Ask rules (force a prompt even for ReadOnly tools)
        for rule in &self.rules {
            if rule.decision != RuleDecision::Ask {
                continue;
            }
            if !self.rule_applies(rule, tool_name) {
                continue;
            }
            if self.matches(&rule.r#match, input) {
                return mew_hooks::PermissionDecision::Prompt;
            }
        }

        // 3. Session allows
        let session = self.session_allows.lock().await;
        if session.contains(tool_name) {
            return mew_hooks::PermissionDecision::AllowOnce;
        }
        drop(session);

        // 4. Default based on sensitivity
        match sensitivity {
            mew_tools::Sensitivity::ReadOnly => mew_hooks::PermissionDecision::AllowOnce,
            _ => mew_hooks::PermissionDecision::Prompt,
        }
    }

    /// Record that a tool is allowed for the remainder of this session.
    pub async fn add_session_allow(&self, tool_name: &str) {
        self.session_allows
            .lock()
            .await
            .insert(tool_name.to_string());
    }

    fn rule_applies(&self, rule: &PermissionRule, tool_name: &str) -> bool {
        rule.tool == "*" || rule.tool == tool_name
    }

    fn matches(&self, conditions: &MatchConditions, input: &Value) -> bool {
        // If no conditions are set, the rule matches everything for the tool.
        if conditions.command_prefix.is_none() && conditions.path_glob.is_none() {
            return true;
        }

        if let Some(ref prefix) = conditions.command_prefix {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                if cmd.starts_with(prefix) {
                    return true;
                }
            }
        }

        if let Some(ref glob) = conditions.path_glob {
            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                if let Ok(g) = globset::Glob::new(glob) {
                    if g.compile_matcher().is_match(path) {
                        return true;
                    }
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_input(path: &str) -> Value {
        serde_json::json!({ "path": path })
    }

    fn make_bash_input(command: &str) -> Value {
        serde_json::json!({ "command": command })
    }

    #[tokio::test]
    async fn test_default_readonly_allow() {
        let engine = PermissionEngine::new(vec![]);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn test_default_mutating_prompt() {
        let engine = PermissionEngine::new(vec![]);
        let decision = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_dangerous_prompt() {
        let engine = PermissionEngine::new(vec![]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("ls"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_allow_rule() {
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "read".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                path_glob: Some("**/*.rs".to_string()),
                ..Default::default()
            },
        }]);
        let decision = engine
            .check(
                "read",
                &make_input("src/lib.rs"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn test_allow_rule_no_match() {
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "read".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                path_glob: Some("**/*.rs".to_string()),
                ..Default::default()
            },
        }]);
        let decision = engine
            .check(
                "read",
                &make_input("readme.md"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce); // default for ReadOnly
    }

    #[tokio::test]
    async fn test_deny_rule() {
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "write".to_string(),
            decision: RuleDecision::Deny,
            r#match: MatchConditions {
                path_glob: Some("/etc/**".to_string()),
                ..Default::default()
            },
        }]);
        let decision = engine
            .check(
                "write",
                &make_input("/etc/passwd"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Deny);
    }

    #[tokio::test]
    async fn test_deny_overrides_allow() {
        let engine = PermissionEngine::new(vec![
            PermissionRule {
                tool: "*".to_string(),
                decision: RuleDecision::Allow,
                r#match: MatchConditions::default(),
            },
            PermissionRule {
                tool: "bash".to_string(),
                decision: RuleDecision::Deny,
                r#match: MatchConditions::default(),
            },
        ]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("ls"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Deny);
    }

    #[tokio::test]
    async fn test_command_prefix() {
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_prefix: Some("git status".to_string()),
                ..Default::default()
            },
        }]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git status"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);

        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -rf /"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_session_allow() {
        let engine = PermissionEngine::new(vec![]);
        engine.add_session_allow("write").await;
        let decision = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn test_wildcard_tool() {
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "*".to_string(),
            decision: RuleDecision::Deny,
            r#match: MatchConditions::default(),
        }]);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Deny);
    }

    #[tokio::test]
    async fn test_session_allow_persists_across_checks() {
        let engine = PermissionEngine::new(vec![]);

        // First check without session allow should prompt
        let d1 = engine
            .check(
                "bash",
                &make_bash_input("ls"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(d1, mew_hooks::PermissionDecision::Prompt);

        // Add session allow
        engine.add_session_allow("bash").await;

        // Second check should allow
        let d2 = engine
            .check(
                "bash",
                &make_bash_input("ls"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(d2, mew_hooks::PermissionDecision::AllowOnce);

        // Other tools should still prompt
        let d3 = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(d3, mew_hooks::PermissionDecision::Prompt);
    }
}
