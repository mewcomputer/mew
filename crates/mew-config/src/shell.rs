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

/// Parse one segment into a program invocation. Returns `None` if the
/// segment cannot be tokenized (e.g. bare assignment like `FOO=bar`).
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

    // Subcommand: the first non-flag arg after the program.
    let subcommand = tokens
        .iter()
        .skip(start_idx + 1)
        .find(|t| !t.starts_with('-'))
        .cloned();

    Some(ProgramInvocation {
        program,
        subcommand,
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
    }

    #[test]
    fn test_subcommand_extraction() {
        let parsed = parse_command("git push origin main");
        assert_eq!(
            parsed.programs[0],
            ProgramInvocation {
                program: "git".into(),
                subcommand: Some("push".into()),
            }
        );
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
