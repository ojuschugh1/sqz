/// Adaptive Entropy-Weighted Truncation (Rate-Distortion Theory).
///
/// Instead of using fixed thresholds for truncate_strings and collapse_arrays,
/// this module computes per-segment entropy and keeps segments above the
/// median entropy while dropping segments below it. This preserves the
/// information-dense parts and drops the redundant tail.
///
/// Based on rate-distortion theory: the optimal cutoff depends on the
/// information density of the content being truncated, not a fixed length.

use crate::error::Result;

/// Configuration for entropy-weighted truncation.
#[derive(Debug, Clone)]
pub struct EntropyTruncConfig {
    /// Minimum entropy (bits) below which a segment is considered low-information.
    /// Segments below this are candidates for removal.
    /// Default: computed dynamically as median of all segments.
    pub min_entropy_override: Option<f64>,
    /// Maximum number of low-entropy segments to keep (even if below threshold).
    /// Ensures we don't drop everything.
    /// Default: 2
    pub min_kept_segments: usize,
    /// Minimum segment length (chars) to analyze. Shorter segments are always kept.
    /// Default: 20
    pub min_segment_length: usize,
}

impl Default for EntropyTruncConfig {
    fn default() -> Self {
        Self {
            min_entropy_override: None,
            min_kept_segments: 2,
            min_segment_length: 20,
        }
    }
}

/// A segment with its computed entropy.
#[derive(Debug, Clone)]
pub struct ScoredSegment {
    /// The segment text.
    pub text: String,
    /// Shannon entropy in bits.
    pub entropy: f64,
    /// Whether this segment was kept in the output.
    pub kept: bool,
}

/// Result of entropy-weighted truncation.
#[derive(Debug, Clone)]
pub struct EntropyTruncResult {
    /// The truncated output.
    pub text: String,
    /// Segments with their scores and keep/drop decisions.
    pub segments: Vec<ScoredSegment>,
    /// Number of segments dropped.
    pub segments_dropped: usize,
    /// Entropy threshold used.
    pub threshold: f64,
}

/// Entropy-weighted truncator.
pub struct EntropyTruncator {
    config: EntropyTruncConfig,
}

impl EntropyTruncator {
    pub fn new() -> Self {
        Self::with_config(EntropyTruncConfig::default())
    }

    pub fn with_config(config: EntropyTruncConfig) -> Self {
        Self { config }
    }

    /// Truncate a string by keeping high-entropy (information-dense) segments
    /// and dropping low-entropy (redundant) segments.
    ///
    /// Segments are split by double-newline (paragraph boundaries) or by
    /// a fixed window if no paragraph breaks exist.
    pub fn truncate_string(&self, text: &str) -> Result<EntropyTruncResult> {
        let segments = split_segments(text);

        if segments.len() <= 1 {
            return Ok(EntropyTruncResult {
                text: text.to_string(),
                segments: segments
                    .into_iter()
                    .map(|s| ScoredSegment {
                        entropy: shannon_entropy(&s),
                        text: s,
                        kept: true,
                    })
                    .collect(),
                segments_dropped: 0,
                threshold: 0.0,
            });
        }

        // Compute entropy for each segment
        let mut scored: Vec<ScoredSegment> = segments
            .into_iter()
            .map(|s| {
                let entropy = shannon_entropy(&s);
                ScoredSegment {
                    text: s,
                    entropy,
                    kept: true,
                }
            })
            .collect();

        // Determine threshold: use override or compute median
        let threshold = self.config.min_entropy_override.unwrap_or_else(|| {
            let mut entropies: Vec<f64> = scored.iter().map(|s| s.entropy).collect();
            entropies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            median(&entropies)
        });

        // Mark segments below threshold for dropping
        let mut kept_count = 0usize;
        let total = scored.len();

        for seg in &mut scored {
            if seg.text.len() < self.config.min_segment_length {
                seg.kept = true;
                kept_count += 1;
            } else if seg.entropy >= threshold {
                seg.kept = true;
                kept_count += 1;
            } else {
                seg.kept = false;
            }
        }

        // Ensure we keep at least min_kept_segments
        if kept_count < self.config.min_kept_segments {
            // Re-add the highest-entropy dropped segments
            let mut dropped_indices: Vec<(usize, f64)> = scored
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.kept)
                .map(|(i, s)| (i, s.entropy))
                .collect();
            dropped_indices.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            for (idx, _) in dropped_indices {
                if kept_count >= self.config.min_kept_segments {
                    break;
                }
                scored[idx].kept = true;
                kept_count += 1;
            }
        }

        let segments_dropped = total - kept_count;

        // Build output
        let kept_texts: Vec<&str> = scored
            .iter()
            .filter(|s| s.kept)
            .map(|s| s.text.as_str())
            .collect();

        let mut text = kept_texts.join("\n\n");
        if segments_dropped > 0 {
            text.push_str(&format!(
                "\n[{segments_dropped} low-information segments omitted]"
            ));
        }

        Ok(EntropyTruncResult {
            text,
            segments: scored,
            segments_dropped,
            threshold,
        })
    }

    /// Truncate a JSON array by keeping high-entropy elements and dropping
    /// low-entropy (redundant) ones.
    pub fn truncate_array(&self, elements: &[serde_json::Value]) -> Result<EntropyTruncArrayResult> {
        if elements.len() <= 2 {
            return Ok(EntropyTruncArrayResult {
                kept: elements.to_vec(),
                dropped_count: 0,
                threshold: 0.0,
            });
        }

        // Serialize each element and compute entropy
        let scored: Vec<(serde_json::Value, f64)> = elements
            .iter()
            .map(|e| {
                let s = serde_json::to_string(e).unwrap_or_default();
                let entropy = shannon_entropy(&s);
                (e.clone(), entropy)
            })
            .collect();

        // Compute median entropy
        let mut entropies: Vec<f64> = scored.iter().map(|(_, e)| *e).collect();
        entropies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let threshold = self.config.min_entropy_override.unwrap_or_else(|| median(&entropies));

        // Keep elements above threshold, plus at least min_kept_segments
        let mut kept = Vec::new();
        let mut dropped = 0usize;

        for (elem, entropy) in &scored {
            if *entropy >= threshold || kept.len() < self.config.min_kept_segments {
                kept.push(elem.clone());
            } else {
                dropped += 1;
            }
        }

        // Always keep at least min_kept_segments
        while kept.len() < self.config.min_kept_segments && dropped > 0 {
            // Add back highest-entropy dropped elements
            let mut remaining: Vec<&(serde_json::Value, f64)> = scored
                .iter()
                .filter(|(e, _)| !kept.contains(e))
                .collect();
            remaining.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((elem, _)) = remaining.first() {
                kept.push((*elem).clone());
                dropped -= 1;
            } else {
                break;
            }
        }

        if dropped > 0 {
            kept.push(serde_json::Value::String(format!(
                "[{dropped} low-information elements omitted]"
            )));
        }

        Ok(EntropyTruncArrayResult {
            kept,
            dropped_count: dropped,
            threshold,
        })
    }
}

impl Default for EntropyTruncator {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of entropy-weighted array truncation.
#[derive(Debug, Clone)]
pub struct EntropyTruncArrayResult {
    /// The kept elements (plus optional summary element).
    pub kept: Vec<serde_json::Value>,
    /// Number of elements dropped.
    pub dropped_count: usize,
    /// Entropy threshold used.
    pub threshold: f64,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compute Shannon entropy of a string in bits.
fn shannon_entropy(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }

    let mut freq = [0u32; 256];
    let len = text.len() as f64;

    for &byte in text.as_bytes() {
        freq[byte as usize] += 1;
    }

    let mut entropy = 0.0f64;
    for &count in &freq {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

/// Compute the median of a sorted slice.
fn median(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Split text into segments at paragraph boundaries (double newlines).
/// Falls back to fixed-size windows if no paragraph breaks exist.
fn split_segments(text: &str) -> Vec<String> {
    // Try splitting on double newlines first
    let segments: Vec<String> = text
        .split("\n\n")
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .collect();

    if segments.len() > 1 {
        return segments;
    }

    // Fall back to splitting on single newlines into groups of ~10 lines
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 10 {
        return vec![text.to_string()];
    }

    let chunk_size = 10;
    lines
        .chunks(chunk_size)
        .map(|chunk| chunk.join("\n"))
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shannon_entropy_empty() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    #[test]
    fn test_shannon_entropy_single_char() {
        // All same character → entropy = 0
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    fn test_shannon_entropy_varied() {
        // More varied text → higher entropy
        let low = shannon_entropy("aaaa");
        let high = shannon_entropy("abcdefghijklmnop");
        assert!(high > low);
    }

    #[test]
    fn test_median_empty() {
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn test_median_odd() {
        assert_eq!(median(&[1.0, 2.0, 3.0]), 2.0);
    }

    #[test]
    fn test_median_even() {
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
    }

    #[test]
    fn test_truncate_string_short_input() {
        let trunc = EntropyTruncator::new();
        let result = trunc.truncate_string("hello world").unwrap();
        assert_eq!(result.text, "hello world");
        assert_eq!(result.segments_dropped, 0);
    }

    #[test]
    fn test_truncate_string_with_paragraphs() {
        let trunc = EntropyTruncator::new();
        // Create text with varied and repetitive paragraphs
        let varied = "The quick brown fox jumps over the lazy dog with many different words and characters.";
        let repetitive = "aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa";
        let text = format!("{varied}\n\n{repetitive}\n\n{varied}");

        let result = trunc.truncate_string(&text).unwrap();
        // The varied paragraphs should be kept, repetitive may be dropped
        assert!(result.text.contains("quick brown fox"));
    }

    #[test]
    fn test_truncate_array_short() {
        let trunc = EntropyTruncator::new();
        let elements = vec![
            serde_json::json!(1),
            serde_json::json!(2),
        ];
        let result = trunc.truncate_array(&elements).unwrap();
        assert_eq!(result.kept.len(), 2);
        assert_eq!(result.dropped_count, 0);
    }

    #[test]
    fn test_truncate_array_with_redundant_elements() {
        let trunc = EntropyTruncator::new();
        let mut elements = Vec::new();
        // Add varied elements (high entropy — many different characters)
        elements.push(serde_json::json!({"name": "Alice Johnson", "role": "senior software engineer", "department": "platform infrastructure", "id": 12345}));
        elements.push(serde_json::json!({"name": "Bob Williams", "role": "product designer", "department": "user experience research", "id": 67890}));
        // Add many identical simple elements (low entropy — very repetitive)
        for _ in 0..8 {
            elements.push(serde_json::json!({"x": 1}));
        }

        let result = trunc.truncate_array(&elements).unwrap();
        // Should keep the varied elements; may drop some repetitive ones
        // At minimum, the result should not be larger than the input
        assert!(result.kept.len() <= elements.len());
    }

    #[test]
    fn test_split_segments_paragraphs() {
        let text = "para 1\n\npara 2\n\npara 3";
        let segments = split_segments(text);
        assert_eq!(segments.len(), 3);
    }

    #[test]
    fn test_split_segments_no_paragraphs() {
        let lines: Vec<String> = (0..25).map(|i| format!("line {i}")).collect();
        let text = lines.join("\n");
        let segments = split_segments(&text);
        assert!(segments.len() > 1);
    }

    #[test]
    fn test_custom_config() {
        let config = EntropyTruncConfig {
            min_entropy_override: Some(3.0),
            min_kept_segments: 1,
            min_segment_length: 5,
        };
        let trunc = EntropyTruncator::with_config(config);
        let result = trunc.truncate_string("hello\n\nworld").unwrap();
        assert!(!result.text.is_empty());
    }

    // ── Property tests ────────────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Truncation never produces empty output from non-empty input.
        #[test]
        fn prop_truncation_non_empty(
            text in "[a-z ]{20,200}"
        ) {
            let trunc = EntropyTruncator::new();
            let result = trunc.truncate_string(&text).unwrap();
            prop_assert!(
                !result.text.is_empty(),
                "truncation should not produce empty output"
            );
        }

        /// Shannon entropy is always non-negative.
        #[test]
        fn prop_entropy_non_negative(
            text in ".{1,100}"
        ) {
            let e = shannon_entropy(&text);
            prop_assert!(e >= 0.0, "entropy should be non-negative, got {e}");
        }

        /// Segments dropped count matches the actual difference.
        #[test]
        fn prop_segments_accounting(
            paragraphs in proptest::collection::vec("[a-z ]{10,50}", 3..=8usize),
        ) {
            let text = paragraphs.join("\n\n");
            let trunc = EntropyTruncator::new();
            let result = trunc.truncate_string(&text).unwrap();

            let total = result.segments.len();
            let kept = result.segments.iter().filter(|s| s.kept).count();
            prop_assert_eq!(
                result.segments_dropped, total - kept,
                "segments_dropped should equal total - kept"
            );
        }
    }
}
