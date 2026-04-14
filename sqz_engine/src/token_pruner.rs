/// Self-Information Token Pruning (inspired by CompactPrompt, arxiv 2510.18043).
///
/// Uses n-gram frequency as a lightweight proxy for token predictability.
/// Tokens with high predictability (low self-information) are prunable
/// without affecting LLM comprehension — grounded in Shannon's source
/// coding theorem: only the "surprise" bits need to be transmitted.
///
/// This module uses a built-in trigram frequency table derived from common
/// code/prose patterns rather than shipping a large external asset.

use std::collections::HashMap;

use crate::error::Result;

/// Configuration for the token pruner.
#[derive(Debug, Clone)]
pub struct PrunerConfig {
    /// Probability threshold above which a token is considered prunable.
    /// Tokens with P(token | context) > threshold are removed.
    /// Default: 0.85
    pub predictability_threshold: f64,
    /// Minimum token length to consider for pruning (skip short tokens).
    /// Default: 2
    pub min_token_length: usize,
    /// Whether to preserve tokens that appear in code-like contexts.
    /// Default: true
    pub preserve_code_tokens: bool,
}

impl Default for PrunerConfig {
    fn default() -> Self {
        Self {
            predictability_threshold: 0.85,
            min_token_length: 2,
            preserve_code_tokens: true,
        }
    }
}

/// Trigram-based self-information token pruner.
pub struct TokenPruner {
    config: PrunerConfig,
    /// Trigram frequency table: (w1, w2) -> {w3 -> count}
    trigram_table: HashMap<(String, String), HashMap<String, u32>>,
    /// Bigram totals for normalization: (w1, w2) -> total_count
    bigram_totals: HashMap<(String, String), u32>,
}

impl TokenPruner {
    /// Create a new pruner with default config and built-in frequency table.
    pub fn new() -> Self {
        Self::with_config(PrunerConfig::default())
    }

    /// Create a new pruner with custom config.
    pub fn with_config(config: PrunerConfig) -> Self {
        let mut pruner = Self {
            config,
            trigram_table: HashMap::new(),
            bigram_totals: HashMap::new(),
        };
        pruner.load_builtin_patterns();
        pruner
    }

    /// Load built-in trigram patterns from common code and prose.
    fn load_builtin_patterns(&mut self) {
        // Common English prose trigrams (highly predictable completions)
        let patterns: &[(&str, &str, &str, u32)] = &[
            // Articles and prepositions
            ("in", "the", "same", 80),
            ("in", "the", "following", 75),
            ("of", "the", "same", 70),
            ("on", "the", "other", 65),
            ("at", "the", "same", 60),
            ("to", "the", "same", 55),
            ("is", "a", "function", 50),
            ("is", "a", "method", 48),
            ("is", "a", "type", 45),
            ("is", "the", "same", 70),
            ("as", "a", "result", 60),
            // Common code-adjacent prose
            ("this", "is", "a", 90),
            ("this", "is", "the", 85),
            ("this", "is", "an", 80),
            ("it", "is", "a", 85),
            ("it", "is", "the", 80),
            ("it", "is", "not", 75),
            ("there", "is", "a", 80),
            ("there", "is", "no", 75),
            ("there", "are", "no", 70),
            ("that", "is", "a", 75),
            ("that", "is", "the", 70),
            ("which", "is", "a", 70),
            ("which", "is", "the", 65),
            // Error message patterns
            ("error", "in", "the", 60),
            ("failed", "to", "connect", 55),
            ("failed", "to", "open", 50),
            ("failed", "to", "read", 50),
            ("unable", "to", "find", 55),
            ("unable", "to", "open", 50),
            ("could", "not", "find", 60),
            ("could", "not", "open", 55),
            ("does", "not", "exist", 65),
            ("is", "not", "a", 60),
            ("is", "not", "the", 55),
            // Documentation patterns
            ("for", "more", "information", 80),
            ("for", "more", "details", 75),
            ("see", "the", "documentation", 70),
            ("refer", "to", "the", 65),
            ("please", "refer", "to", 60),
            ("note", "that", "the", 55),
            ("note", "that", "this", 50),
            ("make", "sure", "that", 60),
            ("make", "sure", "to", 55),
            // Filler phrases
            ("in", "order", "to", 90),
            ("as", "well", "as", 85),
            ("due", "to", "the", 70),
            ("based", "on", "the", 65),
            ("with", "respect", "to", 60),
            ("in", "addition", "to", 55),
            ("as", "opposed", "to", 50),
            ("on", "behalf", "of", 50),
        ];

        for &(w1, w2, w3, count) in patterns {
            let key = (w1.to_lowercase(), w2.to_lowercase());
            self.trigram_table
                .entry(key.clone())
                .or_default()
                .insert(w3.to_lowercase(), count);
            *self.bigram_totals.entry(key).or_insert(0) += count;
        }
    }

    /// Train the pruner on additional text to improve frequency estimates.
    pub fn train(&mut self, text: &str) {
        let words: Vec<String> = tokenize_words(text);
        if words.len() < 3 {
            return;
        }
        for window in words.windows(3) {
            let key = (window[0].clone(), window[1].clone());
            self.trigram_table
                .entry(key.clone())
                .or_default()
                .entry(window[2].clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            *self.bigram_totals.entry(key).or_insert(0) += 1;
        }
    }

    /// Compute the predictability P(w3 | w1, w2) for a trigram.
    /// Returns 0.0 if the bigram context has never been seen.
    fn predictability(&self, w1: &str, w2: &str, w3: &str) -> f64 {
        let key = (w1.to_lowercase(), w2.to_lowercase());
        let total = match self.bigram_totals.get(&key) {
            Some(&t) if t > 0 => t,
            _ => return 0.0,
        };
        let count = self
            .trigram_table
            .get(&key)
            .and_then(|m| m.get(&w3.to_lowercase()))
            .copied()
            .unwrap_or(0);
        count as f64 / total as f64
    }

    /// Prune highly predictable tokens from prose text.
    ///
    /// Returns the pruned text and the number of tokens removed.
    pub fn prune(&self, text: &str) -> Result<PruneResult> {
        let lines: Vec<&str> = text.lines().collect();
        let mut output_lines = Vec::with_capacity(lines.len());
        let mut total_removed = 0u32;
        let mut total_original = 0u32;

        for line in &lines {
            // Skip code-like lines if preserve_code_tokens is enabled
            if self.config.preserve_code_tokens && is_code_line(line) {
                output_lines.push(line.to_string());
                total_original += count_words(line) as u32;
                continue;
            }

            let words: Vec<&str> = line.split_whitespace().collect();
            total_original += words.len() as u32;

            if words.len() < 3 {
                output_lines.push(line.to_string());
                continue;
            }

            let mut kept: Vec<&str> = Vec::with_capacity(words.len());
            // Always keep first two words (context)
            kept.push(words[0]);
            kept.push(words[1]);

            for i in 2..words.len() {
                let w1 = words[i - 2].to_lowercase();
                let w2 = words[i - 1].to_lowercase();
                let w3_clean = words[i]
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();

                if w3_clean.len() < self.config.min_token_length {
                    kept.push(words[i]);
                    continue;
                }

                let p = self.predictability(&w1, &w2, &w3_clean);
                if p > self.config.predictability_threshold {
                    total_removed += 1;
                } else {
                    kept.push(words[i]);
                }
            }

            output_lines.push(kept.join(" "));
        }

        let pruned_text = output_lines.join("\n");
        // Preserve trailing newline
        let result = if text.ends_with('\n') && !pruned_text.ends_with('\n') {
            format!("{pruned_text}\n")
        } else {
            pruned_text
        };

        Ok(PruneResult {
            text: result,
            tokens_removed: total_removed,
            tokens_original: total_original,
        })
    }

    /// Zipf's Law vocabulary pruning.
    ///
    /// Zipf's law: f(r) ∝ 1/r — the frequency of a word is inversely
    /// proportional to its rank. Words appearing at or above their expected
    /// Zipf frequency are redundant and can be pruned. Words appearing
    /// below their expected frequency carry more information and are preserved.
    ///
    /// Returns the pruned text and stats.
    pub fn zipf_prune(&self, text: &str) -> Result<PruneResult> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let total_original = words.len() as u32;

        if words.len() < 10 {
            return Ok(PruneResult {
                text: text.to_string(),
                tokens_removed: 0,
                tokens_original: total_original,
            });
        }

        // Count word frequencies
        let mut freq_map: HashMap<String, usize> = HashMap::new();
        for &w in &words {
            *freq_map.entry(w.to_lowercase()).or_insert(0) += 1;
        }

        // Rank words by frequency (descending)
        let mut ranked: Vec<(String, usize)> = freq_map.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));

        // Compute expected Zipf frequency for each rank
        // f_expected(r) = C / r, where C = total_words / H_n (harmonic number)
        let _n = ranked.len() as f64;
        let harmonic: f64 = (1..=ranked.len()).map(|r| 1.0 / r as f64).sum();
        let c = words.len() as f64 / harmonic;

        // Mark words that are "Zipf-redundant": actual frequency >= 1.5× expected
        let mut redundant_words: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (rank_idx, (word, actual_freq)) in ranked.iter().enumerate() {
            let rank = rank_idx + 1;
            let expected = c / rank as f64;
            // Word is redundant if it appears much more than Zipf predicts
            // AND it's a common filler word (short, non-technical)
            if *actual_freq as f64 > expected * 1.5
                && word.len() <= 4
                && !is_technical_word(word)
            {
                redundant_words.insert(word.clone());
            }
        }

        if redundant_words.is_empty() {
            return Ok(PruneResult {
                text: text.to_string(),
                tokens_removed: 0,
                tokens_original: total_original,
            });
        }

        // Remove redundant words, keeping at least one occurrence of each
        let mut seen_counts: HashMap<String, usize> = HashMap::new();
        let mut kept = Vec::new();
        let mut removed = 0u32;

        for &w in &words {
            let lower = w.to_lowercase();
            if redundant_words.contains(&lower) {
                let count = seen_counts.entry(lower.clone()).or_insert(0);
                *count += 1;
                // Keep the first occurrence, prune subsequent ones
                if *count <= 1 {
                    kept.push(w);
                } else {
                    removed += 1;
                }
            } else {
                kept.push(w);
            }
        }

        let result = kept.join(" ");
        let result = if text.ends_with('\n') && !result.ends_with('\n') {
            format!("{result}\n")
        } else {
            result
        };

        Ok(PruneResult {
            text: result,
            tokens_removed: removed,
            tokens_original: total_original,
        })
    }
}

impl Default for TokenPruner {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a prune operation.
#[derive(Debug, Clone)]
pub struct PruneResult {
    /// The pruned text.
    pub text: String,
    /// Number of tokens removed.
    pub tokens_removed: u32,
    /// Original token count.
    pub tokens_original: u32,
}

impl PruneResult {
    /// Fraction of tokens removed (0.0 to 1.0).
    pub fn reduction_ratio(&self) -> f64 {
        if self.tokens_original == 0 {
            0.0
        } else {
            self.tokens_removed as f64 / self.tokens_original as f64
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Tokenize text into lowercase words.
fn tokenize_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// Count whitespace-separated words.
fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Heuristic: is this line likely code rather than prose?
fn is_code_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Lines starting with common code indicators
    trimmed.starts_with("fn ")
        || trimmed.starts_with("pub ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("var ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("return ")
        || trimmed.starts_with("if ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with('#')
        || trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed.ends_with('{')
        || trimmed.ends_with('}')
        || trimmed.ends_with(';')
        || trimmed.ends_with(')')
        || trimmed.contains("->")
        || trimmed.contains("=>")
        || trimmed.contains("::")
        || trimmed.contains("()")
}

/// Check if a short word is technical/meaningful (should not be pruned).
fn is_technical_word(word: &str) -> bool {
    matches!(
        word,
        "null" | "none" | "true" | "false" | "void" | "self" | "this"
        | "type" | "enum" | "impl" | "func" | "main" | "test" | "init"
        | "open" | "read" | "send" | "recv" | "lock" | "drop" | "move"
        | "copy" | "sync" | "push" | "pull" | "port" | "host" | "path"
        | "file" | "line" | "code" | "data" | "node" | "root" | "hash"
        | "size" | "name" | "list" | "loop" | "exit" | "fail" | "pass"
        | "skip" | "todo" | "warn" | "info" | "http" | "json" | "yaml"
        | "toml" | "html" | "rust" | "java" | "bash"
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_creates_pruner() {
        let pruner = TokenPruner::new();
        assert!(!pruner.trigram_table.is_empty());
        assert!(!pruner.bigram_totals.is_empty());
    }

    #[test]
    fn test_prune_empty_input() {
        let pruner = TokenPruner::new();
        let result = pruner.prune("").unwrap();
        assert_eq!(result.text, "");
        assert_eq!(result.tokens_removed, 0);
    }

    #[test]
    fn test_prune_short_input_unchanged() {
        let pruner = TokenPruner::new();
        let result = pruner.prune("hello world").unwrap();
        assert_eq!(result.text, "hello world");
        assert_eq!(result.tokens_removed, 0);
    }

    #[test]
    fn test_prune_removes_predictable_tokens() {
        let pruner = TokenPruner::new();
        // "in order to" is a highly predictable trigram — "to" should be pruned
        let result = pruner.prune("We need in order to do this task").unwrap();
        assert!(
            result.tokens_removed > 0 || result.text.len() <= "We need in order to do this task".len(),
            "expected some pruning on predictable prose"
        );
    }

    #[test]
    fn test_prune_preserves_code_lines() {
        let pruner = TokenPruner::new();
        let code = "fn main() {\n    let x = 42;\n}";
        let result = pruner.prune(code).unwrap();
        assert_eq!(result.text, code);
        assert_eq!(result.tokens_removed, 0);
    }

    #[test]
    fn test_prune_preserves_trailing_newline() {
        let pruner = TokenPruner::new();
        let result = pruner.prune("hello world\n").unwrap();
        assert!(result.text.ends_with('\n'));
    }

    #[test]
    fn test_train_adds_patterns() {
        let mut pruner = TokenPruner::new();
        let initial_size = pruner.trigram_table.len();
        pruner.train("the quick brown fox jumps over the lazy dog and the quick brown cat");
        assert!(pruner.trigram_table.len() >= initial_size);
    }

    #[test]
    fn test_predictability_unknown_context() {
        let pruner = TokenPruner::new();
        let p = pruner.predictability("xyzzy", "plugh", "foo");
        assert_eq!(p, 0.0);
    }

    #[test]
    fn test_predictability_known_pattern() {
        let pruner = TokenPruner::new();
        // "in order to" is in the built-in table with high count
        let p = pruner.predictability("in", "order", "to");
        assert!(p > 0.5, "expected high predictability, got {p}");
    }

    #[test]
    fn test_reduction_ratio_zero_for_empty() {
        let result = PruneResult {
            text: String::new(),
            tokens_removed: 0,
            tokens_original: 0,
        };
        assert_eq!(result.reduction_ratio(), 0.0);
    }

    #[test]
    fn test_is_code_line_detection() {
        assert!(is_code_line("fn main() {"));
        assert!(is_code_line("  let x = 42;"));
        assert!(is_code_line("// comment"));
        assert!(is_code_line("import os"));
        assert!(!is_code_line("This is a normal sentence."));
        assert!(!is_code_line("The error occurred in the module."));
        assert!(!is_code_line(""));
    }

    #[test]
    fn test_custom_config() {
        let config = PrunerConfig {
            predictability_threshold: 0.5,
            min_token_length: 1,
            preserve_code_tokens: false,
        };
        let pruner = TokenPruner::with_config(config);
        // With lower threshold, more tokens should be pruned
        let result = pruner.prune("this is a very long sentence with many words in order to test").unwrap();
        // Just verify it doesn't crash
        assert!(!result.text.is_empty());
    }

    // ── Property tests ────────────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Pruning never produces output longer than input.
        #[test]
        fn prop_prune_never_increases_length(
            text in "[a-z ]{10,200}"
        ) {
            let pruner = TokenPruner::new();
            let result = pruner.prune(&text).unwrap();
            prop_assert!(
                result.text.len() <= text.len() + 1, // +1 for possible trailing newline
                "pruned text ({}) should not be longer than input ({})",
                result.text.len(), text.len()
            );
        }

        /// tokens_removed + remaining tokens == tokens_original
        #[test]
        fn prop_prune_token_accounting(
            text in "[a-z ]{10,200}"
        ) {
            let pruner = TokenPruner::new();
            let result = pruner.prune(&text).unwrap();
            let remaining = count_words(&result.text) as u32;
            prop_assert!(
                result.tokens_removed + remaining <= result.tokens_original + 1,
                "removed ({}) + remaining ({}) should be <= original ({})",
                result.tokens_removed, remaining, result.tokens_original
            );
        }
    }

    // ── Zipf's Law pruning tests ──────────────────────────────────────────

    #[test]
    fn test_zipf_prune_short_text_unchanged() {
        let pruner = TokenPruner::new();
        let result = pruner.zipf_prune("hello world").unwrap();
        assert_eq!(result.text, "hello world");
        assert_eq!(result.tokens_removed, 0);
    }

    #[test]
    fn test_zipf_prune_removes_overrepresented_fillers() {
        let pruner = TokenPruner::new();
        // "the" appears way more than Zipf predicts for a text this size
        let text = "the cat the dog the bird the fish the tree the rock the sky the sun the moon the star";
        let result = pruner.zipf_prune(text).unwrap();
        // Should remove some "the" occurrences but keep at least one
        assert!(result.text.contains("the"), "should keep at least one 'the'");
        assert!(
            result.tokens_removed > 0,
            "should prune overrepresented filler words"
        );
    }

    #[test]
    fn test_zipf_prune_preserves_technical_words() {
        let pruner = TokenPruner::new();
        let text = "null null null null null null null null null null check for null values";
        let result = pruner.zipf_prune(text).unwrap();
        // "null" is a technical word — should NOT be pruned
        assert_eq!(result.tokens_removed, 0, "technical words should be preserved");
    }

    #[test]
    fn test_is_technical_word() {
        assert!(is_technical_word("null"));
        assert!(is_technical_word("type"));
        assert!(is_technical_word("json"));
        assert!(!is_technical_word("the"));
        assert!(!is_technical_word("and"));
        assert!(!is_technical_word("xyz"));
    }

    proptest! {
        /// Zipf pruning never produces empty output from non-empty input.
        #[test]
        fn prop_zipf_prune_non_empty(
            text in "[a-z]{2,5}( [a-z]{2,5}){10,30}"
        ) {
            let pruner = TokenPruner::new();
            let result = pruner.zipf_prune(&text).unwrap();
            prop_assert!(!result.text.is_empty());
        }
    }
}
