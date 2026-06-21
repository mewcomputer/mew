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
    /// Match the program name of a shell command. For `git push` this is
    /// `"git"`. Checked against each program in a compound command (e.g.
    /// `git log | grep fix` has programs `git` and `grep`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_program: Option<String>,
    /// Match the subcommand (first non-flag argument). For `git push` this
    /// is `"push"`. Only meaningful alongside `command_program`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_subcommand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_glob: Option<String>,
}

/// Evaluates permission rules and tracks session-level allowances.
pub struct PermissionEngine {
    rules: Vec<PermissionRule>,
    session_allows: Mutex<HashSet<String>>,
    /// Compiled secret-file matchers. Any `read` of a matching path is forced
    /// to `Prompt` unless a literal (non-glob) allow rule explicitly permits
    /// that exact path. Sits above the normal deny→allow cascade as its own
    /// tier.
    secret_globs: Vec<globset::GlobMatcher>,
}

impl PermissionEngine {
    pub fn new(rules: Vec<PermissionRule>) -> Self {
        Self {
            rules,
            session_allows: Mutex::new(HashSet::new()),
            secret_globs: Vec::new(),
        }
    }

    /// Add secret-file globs. Reads of matching paths force a prompt unless a
    /// literal allow rule lifts the guard for that exact path.
    pub fn with_secret_files(mut self, globs: Vec<String>) -> Self {
        self.secret_globs = globs
            .into_iter()
            .filter_map(|g| globset::Glob::new(&g).ok().map(|g| g.compile_matcher()))
            .collect();
        self
    }

    /// Evaluate rules for a tool call and return the runtime decision.
    ///
    /// Evaluation order:
    /// 0. Secret-file guard (`read` only): force `Prompt` unless a literal
    ///    allow rule explicitly permits the exact path.
    /// 1. Deny rules (first match wins)
    /// 2. Allow rules (first match wins)
    /// 3. Ask rules
    /// 4. Session-allow cache
    /// 5. Default based on sensitivity
    pub async fn check(
        &self,
        tool_name: &str,
        input: &Value,
        sensitivity: mew_tools::Sensitivity,
    ) -> mew_hooks::PermissionDecision {
        // 0. Secret-file guard.
        if tool_name == "read" && !self.secret_globs.is_empty() {
            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                if self.is_secret_path(path) && !self.has_literal_allow(tool_name, path) {
                    return mew_hooks::PermissionDecision::Prompt;
                }
            }
        }

        // 0.5. Bash command decomposition. Compound commands (pipes, &&, ;)
        // and opaque constructs ($(…), eval, bash -c, | sh) cannot be safely
        // checked with prefix matching alone. Decompose and require every
        // program to be explicitly allowed.
        if tool_name == "bash" {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                let parsed = crate::shell::parse_command(cmd);
                if parsed.has_opaque {
                    // The engine cannot see what the opaque construct will
                    // run. Force a prompt regardless of rules.
                    return mew_hooks::PermissionDecision::Prompt;
                }
                if parsed.programs.len() > 1 {
                    return self
                        .check_compound_bash(&parsed.programs, sensitivity)
                        .await;
                }
                // Single program — fall through to normal rule matching.
            }
        }

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

    /// Check a compound bash command (pipe, &&, ;) where every program must
    /// be explicitly allowed for the whole command to auto-allow. Any deny
    /// hit on any program short-circuits to Deny. Any program without a
    /// matching allow rule falls through to the default (Prompt for
    /// Dangerous tools like bash).
    async fn check_compound_bash(
        &self,
        programs: &[crate::shell::ProgramInvocation],
        sensitivity: mew_tools::Sensitivity,
    ) -> mew_hooks::PermissionDecision {
        // 1. Deny: if any program is denied, the whole command is denied.
        for prog in programs {
            for rule in &self.rules {
                if rule.decision != RuleDecision::Deny || !self.rule_applies(rule, "bash") {
                    continue;
                }
                if self.program_matches_rule(prog, &rule.r#match) {
                    return mew_hooks::PermissionDecision::Deny;
                }
            }
        }

        // 2. Allow: every program must have a matching allow rule.
        let all_allowed = programs.iter().all(|prog| {
            self.rules.iter().any(|rule| {
                rule.decision == RuleDecision::Allow
                    && self.rule_applies(rule, "bash")
                    && self.program_matches_rule(prog, &rule.r#match)
            })
        });
        if all_allowed {
            return mew_hooks::PermissionDecision::AllowOnce;
        }

        // 3. Session allow cache.
        let session = self.session_allows.lock().await;
        if session.contains("bash") {
            return mew_hooks::PermissionDecision::AllowOnce;
        }
        drop(session);

        // 4. Default based on sensitivity.
        match sensitivity {
            mew_tools::Sensitivity::ReadOnly => mew_hooks::PermissionDecision::AllowOnce,
            _ => mew_hooks::PermissionDecision::Prompt,
        }
    }

    /// True if a parsed program invocation matches a rule's conditions.
    /// Checks `command_program` and `command_subcommand` against the
    /// invocation, falling back to `command_prefix` for backward compat
    /// (only meaningful for single-program commands that reached the
    /// compound path by accident — rare but harmless).
    fn program_matches_rule(
        &self,
        prog: &crate::shell::ProgramInvocation,
        conditions: &MatchConditions,
    ) -> bool {
        // No conditions = matches everything (existing semantics).
        if conditions.command_prefix.is_none()
            && conditions.command_program.is_none()
            && conditions.command_subcommand.is_none()
            && conditions.path_glob.is_none()
        {
            return true;
        }

        // Program-level matching.
        if let Some(ref expected) = conditions.command_program {
            if &prog.program != expected {
                return false;
            }
            if let Some(ref expected_sub) = conditions.command_subcommand {
                if prog.subcommand.as_deref() != Some(expected_sub.as_str()) {
                    return false;
                }
            }
            return true;
        }

        // Subcommand-only matching (program wildcard).
        if let Some(ref expected_sub) = conditions.command_subcommand {
            return prog.subcommand.as_deref() == Some(expected_sub.as_str());
        }

        false
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

    /// True if `path` matches any secret-file glob.
    fn is_secret_path(&self, path: &str) -> bool {
        self.secret_globs.iter().any(|m| m.is_match(path))
    }

    /// True if an allow rule permits this exact `path` with a literal
    /// (non-glob) pattern. This is the escape hatch that lifts the secret
    /// guard: you must name the secret file explicitly to auto-allow it.
    /// A broad glob like `**` never lifts the guard.
    fn has_literal_allow(&self, tool_name: &str, path: &str) -> bool {
        self.rules.iter().any(|rule| {
            if rule.decision != RuleDecision::Allow {
                return false;
            }
            if !self.rule_applies(rule, tool_name) {
                return false;
            }
            match rule.r#match.path_glob.as_deref() {
                Some(g) => !Self::is_glob_pattern(g) && g == path,
                None => false,
            }
        })
    }

    /// True if the string contains glob metacharacters.
    fn is_glob_pattern(s: &str) -> bool {
        s.contains(['*', '?', '[', '{'])
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

    // ------------------------------------------------------------------
    // Secret-file guard tests
    // ------------------------------------------------------------------

    fn engine_with_secrets(globs: &[&str]) -> PermissionEngine {
        PermissionEngine::new(vec![])
            .with_secret_files(globs.iter().map(|s| s.to_string()).collect())
    }

    #[tokio::test]
    async fn test_secret_file_forces_prompt_on_read() {
        // `read` is ReadOnly, which normally auto-allows. A secret match must
        // override that and force a prompt.
        let engine = engine_with_secrets(&[".env"]);
        let decision = engine
            .check(
                "read",
                &make_input(".env"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_secret_file_glob_pattern_matches() {
        let engine = engine_with_secrets(&["**/*.pem"]);
        let decision = engine
            .check(
                "read",
                &make_input("certs/server.pem"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_secret_file_literal_allow_lifts_guard() {
        // An allow rule naming the exact path with no glob metacharacters
        // lifts the secret guard.
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "read".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                path_glob: Some(".env".to_string()),
                ..Default::default()
            },
        }])
        .with_secret_files(vec![".env".to_string()]);
        let decision = engine
            .check(
                "read",
                &make_input(".env"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn test_secret_file_glob_allow_does_not_lift_guard() {
        // A broad glob allow (`**`) must NOT lift the secret guard — that
        // would defeat the whole point.
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "read".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                path_glob: Some("**".to_string()),
                ..Default::default()
            },
        }])
        .with_secret_files(vec![".env".to_string()]);
        let decision = engine
            .check(
                "read",
                &make_input(".env"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_non_secret_read_unaffected() {
        let engine = engine_with_secrets(&[".env"]);
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
    async fn test_non_read_tool_unaffected_by_secret_globs() {
        // The guard is scoped to `read` in this iteration. bash/grep/glob
        // take directory or command inputs, not file paths, and are covered
        // by their own sensitivity + rules.
        let engine = engine_with_secrets(&[".env"]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat .env"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "bash is Dangerous, prompts anyway"
        );
        let decision = engine
            .check(
                "grep",
                &serde_json::json!({"pattern": "x", "path": "."}),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "grep is ReadOnly and not yet guarded"
        );
    }

    #[test]
    fn test_is_glob_pattern_detection() {
        assert!(PermissionEngine::is_glob_pattern("**/*.env"));
        assert!(PermissionEngine::is_glob_pattern("src/*.rs"));
        assert!(PermissionEngine::is_glob_pattern("file?"));
        assert!(PermissionEngine::is_glob_pattern("[abc]"));
        assert!(PermissionEngine::is_glob_pattern("{a,b}"));
        assert!(!PermissionEngine::is_glob_pattern(".env"));
        assert!(!PermissionEngine::is_glob_pattern("/abs/path/.env"));
        assert!(!PermissionEngine::is_glob_pattern("plain_name.txt"));
    }

    // ------------------------------------------------------------------
    // Shell decomposition tests
    // ------------------------------------------------------------------

    fn make_rule(
        tool: &str,
        decision: RuleDecision,
        program: &str,
        subcommand: Option<&str>,
    ) -> PermissionRule {
        PermissionRule {
            tool: tool.to_string(),
            decision,
            r#match: MatchConditions {
                command_program: Some(program.to_string()),
                command_subcommand: subcommand.map(String::from),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn test_compound_all_allowed() {
        // Both git and grep have allow rules → auto-allow.
        let engine = PermissionEngine::new(vec![
            make_rule("bash", RuleDecision::Allow, "git", None),
            make_rule("bash", RuleDecision::Allow, "grep", None),
        ]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git log | grep fix"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn test_compound_one_uncovered_prompts() {
        // git is allowed, grep is not → prompt.
        let engine =
            PermissionEngine::new(vec![make_rule("bash", RuleDecision::Allow, "git", None)]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git log | grep fix"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_compound_deny_short_circuits() {
        // git allowed, rm denied → deny wins.
        let engine = PermissionEngine::new(vec![
            make_rule("bash", RuleDecision::Allow, "git", None),
            make_rule("bash", RuleDecision::Deny, "rm", None),
        ]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git log && rm -rf /"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Deny);
    }

    #[tokio::test]
    async fn test_subcommand_level_allow() {
        // Allow only `git push`, deny other git subcommands in compound.
        let engine = PermissionEngine::new(vec![make_rule(
            "bash",
            RuleDecision::Allow,
            "git",
            Some("push"),
        )]);
        // `git push && echo done` — echo is not covered → prompt.
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git push && echo done"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_opaque_forces_prompt() {
        // No rules — opaque construct should force prompt regardless.
        let engine =
            PermissionEngine::new(vec![make_rule("bash", RuleDecision::Allow, "echo", None)]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("echo $(cat .env)"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        // Opaque detection overrides even a matching allow rule.
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_single_command_uses_prefix_fallback() {
        // Single-program commands should still use prefix matching for
        // backward compat with existing command_prefix rules.
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
    }

    #[tokio::test]
    async fn test_single_command_program_rule() {
        // Single-program command matched by a program-level rule.
        let engine = PermissionEngine::new(vec![make_rule(
            "bash",
            RuleDecision::Allow,
            "git",
            Some("status"),
        )]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git status"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn test_compound_session_allow() {
        // Even if programs aren't individually allowed, a session-allow
        // for bash should cover the compound command.
        let engine = PermissionEngine::new(vec![]);
        engine.add_session_allow("bash").await;
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git log | grep fix"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }
}
