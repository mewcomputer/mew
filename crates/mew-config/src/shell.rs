//! Shell command decomposition for the permission engine.
//!
//! Splits a compound shell command into individual program invocations so
//! the rule engine can check each independently. Without this, a rule that
//! allows `git status` would also allow `git status && cat /etc/passwd`
//! because the whole string starts with the allowed prefix.
//!
//! Also detects opaque constructs (`$(...)`, backticks, `eval`, `bash -c`,
//! `| sh`, `| xargs`, `<(...)`) that the rule engine cannot inspect. When
//! any are present, the engine forces a prompt regardless of rules,
//! because it cannot see what those constructs will actually run.

/// A single program invocation extracted from a compound command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramInvocation {
    /// The program name (first token). For `git push origin main` this is
    /// `"git"`.
    pub program: String,
    /// The first non-flag argument, treated as a subcommand. For
    /// `git push origin main` this is `"push"`. `None` when the program
    /// has no subcommand-shaped argument (e.g. `ls -la`).
    pub subcommand: Option<String>,
    /// Positional arguments after the subcommand. Flags and `--`-separated
    /// arguments are handled per the rule documented on `parse_segment`:
    /// tokens before `--` are scanned for the first non-flag arg (the
    /// subcommand), and only tokens after the subcommand that don't start
    /// with `-` are collected; everything after a `--` is treated as
    /// positional regardless of leading `-`. This is the same surface the
    /// workspace-escape tier inspects, so the two stay in lockstep.
    pub args: Vec<String>,
}

/// Parse a command string into a list of program invocations plus an
/// opaque-construct flag.
///
/// Splitting happens on shell separators: `|`, `||`, `&&`, `;`, and
/// newlines. Each segment is then tokenized with `shell-words` to extract
/// the program name and subcommand.
///
/// Opaque constructs are detected by scanning the raw command for patterns
/// that cannot be statically resolved to a program name:
/// - `$(...)` command substitution
/// - Backtick command substitution
/// - `<(...)` process substitution
/// - `eval`, `bash -c`, `sh -c`, `python -c`, `python3 -c`, `node -e`
/// - `| sh`, `| bash`, `| xargs` (piping into an executor)
pub fn parse_command(command: &str) -> ParsedCommand {
    let has_opaque = detect_opaque(command);
    let programs = split_segments(command)
        .filter_map(|seg| parse_segment(&seg))
        .collect();
    ParsedCommand {
        programs,
        has_opaque,
    }
}

/// Result of parsing a shell command.
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    /// The individual program invocations, in order. May be empty if the
    /// command could not be parsed (e.g. a bare variable assignment).
    pub programs: Vec<ProgramInvocation>,
    /// True if the command contains constructs the engine cannot inspect.
    /// When true, the engine forces a prompt regardless of rules.
    pub has_opaque: bool,
}

/// Split on shell separators. Handles `|`, `||`, `&&`, `;`, and newlines.
/// Respects single and double quotes so a `;` inside a string is not
/// treated as a separator.
fn split_segments(command: &str) -> impl Iterator<Item = String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                current.push(c);
                // Consume until closing single quote.
                for ic in chars.by_ref() {
                    current.push(ic);
                    if ic == '\'' {
                        break;
                    }
                }
            }
            '"' => {
                current.push(c);
                for ic in chars.by_ref() {
                    current.push(ic);
                    if ic == '"' {
                        break;
                    }
                }
            }
            '|' => {
                if chars.peek() == Some(&'|') {
                    chars.next();
                }
                push_segment(&mut segments, &mut current);
            }
            '&' => {
                if chars.peek() == Some(&'&') {
                    chars.next();
                    push_segment(&mut segments, &mut current);
                } else {
                    // Single & (background). Split here too — the rest is
                    // a new command context.
                    current.push(c);
                }
            }
            ';' | '\n' => {
                push_segment(&mut segments, &mut current);
            }
            _ => {
                current.push(c);
            }
        }
    }
    push_segment(&mut segments, &mut current);
    segments.into_iter()
}

fn push_segment(segments: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        segments.push(trimmed);
    }
    current.clear();
}

/// True if a token looks like a short flag (one or more non-dash chars after
/// a single leading `-`, no `=`). `-n`, `-rf`, `-I` all count; `--foo`,
/// `-`, and `--` do not. Used by `parse_segment` to decide whether to
/// consume the next token as a flag value.
fn is_short_flag(token: &str) -> bool {
    let mut chars = token.chars();
    if chars.next() != Some('-') {
        return false;
    }
    let mut saw_dash = false;
    for c in chars {
        if c == '-' {
            saw_dash = true;
            continue;
        }
        if saw_dash {
            // A second dash means we're into long-flag territory.
            return false;
        }
        // First non-dash char after leading `-` exists → short flag.
        return true;
    }
    // Token was just `-` (handled by caller) or `--` (ditto).
    false
}

/// True if a token looks like a path (would be subject to the
/// workspace-escape check). The escape check is path-shaped, so we mirror
/// the same predicate here: starts with `/`, `~/`, `./`, `../`, contains
/// `/`, contains a glob meta (`*`, `?`, `[`, `{`), or starts with `$`.
/// This is purely a heuristic for the parser — the real resolution
/// happens in the permission engine.
///
/// `pub(crate)` so the engine module can reuse this exact predicate
/// instead of duplicating it. Keeping a single source of truth prevents
/// the parser's view of "positional" from desyncing from the engine's
/// view of "path-shaped" — a divergence would either over- or
/// under-trigger the escape tier silently.
pub(crate) fn is_path_shaped(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.starts_with('/')
        || token.starts_with("~/")
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with('$')
    {
        return true;
    }
    if token.contains('/') {
        return true;
    }
    if token.chars().any(|c| matches!(c, '*' | '?' | '[' | '{')) {
        return true;
    }
    false
}

/// Parse one segment into a program invocation. Returns `None` if the
/// segment cannot be tokenized (e.g. bare assignment like `FOO=bar`).
///
/// Token decomposition for `args` (used by the workspace-escape tier):
///
/// 1. If a `--` token appears, every token after it is positional (a path
///    arg that starts with `-` is still a path, e.g. `-- -file`).
/// 2. For tokens before `--` (or all tokens if no `--`): find the first
///    non-flag token after the program — that's the subcommand — and
///    collect every subsequent token that doesn't start with `-` as a
///    positional arg. Tokens that look like `-x` or `--flag` (with or
///    without a value) are treated as flags and skipped. Flag values
///    (e.g. the `5` in `-n 5`) are *not* split out; this is a conservative
///    approximation, but the escape check is path-shaped (a bare `5`
///    won't trigger it) so the approximation is safe.
fn parse_segment(segment: &str) -> Option<ProgramInvocation> {
    let tokens = shell_words::split(segment).ok()?;
    let program = tokens.first()?.clone();
    if program.is_empty() {
        return None;
    }

    // Skip env-var assignments like `FOO=bar baz` — `baz` is the real
    // program. We want the first token that isn't an assignment.
    let (program, start_idx) = if program.contains('=') && !program.starts_with("exec=") {
        // Find the first non-assignment token.
        let real = tokens.iter().position(|t| !t.contains('='))?;
        (tokens[real].clone(), real)
    } else {
        (program, 0)
    };

    // Walk tokens after the program. Track the first non-flag token as the
    // subcommand, then collect every later non-flag token as a positional
    // arg. A `--` token switches into "everything after is positional" mode.
    //
    // Flag-value handling: a short flag whose token doesn't contain `=` (e.g.
    // `-n`, `-I`, `-L`) is treated as taking the immediately-following token
    // as its value. Long flags like `--pretty=format:foo` carry the value
    // in the same token (the `=` form is the idiomatic long-flag-with-value
    // syntax), and standalone `--foo` is a boolean. This approximation makes
    // `git log --oneline -n 5` produce `args = []` (the `5` is consumed as
    // `-n`'s value) and `cp -r src dst` produce `args = ["src", "dst"]`
    // (when `cp` is unknown) only by sheer luck — for the escape check this
    // is fine because path-shaped tokens (containing `/` or starting with
    // `.`, `~`, `$`) will land in `args` regardless of the flag-value guess.
    let mut subcommand: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut after_double_dash = false;
    let mut consume_next_as_value = false;
    for token in tokens.iter().skip(start_idx + 1) {
        if after_double_dash {
            args.push(token.clone());
            continue;
        }
        if token == "--" {
            after_double_dash = true;
            consume_next_as_value = false;
            continue;
        }
        if token.starts_with('-') {
            // Flag. A short flag without `=` is assumed to take a value in
            // the next token (matches `-n 5`, `-I/usr/include`, etc.). The
            // escape check tolerates the value token being wrongly
            // classified as a positional — only path-shaped tokens matter.
            consume_next_as_value = !token.contains('=') && is_short_flag(token);
            continue;
        }
        if consume_next_as_value {
            // Treat this token as the previous flag's value, but still
            // surface it as a positional arg if it looks path-shaped.
            // That way `-I /usr/include` ends up with `args = ["/usr/include"]`
            // even though the `5` in `-n 5` is correctly consumed. The
            // escape check is path-shaped, so a non-path value is
            // safely suppressed.
            consume_next_as_value = false;
            if is_path_shaped(token) {
                args.push(token.clone());
            }
            continue;
        }
        // Non-flag token, not consumed as a value.
        if subcommand.is_none() {
            subcommand = Some(token.clone());
        } else {
            args.push(token.clone());
        }
    }

    Some(ProgramInvocation {
        program,
        subcommand,
        args,
    })
}

/// Detect opaque constructs that prevent static program analysis.
fn detect_opaque(command: &str) -> bool {
    // Command substitution: $(...)
    if command.contains("$(") {
        return true;
    }
    // Backtick substitution.
    if command.contains('`') {
        return true;
    }
    // Process substitution: <(...) or >(...)
    if command.contains("<(") || command.contains(">(") {
        return true;
    }
    // Indirection via executors. We tokenize and check for known executor
    // programs followed by -c / -e flags, or pipes into shells.
    let lower = command.to_lowercase();
    for pattern in &[
        "eval ",
        "bash -c",
        "sh -c",
        "python -c",
        "python3 -c",
        "node -e",
        "| sh",
        "| bash",
        "| xargs",
    ] {
        if lower.contains(pattern) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_command() {
        let parsed = parse_command("git status");
        assert_eq!(
            parsed.programs,
            vec![ProgramInvocation {
                program: "git".into(),
                subcommand: Some("status".into()),
                args: vec![],
            }]
        );
        assert!(!parsed.has_opaque);
    }

    #[test]
    fn test_command_with_flags() {
        let parsed = parse_command("ls -la --color=auto");
        assert_eq!(parsed.programs.len(), 1);
        assert_eq!(parsed.programs[0].program, "ls");
        assert_eq!(parsed.programs[0].subcommand, None);
        assert!(parsed.programs[0].args.is_empty());
    }

    #[test]
    fn test_pipe_splits() {
        let parsed = parse_command("git log | grep fix | wc -l");
        assert_eq!(parsed.programs.len(), 3);
        assert_eq!(parsed.programs[0].program, "git");
        assert_eq!(parsed.programs[1].program, "grep");
        assert_eq!(parsed.programs[2].program, "wc");
    }

    #[test]
    fn test_and_splits() {
        let parsed = parse_command("cargo build && cargo test");
        assert_eq!(parsed.programs.len(), 2);
        assert_eq!(parsed.programs[0].program, "cargo");
        assert_eq!(parsed.programs[1].program, "cargo");
    }

    #[test]
    fn test_sequence_splits() {
        let parsed = parse_command("echo a; echo b");
        assert_eq!(parsed.programs.len(), 2);
    }

    #[test]
    fn test_env_prefix_stripped() {
        let parsed = parse_command("FOO=bar RUST_LOG=debug cargo test");
        assert_eq!(parsed.programs.len(), 1);
        assert_eq!(parsed.programs[0].program, "cargo");
        assert_eq!(parsed.programs[0].subcommand, Some("test".into()));
        assert!(parsed.programs[0].args.is_empty());
    }

    #[test]
    fn test_subcommand_extraction() {
        let parsed = parse_command("git push origin main");
        assert_eq!(
            parsed.programs[0],
            ProgramInvocation {
                program: "git".into(),
                subcommand: Some("push".into()),
                args: vec!["origin".into(), "main".into()],
            }
        );
    }

    // -- args extraction tests (workspace-escape tier input) --

    #[test]
    fn test_args_after_subcommand() {
        // `git status` → no positional args
        let parsed = parse_command("git status");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("status"));
        assert_eq!(parsed.programs[0].args, Vec::<String>::new());

        // `git log --oneline -n 5` → subcommand is `log`, `-n 5` is flag+value
        let parsed = parse_command("git log --oneline -n 5");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("log"));
        assert!(parsed.programs[0].args.is_empty());

        // `git log --oneline -n 5 -- .. /etc/passwd` → .. and /etc/passwd
        // are positional after `--`
        let parsed = parse_command("git log --oneline -n 5 -- .. /etc/passwd");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("log"));
        assert_eq!(
            parsed.programs[0].args,
            vec!["..".to_string(), "/etc/passwd".to_string()]
        );

        // `cp src dst` → first non-flag is subcommand-like, second is arg
        let parsed = parse_command("cp src dst");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("src"));
        assert_eq!(parsed.programs[0].args, vec!["dst"]);

        // `cp -r src dst` → -r is a short flag, so `src` is consumed as
        // its value (since `src` is not path-shaped) and `dst` is the
        // first non-flag token after that, becoming the subcommand.
        // The escape check tolerates this: any path-shaped value
        // consumed as a flag value (e.g. `-I/usr/include`) is still
        // surfaced into `args` via `is_path_shaped`.
        let parsed = parse_command("cp -r src dst");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("dst"));
        assert!(parsed.programs[0].args.is_empty());

        // `cat README.md` → README.md is the subcommand, no extra args
        let parsed = parse_command("cat README.md");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("README.md"));
        assert!(parsed.programs[0].args.is_empty());

        // `ls -la` → no subcommand, no args
        let parsed = parse_command("ls -la");
        assert_eq!(parsed.programs[0].subcommand, None);
        assert!(parsed.programs[0].args.is_empty());
    }

    #[test]
    fn test_args_redirection() {
        // `cat > newfile.txt` — `>` is a non-flag token, so it becomes
        // the subcommand and `newfile.txt` becomes a positional arg.
        let parsed = parse_command("cat > newfile.txt");
        assert_eq!(parsed.programs[0].program, "cat");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some(">"));
        assert_eq!(parsed.programs[0].args, vec!["newfile.txt"]);

        // `cat > ../newfile.txt` — the `../newfile.txt` is path-shaped
        // and surfaces in args.
        let parsed = parse_command("cat > ../newfile.txt");
        assert_eq!(parsed.programs[0].args, vec!["../newfile.txt"]);

        // `cat >> out.txt` — `>>` is the subcommand.
        let parsed = parse_command("cat >> out.txt");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some(">>"));
        assert_eq!(parsed.programs[0].args, vec!["out.txt"]);

        // `echo hello 2>&1` — `2>&1` starts with a digit, not `-`, so
        // it's treated as a positional arg. The escape check is
        // path-shaped so this is harmless.
        let parsed = parse_command("echo hello 2>&1");
        assert_eq!(parsed.programs[0].program, "echo");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("hello"));
        assert_eq!(parsed.programs[0].args, vec!["2>&1"]);
    }

    #[test]
    fn test_args_multiple_double_dash() {
        // Only the first `--` is special; a second `--` after it is
        // just a positional arg.
        let parsed = parse_command("git log -- --foo --");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("log"));
        assert_eq!(
            parsed.programs[0].args,
            vec!["--foo".to_string(), "--".to_string()]
        );
    }

    #[test]
    fn test_args_quoted_paths() {
        // Quoted path with a space — shell-words handles the quotes, so
        // the token is the full path.
        let parsed = parse_command("cat 'my file.txt'");
        assert_eq!(parsed.programs[0].program, "cat");
        assert_eq!(
            parsed.programs[0].subcommand.as_deref(),
            Some("my file.txt")
        );
        assert!(parsed.programs[0].args.is_empty());

        // Quoted path with a separator inside — not split.
        let parsed = parse_command("echo 'hello; world'");
        assert_eq!(parsed.programs.len(), 1);
        assert_eq!(parsed.programs[0].program, "echo");
        assert_eq!(
            parsed.programs[0].subcommand.as_deref(),
            Some("hello; world")
        );
    }

    #[test]
    fn test_args_env_prefix_with_flags() {
        // Env prefix + flags + positional path.
        let parsed = parse_command("FOO=bar cargo test --release -- --nocapture");
        assert_eq!(parsed.programs[0].program, "cargo");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("test"));
        // After `--`, `--nocapture` is positional.
        assert_eq!(parsed.programs[0].args, vec!["--nocapture"]);
    }

    #[test]
    fn test_args_long_flag_with_equals() {
        // `--flag=value` — value is in the same token, so it's treated
        // as a single flag and skipped.
        let parsed = parse_command("grep --color=always pattern file.txt");
        assert_eq!(parsed.programs[0].program, "grep");
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("pattern"));
        assert_eq!(parsed.programs[0].args, vec!["file.txt"]);
    }

    #[test]
    fn test_args_short_flag_with_attached_value() {
        // `-I/usr/include` — the value is attached to the flag. The
        // token starts with `-` and `is_short_flag` returns true, so
        // the parser ALSO consumes the next token as a value. This is
        // a known approximation: the attached-value form consumes the
        // next token too, which means `main.c` is consumed as the value
        // (and suppressed since it's not path-shaped). The escape check
        // won't catch `-I/usr/include` in the attached form — only the
        // separate `-I /usr/include` form surfaces the path.
        let parsed = parse_command("gcc -I/usr/include main.c");
        assert_eq!(parsed.programs[0].program, "gcc");
        // `-I/usr/include` is a short flag → consume_next_as_value=true.
        // `main.c` is consumed as the value (not path-shaped → suppressed).
        // No subcommand, no args.
        assert_eq!(parsed.programs[0].subcommand, None);
        assert!(parsed.programs[0].args.is_empty());
    }

    #[test]
    fn test_args_short_flag_with_separate_value() {
        // `-I /usr/include` — the value is in the next token, and
        // `is_path_shaped` surfaces it into args.
        let parsed = parse_command("gcc -I /usr/include main.c");
        assert_eq!(parsed.programs[0].program, "gcc");
        // `/usr/include` is path-shaped, so it's surfaced into args
        // even though it was consumed as `-I`'s value.
        assert_eq!(parsed.programs[0].args, vec!["/usr/include"]);
        // `main.c` is the next non-flag token → subcommand.
        assert_eq!(parsed.programs[0].subcommand.as_deref(), Some("main.c"));
    }

    #[test]
    fn test_opaque_command_substitution() {
        assert!(parse_command("echo $(whoami)").has_opaque);
        assert!(parse_command("RESULT=`whoami`").has_opaque);
        assert!(parse_command("diff <(ls a) <(ls b)").has_opaque);
    }

    #[test]
    fn test_opaque_executors() {
        assert!(parse_command("eval 'rm -rf /'").has_opaque);
        assert!(parse_command("bash -c 'rm -rf /'").has_opaque);
        assert!(parse_command("echo foo | sh").has_opaque);
        assert!(parse_command("echo foo | xargs rm").has_opaque);
        assert!(parse_command("python -c 'import os'").has_opaque);
    }

    #[test]
    fn test_non_opaque_clean() {
        assert!(!parse_command("git status").has_opaque);
        assert!(!parse_command("cargo build --release").has_opaque);
        assert!(!parse_command("echo hello world").has_opaque);
    }

    #[test]
    fn test_quoted_separator_not_split() {
        let parsed = parse_command("echo 'hello; world'");
        assert_eq!(parsed.programs.len(), 1);
        assert_eq!(parsed.programs[0].program, "echo");
    }

    #[test]
    fn test_empty_command() {
        let parsed = parse_command("");
        assert!(parsed.programs.is_empty());
        assert!(!parsed.has_opaque);
    }
}
