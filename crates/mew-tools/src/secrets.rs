//! Shared secret-redaction helpers used by tools that return file or
//! command output.
//!
//! Two modes:
//! - [`redact_secret_words`] replaces each occurrence of a configured secret
//!   word with `[REDACTED]`, preserving surrounding context. Used by `read`
//!   and `bash` where the model benefits from seeing the structure (variable
//!   names, line shape) without the actual secret values.
//! - [`drop_secret_files`] filters out lines whose leading `path:` segment
//!   matches a secret-file glob. Used by `grep`/`glob` search results where a
//!   file-level match means the whole hit should disappear.

use crate::SecretSet;

/// Replace every occurrence of each configured secret word with
/// `[REDACTED]`. Returns the redacted text and the total number of
/// replacements made.
///
/// Empty words are ignored (a `""` entry would match everything, which is
/// never what the user intended). Words are matched as plain substrings —
/// case-sensitive — so configuring `AKIAIOSFODNN7EXAMPLE` will not catch
/// lowercase variants. That is intentional: the word list should hold
/// high-entropy values that do not appear by accident.
pub fn redact_secret_words(text: &str, secrets: &SecretSet) -> (String, usize) {
    let words: Vec<&str> = secrets
        .words
        .iter()
        .map(|s| s.as_str())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = text.to_string();
    let mut count = 0usize;
    for word in words {
        let occurrences = out.matches(word).count();
        if occurrences > 0 {
            count += occurrences;
            out = out.replace(word, "[REDACTED]");
        }
    }
    (out, count)
}

/// Append a one-line summary noting how many secrets were stripped, so the
/// model knows the output is not complete and can ask the user for the
/// value if it actually needs it. Returns the text unchanged when nothing
/// was redacted.
pub fn annotate_redaction(text: String, redacted: usize) -> String {
    if redacted == 0 {
        text
    } else {
        format!("{text}\n[{redacted} secret value(s) redacted from output]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(words: &[&str]) -> SecretSet {
        SecretSet {
            words: words.iter().map(|s| s.to_string()).collect(),
            globs: vec![],
        }
    }

    #[test]
    fn test_redact_replaces_known_word() {
        let (out, n) = redact_secret_words(
            "key=AKIAIOSFODNN7EXAMPLE\nother=foo",
            &secrets(&["AKIAIOSFODNN7EXAMPLE"]),
        );
        assert_eq!(out, "key=[REDACTED]\nother=foo");
        assert_eq!(n, 1);
    }

    #[test]
    fn test_redact_preserves_structure() {
        let (out, _) = redact_secret_words(
            "API_KEY=AKIAIOSFODNN7EXAMPLE",
            &secrets(&["AKIAIOSFODNN7EXAMPLE"]),
        );
        // The variable name survives; only the value is stripped.
        assert_eq!(out, "API_KEY=[REDACTED]");
    }

    #[test]
    fn test_redact_handles_multiple_words() {
        let (out, n) = redact_secret_words(
            "a=ghp_abc123\nb=AKIADEF\nc=plain",
            &secrets(&["ghp_abc123", "AKIADEF"]),
        );
        assert_eq!(out, "a=[REDACTED]\nb=[REDACTED]\nc=plain");
        assert_eq!(n, 2);
    }

    #[test]
    fn test_redact_handles_repeated_word() {
        let (out, n) = redact_secret_words("token=abc; refresh=abc", &secrets(&["abc"]));
        assert_eq!(out, "token=[REDACTED]; refresh=[REDACTED]");
        assert_eq!(n, 2);
    }

    #[test]
    fn test_redact_empty_secrets_is_noop() {
        let (out, n) = redact_secret_words("anything", &SecretSet::default());
        assert_eq!(out, "anything");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_redact_empty_word_ignored() {
        // A `""` entry must not redact the entire output.
        let (out, n) = redact_secret_words("hello", &secrets(&[""]));
        assert_eq!(out, "hello");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_annotate_only_when_redacted() {
        assert_eq!(annotate_redaction("clean".into(), 0), "clean");
        assert_eq!(
            annotate_redaction("a=[REDACTED]".into(), 1),
            "a=[REDACTED]\n[1 secret value(s) redacted from output]"
        );
    }
}
