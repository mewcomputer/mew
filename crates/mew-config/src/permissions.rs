use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
    /// Workspace roots used by the escape tier. Configured via
    /// `with_workspace_roots`. Empty means the escape tier is disabled —
    /// the helper short-circuits to `false` when no roots are configured.
    workspace_roots: Vec<PathBuf>,
    /// Default cwd for the escape tier when neither `input["cwd"]` nor the
    /// caller-supplied `cwd` is available. Set via `with_workspace_roots`
    /// alongside the roots.
    default_cwd: PathBuf,
}

impl PermissionEngine {
    pub fn new(rules: Vec<PermissionRule>) -> Self {
        Self {
            rules,
            session_allows: Mutex::new(HashSet::new()),
            secret_globs: Vec::new(),
            mode: Arc::new(AtomicU8::new(mew_hooks::PermissionMode::Standard as u8)),
            workspace_roots: Vec::new(),
            default_cwd: PathBuf::from("."),
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
            x if x == mew_hooks::PermissionMode::Auto as u8 => mew_hooks::PermissionMode::Auto,
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

    /// Set the workspace roots and the engine's default cwd. Both are read
    /// by the escape tier (the new workspace-escape permission escalation
    /// for `bash` / `shell_background` / `shell_monitor`).
    ///
    /// An empty `roots` slice is preserved as empty — the escape helper
    /// short-circuits to `false` in that case, preserving the
    /// "no protection" opt-out. The 4 main.rs call sites always pass
    /// `cfg.workspace.roots.clone()` (which may be empty if the user has
    /// not configured any), so the user's empty config is honored
    /// literally.
    ///
    /// `cwd` is the engine's fallback for the escape tier when neither
    /// `input["cwd"]` nor the caller-supplied `cwd` argument provides one.
    pub fn with_workspace_roots(mut self, roots: Vec<PathBuf>, cwd: PathBuf) -> Self {
        self.workspace_roots = roots;
        self.default_cwd = cwd;
        self
    }

    /// Resolve the effective cwd for the escape tier.
    ///
    /// Precedence:
    /// 1. `input["cwd"]` for `shell_background` / `shell_monitor` if present
    ///    and a string (those tools expose per-call `cwd`).
    /// 2. The caller-supplied `cwd` argument to `check()`.
    /// 3. `self.default_cwd` (set via `with_workspace_roots`).
    /// 4. `Path::new(".")` (last-resort fallback matching the agent's
    ///    behavior).
    fn resolve_effective_cwd(&self, input: &Value, cwd: &Path) -> PathBuf {
        if let Some(s) = input.get("cwd").and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return PathBuf::from(s);
            }
        }
        if !cwd.as_os_str().is_empty() {
            return cwd.to_path_buf();
        }
        if !self.default_cwd.as_os_str().is_empty() {
            return self.default_cwd.clone();
        }
        PathBuf::from(".")
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
        cwd: &Path,
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
        // Applies to all three shell-style tools (bash, shell_background,
        // shell_monitor) because they all run shell commands.
        if matches!(tool_name, "bash" | "shell_background" | "shell_monitor") {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                let parsed = crate::shell::parse_command(cmd);
                if parsed.has_opaque {
                    return mew_hooks::PermissionDecision::Prompt;
                }
                if parsed.programs.len() > 1 {
                    return self
                        .check_compound_bash(&parsed.programs, sensitivity, cwd, input)
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

        // 3b. Workspace-escape tier (single-program path). Re-parse the
        // command here (rather than carrying parsed state through `&self`)
        // and check whether any path-shaped positional arg resolves
        // outside `workspace_roots`. Sits between deny rules and the
        // Permissive short-circuit so user deny rules still win, and so
        // Permissive mode respects it like the secret-file guard and
        // bash decomposition. Mirrors the same tier's role inside
        // `check_compound_bash` for the multi-program path.
        if matches!(tool_name, "bash" | "shell_background" | "shell_monitor")
            && !self.workspace_roots.is_empty()
        {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                let parsed = crate::shell::parse_command(cmd);
                if !parsed.has_opaque && parsed.programs.len() == 1 {
                    let effective_cwd = self.resolve_effective_cwd(input, cwd);
                    if command_escapes_workspace(
                        &parsed.programs,
                        &effective_cwd,
                        &self.workspace_roots,
                    ) {
                        return mew_hooks::PermissionDecision::Prompt;
                    }
                }
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
        cwd: &Path,
        input: &Value,
    ) -> mew_hooks::PermissionDecision {
        // Resolve cwd once. A single shell pipeline runs in one shell
        // process with one cwd, so per-segment cwd would be meaningless.
        let effective_cwd = self.resolve_effective_cwd(input, cwd);

        // 0. Workspace-escape tier. Mirrors the single-program tier in
        // `check()`: if any path-shaped positional arg in any program
        // resolves outside the workspace roots, escalate to Prompt before
        // any allow rules fire. Fires in Standard and Permissive (the
        // caller has already routed around Dangerous / Auto / Auto+).
        if !self.workspace_roots.is_empty()
            && command_escapes_workspace(programs, &effective_cwd, &self.workspace_roots)
        {
            return mew_hooks::PermissionDecision::Prompt;
        }

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

/// True if any path-shaped positional arg in any program in `programs`
/// resolves to a path outside any of `workspace_roots` when joined to
/// `cwd`. The same predicate applies uniformly to single-program and
/// compound commands — the difference is only what slice of
/// `ProgramInvocation` the caller passes in.
///
/// Conservative: a token is path-shaped if it starts with `/`, `~/`, `./`,
/// `../`, contains `/`, contains a glob meta (`*`, `?`, `[`, `{`), or starts
/// with `$`. Anything else (e.g. `--message=foo`, `5`, `origin`) is skipped.
///
/// Resolution:
/// - **Env-var / tilde prefix short-circuit**: tokens starting with `$`
///   (e.g. `$HOME/.ssh/id_rsa`) or `~` (e.g. `~/.ssh/id_rsa`) flag as
///   escape IMMEDIATELY, without going through `cwd.join` / `canonicalize`.
///   Their literal value can't be evaluated statically and they almost
///   always resolve outside the workspace.
/// - Absolute paths: tested directly against roots.
/// - Relative paths: `cwd.join(arg)`, then `canonicalize` if the file
///   exists.
/// - Non-existent paths: fall back to lexical `cwd.join(arg)`; treat as
///   escape if the lexical path contains `..` or is absolute.
/// - Globs: do NOT expand. Treat the unexpanded form. If it contains `..`,
///   flag as escape.
/// - Symlinks: `canonicalize` follows them. A symlink inside the workspace
///   pointing outside resolves to outside and is caught. A non-existent
///   symlink target falls into the conservative branch above.
///
/// Short-circuits to `false` if `workspace_roots` is empty (escape tier
/// disabled — see AC.6 in the plan).
fn command_escapes_workspace(
    programs: &[crate::shell::ProgramInvocation],
    cwd: &Path,
    workspace_roots: &[PathBuf],
) -> bool {
    if workspace_roots.is_empty() {
        return false;
    }

    for prog in programs {
        // The subcommand slot is the first non-flag token after the
        // program. For programs that take a real subcommand (`git push`)
        // this is `push`; for programs that take a file arg as the first
        // token (`cat README.md`), this is `README.md`. The escape check
        // is path-shaped, so the difference is harmless: `push`, `status`,
        // `log` aren't path-shaped, but `../README.md` is.
        if let Some(sub) = &prog.subcommand {
            if is_path_shaped_token(sub) && token_escapes(sub, cwd, workspace_roots) {
                return true;
            }
        }
        for arg in &prog.args {
            if is_path_shaped_token(arg) && token_escapes(arg, cwd, workspace_roots) {
                return true;
            }
        }
    }
    false
}

/// Re-export of `crate::shell::is_path_shaped` so the parser and the
/// engine share one source of truth for the path-shape predicate. Kept
/// as a local `fn` (rather than a `use` alias) to keep the call-site
/// documentation in one place and to make the dependency explicit.
fn is_path_shaped_token(token: &str) -> bool {
    crate::shell::is_path_shaped(token)
}

/// Resolve a single path-shaped token against `cwd` and `workspace_roots`.
/// Returns true if the token resolves (or lexically points) outside every
/// root. Cwd-relative tokens that exist on disk are `canonicalize`d so
/// symlinks are followed; tokens that don't exist fall back to lexical
/// resolution and are treated as escape if the unexpanded form contains
/// `..` or is absolute.
fn token_escapes(token: &str, cwd: &Path, workspace_roots: &[PathBuf]) -> bool {
    // Env-var / tilde short-circuit: literal value cannot be evaluated
    // statically and almost always resolves outside the workspace.
    if token.starts_with('$') {
        return true;
    }
    if token.starts_with('~') {
        // `~` alone, `~/...`, `~user/...` — all flag as escape.
        return true;
    }

    // Absolute path: test directly.
    let candidate = PathBuf::from(token);
    let absolute: PathBuf = if candidate.is_absolute() {
        candidate.clone()
    } else {
        cwd.join(&candidate)
    };

    // Try canonicalize first; if the path doesn't exist, fall back to the
    // lexical form.
    match absolute.canonicalize() {
        Ok(canonical) => {
            // File exists on disk. The canonicalized form is the true
            // resolved path. If it's not inside any root, it's an escape
            // — regardless of whether the token had `..` or not.
            if is_inside_any_root(&canonical, workspace_roots) {
                return false;
            }
            // Also check the un-normalized absolute form in case the
            // roots themselves aren't canonicalized.
            if is_inside_any_root(&absolute, workspace_roots) {
                return false;
            }
            // Exists and resolves outside all roots → escape.
            true
        }
        Err(_) => {
            // File doesn't exist on disk. Fall back to lexical
            // resolution against the un-normalized `absolute` form.
            if is_inside_any_root(&absolute, workspace_roots) {
                return false;
            }
            // Lexical conservative check: if the unexpanded form
            // contains `..`, the user is clearly trying to reach
            // outside. Flag as escape.
            if !candidate.is_absolute() && token.contains("..") {
                return true;
            }
            // Absolute path that isn't inside any root: escape.
            if candidate.is_absolute() {
                return true;
            }
            // Relative path that doesn't exist, doesn't contain `..`,
            // and isn't absolute — conservative: don't flag (could be
            // a typo for an in-workspace file). Mirrors the plan:
            // "Non-existent paths: fall back to lexical `cwd.join(arg)`;
            // treat as escape if the lexical path contains `..` or is
            // absolute."
            false
        }
    }
}

/// True if `path` is inside (or equal to) any of `roots`. Symlink-aware
/// is not required here — we operate on canonicalized paths. The lexical
/// fallback also gets checked so non-existent paths can still match.
fn is_inside_any_root(path: &Path, roots: &[PathBuf]) -> bool {
    for root in roots {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        if path.starts_with(&canonical_root) {
            return true;
        }
    }
    false
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

    /// Default cwd for the existing tests. The escape tier is only active
    /// when workspace roots are configured and the command contains a
    /// path-shaped arg; the default-`"."` cwd keeps the old tests
    /// "no workspace configured" effectively. For the new escape-tier
    /// tests we use a `tempfile::tempdir()` cwd explicitly.
    fn test_cwd() -> std::path::PathBuf {
        std::path::PathBuf::from(".")
    }

    #[tokio::test]
    async fn test_default_readonly_allow() {
        let engine = PermissionEngine::new(vec![]);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);

        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -rf /"),
                mew_tools::Sensitivity::Dangerous,
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
            )
            .await;
        assert_eq!(d2, mew_hooks::PermissionDecision::AllowOnce);

        // Other tools should still prompt
        let d3 = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    // ------------------------------------------------------------------
    // Workspace-escape tier
    // ------------------------------------------------------------------
    //
    // The escape tier fires for `bash` / `shell_background` / `shell_monitor`
    // when the engine is configured with workspace roots and the parsed
    // command contains a path-shaped positional arg that resolves outside
    // the roots. It escalates `AllowOnce` to `Prompt`; deny rules still
    // win.

    /// Construct an engine configured with a single workspace root =
    /// `tempdir`. Returns `(engine, tempdir)` — the tempdir is the
    /// workspace and the cwd to use for `check()`.
    fn engine_with_workspace(roots: Vec<std::path::PathBuf>) -> PermissionEngine {
        PermissionEngine::new(vec![]).with_workspace_roots(roots, std::path::PathBuf::from("."))
    }

    #[tokio::test]
    async fn test_escape_ac1_grep_dotdot_glob_prompts() {
        // AC.1: `grep ./../*` against a workspace where `*` matches files
        // outside the root produces Prompt, even with an allow rule for grep.
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        // Create a file outside the workspace root that the glob will match.
        std::fs::write(outside.path().join("secret.txt"), b"top secret").unwrap();
        // The command's cwd (tmp) is one level above the workspace root
        // (tmp.path()), so `./../*` lexically points at the parent of tmp
        // — outside the workspace.
        let parent = tmp.path().parent().unwrap().to_path_buf();
        // Build workspace root as tmp.path() (its inner dir).
        let workspace = tmp.path().to_path_buf();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("grep".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(vec![workspace], std::path::PathBuf::from("."));
        let decision = engine
            .check(
                "bash",
                &serde_json::json!({"command": "grep -r 'foo' ./../*"}),
                mew_tools::Sensitivity::Dangerous,
                &parent,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "escape tier should escalate grep with ./../* to Prompt"
        );
    }

    #[tokio::test]
    async fn test_escape_ac2_cat_in_workspace_allow_rule() {
        // AC.2: `cat README.md` (in-workspace) with an allow rule for `cat`
        // returns AllowOnce — no regression.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), b"hi").unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("cat".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat README.md"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "in-workspace cat should still auto-allow with an allow rule"
        );
    }

    #[tokio::test]
    async fn test_escape_ac3_cat_absolute_path_prompts() {
        // AC.3: `cat /etc/passwd` returns Prompt regardless of any allow
        // rule for `cat`.
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("cat".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat /etc/passwd"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "absolute-path cat should escape and force a prompt"
        );
    }

    #[tokio::test]
    async fn test_escape_ac4_opaque_bash_prompt_first() {
        // AC.4: `bash -c 'rm -rf /'` is caught by the opaque-construct
        // check first (the engine's existing behavior). The escape tier
        // never even gets consulted because the command is opaque.
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine_with_workspace(vec![tmp.path().to_path_buf()]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("bash -c 'rm -rf /'"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "opaque bash -c must always prompt"
        );
    }

    #[tokio::test]
    async fn test_escape_ac5_dangerous_mode_bypasses_tier() {
        // AC.5: Dangerous mode short-circuits before the escape tier.
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![])
            .with_workspace_roots(
                vec![tmp.path().to_path_buf()],
                std::path::PathBuf::from("."),
            )
            .with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat /etc/passwd"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "Dangerous mode must short-circuit before the escape tier"
        );
    }

    #[tokio::test]
    async fn test_escape_ac6_empty_roots_no_extra_prompts() {
        // AC.6: With no workspace roots configured (empty `workspace_roots`),
        // behavior is identical to before — no extra prompts from the
        // escape tier.
        let engine = PermissionEngine::new(vec![]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat /etc/passwd"),
                mew_tools::Sensitivity::Dangerous,
                &test_cwd(),
            )
            .await;
        // Falls through to sensitivity default → Prompt for Dangerous.
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_escape_ac7_shell_background_and_monitor() {
        // AC.7: shell_background and shell_monitor get the same treatment
        // as bash — explicit tests.
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![]).with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );

        let d_bg = engine
            .check(
                "shell_background",
                &make_bash_input("cat ../foo"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            d_bg,
            mew_hooks::PermissionDecision::Prompt,
            "shell_background with cat ../foo should prompt"
        );

        let d_mon = engine
            .check(
                "shell_monitor",
                &make_bash_input("cat ../foo"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            d_mon,
            mew_hooks::PermissionDecision::Prompt,
            "shell_monitor with cat ../foo should prompt"
        );
    }

    #[tokio::test]
    async fn test_escape_ac8_glob_with_dotdot_prompts() {
        // AC.8: Globs containing `..` (e.g. `./*/../foo`) are flagged as
        // escapes without trying to expand.
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![]).with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("grep -r 'foo' ./*/../foo"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_escape_ac9_compound_prompts() {
        // AC.9: Compound commands like `git log | grep ../foo` get the
        // escape check even when both programs are allow-listed.
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![
            make_rule("bash", RuleDecision::Allow, "git", None),
            make_rule("bash", RuleDecision::Allow, "grep", None),
        ])
        .with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("git log | grep ../foo"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "compound with ../foo should escape even with allow rules on both programs"
        );
    }

    #[tokio::test]
    async fn test_escape_ac10_deny_rule_wins() {
        // AC.10: A user deny rule always wins over the escape tier.
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Deny,
            r#match: MatchConditions {
                command_program: Some("rm".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -r ../foo"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Deny,
            "deny rule must win over escape tier"
        );
    }

    #[tokio::test]
    async fn test_escape_ac11_permissive_mode_respects_tier() {
        // AC.11: Permissive mode respects the escape tier like the
        // secret-file guard.
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![])
            .with_workspace_roots(
                vec![tmp.path().to_path_buf()],
                std::path::PathBuf::from("."),
            )
            .with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat ../foo"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "Permissive mode must still respect the escape tier"
        );
    }

    #[tokio::test]
    async fn test_escape_ac12_roots_honored_with_external_cwd() {
        // AC.12: workspace_roots is honored for path resolution even when
        // the engine's default cwd is somewhere else. We use an allow
        // rule for `cat` so the in-workspace case returns AllowOnce
        // (proving the escape tier didn't fire) instead of Prompt (which
        // could be either escape or Dangerous default).
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("file.txt"), b"x").unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("cat".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(
            vec![workspace.path().to_path_buf()],
            tmp.path().to_path_buf(),
        );

        // `cat /etc/passwd` (absolute, outside workspace) escapes
        // regardless of what the default cwd is set to.
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat /etc/passwd"),
                mew_tools::Sensitivity::Dangerous,
                workspace.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "absolute /etc/passwd must escape even when default_cwd is elsewhere"
        );

        // `cat file.txt` from inside the workspace dir → no escape.
        // With the allow rule, AllowOnce proves the escape tier didn't
        // fire (a Dangerous default → Prompt would mask a regression).
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat file.txt"),
                mew_tools::Sensitivity::Dangerous,
                workspace.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "in-workspace file should not escape (AllowOnce proves it)"
        );
    }

    #[tokio::test]
    async fn test_escape_ac13_input_cwd_takes_precedence() {
        // AC.13: input["cwd"] on shell_background / shell_monitor takes
        // precedence over the engine's default cwd.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        // Default cwd is the workspace dir, but input["cwd"] = /tmp.
        // `cat ../foo` from /tmp resolves to /foo, which is outside the
        // workspace — should escape.
        let engine = PermissionEngine::new(vec![]).with_workspace_roots(
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
        );
        let decision = engine
            .check(
                "shell_background",
                &serde_json::json!({"command": "cat ../foo", "cwd": "/tmp"}),
                mew_tools::Sensitivity::Dangerous,
                workspace.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "input['cwd'] must override the engine default for escape resolution"
        );

        // Without input["cwd"], falls back to caller-supplied cwd.
        let decision = engine
            .check(
                "shell_background",
                &make_bash_input("cat ../foo"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "without input['cwd'], caller-supplied cwd wins"
        );
    }

    #[tokio::test]
    async fn test_escape_mode_interaction() {
        // Mode interaction matrix. Each test case configures the engine
        // with workspace roots + a mode, runs a bash command that
        // escapes, and asserts the expected decision.
        let tmp = tempfile::tempdir().unwrap();
        let cases = [
            (
                "Dangerous",
                mew_hooks::PermissionMode::Dangerous,
                mew_hooks::PermissionDecision::AllowOnce,
            ),
            (
                "Auto",
                mew_hooks::PermissionMode::Auto,
                mew_hooks::PermissionDecision::Prompt,
            ),
            (
                "AutoPlus",
                mew_hooks::PermissionMode::AutoPlus,
                mew_hooks::PermissionDecision::Prompt,
            ),
            (
                "Permissive",
                mew_hooks::PermissionMode::Permissive,
                mew_hooks::PermissionDecision::Prompt,
            ),
            (
                "Standard",
                mew_hooks::PermissionMode::Standard,
                mew_hooks::PermissionDecision::Prompt,
            ),
        ];
        for (label, mode, expected) in cases {
            let engine = PermissionEngine::new(vec![])
                .with_workspace_roots(
                    vec![tmp.path().to_path_buf()],
                    std::path::PathBuf::from("."),
                )
                .with_mode(mode);
            let decision = engine
                .check(
                    "bash",
                    &make_bash_input("cat ../foo"),
                    mew_tools::Sensitivity::Dangerous,
                    tmp.path(),
                )
                .await;
            assert_eq!(
                decision, expected,
                "mode {label} with workspace escape: expected {expected:?}, got {decision:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_escape_symlink_inside_workspace_to_outside() {
        // A symlink inside the workspace pointing outside is caught by
        // canonicalize.
        let tmp = tempfile::tempdir().unwrap();
        // Create the symlink target outside the workspace first.
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("secret.txt");
        std::fs::write(&target, b"top secret").unwrap();
        // Now create the symlink inside the workspace.
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let engine = engine_with_workspace(vec![tmp.path().to_path_buf()]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat link"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "symlink to outside should escape via canonicalize"
        );
    }

    #[tokio::test]
    async fn test_escape_broken_symlink_conservative() {
        // A broken symlink inside the workspace doesn't escape if the
        // unexpanded form is a plain in-workspace name. We use an allow
        // rule for `cat` so that the ONLY way to get Prompt is the escape
        // tier firing — a Dangerous-sensitivity default would also
        // produce Prompt, which would mask a regression. With the allow
        // rule, no escape → AllowOnce; escape → Prompt.
        let tmp = tempfile::tempdir().unwrap();
        let broken = tmp.path().join("broken");
        std::os::unix::fs::symlink("/nonexistent/target", &broken).unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("cat".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat broken"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        // `broken` is the subcommand slot, not path-shaped → not escape.
        // With the allow rule, the decision is AllowOnce — proving the
        // escape tier did NOT fire. A regression that wrongly flagged
        // `broken` as an escape would flip this to Prompt.
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "broken symlink with non-escape name should not trigger escape tier (AllowOnce proves it)"
        );
    }

    #[tokio::test]
    async fn test_escape_in_workspace_with_subdir_root() {
        // Workspace root is a subdirectory of the cwd. A command that
        // targets that subdir should not escape. We use an allow rule
        // for `find` so AllowOnce proves the escape tier didn't fire.
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.txt"), b"x").unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("find".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(vec![sub.clone()], std::path::PathBuf::from("."));
        let decision = engine
            .check(
                "bash",
                &make_bash_input("find ./sub -name '*.txt'"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        // `./sub` is the subcommand, path-shaped (contains `/`). The
        // canonicalized form is tmp/sub which is a workspace root → no
        // escape. With the allow rule, AllowOnce proves the escape tier
        // didn't fire.
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "find ./sub from cwd tmp should not escape when workspace root is tmp/sub"
        );

        // `./other` from the same cwd escapes: we create `tmp/other` on
        // disk so canonicalize succeeds, and the resolved path `tmp/other`
        // is outside the configured root `tmp/sub` → escape.
        std::fs::write(tmp.path().join("other"), b"x").unwrap();
        let decision = engine
            .check(
                "bash",
                &make_bash_input("find ./other -name '*.txt'"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "./other from cwd tmp should escape when workspace root is tmp/sub and ./other exists outside it"
        );
    }

    #[tokio::test]
    async fn test_escape_multiple_roots() {
        // Workspace has multiple roots; path inside root B but not root
        // A is allowed. We use an allow rule for `cat` so AllowOnce
        // proves the escape tier did NOT fire (a Dangerous default →
        // Prompt would mask a regression).
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        std::fs::write(root_b.path().join("file.txt"), b"x").unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("cat".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(
            vec![root_a.path().to_path_buf(), root_b.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat file.txt"),
                mew_tools::Sensitivity::Dangerous,
                root_b.path(),
            )
            .await;
        // file.txt is subcommand, not path-shaped → no escape. With the
        // allow rule, AllowOnce proves the escape tier didn't fire even
        // though root A doesn't contain the file.
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "in-workspace file with multiple roots should not escape (AllowOnce proves it)"
        );
    }

    #[tokio::test]
    async fn test_escape_find_in_workspace_no_escape() {
        // `find . -name "*.txt"` from the workspace root: `.` is the
        // subcommand, path-shaped, resolves into workspace → no escape.
        // We use an allow rule for `find` so that AllowOnce proves the
        // escape tier did NOT fire (otherwise Dangerous default → Prompt
        // would mask a regression).
        let tmp = tempfile::tempdir().unwrap();
        let engine = PermissionEngine::new(vec![PermissionRule {
            tool: "bash".to_string(),
            decision: RuleDecision::Allow,
            r#match: MatchConditions {
                command_program: Some("find".to_string()),
                ..Default::default()
            },
        }])
        .with_workspace_roots(
            vec![tmp.path().to_path_buf()],
            std::path::PathBuf::from("."),
        );
        let decision = engine
            .check(
                "bash",
                &make_bash_input("find . -name '*.txt'"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        // `.` resolves to tmp (workspace root) → not escape. With the
        // allow rule, the decision is AllowOnce — proving the escape tier
        // did NOT fire.
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::AllowOnce,
            "find . from workspace root should not escape (AllowOnce proves it)"
        );
    }

    #[tokio::test]
    async fn test_escape_find_parent_no_escape() {
        // `find .. -name "*.txt"` from a subdir of the workspace:
        // `..` resolves to the parent of cwd, which is outside the
        // workspace root → escape.
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let engine = engine_with_workspace(vec![tmp.path().to_path_buf()]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("find .. -name '*.txt'"),
                mew_tools::Sensitivity::Dangerous,
                &sub,
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "`..` from a subdir escapes when workspace root is the parent"
        );
    }

    #[tokio::test]
    async fn test_escape_env_var_prompts() {
        // `$HOME/.ssh/id_rsa` is conservatively flagged as an escape.
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine_with_workspace(vec![tmp.path().to_path_buf()]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat $HOME/.ssh/id_rsa"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "$HOME/.ssh/id_rsa should conservatively escape"
        );
    }

    #[tokio::test]
    async fn test_escape_bare_env_var_prompts() {
        // `cat $SECRET` (no slash) — the escape helper short-circuits on
        // any `$` prefix. Pinned here so a regression in `is_path_shaped`
        // that drops the `$` branch would surface immediately. (Note:
        // `$SECRET` is parsed by `parse_segment` as a single token since
        // shell-words doesn't expand it.)
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine_with_workspace(vec![tmp.path().to_path_buf()]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat $SECRET"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "bare $SECRET (no slash) should still trigger escape"
        );

        // Braced form: `cat ${SECRET}`.
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat ${SECRET}"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "braced $SECRET should trigger escape"
        );
    }

    #[tokio::test]
    async fn test_escape_tilde_prefix_prompts() {
        // `~/...` and bare `~` are conservatively flagged. The short-circuit
        // catches these without trying to resolve the home directory.
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine_with_workspace(vec![tmp.path().to_path_buf()]);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("cat ~/.ssh/id_rsa"),
                mew_tools::Sensitivity::Dangerous,
                tmp.path(),
            )
            .await;
        assert_eq!(
            decision,
            mew_hooks::PermissionDecision::Prompt,
            "tilde-prefix should escape"
        );
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("rm -rf /tmp/something"),
                mew_tools::Sensitivity::Dangerous,
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("eval $(cat .env)"),
                mew_tools::Sensitivity::Dangerous,
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Dangerous);
        engine.add_session_allow("write").await;
        let decision = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
                &test_cwd(),
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    // ------------------------------------------------------------------
    // PermissionMode::Permissive (middle tier)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_permissive_mode_auto_allows_readonly() {
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
                &test_cwd(),
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn test_permissive_mode_auto_allows_mutating() {
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "write",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::Mutating,
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("ls"),
                mew_tools::Sensitivity::Dangerous,
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Permissive);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("eval $(cat .env)"),
                mew_tools::Sensitivity::Dangerous,
                &test_cwd(),
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
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Auto);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Auto);
        let decision = engine
            .check(
                "echo",
                &serde_json::json!({"input": "hello"}),
                mew_tools::Sensitivity::ReadOnly,
                &test_cwd(),
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
                &test_cwd(),
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
                &test_cwd(),
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }

    #[tokio::test]
    async fn test_auto_mode_skips_opaque_bash_prompt() {
        // Opaque bash constructs don't force-prompt in Auto — the classifier
        // sees the raw command and decides.
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::Auto);
        let decision = engine
            .check(
                "bash",
                &make_bash_input("eval $(cat .env)"),
                mew_tools::Sensitivity::Dangerous,
                &test_cwd(),
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
        let engine = PermissionEngine::new(vec![]).with_mode(mew_hooks::PermissionMode::AutoPlus);
        let decision = engine
            .check(
                "read",
                &make_input("foo.rs"),
                mew_tools::Sensitivity::ReadOnly,
                &test_cwd(),
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
                &test_cwd(),
            )
            .await;
        assert_eq!(decision, mew_hooks::PermissionDecision::Prompt);
    }
}
