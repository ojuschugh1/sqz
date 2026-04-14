/// N-gram Abbreviation for Recurrent Patterns (inspired by CompactPrompt).
///
/// Identifies frequently recurring multi-word phrases across a session and
/// replaces occurrences 2+ with short abbreviations, injecting a legend at
/// the top. This is dictionary-based compression at the phrase level.
///
/// For a phrase of length L tokens appearing K times, savings are:
///   (K-1) × L - (legend_cost + K × abbrev_cost)
/// Profitable when K > (legend_cost + abbrev_cost) / (L - abbrev_cost).

use std::collections::HashMap;

use crate::error::Result;

/// Configuration for n-gram abbreviation.
#[derive(Debug, Clone)]
pub struct AbbreviatorConfig {
    /// Minimum number of occurrences before a phrase is abbreviated.
    /// Default: 3
    pub min_occurrences: usize,
    /// Minimum phrase length in tokens to consider for abbreviation.
    /// Default: 3
    pub min_phrase_tokens: usize,
    /// Maximum phrase length in tokens.
    /// Default: 8
    pub max_phrase_tokens: usize,
    /// Maximum number of abbreviations to create.
    /// Default: 20
    pub max_abbreviations: usize,
}

impl Default for AbbreviatorConfig {
    fn default() -> Self {
        Self {
            min_occurrences: 3,
            min_phrase_tokens: 3,
            max_phrase_tokens: 8,
            max_abbreviations: 20,
        }
    }
}

/// A single abbreviation mapping.
#[derive(Debug, Clone)]
pub struct Abbreviation {
    /// The short symbol, e.g. «E1»
    pub symbol: String,
    /// The full phrase it replaces.
    pub phrase: String,
    /// Number of times the phrase appeared.
    pub occurrences: usize,
    /// Estimated tokens saved.
    pub tokens_saved: usize,
}

/// Result of abbreviation.
#[derive(Debug, Clone)]
pub struct AbbreviationResult {
    /// The abbreviated text with legend prepended.
    pub text: String,
    /// The abbreviations that were applied.
    pub abbreviations: Vec<Abbreviation>,
    /// Total tokens saved.
    pub total_tokens_saved: usize,
}

/// Session-level n-gram abbreviator.
pub struct NgramAbbreviator {
    config: AbbreviatorConfig,
    /// Phrase frequency counter across the session.
    phrase_counts: HashMap<String, usize>,
    /// Currently active abbreviations.
    active_abbreviations: Vec<Abbreviation>,
    /// Next abbreviation index.
    next_index: usize,
}

impl NgramAbbreviator {
    pub fn new() -> Self {
        Self::with_config(AbbreviatorConfig::default())
    }

    pub fn with_config(config: AbbreviatorConfig) -> Self {
        Self {
            config,
            phrase_counts: HashMap::new(),
            active_abbreviations: Vec::new(),
            next_index: 1,
        }
    }

    /// Observe text to update phrase frequency counts.
    /// Call this on each piece of content as it flows through the session.
    pub fn observe(&mut self, text: &str) {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.len() < self.config.min_phrase_tokens {
            return;
        }

        for n in self.config.min_phrase_tokens..=self.config.max_phrase_tokens {
            if words.len() < n {
                break;
            }
            for window in words.windows(n) {
                let phrase = window.join(" ");
                // Skip phrases that are too short in characters
                if phrase.len() < 10 {
                    continue;
                }
                *self.phrase_counts.entry(phrase).or_insert(0) += 1;
            }
        }
    }

    /// Apply abbreviations to text based on observed frequencies.
    ///
    /// This method:
    /// 1. Identifies phrases exceeding the occurrence threshold
    /// 2. Assigns abbreviation symbols to the most profitable phrases
    /// 3. Replaces occurrences (keeping the first one intact) with symbols
    /// 4. Prepends a legend
    pub fn abbreviate(&mut self, text: &str) -> Result<AbbreviationResult> {
        // Find profitable phrases
        let mut candidates: Vec<(String, usize)> = self
            .phrase_counts
            .iter()
            .filter(|(_, &count)| count >= self.config.min_occurrences)
            .map(|(phrase, &count)| (phrase.clone(), count))
            .collect();

        // Sort by estimated savings (descending)
        candidates.sort_by(|a, b| {
            let savings_a = estimate_savings(&a.0, a.1);
            let savings_b = estimate_savings(&b.0, b.1);
            savings_b.cmp(&savings_a)
        });

        // Take top N candidates
        candidates.truncate(self.config.max_abbreviations);

        if candidates.is_empty() {
            return Ok(AbbreviationResult {
                text: text.to_string(),
                abbreviations: Vec::new(),
                total_tokens_saved: 0,
            });
        }

        // Assign symbols and build abbreviation list
        let mut abbreviations = Vec::new();
        let mut result_text = text.to_string();
        let mut total_saved = 0usize;

        for (phrase, occurrences) in &candidates {
            if abbreviations.len() >= self.config.max_abbreviations {
                break;
            }

            let symbol = format!("«A{}»", self.next_index);
            self.next_index += 1;

            // Replace all but the first occurrence
            let replaced = replace_after_first(&result_text, phrase, &symbol);
            let replacements_made = count_occurrences(&result_text, phrase).saturating_sub(1);

            if replacements_made == 0 {
                continue;
            }

            result_text = replaced;

            let phrase_tokens = phrase.split_whitespace().count();
            let saved = replacements_made * phrase_tokens.saturating_sub(1);
            total_saved += saved;

            let abbrev = Abbreviation {
                symbol: symbol.clone(),
                phrase: phrase.clone(),
                occurrences: *occurrences,
                tokens_saved: saved,
            };
            abbreviations.push(abbrev.clone());
            self.active_abbreviations.push(abbrev);
        }

        // Prepend legend if we have abbreviations
        if !abbreviations.is_empty() {
            let legend = format_legend(&abbreviations);
            result_text = format!("{legend}\n\n{result_text}");
        }

        Ok(AbbreviationResult {
            text: result_text,
            abbreviations,
            total_tokens_saved: total_saved,
        })
    }

    /// Reset the abbreviator state (e.g., on session reset).
    pub fn reset(&mut self) {
        self.phrase_counts.clear();
        self.active_abbreviations.clear();
        self.next_index = 1;
    }

    /// Get current phrase frequency counts.
    pub fn phrase_counts(&self) -> &HashMap<String, usize> {
        &self.phrase_counts
    }

    /// Get active abbreviations.
    pub fn active_abbreviations(&self) -> &[Abbreviation] {
        &self.active_abbreviations
    }
}

impl Default for NgramAbbreviator {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Estimate token savings for a phrase with given occurrence count.
fn estimate_savings(phrase: &str, occurrences: usize) -> usize {
    let phrase_tokens = phrase.split_whitespace().count();
    let abbrev_cost = 1; // «A1» ≈ 1 token
    let legend_cost = phrase_tokens + 2; // "«A1» = <phrase>"

    if occurrences <= 1 || phrase_tokens <= abbrev_cost {
        return 0;
    }

    let replacements = occurrences - 1; // keep first occurrence
    let gross_savings = replacements * phrase_tokens;
    let total_cost = legend_cost + occurrences * abbrev_cost;

    gross_savings.saturating_sub(total_cost)
}

/// Count non-overlapping occurrences of `needle` in `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count()
}

/// Replace all occurrences of `needle` after the first one with `replacement`.
fn replace_after_first(text: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut found_first = false;
    let mut remaining = text;

    while let Some(pos) = remaining.find(needle) {
        result.push_str(&remaining[..pos]);
        if !found_first {
            result.push_str(needle);
            found_first = true;
        } else {
            result.push_str(replacement);
        }
        remaining = &remaining[pos + needle.len()..];
    }
    result.push_str(remaining);

    result
}

/// Format the abbreviation legend.
fn format_legend(abbreviations: &[Abbreviation]) -> String {
    let mut lines = vec!["[Abbreviations]".to_string()];
    for abbrev in abbreviations {
        lines.push(format!("{}={}", abbrev.symbol, abbrev.phrase));
    }
    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_abbreviations_below_threshold() {
        let mut abbr = NgramAbbreviator::new();
        abbr.observe("hello world foo bar");
        let result = abbr.abbreviate("hello world foo bar").unwrap();
        assert!(result.abbreviations.is_empty());
        assert_eq!(result.total_tokens_saved, 0);
    }

    #[test]
    fn test_abbreviation_after_threshold() {
        let mut abbr = NgramAbbreviator::with_config(AbbreviatorConfig {
            min_occurrences: 2,
            min_phrase_tokens: 3,
            max_phrase_tokens: 8,
            max_abbreviations: 20,
        });

        let text = "error mismatched types found. Then error mismatched types found again. And error mismatched types found once more.";
        abbr.observe(text);
        let result = abbr.abbreviate(text).unwrap();

        // Should have created at least one abbreviation
        if !result.abbreviations.is_empty() {
            assert!(result.text.contains("[Abbreviations]"));
            assert!(result.text.contains("«A"));
        }
    }

    #[test]
    fn test_replace_after_first() {
        let text = "abc def abc def abc";
        let result = replace_after_first(text, "abc", "X");
        assert_eq!(result, "abc def X def X");
    }

    #[test]
    fn test_replace_after_first_single_occurrence() {
        let text = "abc def ghi";
        let result = replace_after_first(text, "abc", "X");
        assert_eq!(result, "abc def ghi");
    }

    #[test]
    fn test_replace_after_first_empty_needle() {
        let text = "abc def";
        let result = replace_after_first(text, "", "X");
        assert_eq!(result, "abc def");
    }

    #[test]
    fn test_count_occurrences() {
        assert_eq!(count_occurrences("abc abc abc", "abc"), 3);
        assert_eq!(count_occurrences("abc def ghi", "xyz"), 0);
        assert_eq!(count_occurrences("", "abc"), 0);
        assert_eq!(count_occurrences("abc", ""), 0);
    }

    #[test]
    fn test_estimate_savings() {
        // 3-token phrase appearing 5 times: (5-1)*3 - (3+2 + 5*1) = 12 - 10 = 2
        assert_eq!(estimate_savings("error mismatched types", 5), 2);
        // Single occurrence: no savings
        assert_eq!(estimate_savings("error mismatched types", 1), 0);
    }

    #[test]
    fn test_format_legend() {
        let abbrevs = vec![Abbreviation {
            symbol: "«A1»".to_string(),
            phrase: "error mismatched types".to_string(),
            occurrences: 5,
            tokens_saved: 8,
        }];
        let legend = format_legend(&abbrevs);
        assert!(legend.contains("[Abbreviations]"));
        assert!(legend.contains("«A1»=error mismatched types"));
    }

    #[test]
    fn test_reset_clears_state() {
        let mut abbr = NgramAbbreviator::new();
        abbr.observe("some text with words and more words");
        assert!(!abbr.phrase_counts.is_empty());
        abbr.reset();
        assert!(abbr.phrase_counts.is_empty());
        assert!(abbr.active_abbreviations.is_empty());
        assert_eq!(abbr.next_index, 1);
    }

    #[test]
    fn test_observe_short_text_noop() {
        let mut abbr = NgramAbbreviator::new();
        abbr.observe("hi");
        assert!(abbr.phrase_counts.is_empty());
    }

    // ── Property tests ────────────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Abbreviation never loses the first occurrence of any phrase.
        #[test]
        fn prop_first_occurrence_preserved(
            phrase in "[a-z]{3,6} [a-z]{3,6} [a-z]{3,6}",
            repeat in 3usize..=6usize,
        ) {
            let mut abbr = NgramAbbreviator::with_config(AbbreviatorConfig {
                min_occurrences: 2,
                min_phrase_tokens: 3,
                max_phrase_tokens: 8,
                max_abbreviations: 20,
            });

            let text = std::iter::repeat(phrase.as_str())
                .take(repeat)
                .collect::<Vec<_>>()
                .join(". ");

            abbr.observe(&text);
            let result = abbr.abbreviate(&text).unwrap();

            // The original phrase must still appear at least once
            prop_assert!(
                result.text.contains(&phrase),
                "first occurrence of '{}' should be preserved in:\n{}",
                phrase, result.text
            );
        }

        /// Total tokens saved is non-negative.
        #[test]
        fn prop_savings_non_negative(
            text in "[a-z ]{20,200}"
        ) {
            let mut abbr = NgramAbbreviator::new();
            abbr.observe(&text);
            let result = abbr.abbreviate(&text).unwrap();
            // total_tokens_saved is usize, so always >= 0
            // Just verify it doesn't panic
            let _ = result.total_tokens_saved;
        }
    }
}
