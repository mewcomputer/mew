use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
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
    /// Runtime permission mode. Stored as a u8 (0 = Standard, 1 = Dangerous)
    /// for lock-free reads on the hot path. Mutated by `set_mode` and read by
    /// `check` on every tool call.
    mode: Arc<AtomicU8>,
}

impl PermissionEngine {
    pub fn new(rules: Vec<PermissionRule>) -> Self {
        Self {
            rules,
            session_allows: Mutex::new(HashSet::new()),
            secret_globs: Vec::new(),
            mode: Arc::new(AtomicU8::new(mew_hooks::PermissionMode::Standard as u8)),
        }
    }

    /// Construct with an initial permission mode (e.g. `Dangerous` when
    /// `--dangerously-skip-permissions` is set on the CLI).
    pub fn with_mode(self, mode: mew_hooks::PermissionMode) -> Self {
        self.mode.store(mode as u8, Ordering::Relaxed);
        self
    }

    /// Switch the runtime mode. Called by `Agent::set_permission_mode` from
    /// the `/permissions` slash command. Cheap (atomic store); the next
    /// `check` call observes the new mode.
    pub fn set_mode(&self, mode: mew_hooks::PermissionMode) {
        self.mode.store(mode as u8, Ordering::Relaxed);
    }

    /// Current mode. Lock-free read.
    pub fn mode(&self) -> mew_hooks::PermissionMode {
        match self.mode.load(Ordering::Relaxed) {
            x if x == mew_hooks::PermissionMode::Standard as u8 => {
                mew_hooks::PermissionMode::Standard
            }
            x if x == mew_hooks::PermissionMode::Permissive as u8 => {
                mew_hooks::PermissionMode::Permissive
            }
            x if x == mew_hooks::PermissionMode::Auto as u8 => {
                mew_hooks::PermissionMode::Auto
            }
            x if x == mew_hooks::PermissionMode::AutoPlus as u8 => {
                mew_hooks::PermissionMode::AutoPlus
            }
            x if x == mew_hooks::PermissionMode::Dangerous as u8 => {
                mew_hooks::PermissionMode::Dangerous
            }
            _ => mew_hooks::PermissionMode::Standard,
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
    /// Mode-aware cascade:
    ///
    /// - **Dangerous**: short-circuits to `AllowOnce` for everything. No
    ///   prompts, no rule checks, no secret-file guard, no bash decomposition.
    ///   The user has explicitly opted into "no holds barred." Output
    ///   redaction in tools still applies.
    ///
    /// - **Auto**: short-circuits to `Prompt` for everything. Every tool
    ///   call needs to be classified by an external LLM (the classifier
    ///   path is implemented at the agent layer, not here). The engine
    ///   just signals "this needs a decision"; the agent's classifier call
    ///   converts that into AllowOnce / Deny / escalate-to-user. No rules,
    ///   secret guard, or bash decomposition fire here — the classifier is
    ///   the only gate.
    ///
    /// - **Permissive**: secret-file guard and bash decomposition still
    ///   fire (real safety tiers); deny / allow / ask rules and session
    ///   cache are SKIPPED; default by sensitivity (ReadOnly → AllowOnce,
    ///   Mutating → AllowOnce, Dangerous → Prompt).
    ///
    /// - **Standard** (default): the full cascade — secret guard, bash
    ///   decomposition, deny rules, allow rules, ask rules, session-allow
    ///   cache, sensitivity default.
    pub async fn check(
        &self,
        tool_name: &str,
        input: &Value,
        sensitivity: mew_tools::Sensitivity,
    ) -> mew_hooks::PermissionDecision {
        // 0a. Dangerous mode: full override. No prompts, no rule checks,
        // no secret guard, no bash decomposition. Pure bypass.
        if self.mode() == mew_hooks::PermissionMode::Dangerous {
            return mew_hooks::PermissionDecision::AllowOnce;
        }

        // 0b. Auto / Auto+ mode: every call needs a classifier decision.
        // Return Prompt so the agent's permission flow routes to the
        // classifier LLM. The engine doesn't distinguish Auto from Auto+
        // here — the difference lives in the agent's classifier wiring
        // (Auto falls back to user on escalate/failure; Auto+ denies).
        if matches!(
            self.mode(),
            mew_hooks::PermissionMode::Auto | mew_hooks::PermissionMode::AutoPlus
        ) {
            return mew_hooks::PermissionDecision::Prompt;
        }

        // 1. Secret-file guard (`read` only): force `Prompt` unless a literal
        // allow rule explicitly permits the exact path. Fires in Standard and
        // Permissive.
        if tool_name == "read" && !self.secret_globs.is_empty() {
            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                if self.is_secret_path(path) && !self.has_literal_allow(tool_name, path) {
                    return mew_hooks::PermissionDecision::Prompt;
                }
            }
        }

        // 2. Bash command decomposition. Compound commands (pipes, &&, ;)
        // and opaque constructs ($(…), eval, bash -c, | sh) cannot be safely
        // checked with prefix matching alone. Fires in Standard and Permissive.
        if tool_name == "bash" {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                let parsed = crate::shell::parse_command(cmd);
                if parsed.has_opaque {
                    return mew_hooks::PermissionDecision::Prompt;
                }
                if parsed.programs.len() > 1 {
                    return self
                        .check_compound_bash(&parsed.programs, sensitivity)
                        .await;
                }
            }
        }

        // 3. Deny rules fire in Standard AND Permissive (user safety rails).
        // In Dangerous mode this is skipped via the short-circuit at step 0.
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

        // 4. Permissive mode short-circuit. Auto-allow ReadOnly/Mutating;
        // still prompt on Dangerous. No allow rules, ask rules, or
        // session-allow cache consulted.
        if self.mode() == mew_hooks::PermissionMode::Permissive {
            return self.check_permissive_mode(sensitivity);
        }

        // 5. Allow rules (first match wins) — Standard only.
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

        // 6. Ask rules (force a prompt even for ReadOnly tools) — Standard only.
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

        // 7. Session allows — Standard only.
        let session = self.session_allows.lock().await;
        if session.contains(tool_name) {
            return mew_hooks::PermissionDecision::AllowOnce;
        }
        drop(session);

        // 8. Default based on sensitivity — Standard only.
        match sensitivity {
            mew_tools::Sensitivity::ReadOnly => mew_hooks::PermissionDecision::AllowOnce,
            _ => mew_hooks::PermissionDecision::Prompt,
        }
    }

    /// Permissive-mode default: ReadOnly and Mutating tools auto-allow;
    /// Dangerous tools (bash, shell_background, shell_monitor) still prompt.
    /// Called after secret-file guard, bash decomposition, and deny rules
    /// have already had their chance to fire.
    fn check_permissive_mode(
        &self,
        sensitivity: mew_tools::Sensitivity,
    ) -> mew_hooks::PermissionDecision {
        match sensitivity {
            mew_tools::Sensitivity::ReadOnly | mew_tools::Sensitivity::Mutating => {
                mew_hooks::PermissionDecision::AllowOnce
            }
            mew_tools::Sensitivity::Dangerous => mew_hooks::PermissionDecision::Prompt,
        }
    }

    /// Record that a tool is allowed for the remainder of this session.
    pub async fn add_session_allow(&self, tool_name: &str) {
        self.session_allows
            .lock()
            .await
            .insert(tool_name.to_string());
    }

    /// Dangerous-mode check: only deny rules are consulted. Everything else
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

    // ------------------------------------------------------------------
    // PermissionMode (Dangerous! / bypass)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_mode_default_is_standard() {
        let engine = PermissionEngine::new(vec![]);
        assert_eq!(engine.mode(), mew_hooks::PermissionMode::Standard);
    }

    #[tokio::test]
    async fn test_with_mode_constructor_sets_initial_mode() {
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        assert_eq!(engine.mode(), mew_hooks::PermissionMode::Dangerous);
    }

    #[tokio::test]
    async fn test_mode_can_be_toggled_at_runtime() {
        let engine = PermissionEngine::new(vec![]);
        assert_eq!(engine.mode(), mew_hooks::PermissionMode::Standard);
        engine.set_mode(mew_hooks::PermissionMode::Dangerous);
        assert_eq!(engine.mode(), mew_hooks::PermissionMode::Dangerous);
        engine.set_mode(mew_hooks::PermissionMode::Standard);
        assert_eq!(engine.mode(), mew_hooks::PermissionMode::Standard);
    }

    #[tokio::test]
    async fn test_dangerous_mode_auto_allows_bash_without_rules() {
        // No rules, Dangerous mode → bash runs without prompt.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -rf /tmp/something"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "Dangerous mode must auto-allow bash without prompting"
        );
    }

    #[tokio::test]
    async fn test_dangerous_mode_auto_allows_write() {
        // No rules, Dangerous mode → Mutating write auto-allows.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "Dangerous mode must auto-allow Mutating tools without prompting"
        );
    }

    #[tokio::test]
    async fn test_dangerous_mode_overrides_deny_rules() {
        // Dangerous mode overrides EVERYTHING, including user-configured deny
        // rules. The user has explicitly opted into "no holds barred" — they
        // know what they're doing. Pinned here so the override-everything
        // contract is documented.
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Deny,
            r#match: MatchConditions {
                command_prefix: Some("rm".to_string()),
                ..Default::default()
            },
        }])
        .with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -rf /"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "Dangerous mode must override deny rules — pure bypass"
        );
    }

    #[tokio::test]
    async fn test_dangerous_mode_skips_secret_guard() {
        // Even secret-file reads auto-allow in Dangerous mode (the user has
        // opted out of all prompts). Secret redaction in tool output still
        // applies; this is only about the permission gate.
        let engine = PermissionEngine::new(vec![])
            .with_secret_files(vec![".env".into()])
            .with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "read",
                &make_input(".env"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "Dangerous mode must skip the secret-file prompt"
        );
    }

    #[tokio::test]
    async fn test_dangerous_mode_skips_opaque_bash_prompt() {
        // Opaque bash constructs (eval, $()) normally force Prompt. In
        // Dangerous mode they auto-allow like everything else.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("eval $(cat .env)"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "Dangerous mode must skip opaque-construct prompt"
        );
    }

    #[tokio::test]
    async fn test_dangerous_mode_does_not_override_session_allows() {
        // session_allows are still honored in Dangerous mode (which they
        // would be anyway since everything auto-allows). This test pins that
        // we don't accidentally crash or behave weirdly.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
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

    // ------------------------------------------------------------------
    // PermissionMode::Permissive (middle tier)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_permissive_mode_auto_allows_readonly() {
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
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
    async fn test_permissive_mode_auto_allows_mutating() {
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "Permissive mode must auto-allow Mutating tools"
        );
    }

    #[tokio::test]
    async fn test_permissive_mode_prompts_on_dangerous() {
        // bash is Dangerous sensitivity. Permissive mode still prompts.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("ls"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "Permissive mode must still prompt for Dangerous tools"
        );
    }

    #[tokio::test]
    async fn test_permissive_mode_respects_deny_rules() {
        // Even though Permissive auto-allows Mutating, deny rules still fire.
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "write".to_string(),
            decision: RuleDecision::Deny,
            r#match: MatchConditions {
                path_glob: Some("/etc/**".to_string()),
                ..Default::default()
            },
        }])
        .with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "write",
                &make_input("/etc/passwd"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Deny,
            "Permissive mode must respect user-configured deny rules"
        );
    }

    #[tokio::test]
    async fn test_permissive_mode_respects_secret_guard() {
        // Secret-file guard fires in Permissive (real safety tier).
        let engine = PermissionEngine::new(vec![])
            .with_secret_files(vec![".env".into()])
            .with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "read",
                &make_input(".env"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "Permissive mode must respect the secret-file guard"
        );
    }

    #[tokio::test]
    async fn test_permissive_mode_respects_bash_decomposition() {
        // Opaque bash constructs (eval, $()) force Prompt in Permissive.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("eval $(cat .env)"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "Permissive mode must respect bash decomposition"
        );
    }

    #[tokio::test]
    async fn test_permissive_mode_skips_allow_rules() {
        // In Permissive mode, allow rules don't run (Mutating auto-allows
        // anyway). Pinned so we don't accidentally consult them.
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "write".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                path_glob: Some("/special/**".to_string()),
                ..Default::default()
            },
        }])
        .with_mode(mew_hooks::PermissionMode::Permissive);
        // Should auto-allow via the sensitivity default, not via the rule.
        let decision = engine
            .check(
                "write",
                &make_input("not/special.rs"),
                mew_tools::Sensitivity::Mutating,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    // ------------------------------------------------------------------
    // PermissionMode::Auto (classifier-driven)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_auto_mode_short_circuits_to_prompt() {
        // Auto mode forces Prompt for every call regardless of sensitivity
        // or rules — the classifier (at the agent layer) will decide what
        // to do with that Prompt.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Auto);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "Auto mode must short-circuit to Prompt for ReadOnly too"
        );
    }

    #[tokio::test]
    async fn test_auto_mode_prompts_even_for_readonly() {
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Auto);
        let decision = engine
            .check(
                "echo",
                &serde_json::json!({"input": "hello"}),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_auto_mode_skips_deny_rules() {
        // In Auto mode deny rules don't fire — the classifier is the only
        // gate. Same posture as Dangerous.
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Deny,
            r#match: MatchConditions {
                command_prefix: Some("rm".to_string()),
                ..Default::default()
            },
        }])
        .with_mode(mew_hooks::PermissionMode::Auto);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -rf /"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "Auto mode must skip deny rules — classifier is the gate"
        );
    }

    #[tokio::test]
    async fn test_auto_mode_skips_secret_guard() {
        // The classifier is responsible for recognizing sensitive reads;
        // the secret-file guard doesn't fire in Auto mode.
        let engine = PermissionEngine::new(vec![])
            .with_secret_files(vec![".env".into()])
            .with_mode(mew_hooks::PermissionMode::Auto);
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
    async fn test_auto_mode_skips_opaque_bash_prompt() {
        // Opaque bash constructs don't force-prompt in Auto — the classifier
        // sees the raw command and decides.
        let engine =
            PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Auto);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("eval $(cat .env)"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    // ------------------------------------------------------------------
    // PermissionMode::AutoPlus (Auto with fail-closed semantics)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_auto_plus_mode_short_circuits_to_prompt() {
        // Same engine-level behavior as Auto. The Auto+/Auto difference
        // lives in the agent's classifier wiring (fail-closed vs fall-through
        // to user). Pinned so the engine treats them the same.
        let engine = PermissionEngine::new(vec![])
            .with_mode(mew_hooks::PermissionMode::AutoPlus);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_auto_plus_mode_skips_deny_rules() {
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Deny,
            r#match: MatchConditions {
                command_prefix: Some("rm".to_string()),
                ..Default::default()
            },
        }])
        .with_mode(mew_hooks::PermissionMode::AutoPlus);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -rf /"),
                mew_tools::Sensitivity::Dangerous,
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }
}
