//! Reasoning truncation: stop the "But.. Wait.. Actually.." loop that
//! some open models (notably GLM-class) fall into when their thinking
//! budget exceeds a threshold.
//!
//! Technique (per a published write-up on TerminalBench-style runs):
//! - When a single reasoning trace exceeds `threshold` tokens, append a
//!   forged assistant acknowledgement to the message history and force
//!   the next model call to use `tool_choice: required`. The combination
//!     - a short, on-brand "I was overthinking, committing to next action"
//!       sentence in the model's own prior turn, plus a hard requirement
//!       to call a tool — breaks the loop without abandoning the turn.
//!
//! Token counting is approximate (4 chars per token). The threshold is
//! a soft cap tuned by the operator; this isn't an exact tokenizer.
//! Setting `threshold = 0` disables truncation.

/// Default threshold in approximate tokens. 5k matches the recommendation
/// from the publish that motivated this feature — 1k was found too
/// aggressive in practice.
pub const DEFAULT_REASONING_TRUNCATION_THRESHOLD: u32 = 5000;

/// The forged acknowledgement we inject into the assistant message
/// history after a truncation. It is short, on-voice (the model
/// "thinks" it produced it), and explicitly commits to action.
pub const TRUNCATION_ACK_TEXT: &str = "I've been thinking too long. \
Acknowledging overthinking — committing to my next action now and stopping further deliberation.";

/// Truncation state carried across turns. Holds the configured
/// threshold plus a one-shot flag that asks the next model request to
/// use `tool_choice: required`.
#[derive(Debug, Clone)]
pub struct ReasoningTruncator {
    /// Approximate-token cap on a single reasoning trace. 0 = disabled.
    pub threshold: u32,
    /// When true, the next model request must call a tool. Set by
    /// `mark_truncated()`; consumed by `take_force_tool_choice()`.
    pub force_tool_choice_next: bool,
}

impl Default for ReasoningTruncator {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_REASONING_TRUNCATION_THRESHOLD,
            force_tool_choice_next: false,
        }
    }
}

impl ReasoningTruncator {
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold,
            force_tool_choice_next: false,
        }
    }

    /// Approximate the token count for a text. We deliberately keep this
    /// a cheap character-based heuristic — exact tokenization would mean
    /// pulling in each model's tokenizer, which isn't worth it for a
    /// soft cap.
    fn approx_tokens(text: &str) -> usize {
        // 4 chars/token is a fine average for English/code; round up so
        // a 1-char string rounds to 1 token.
        text.chars().count().div_ceil(4)
    }

    /// If `text` exceeds the threshold, return a truncated copy with a
    /// trailing marker explaining the truncation. Otherwise return None.
    ///
    /// Does NOT mutate state — callers that decide to act on the result
    /// should also call `mark_truncated()` so the next turn forces a
    /// tool call.
    pub fn maybe_truncate(&self, text: &str) -> Option<String> {
        if self.threshold == 0 {
            return None;
        }
        let approx = Self::approx_tokens(text);
        if approx <= self.threshold as usize {
            return None;
        }
        // Keep the first N chars (≈ N tokens worth) and append a
        // truncation marker. The marker itself is part of the
        // persisted reasoning content, so the model sees the
        // acknowledgement when it looks back.
        let max_chars = self.threshold as usize * 4;
        let truncated: String = text.chars().take(max_chars).collect();
        Some(format!(
            "{truncated}\n\n[… reasoning truncated at ~{} tokens: model was overthinking — please commit to your next action …]",
            self.threshold
        ))
    }

    /// Mark that the previous turn's reasoning was truncated. The next
    /// `take_force_tool_choice()` call will return true.
    pub fn mark_truncated(&mut self) {
        self.force_tool_choice_next = true;
    }

    /// Consume (and reset) the flag that says "next model request must
    /// call a tool". Returns the previous value.
    pub fn take_force_tool_choice(&mut self) -> bool {
        std::mem::replace(&mut self.force_tool_choice_next, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_tokens_handles_short_strings() {
        assert_eq!(ReasoningTruncator::approx_tokens(""), 0);
        assert_eq!(ReasoningTruncator::approx_tokens("a"), 1);
        assert_eq!(ReasoningTruncator::approx_tokens("abcd"), 1);
        assert_eq!(ReasoningTruncator::approx_tokens("abcde"), 2);
    }

    #[test]
    fn disabled_when_threshold_zero() {
        let t = ReasoningTruncator::new(0);
        let long = "x".repeat(100_000);
        assert!(t.maybe_truncate(&long).is_none());
    }

    #[test]
    fn no_truncation_under_threshold() {
        let t = ReasoningTruncator::new(100);
        // ~25 tokens, well under 100.
        let short = "x".repeat(100);
        assert!(t.maybe_truncate(&short).is_none());
    }

    #[test]
    fn truncates_over_threshold() {
        let t = ReasoningTruncator::new(10);
        let long = "x".repeat(200); // ~50 tokens
        let out = t.maybe_truncate(&long).expect("should truncate");
        assert!(out.starts_with("xxxxxxxxxx"));
        assert!(out.contains("truncated at ~10 tokens"));
        // The truncated content is ~threshold*4 chars + the marker.
        assert!(out.len() < long.len());
    }

    #[test]
    fn mark_truncated_then_take_consumes() {
        let mut t = ReasoningTruncator::new(100);
        assert!(!t.take_force_tool_choice());
        t.mark_truncated();
        assert!(t.take_force_tool_choice());
        assert!(
            !t.take_force_tool_choice(),
            "second take should return false"
        );
    }

    #[test]
    fn default_threshold_is_5000() {
        let t = ReasoningTruncator::default();
        assert_eq!(t.threshold, 5000);
        assert!(!t.force_tool_choice_next);
    }

    #[test]
    fn truncation_marker_acknowledges_overthinking() {
        let t = ReasoningTruncator::new(10);
        let long = "thinking ".repeat(200);
        let out = t.maybe_truncate(&long).expect("should truncate");
        assert!(out.contains("overthinking"));
    }
}
