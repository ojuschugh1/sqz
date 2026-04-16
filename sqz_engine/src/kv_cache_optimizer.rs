//! KV Cache Attention Sink Preservation — compresses content while preserving
//! tokens critical for LLM KV cache efficiency.
//!
//! LLMs allocate disproportionate attention to the first few tokens ("attention
//! sinks") and to tokens at provider-specific cache boundaries. This module
//! ensures those tokens are never removed during compression.
//!
//! # Provider-specific behavior
//! - **Anthropic**: preserves tokens after `cache_control` markers (detected by
//!   `[cache_break]` sentinel in content) and the first N attention sink tokens.
//! - **OpenAI**: preserves the first 1024 tokens (automatic cache boundary) and
//!   the first N attention sink tokens.
//! - **Google/Gemini**: preserves only the first N attention sink tokens (no
//!   provider cache boundary).
//! - **Local**: preserves only the first N attention sink tokens.
//!
//! # Example
//! ```
//! use sqz_engine::kv_cache_optimizer::compress_with_sinks;
//! use sqz_engine::types::ModelFamily;
//!
//! let content = "Important system prompt.\n".repeat(100);
//! let result = compress_with_sinks(&content, &ModelFamily::AnthropicClaude);
//! assert!(!result.is_empty());
//! ```

use crate::types::ModelFamily;

/// Default number of attention sink tokens to always preserve.
const DEFAULT_SINK_TOKENS: usize = 4;

/// Approximate characters per token (used for heuristic token boundaries).
const CHARS_PER_TOKEN: usize = 4;

/// OpenAI automatic cache boundary in tokens.
const OPENAI_CACHE_TOKENS: usize = 1024;

/// Sentinel marker for Anthropic cache_control breakpoints in content.
const ANTHROPIC_CACHE_SENTINEL: &str = "[cache_break]";

/// Compress content while preserving attention sink tokens and provider
/// cache boundaries.
///
/// # Arguments
/// * `content` — the full text content to compress
/// * `model_family` — the model family, used to determine cache boundary rules
///
/// # Returns
/// Compressed content string with critical tokens preserved.
pub fn compress_with_sinks(content: &str, model_family: &ModelFamily) -> String {
    if content.is_empty() {
        return String::new();
    }

    let sink_chars = DEFAULT_SINK_TOKENS * CHARS_PER_TOKEN;

    // Determine the protected prefix length based on provider
    let protected_prefix = match model_family {
        ModelFamily::AnthropicClaude => {
            // Protect attention sinks + content up to the last cache_control marker
            let cache_end = find_anthropic_cache_end(content);
            sink_chars.max(cache_end)
        }
        ModelFamily::OpenAiGpt => {
            // Protect attention sinks + first 1024 tokens (≈4096 chars)
            let openai_boundary = OPENAI_CACHE_TOKENS * CHARS_PER_TOKEN;
            sink_chars.max(openai_boundary)
        }
        ModelFamily::GoogleGemini | ModelFamily::Local(_) => {
            // Only protect attention sinks
            sink_chars
        }
    };

    // Clamp to content length
    let protected_prefix = protected_prefix.min(content.len());

    // Split content into protected and compressible regions
    let protected = &content[..protected_prefix];
    let compressible = &content[protected_prefix..];

    if compressible.is_empty() {
        return content.to_string();
    }

    // Compress the non-protected region by removing redundant whitespace
    // and collapsing repeated lines
    let compressed_tail = compress_region(compressible);

    format!("{protected}{compressed_tail}")
}

/// Compress content with a custom sink token count.
///
/// Same as [`compress_with_sinks`] but allows overriding the default
/// attention sink token count.
pub fn compress_with_custom_sinks(
    content: &str,
    model_family: &ModelFamily,
    sink_tokens: usize,
) -> String {
    if content.is_empty() {
        return String::new();
    }

    let sink_chars = sink_tokens * CHARS_PER_TOKEN;

    let protected_prefix = match model_family {
        ModelFamily::AnthropicClaude => {
            let cache_end = find_anthropic_cache_end(content);
            sink_chars.max(cache_end)
        }
        ModelFamily::OpenAiGpt => {
            let openai_boundary = OPENAI_CACHE_TOKENS * CHARS_PER_TOKEN;
            sink_chars.max(openai_boundary)
        }
        ModelFamily::GoogleGemini | ModelFamily::Local(_) => sink_chars,
    };

    let protected_prefix = protected_prefix.min(content.len());
    let protected = &content[..protected_prefix];
    let compressible = &content[protected_prefix..];

    if compressible.is_empty() {
        return content.to_string();
    }

    let compressed_tail = compress_region(compressible);
    format!("{protected}{compressed_tail}")
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Find the byte offset after the last `[cache_break]` sentinel in content.
/// Returns 0 if no sentinel is found.
fn find_anthropic_cache_end(content: &str) -> usize {
    if let Some(pos) = content.rfind(ANTHROPIC_CACHE_SENTINEL) {
        pos + ANTHROPIC_CACHE_SENTINEL.len()
    } else {
        0
    }
}

/// Simple compression of a text region: collapse consecutive blank lines,
/// remove trailing whitespace, and deduplicate consecutive identical lines.
fn compress_region(text: &str) -> String {
    let mut result = Vec::new();
    let mut prev_line: Option<&str> = None;
    let mut blank_count = 0;

    for line in text.lines() {
        let trimmed = line.trim_end();

        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push(String::new());
            }
            prev_line = None;
            continue;
        }

        blank_count = 0;

        // Skip consecutive duplicate lines
        if prev_line == Some(trimmed) {
            continue;
        }

        result.push(trimmed.to_string());
        prev_line = Some(trimmed);
    }

    // Preserve trailing newline if original had one
    let mut out = result.join("\n");
    if text.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_content() {
        let result = compress_with_sinks("", &ModelFamily::AnthropicClaude);
        assert!(result.is_empty());
    }

    #[test]
    fn test_short_content_preserved() {
        // Content shorter than the sink region should be fully preserved
        let content = "hello";
        let result = compress_with_sinks(content, &ModelFamily::AnthropicClaude);
        assert_eq!(result, content);
    }

    #[test]
    fn test_anthropic_cache_sentinel_preserved() {
        let mut content = String::new();
        content.push_str("System prompt line 1\n");
        content.push_str("System prompt line 2\n");
        content.push_str(ANTHROPIC_CACHE_SENTINEL);
        content.push('\n');
        // Add compressible content after the sentinel
        for _ in 0..50 {
            content.push_str("repeated line\n");
        }

        let result = compress_with_sinks(&content, &ModelFamily::AnthropicClaude);

        // Everything up to and including the sentinel should be preserved
        assert!(
            result.contains(ANTHROPIC_CACHE_SENTINEL),
            "cache sentinel should be preserved"
        );
        // The repeated lines after should be compressed
        assert!(
            result.len() < content.len(),
            "compressed ({}) should be shorter than original ({})",
            result.len(),
            content.len()
        );
    }

    #[test]
    fn test_openai_preserves_first_1024_tokens() {
        // Create content longer than 1024 tokens (≈4096 chars)
        let prefix = "x".repeat(4096);
        let suffix = "repeated line\n".repeat(100);
        let content = format!("{prefix}{suffix}");

        let result = compress_with_sinks(&content, &ModelFamily::OpenAiGpt);

        // The first 4096 chars should be preserved exactly
        assert!(
            result.starts_with(&prefix),
            "first 4096 chars should be preserved for OpenAI"
        );
        // Overall should be compressed
        assert!(result.len() < content.len());
    }

    #[test]
    fn test_local_model_only_preserves_sinks() {
        let content = "important\n".repeat(100);
        let result = compress_with_sinks(&content, &ModelFamily::Local("llama".into()));

        // First DEFAULT_SINK_TOKENS * CHARS_PER_TOKEN chars preserved
        let sink_chars = DEFAULT_SINK_TOKENS * CHARS_PER_TOKEN;
        let expected_prefix = &content[..sink_chars.min(content.len())];
        assert!(
            result.starts_with(expected_prefix),
            "sink tokens should be preserved"
        );
    }

    #[test]
    fn test_custom_sink_count() {
        let content = "a".repeat(200);
        let result = compress_with_custom_sinks(
            &content,
            &ModelFamily::GoogleGemini,
            10, // 10 sink tokens = 40 chars
        );
        // First 40 chars should be preserved
        assert!(result.starts_with(&"a".repeat(40)));
    }

    #[test]
    fn test_compress_region_deduplicates() {
        let text = "line a\nline a\nline a\nline b\nline b\n";
        let compressed = compress_region(text);
        assert_eq!(compressed, "line a\nline b\n");
    }

    #[test]
    fn test_compress_region_collapses_blanks() {
        let text = "line a\n\n\n\n\nline b\n";
        let compressed = compress_region(text);
        assert_eq!(compressed, "line a\n\nline b\n");
    }

    // ── Property-based tests ──────────────────────────────────────────────

    use proptest::prelude::*;

    fn arb_model_family() -> impl Strategy<Value = ModelFamily> {
        prop_oneof![
            Just(ModelFamily::AnthropicClaude),
            Just(ModelFamily::OpenAiGpt),
            Just(ModelFamily::GoogleGemini),
            Just(ModelFamily::Local("test".into())),
        ]
    }

    proptest! {
        /// Compressed output is never longer than the original.
        #[test]
        fn prop_compressed_not_longer(
            content in "[a-z \n]{10,500}",
            model in arb_model_family(),
        ) {
            let result = compress_with_sinks(&content, &model);
            prop_assert!(
                result.len() <= content.len(),
                "compressed ({}) should be <= original ({})",
                result.len(),
                content.len()
            );
        }

        /// The attention sink prefix is always preserved.
        #[test]
        fn prop_sink_prefix_preserved(
            content in "[a-z]{50,200}",
            model in arb_model_family(),
        ) {
            let result = compress_with_sinks(&content, &model);
            let sink_chars = (DEFAULT_SINK_TOKENS * CHARS_PER_TOKEN).min(content.len());
            let expected_prefix = &content[..sink_chars];
            prop_assert!(
                result.starts_with(expected_prefix),
                "sink prefix should be preserved"
            );
        }

        /// Empty input always produces empty output.
        #[test]
        fn prop_empty_in_empty_out(model in arb_model_family()) {
            let result = compress_with_sinks("", &model);
            prop_assert!(result.is_empty());
        }
    }
}
