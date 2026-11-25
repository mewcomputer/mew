//! Built-in prompt templates for context checkpoint compaction.

/// System prompt sent to the model when summarizing compacted conversation
/// history into a handoff checkpoint.
pub fn prompt() -> &'static str {
    crate::vfs::read_builtin("compact/prompt")
        .expect("resources/compact/prompt.md must be embedded")
        .trim()
}

/// Prefix prepended when re-injecting a compacted handoff summary into the
/// model-facing conversation history.
pub fn summary_prefix() -> &'static str {
    crate::vfs::read_builtin("compact/summary_prefix")
        .expect("resources/compact/summary_prefix.md must be embedded")
        .trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_prompt_contains_checkpoint_framing_and_structured_sections() {
        let p = prompt();
        assert!(!p.is_empty());
        assert!(p.contains("CONTEXT CHECKPOINT COMPACTION"));
        assert!(p.contains("## Goal"));
        assert!(p.contains("## Next"));
        assert!(!p.contains("software engineering"));
    }

    #[test]
    fn compact_summary_prefix_contains_handoff_framing() {
        let prefix = summary_prefix();
        assert!(!prefix.is_empty());
        assert!(prefix.contains("Another language model started to solve this problem"));
    }
}
