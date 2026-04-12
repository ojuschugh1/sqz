//! Entropy Analyzer — classifies logical blocks by Shannon entropy.
//!
//! Builds on top of [`file_reader::compute_entropy`] and
//! [`file_reader::analyze_block_entropies`] to provide higher-level
//! classification (HighInfo / MediumInfo / LowInfo) with configurable
//! percentile thresholds.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::file_reader::{analyze_block_entropies, BlockEntropy};

// ── InfoLevel ─────────────────────────────────────────────────────────────

/// Classification of a block's information density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InfoLevel {
    /// Above the high-percentile threshold — retain.
    HighInfo,
    /// Between low and high thresholds — compress.
    MediumInfo,
    /// Below the low-percentile threshold — discard (boilerplate).
    LowInfo,
}

// ── AnalyzedBlock ─────────────────────────────────────────────────────────

/// A logical block with its entropy score and classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedBlock {
    /// The text content of the block.
    pub text: String,
    /// Shannon entropy in bits per character.
    pub entropy: f64,
    /// Classification based on percentile thresholds.
    pub info_level: InfoLevel,
    /// Line range (start inclusive, end exclusive).
    pub line_range: Range<usize>,
}

// ── EntropyAnalyzer ───────────────────────────────────────────────────────

/// Configurable entropy analyzer that classifies blocks into three tiers.
///
/// - Blocks at or above `high_percentile` → `HighInfo`
/// - Blocks at or above `low_percentile` but below `high_percentile` → `MediumInfo`
/// - Blocks below `low_percentile` → `LowInfo`
pub struct EntropyAnalyzer {
    /// Percentile threshold for HighInfo (default 60.0).
    high_percentile: f64,
    /// Percentile threshold separating LowInfo from MediumInfo (default 25.0).
    low_percentile: f64,
}

impl Default for EntropyAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl EntropyAnalyzer {
    /// Create an analyzer with default thresholds (high=60, low=25).
    pub fn new() -> Self {
        Self {
            high_percentile: 60.0,
            low_percentile: 25.0,
        }
    }

    /// Create an analyzer with custom percentile thresholds.
    ///
    /// `high_percentile` — blocks at or above this are `HighInfo`.
    /// `low_percentile`  — blocks below this are `LowInfo`.
    pub fn with_thresholds(high_percentile: f64, low_percentile: f64) -> Self {
        Self {
            high_percentile,
            low_percentile,
        }
    }

    /// Analyze source text and return classified blocks.
    pub fn analyze(&self, source: &str) -> Vec<AnalyzedBlock> {
        let blocks = analyze_block_entropies(source);
        if blocks.is_empty() {
            return Vec::new();
        }

        let (high_thresh, low_thresh) = Self::compute_thresholds(&blocks, self.high_percentile, self.low_percentile);

        blocks
            .into_iter()
            .map(|b| {
                let info_level = if b.entropy >= high_thresh {
                    InfoLevel::HighInfo
                } else if b.entropy >= low_thresh {
                    InfoLevel::MediumInfo
                } else {
                    InfoLevel::LowInfo
                };
                AnalyzedBlock {
                    text: b.text,
                    entropy: b.entropy,
                    info_level,
                    line_range: b.start_line..b.end_line,
                }
            })
            .collect()
    }

    /// Return only the high-info blocks from the analysis.
    pub fn high_info_blocks(&self, source: &str) -> Vec<AnalyzedBlock> {
        self.analyze(source)
            .into_iter()
            .filter(|b| b.info_level == InfoLevel::HighInfo)
            .collect()
    }

    /// Get the configured high percentile.
    pub fn high_percentile(&self) -> f64 {
        self.high_percentile
    }

    /// Get the configured low percentile.
    pub fn low_percentile(&self) -> f64 {
        self.low_percentile
    }

    // ── internal ──────────────────────────────────────────────────────────

    /// Compute the entropy value at a given percentile from a set of blocks.
    fn percentile_value(blocks: &[BlockEntropy], pct: f64) -> f64 {
        let mut entropies: Vec<f64> = blocks.iter().map(|b| b.entropy).collect();
        entropies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((pct / 100.0) * (entropies.len() as f64 - 1.0)).round() as usize;
        entropies[idx.min(entropies.len() - 1)]
    }

    fn compute_thresholds(blocks: &[BlockEntropy], high_pct: f64, low_pct: f64) -> (f64, f64) {
        let high = Self::percentile_value(blocks, high_pct);
        let low = Self::percentile_value(blocks, low_pct);
        (high, low)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_source() -> &'static str {
        r#"use std::collections::HashMap;
use std::path::Path;

/// A configuration struct.
pub struct Config {
    pub name: String,
    pub value: i32,
}

impl Config {
    pub fn new(name: &str, value: i32) -> Self {
        Self {
            name: name.to_string(),
            value,
        }
    }

    pub fn validate(&self) -> bool {
        !self.name.is_empty() && self.value > 0
    }
}

pub fn process(config: &Config) -> String {
    let mut result = String::new();
    for i in 0..config.value {
        result.push_str(&format!("item {}: {}\n", i, config.name));
    }
    result
}

// boilerplate
// boilerplate
// boilerplate
"#
    }

    #[test]
    fn test_analyze_returns_blocks() {
        let analyzer = EntropyAnalyzer::new();
        let blocks = analyzer.analyze(sample_source());
        assert!(!blocks.is_empty(), "should produce at least one block");
    }

    #[test]
    fn test_all_blocks_have_valid_info_level() {
        let analyzer = EntropyAnalyzer::new();
        let blocks = analyzer.analyze(sample_source());
        for block in &blocks {
            assert!(
                block.info_level == InfoLevel::HighInfo
                    || block.info_level == InfoLevel::MediumInfo
                    || block.info_level == InfoLevel::LowInfo
            );
        }
    }

    #[test]
    fn test_high_info_blocks_subset() {
        let analyzer = EntropyAnalyzer::new();
        let all = analyzer.analyze(sample_source());
        let high = analyzer.high_info_blocks(sample_source());
        assert!(high.len() <= all.len());
        for h in &high {
            assert_eq!(h.info_level, InfoLevel::HighInfo);
        }
    }

    #[test]
    fn test_empty_input() {
        let analyzer = EntropyAnalyzer::new();
        let blocks = analyzer.analyze("");
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_single_block() {
        let analyzer = EntropyAnalyzer::new();
        let blocks = analyzer.analyze("fn main() { println!(\"hello\"); }");
        assert_eq!(blocks.len(), 1);
        // Single block is always at the 100th percentile of itself → HighInfo
        assert_eq!(blocks[0].info_level, InfoLevel::HighInfo);
    }

    #[test]
    fn test_custom_thresholds() {
        let analyzer = EntropyAnalyzer::with_thresholds(90.0, 10.0);
        assert!((analyzer.high_percentile() - 90.0).abs() < f64::EPSILON);
        assert!((analyzer.low_percentile() - 10.0).abs() < f64::EPSILON);
        let blocks = analyzer.analyze(sample_source());
        assert!(!blocks.is_empty());
    }

    #[test]
    fn test_line_ranges_are_valid() {
        let analyzer = EntropyAnalyzer::new();
        let blocks = analyzer.analyze(sample_source());
        for block in &blocks {
            assert!(
                block.line_range.start < block.line_range.end,
                "line range should be non-empty: {:?}",
                block.line_range
            );
        }
    }

    #[test]
    fn test_entropy_is_non_negative() {
        let analyzer = EntropyAnalyzer::new();
        let blocks = analyzer.analyze(sample_source());
        for block in &blocks {
            assert!(block.entropy >= 0.0, "entropy should be non-negative");
        }
    }

    // ── Property-based tests ──────────────────────────────────────────

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        // Feature: sqz, Property 38: Entropy analysis preserves high-information content
        // **Validates: Requirements 33.1, 33.2**

        /// Generate a high-entropy code block (function/class with varied content).
        fn high_entropy_block() -> impl Strategy<Value = String> {
            prop::collection::vec(
                prop::sample::select(vec![
                    "fn process(x: i32) -> Result<String, Error> {",
                    "    let mut map = HashMap::new();",
                    "    for (key, val) in items.iter().enumerate() {",
                    "        if val > threshold { map.insert(key, val * 2); }",
                    "    }",
                    "    match result { Ok(v) => v, Err(e) => return Err(e) }",
                    "}",
                    "pub struct Config { name: String, value: i32, enabled: bool }",
                    "impl Config { pub fn validate(&self) -> bool { !self.name.is_empty() } }",
                    "type ResultMap = HashMap<String, Vec<(usize, f64)>>;",
                ]),
                3..8,
            )
            .prop_map(|lines| lines.join("\n"))
        }

        /// Generate a low-entropy block (repetitive boilerplate).
        fn low_entropy_block() -> impl Strategy<Value = String> {
            prop::collection::vec(
                prop::sample::select(vec![
                    "// comment",
                    "// comment",
                    "// comment",
                    "// ------",
                    "// ------",
                ]),
                3..6,
            )
            .prop_map(|lines| lines.join("\n"))
        }

        /// Generate source text with multiple blocks separated by blank lines,
        /// mixing high-entropy and low-entropy content.
        fn multi_block_source() -> impl Strategy<Value = String> {
            (
                prop::collection::vec(high_entropy_block(), 1..4),
                prop::collection::vec(low_entropy_block(), 1..4),
            )
                .prop_map(|(high_blocks, low_blocks)| {
                    let mut all = Vec::new();
                    for b in &high_blocks {
                        all.push(b.as_str());
                    }
                    for b in &low_blocks {
                        all.push(b.as_str());
                    }
                    all.join("\n\n")
                })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(100))]

            /// Property: The highest-entropy block in any multi-block input
            /// is always classified as HighInfo.
            #[test]
            fn highest_entropy_block_is_high_info(source in multi_block_source()) {
                let analyzer = EntropyAnalyzer::new();
                let blocks = analyzer.analyze(&source);

                // Only check inputs that produce multiple blocks
                if blocks.len() >= 2 {
                    let max_entropy = blocks
                        .iter()
                        .map(|b| b.entropy)
                        .fold(f64::NEG_INFINITY, f64::max);

                    let highest_block = blocks
                        .iter()
                        .find(|b| (b.entropy - max_entropy).abs() < f64::EPSILON)
                        .unwrap();

                    prop_assert_eq!(
                        highest_block.info_level,
                        InfoLevel::HighInfo,
                        "Highest-entropy block (entropy={:.4}) should be HighInfo, got {:?}",
                        highest_block.entropy,
                        highest_block.info_level
                    );
                }
            }

            /// Property: For any source with multiple blocks, high-entropy
            /// blocks are never classified as LowInfo.
            /// A block is considered "high-entropy" if its entropy is above
            /// the median entropy of all blocks.
            #[test]
            fn high_entropy_blocks_never_low_info(source in multi_block_source()) {
                let analyzer = EntropyAnalyzer::new();
                let blocks = analyzer.analyze(&source);

                if blocks.len() >= 2 {
                    // Compute median entropy
                    let mut entropies: Vec<f64> = blocks.iter().map(|b| b.entropy).collect();
                    entropies.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let median = entropies[entropies.len() / 2];

                    for block in &blocks {
                        if block.entropy >= median {
                            prop_assert_ne!(
                                block.info_level,
                                InfoLevel::LowInfo,
                                "Block with entropy {:.4} (>= median {:.4}) should not be LowInfo",
                                block.entropy,
                                median
                            );
                        }
                    }
                }
            }
        }
    }
}
