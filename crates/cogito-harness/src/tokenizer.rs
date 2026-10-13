//! One token counter for the whole binary.
//!
//! Two places need to know how many tokens a string occupies: `cogito bench`
//! reports a token comparison, and the `context_pack` tool has to stop adding
//! fragments at a token budget. Both must use the *same* counter, or the two
//! numbers stop being comparable, so the counter lives here rather than inside
//! either caller.
//!
//! The vocabulary is `cl100k_base`, the closest available match to the
//! tokenizers current coding agents use.

use tiktoken_rs::{CoreBPE, cl100k_base};

/// Token counter using tiktoken-rs with the cl100k_base vocabulary.
/// Falls back to a byte-count / 4 estimate if the tokenizer fails to initialise.
pub struct TokenCounter {
    inner: Option<CoreBPE>,
}

impl TokenCounter {
    /// Create a new token counter. Logs a warning and falls back to byte/4
    /// estimation if tiktoken-rs fails to load.
    pub fn new() -> Self {
        let inner = cl100k_base().ok();
        if inner.is_none() {
            eprintln!(
                "warning: tiktoken-rs failed to initialise cl100k_base; \
                 falling back to byte_count/4 estimate"
            );
        }
        Self { inner }
    }

    /// Count tokens in `text`. Returns (tokens, raw_bytes) for transparency.
    pub fn count(&self, text: &str) -> (usize, usize) {
        let bytes = text.len();
        match &self.inner {
            Some(encoder) => {
                let tokens = encoder.encode_ordinary(text).len();
                (tokens, bytes)
            }
            None => (bytes / 4, bytes),
        }
    }

    /// Which counter is actually in use, for reporting.
    ///
    /// A caller that publishes a token number has to say whether it came from
    /// the real vocabulary or from the fallback estimate, or the number is not
    /// comparable with anything.
    pub fn name(&self) -> &'static str {
        if self.inner.is_some() {
            "cl100k_base"
        } else {
            "byte_count/4_estimate"
        }
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::TokenCounter;

    #[test]
    fn empty_text_costs_nothing_and_text_costs_something() {
        let counter = TokenCounter::new();
        assert_eq!(counter.count("").0, 0, "empty text has no tokens");
        assert!(
            counter
                .count("fn require_session(input: &str) -> bool {}")
                .0
                > 0,
            "real text must consume tokens"
        );
    }

    #[test]
    fn longer_text_costs_more_than_shorter_text() {
        let counter = TokenCounter::new();
        let short = counter.count("let x = 1;").0;
        let long = counter.count(&"let x = 1;".repeat(20)).0;
        assert!(
            long > short,
            "repeated text must cost more: {long} vs {short}"
        );
    }

    #[test]
    fn count_reports_tokens_and_raw_bytes_and_the_name_identifies_the_counter() {
        let counter = TokenCounter::new();
        let text = "pub fn handler() {}";
        let (tokens, bytes) = counter.count(text);
        assert!(tokens > 0);
        assert_eq!(bytes, text.len(), "byte count is the raw length");
        assert!(
            matches!(counter.name(), "cl100k_base" | "byte_count/4_estimate"),
            "unexpected counter name: {}",
            counter.name()
        );
    }
}
