/// Compression Quality Measurement using information-theoretic bounds.
///
/// Shannon's source coding theorem: the minimum average bits per symbol
/// is the entropy H(X). Arithmetic coding achieves within 1 bit of this
/// bound. By comparing sqz's actual output size to the theoretical minimum,
/// we get a scientifically grounded "compression efficiency" metric.
///
/// efficiency = H(input) / actual_bits_per_symbol
///
/// A value of 1.0 means sqz achieved the theoretical optimum.
/// Values < 1.0 indicate room for improvement.
/// Values > 1.0 are impossible (would violate Shannon's theorem).

/// Compression quality metrics for a single compression operation.
#[derive(Debug, Clone)]
pub struct CompressionQuality {
    /// Shannon entropy of the input (bits per byte).
    pub input_entropy: f64,
    /// Theoretical minimum size in tokens (entropy-based lower bound).
    pub theoretical_min_tokens: u32,
    /// Actual compressed size in tokens.
    pub actual_tokens: u32,
    /// Compression efficiency: theoretical_min / actual (0.0–1.0).
    /// 1.0 = optimal, lower = room for improvement.
    pub efficiency: f64,
    /// How many additional tokens could theoretically be saved.
    pub headroom_tokens: u32,
    /// Human-readable quality grade.
    pub grade: QualityGrade,
}

/// Quality grade based on compression efficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityGrade {
    /// efficiency >= 0.9 — near-optimal compression
    Excellent,
    /// efficiency >= 0.7 — good compression
    Good,
    /// efficiency >= 0.5 — moderate compression, room for improvement
    Fair,
    /// efficiency < 0.5 — significant room for improvement
    Poor,
}

impl QualityGrade {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Excellent => "excellent",
            Self::Good => "good",
            Self::Fair => "fair",
            Self::Poor => "poor",
        }
    }
}

/// Compute compression quality metrics.
///
/// `input` is the original text, `tokens_original` and `tokens_compressed`
/// are the token counts before and after compression.
pub fn measure_quality(
    input: &str,
    tokens_original: u32,
    tokens_compressed: u32,
) -> CompressionQuality {
    let entropy = shannon_entropy_bits_per_byte(input);

    // Theoretical minimum: entropy * input_bytes / bits_per_token
    // Assuming ~4 chars per token (BPE average), ~8 bits per char
    let input_bytes = input.len() as f64;
    let bits_per_token = 32.0; // ~4 chars × 8 bits
    let theoretical_min_bits = entropy * input_bytes;
    let theoretical_min_tokens = (theoretical_min_bits / bits_per_token).ceil() as u32;

    // Clamp: theoretical min can't exceed original
    let theoretical_min_tokens = theoretical_min_tokens.min(tokens_original);

    let efficiency = if tokens_compressed == 0 {
        1.0
    } else if theoretical_min_tokens == 0 {
        1.0
    } else {
        (theoretical_min_tokens as f64 / tokens_compressed as f64).min(1.0)
    };

    let headroom = tokens_compressed.saturating_sub(theoretical_min_tokens);

    let grade = if efficiency >= 0.9 {
        QualityGrade::Excellent
    } else if efficiency >= 0.7 {
        QualityGrade::Good
    } else if efficiency >= 0.5 {
        QualityGrade::Fair
    } else {
        QualityGrade::Poor
    };

    CompressionQuality {
        input_entropy: entropy,
        theoretical_min_tokens,
        actual_tokens: tokens_compressed,
        efficiency,
        headroom_tokens: headroom,
        grade,
    }
}

/// Compute Shannon entropy in bits per byte.
fn shannon_entropy_bits_per_byte(text: &str) -> f64 {
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

/// Format a quality report as a human-readable string.
pub fn format_quality_report(q: &CompressionQuality) -> String {
    format!(
        "entropy: {:.2} bits/byte | min: {} tokens | actual: {} tokens | \
         efficiency: {:.0}% | headroom: {} tokens | grade: {}",
        q.input_entropy,
        q.theoretical_min_tokens,
        q.actual_tokens,
        q.efficiency * 100.0,
        q.headroom_tokens,
        q.grade.as_str(),
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        let q = measure_quality("", 0, 0);
        assert_eq!(q.input_entropy, 0.0);
        assert_eq!(q.efficiency, 1.0);
        assert_eq!(q.grade, QualityGrade::Excellent);
    }

    #[test]
    fn test_no_compression() {
        let input = "hello world this is a test string with some content";
        let tokens = 12;
        let q = measure_quality(input, tokens, tokens);
        assert!(q.efficiency <= 1.0);
        assert!(q.headroom_tokens <= tokens);
    }

    #[test]
    fn test_perfect_compression() {
        let input = "aaaa"; // very low entropy
        let q = measure_quality(input, 1, 1);
        // Low entropy input compressed to 1 token — should be excellent
        assert_eq!(q.grade, QualityGrade::Excellent);
    }

    #[test]
    fn test_high_entropy_input() {
        // High entropy: many different characters
        let input: String = (0..200).map(|i| (b'a' + (i % 26)) as char).collect();
        let q = measure_quality(&input, 50, 50);
        assert!(q.input_entropy > 3.0, "high entropy input should have entropy > 3");
    }

    #[test]
    fn test_efficiency_bounded() {
        let q = measure_quality("test content here", 5, 3);
        assert!(q.efficiency >= 0.0 && q.efficiency <= 1.0);
    }

    #[test]
    fn test_grade_thresholds() {
        assert_eq!(
            measure_quality("a", 100, 1).grade,
            QualityGrade::Excellent
        );
    }

    #[test]
    fn test_format_quality_report() {
        let q = measure_quality("hello world test content", 6, 4);
        let report = format_quality_report(&q);
        assert!(report.contains("entropy:"));
        assert!(report.contains("efficiency:"));
        assert!(report.contains("grade:"));
    }

    #[test]
    fn test_shannon_entropy_single_char() {
        assert_eq!(shannon_entropy_bits_per_byte("aaaa"), 0.0);
    }

    #[test]
    fn test_shannon_entropy_varied() {
        let low = shannon_entropy_bits_per_byte("aaaa");
        let high = shannon_entropy_bits_per_byte("abcdefghijklmnop");
        assert!(high > low);
    }

    use proptest::prelude::*;

    proptest! {
        /// Efficiency is always in [0.0, 1.0].
        #[test]
        fn prop_efficiency_bounded(
            text in "[a-z ]{10,200}",
            original in 5u32..=100u32,
            compressed in 1u32..=100u32,
        ) {
            let q = measure_quality(&text, original, compressed.min(original));
            prop_assert!(
                q.efficiency >= 0.0 && q.efficiency <= 1.0,
                "efficiency out of bounds: {}",
                q.efficiency
            );
        }

        /// Entropy is always non-negative.
        #[test]
        fn prop_entropy_non_negative(text in ".{1,100}") {
            let e = shannon_entropy_bits_per_byte(&text);
            prop_assert!(e >= 0.0, "entropy should be non-negative: {e}");
        }
    }
}
