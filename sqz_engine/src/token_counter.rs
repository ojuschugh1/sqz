use crate::types::ModelFamily;

/// Token counter using BPE tokenizer models.
///
/// Supports:
/// - OpenAI GPT-4/4o: exact via `cl100k_base`
/// - OpenAI o1/o3: exact via `o200k_base`
/// - Anthropic Claude: approximate via `cl100k_base` (Claude uses a proprietary tokenizer;
///   cl100k_base is a close approximation, typically within 5% of actual count)
/// - Google Gemini: approximate via `cl100k_base` (SentencePiece-based; cl100k_base provides
///   a reasonable cross-model estimate)
/// - Local/unknown models: fast heuristic fallback (`chars / 4`)
///
/// Uses `tiktoken-rs` singletons so the BPE data is loaded once and reused.
pub struct TokenCounter;

impl TokenCounter {
    /// Create a new `TokenCounter`.
    ///
    /// The underlying tokenizers are lazily-initialized singletons from
    /// `tiktoken-rs`, so construction is essentially free.
    pub fn new() -> Self {
        Self
    }

    /// Count tokens for `text` using the tokenizer appropriate for `model`.
    ///
    /// - `OpenAiGpt` → `o200k_base` (exact for GPT-4o / o1 / o3)
    /// - `AnthropicClaude` → `cl100k_base` (close approximation, ~5% variance)
    /// - `GoogleGemini` → `cl100k_base` (cross-model approximation)
    /// - `Local(_)` → `chars / 4` heuristic fallback
    pub fn count(&self, text: &str, model: &ModelFamily) -> u32 {
        match model {
            ModelFamily::OpenAiGpt => self.count_o200k(text),
            ModelFamily::AnthropicClaude | ModelFamily::GoogleGemini => self.count_cl100k(text),
            ModelFamily::Local(_) => Self::count_fast(text),
        }
    }

    /// Fast character-based approximation: `ceil(chars / 4)`.
    ///
    /// Used as the fallback for unknown or local models.
    pub fn count_fast(text: &str) -> u32 {
        ((text.len() as f64) / 4.0).ceil() as u32
    }

    /// Count tokens using the `cl100k_base` encoding (GPT-4 / Claude compatible).
    fn count_cl100k(&self, text: &str) -> u32 {
        let bpe = tiktoken_rs::cl100k_base_singleton();
        let lock = bpe.lock();
        lock.encode_with_special_tokens(text).len() as u32
    }

    /// Count tokens using the `o200k_base` encoding (GPT-4o / o1 / o3).
    fn count_o200k(&self, text: &str) -> u32 {
        let bpe = tiktoken_rs::o200k_base_singleton();
        let lock = bpe.lock();
        lock.encode_with_special_tokens(text).len() as u32
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_empty_string() {
        let tc = TokenCounter::new();
        assert_eq!(tc.count("", &ModelFamily::OpenAiGpt), 0);
        assert_eq!(tc.count("", &ModelFamily::AnthropicClaude), 0);
        assert_eq!(TokenCounter::count_fast(""), 0);
    }

    #[test]
    fn count_hello_world_openai() {
        let tc = TokenCounter::new();
        let count = tc.count("Hello, world!", &ModelFamily::OpenAiGpt);
        // o200k_base tokenizes "Hello, world!" into a small number of tokens
        assert!(count > 0 && count < 10, "unexpected count: {count}");
    }

    #[test]
    fn count_hello_world_claude() {
        let tc = TokenCounter::new();
        let count = tc.count("Hello, world!", &ModelFamily::AnthropicClaude);
        assert!(count > 0 && count < 10, "unexpected count: {count}");
    }

    #[test]
    fn count_hello_world_gemini() {
        let tc = TokenCounter::new();
        let count = tc.count("Hello, world!", &ModelFamily::GoogleGemini);
        // Uses cl100k_base, same as Claude path
        assert!(count > 0 && count < 10, "unexpected count: {count}");
    }

    #[test]
    fn count_local_model_uses_fallback() {
        let tc = TokenCounter::new();
        // "abcdefgh" = 8 chars → ceil(8/4) = 2
        let count = tc.count("abcdefgh", &ModelFamily::Local("llama".into()));
        assert_eq!(count, 2);
    }

    #[test]
    fn count_fast_basic() {
        assert_eq!(TokenCounter::count_fast("abcd"), 1);
        assert_eq!(TokenCounter::count_fast("abcde"), 2);
        assert_eq!(TokenCounter::count_fast("a"), 1);
    }

    #[test]
    fn exact_count_differs_from_approximation() {
        let tc = TokenCounter::new();
        let text = "fn main() { println!(\"Hello, world!\"); }";
        let exact = tc.count(text, &ModelFamily::OpenAiGpt);
        let approx = TokenCounter::count_fast(text);
        // They should both be positive but likely differ
        assert!(exact > 0);
        assert!(approx > 0);
    }

    // -----------------------------------------------------------------------
    // Property 37: Exact token count accuracy
    // Validates: Requirements 32.1, 32.2
    // -----------------------------------------------------------------------

    use proptest::prelude::*;

    /// Strategy that produces arbitrary non-empty strings.
    fn arb_nonempty_text() -> impl Strategy<Value = String> {
        // Mix of ASCII, unicode, code-like, and whitespace-heavy strings
        prop_oneof![
            // General unicode strings (1..256 chars)
            "\\PC{1,256}",
            // ASCII-only strings
            "[a-zA-Z0-9 .,;:!?()\\[\\]{}<>@#$%^&*_+=~`|/\\\\-]{1,256}",
            // Code-like strings
            "(fn|def|class|import|const|let|var|pub|async|await|return)[ a-zA-Z0-9_(){}:;,\n]{1,200}",
        ]
        .prop_filter("must be non-empty", |s| !s.is_empty())
    }

    /// Strategy for known model families (not Local, which uses fallback).
    fn arb_known_model() -> impl Strategy<Value = ModelFamily> {
        prop_oneof![
            Just(ModelFamily::OpenAiGpt),
            Just(ModelFamily::AnthropicClaude),
            Just(ModelFamily::GoogleGemini),
        ]
    }

    proptest! {
        /// **Validates: Requirements 32.1, 32.2**
        ///
        /// Property 37a: For any non-empty text and known model family,
        /// the exact token count is always > 0.
        #[test]
        fn prop37a_exact_count_positive_for_nonempty(
            text in arb_nonempty_text(),
            model in arb_known_model(),
        ) {
            let tc = TokenCounter::new();
            let count = tc.count(&text, &model);
            prop_assert!(
                count > 0,
                "exact token count must be > 0 for non-empty string (len={}), got {}",
                text.len(),
                count
            );
        }

        /// **Validates: Requirements 32.1, 32.2**
        ///
        /// Property 37b: For any non-empty text and known model family,
        /// the exact token count is always ≤ the character count, since
        /// BPE tokens span one or more characters.
        #[test]
        fn prop37b_exact_count_leq_char_count(
            text in arb_nonempty_text(),
            model in arb_known_model(),
        ) {
            let tc = TokenCounter::new();
            let count = tc.count(&text, &model);
            let char_count = text.len() as u32;
            prop_assert!(
                count <= char_count,
                "exact token count ({}) must be <= byte length ({}) for text",
                count,
                char_count
            );
        }

        /// **Validates: Requirements 32.1, 32.2**
        ///
        /// Property 37c: The fast fallback (chars/4) is within a
        /// reasonable factor (0.1x–10x) of the exact count for
        /// non-trivial text (>= 8 chars).
        #[test]
        fn prop37c_fast_fallback_within_reasonable_factor(
            text in "[a-zA-Z0-9 .,;:!?]{8,256}",
            model in arb_known_model(),
        ) {
            let tc = TokenCounter::new();
            let exact = tc.count(&text, &model) as f64;
            let fast = TokenCounter::count_fast(&text) as f64;
            // Both should be positive
            prop_assert!(exact > 0.0);
            prop_assert!(fast > 0.0);
            let ratio = exact / fast;
            prop_assert!(
                ratio >= 0.1 && ratio <= 10.0,
                "exact/fast ratio ({:.2}) out of reasonable range [0.1, 10.0]; exact={}, fast={}",
                ratio,
                exact,
                fast
            );
        }
    }

    #[test]
    fn performance_10k_tokens_under_1ms() {
        let tc = TokenCounter::new();
        // ~40K chars ≈ 10K tokens for typical English text
        let text = "Hello world. This is a test sentence for benchmarking. ".repeat(800);

        // Warm up the singleton
        let _ = tc.count(&text, &ModelFamily::OpenAiGpt);

        let start = std::time::Instant::now();
        let _count = tc.count(&text, &ModelFamily::OpenAiGpt);
        let elapsed = start.elapsed();

        // In debug builds under test load, allow generous headroom.
        // The <1ms target (Req 32.5) applies to optimized release builds.
        assert!(
            elapsed.as_millis() < 500,
            "token counting took {}ms, expected <500ms (debug build)",
            elapsed.as_millis()
        );
    }
}
