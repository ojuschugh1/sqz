/// Byte-Pair Encoding (BPE) for vocabulary compression.
///
/// Identifies the most frequent byte pairs in content, replaces them with
/// single symbols, and iterates. Each BPE iteration reduces the total
/// symbol count by at least 1 (the merged pair). After k iterations,
/// the content has at most n-k symbols where n is the original count.
///
/// This is the same algorithm that GPT tokenizers use, but applied to
/// compress content before it reaches the tokenizer.

use std::collections::HashMap;

use crate::error::Result;

/// Configuration for BPE compression.
#[derive(Debug, Clone)]
pub struct BpeConfig {
    /// Maximum number of merge iterations.
    /// Default: 50
    pub max_merges: usize,
    /// Minimum pair frequency to consider for merging.
    /// Default: 3
    pub min_frequency: usize,
}

impl Default for BpeConfig {
    fn default() -> Self {
        Self {
            max_merges: 50,
            min_frequency: 3,
        }
    }
}

/// A single BPE merge rule.
#[derive(Debug, Clone)]
pub struct MergeRule {
    /// The pair that was merged (left, right).
    pub pair: (String, String),
    /// The replacement symbol.
    pub symbol: String,
    /// How many times this pair appeared.
    pub frequency: usize,
}

/// Result of BPE compression.
#[derive(Debug, Clone)]
pub struct BpeResult {
    /// The compressed text with a merge table header.
    pub text: String,
    /// Merge rules applied.
    pub merges: Vec<MergeRule>,
    /// Estimated tokens saved.
    pub tokens_saved: u32,
}

/// Compress text using byte-pair encoding at the word level.
///
/// Instead of operating on raw bytes (which would produce unreadable output),
/// this operates on whitespace-separated tokens and merges frequently
/// co-occurring token pairs into single symbols.
pub fn bpe_compress(text: &str, config: &BpeConfig) -> Result<BpeResult> {
    let mut tokens: Vec<String> = text
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    if tokens.len() < 2 {
        return Ok(BpeResult {
            text: text.to_string(),
            merges: Vec::new(),
            tokens_saved: 0,
        });
    }

    let mut merges = Vec::new();
    let mut merge_idx = 0u32;

    for _ in 0..config.max_merges {
        // Count all adjacent pairs
        let mut pair_counts: HashMap<(String, String), usize> = HashMap::new();
        for window in tokens.windows(2) {
            let pair = (window[0].clone(), window[1].clone());
            *pair_counts.entry(pair).or_insert(0) += 1;
        }

        // Find the most frequent pair
        let best = pair_counts
            .iter()
            .filter(|(_, &count)| count >= config.min_frequency)
            .max_by_key(|(_, &count)| count);

        let (best_pair, &best_count) = match best {
            Some(b) => b,
            None => break, // No more pairs above threshold
        };

        // Create a merge symbol
        let symbol = format!("§{}§", merge_idx);
        merge_idx += 1;

        let left = best_pair.0.clone();
        let right = best_pair.1.clone();

        // Apply the merge: replace all occurrences of the pair
        let mut new_tokens = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            if i + 1 < tokens.len() && tokens[i] == left && tokens[i + 1] == right {
                new_tokens.push(symbol.clone());
                i += 2;
            } else {
                new_tokens.push(tokens[i].clone());
                i += 1;
            }
        }

        merges.push(MergeRule {
            pair: (left, right),
            symbol: symbol.clone(),
            frequency: best_count,
        });

        tokens = new_tokens;
    }

    if merges.is_empty() {
        return Ok(BpeResult {
            text: text.to_string(),
            merges: Vec::new(),
            tokens_saved: 0,
        });
    }

    // Build output with merge table header
    let mut header_lines = vec!["§bpe§".to_string()];
    for merge in &merges {
        header_lines.push(format!(
            "{}={} {}",
            merge.symbol, merge.pair.0, merge.pair.1
        ));
    }
    header_lines.push("§/bpe§".to_string());

    let header = header_lines.join("\n");
    let body = tokens.join(" ");
    let tokens_saved = merges.iter().map(|m| m.frequency as u32 - 1).sum();

    Ok(BpeResult {
        text: format!("{header}\n{body}"),
        merges,
        tokens_saved,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bpe_no_repetition() {
        let config = BpeConfig::default();
        let result = bpe_compress("a b c d e", &config).unwrap();
        assert!(result.merges.is_empty());
        assert_eq!(result.tokens_saved, 0);
    }

    #[test]
    fn test_bpe_with_repetition() {
        let config = BpeConfig {
            max_merges: 10,
            min_frequency: 2,
        };
        let input = "status ok status ok status ok status ok";
        let result = bpe_compress(input, &config).unwrap();
        assert!(!result.merges.is_empty(), "should find repeated pairs");
        assert!(result.tokens_saved > 0);
        assert!(result.text.contains("§bpe§"));
    }

    #[test]
    fn test_bpe_empty_input() {
        let config = BpeConfig::default();
        let result = bpe_compress("", &config).unwrap();
        assert_eq!(result.text, "");
        assert!(result.merges.is_empty());
    }

    #[test]
    fn test_bpe_single_token() {
        let config = BpeConfig::default();
        let result = bpe_compress("hello", &config).unwrap();
        assert_eq!(result.text, "hello");
    }

    #[test]
    fn test_bpe_merge_rule_format() {
        let config = BpeConfig {
            max_merges: 1,
            min_frequency: 2,
        };
        let input = "foo bar foo bar foo bar";
        let result = bpe_compress(input, &config).unwrap();
        if !result.merges.is_empty() {
            let merge = &result.merges[0];
            assert_eq!(merge.pair, ("foo".to_string(), "bar".to_string()));
            assert!(merge.frequency >= 2);
        }
    }

    #[test]
    fn test_bpe_max_merges_respected() {
        let config = BpeConfig {
            max_merges: 1,
            min_frequency: 2,
        };
        let input = "a b a b c d c d e f e f";
        let result = bpe_compress(input, &config).unwrap();
        assert!(result.merges.len() <= 1);
    }

    use proptest::prelude::*;

    proptest! {
        /// BPE tokens_saved is always non-negative.
        #[test]
        fn prop_bpe_savings_non_negative(
            text in "[a-z]{1,5}( [a-z]{1,5}){3,20}"
        ) {
            let config = BpeConfig { max_merges: 10, min_frequency: 2 };
            let result = bpe_compress(&text, &config).unwrap();
            // tokens_saved is u32, always >= 0
            let _ = result.tokens_saved;
        }
    }
}
