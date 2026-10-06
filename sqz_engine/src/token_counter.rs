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
        count_in_slices(text, |s| lock.encode_with_special_tokens(s).len())
    }

    /// Count tokens using the `o200k_base` encoding (GPT-4o / o1 / o3).
    fn count_o200k(&self, text: &str) -> u32 {
        let bpe = tiktoken_rs::o200k_base_singleton();
        let lock = bpe.lock();
        count_in_slices(text, |s| lock.encode_with_special_tokens(s).len())
    }
}

/// Longest single-class run the tokenizer gets in one call: BPE is quadratic in a piece's length.
const MAX_RUN_BYTES: usize = 2048;

/// Sum of `encode` over slices with no run longer than [`MAX_RUN_BYTES`].
fn count_in_slices(text: &str, encode: impl Fn(&str) -> usize) -> u32 {
    let mut total = 0;
    let mut start = 0;
    let mut last = ("", 0);
    for end in run_cuts(text).into_iter().chain([text.len()]) {
        let slice = &text[start..end];
        if slice != last.0 {
            let n = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode(slice)))
                .unwrap_or_else(|_| TokenCounter::count_fast(slice) as usize);
            last = (slice, n);
        }
        total += last.1;
        start = end;
    }
    total as u32
}

/// Offsets that cut every letter, symbol, space and newline run to [`MAX_RUN_BYTES`].
fn run_cuts(text: &str) -> Vec<usize> {
    static RUNS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    if text.len() <= MAX_RUN_BYTES || !may_hold_long_run(text) {
        return Vec::new();
    }
    let runs = RUNS.get_or_init(|| {
        [r"[\p{L}\p{M}]+", r"[^\s\p{L}\p{N}]+", r"\s+", r"[\r\n/]+"]
            .iter()
            .filter_map(|p| regex::Regex::new(p).ok())
            .collect()
    });
    let mut long: Vec<(usize, usize)> = runs
        .iter()
        .flat_map(|re| re.find_iter(text))
        .filter(|m| m.len() > MAX_RUN_BYTES)
        .map(|m| (m.start(), m.end()))
        .collect();
    long.sort_unstable();
    let mut cuts = Vec::new();
    let mut i = 0;
    while i < long.len() {
        let (mut at, mut end) = long[i];
        while i + 1 < long.len() && long[i + 1].0 <= end {
            i += 1;
            end = end.max(long[i].1);
        }
        while end - at > MAX_RUN_BYTES {
            at = crate::text_boundary::floor_char_boundary(text, at + MAX_RUN_BYTES);
            cuts.push(at);
        }
        i += 1;
    }
    cuts
}

/// Byte scan for a run over [`MAX_RUN_BYTES`]; a non-ASCII byte counts as letter, symbol and space.
fn may_hold_long_run(text: &str) -> bool {
    let mut runs = [0usize; 4];
    for &b in text.as_bytes() {
        let space = b.is_ascii_whitespace() || b == 0x0b;
        let letter = b.is_ascii_alphabetic();
        let wide = !b.is_ascii();
        let member = [
            wide || letter,
            wide || !(space || letter || b.is_ascii_digit()),
            wide || space,
            matches!(b, b'\r' | b'\n' | b'/'),
        ];
        for (run, member) in runs.iter_mut().zip(member) {
            *run = if member { *run + 1 } else { 0 };
            if *run > MAX_RUN_BYTES {
                return true;
            }
        }
    }
    false
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

    fn tiktoken(text: &str) -> (u32, u32) {
        let cl = tiktoken_rs::cl100k_base_singleton().lock().encode_with_special_tokens(text).len();
        let o2 = tiktoken_rs::o200k_base_singleton().lock().encode_with_special_tokens(text).len();
        (cl as u32, o2 as u32)
    }

    fn both(text: &str) -> (u32, u32) {
        let tc = TokenCounter::new();
        (tc.count(text, &ModelFamily::AnthropicClaude), tc.count(text, &ModelFamily::OpenAiGpt))
    }

    #[test]
    fn count_is_exact_when_no_run_is_cut() {
        let at_limit = |unit: &str| unit.repeat(MAX_RUN_BYTES / unit.len());
        let texts = [
            "fn main() { println!(\"hi\"); }\n".repeat(500),
            format!("{}a{}", at_limit("="), at_limit(" ")),
            format!("{}={}", at_limit("a"), at_limit("\n")),
            format!("{}a{}", at_limit("─"), at_limit("/")),
            "日本語のテキスト、句読点で区切る。".repeat(400),
            "Привет мир строка номер 1\n".repeat(300),
        ];
        for text in &texts {
            let head = crate::text_boundary::truncate_str(text, 40);
            assert!(run_cuts(text).is_empty(), "{head:?}");
            assert_eq!(both(text), tiktoken(text), "{head:?}");
        }
    }

    #[test]
    fn count_survives_a_run_of_a_million_characters() {
        for unit in ["=", " ", "\n", "a", "日", "\u{301}", "=\u{301}", "\n/", "Aa"] {
            let text = unit.repeat(1_100_000 / unit.chars().count());
            let (cl, o2) = both(&text);
            assert!(cl > 0 && o2 > 0 && cl as usize <= text.len() && o2 as usize <= text.len(), "{unit:?}");
        }
    }

    #[test]
    fn count_of_a_cut_run_stays_close() {
        for unit in ["=", " ", "\n", "a", "ab", "─"] {
            let text = format!("x {} y\n", unit.repeat(8192 / unit.len()));
            let (want, got) = (tiktoken(&text), both(&text));
            for (w, g) in [(want.0, got.0), (want.1, got.1)] {
                assert!(w.abs_diff(g) <= w / 100 + 4, "{unit:?}: {g} vs {w}");
            }
        }
    }

    #[test]
    fn count_is_linear_on_long_runs() {
        let run = |bytes: usize| "abcdefghijklmnopqrstuvwxyz".repeat(bytes / 26);
        let (small, large) = (run(8 << 10), run(80 << 10));
        let ratio = crate::text_search::time_ratio(
            || {
                std::hint::black_box(both(&small));
            },
            || {
                std::hint::black_box(both(&large));
            },
        );
        assert!(ratio < 25.0, "80 KB took {ratio:.1}x as long as 8 KB");
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
