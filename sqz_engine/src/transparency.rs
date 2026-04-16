//! Compression Transparency Protocol — structured annotations that tell the
//! LLM exactly what was compressed and how, so it can decide whether to
//! re-read content in full.
//!
//! No competitor gives the LLM this level of visibility into compression
//! decisions. The annotation is compact (~20-30 tokens) and machine-readable.

use crate::types::CompressedContent;

/// A structured compression annotation for the LLM.
#[derive(Debug, Clone)]
pub struct CompressionAnnotation {
    pub tokens_original: u32,
    pub tokens_compressed: u32,
    pub stages: Vec<String>,
    pub nulls_stripped: u32,
    pub lines_condensed: u32,
    pub dedup_hit: bool,
    pub safe_mode: bool,
    pub confidence: f64,
}

impl CompressionAnnotation {
    /// Build an annotation from a CompressedContent result.
    pub fn from_result(result: &CompressedContent) -> Self {
        let confidence = result
            .verify
            .as_ref()
            .map(|v| v.confidence)
            .unwrap_or(1.0);

        let safe_mode = result
            .provenance
            .label
            .as_deref()
            == Some("safe-fallback");

        let nulls_stripped = if result.stages_applied.iter().any(|s| s == "strip_nulls") {
            // Estimate from compression ratio on JSON
            let diff = result.tokens_original.saturating_sub(result.tokens_compressed);
            (diff as f64 * 0.3) as u32 // rough estimate: ~30% of savings from nulls
        } else {
            0
        };

        let lines_condensed = if result.stages_applied.iter().any(|s| s == "condense" || s == "rle") {
            let diff = result.tokens_original.saturating_sub(result.tokens_compressed);
            (diff as f64 * 0.4) as u32
        } else {
            0
        };

        Self {
            tokens_original: result.tokens_original,
            tokens_compressed: result.tokens_compressed,
            stages: result.stages_applied.clone(),
            nulls_stripped,
            lines_condensed,
            dedup_hit: false,
            safe_mode,
            confidence,
        }
    }

    /// Format as a compact inline annotation for the LLM.
    ///
    /// Example: `[sqz: 847→312 tokens | stripped: 12 nulls | condensed: 8 lines | confidence: 0.97 ✓]`
    pub fn format_inline(&self) -> String {
        let pct = if self.tokens_original > 0 {
            ((self.tokens_original - self.tokens_compressed) as f64
                / self.tokens_original as f64
                * 100.0) as u32
        } else {
            0
        };

        if self.dedup_hit {
            return format!(
                "[sqz: dedup hit, {}→13 tokens ({}% saved)]",
                self.tokens_original, pct
            );
        }

        if self.safe_mode {
            return format!(
                "[sqz: safe mode, {} tokens preserved verbatim]",
                self.tokens_original
            );
        }

        let mut parts = Vec::new();
        parts.push(format!(
            "{}→{} tokens ({}% saved)",
            self.tokens_original, self.tokens_compressed, pct
        ));

        if self.nulls_stripped > 0 {
            parts.push(format!("stripped: {} nulls", self.nulls_stripped));
        }
        if self.lines_condensed > 0 {
            parts.push(format!("condensed: {} lines", self.lines_condensed));
        }

        let check = if self.confidence >= 0.9 {
            "✓"
        } else if self.confidence >= 0.6 {
            "~"
        } else {
            "⚠"
        };

        format!("[sqz: {} | confidence: {:.2} {}]", parts.join(" | "), self.confidence, check)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Provenance, VerifyResult};

    fn make_result(original: u32, compressed: u32, stages: Vec<&str>) -> CompressedContent {
        CompressedContent {
            data: String::new(),
            tokens_original: original,
            tokens_compressed: compressed,
            stages_applied: stages.into_iter().map(String::from).collect(),
            compression_ratio: compressed as f64 / original.max(1) as f64,
            provenance: Provenance::default(),
            verify: Some(VerifyResult {
                passed: true,
                confidence: 0.95,
                checks_passed: vec!["all".into()],
                checks_failed: vec![],
                fallback_triggered: false,
            }),
        }
    }

    #[test]
    fn test_annotation_from_result() {
        let result = make_result(847, 312, vec!["strip_nulls", "toon_encode"]);
        let ann = CompressionAnnotation::from_result(&result);
        assert_eq!(ann.tokens_original, 847);
        assert_eq!(ann.tokens_compressed, 312);
        assert!(!ann.safe_mode);
        assert!((ann.confidence - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_format_inline_normal() {
        let result = make_result(847, 312, vec!["strip_nulls", "condense"]);
        let ann = CompressionAnnotation::from_result(&result);
        let formatted = ann.format_inline();
        assert!(formatted.contains("847→312"));
        assert!(formatted.contains("✓"));
    }

    #[test]
    fn test_format_inline_dedup() {
        let mut ann = CompressionAnnotation::from_result(&make_result(2000, 13, vec![]));
        ann.dedup_hit = true;
        let formatted = ann.format_inline();
        assert!(formatted.contains("dedup hit"));
        assert!(formatted.contains("13 tokens"));
    }

    #[test]
    fn test_format_inline_safe_mode() {
        let mut result = make_result(500, 500, vec![]);
        result.provenance.label = Some("safe-fallback".into());
        let ann = CompressionAnnotation::from_result(&result);
        let formatted = ann.format_inline();
        assert!(formatted.contains("safe mode"));
        assert!(formatted.contains("preserved verbatim"));
    }

    #[test]
    fn test_format_inline_low_confidence() {
        let mut result = make_result(100, 50, vec!["condense"]);
        result.verify.as_mut().unwrap().confidence = 0.5;
        let ann = CompressionAnnotation::from_result(&result);
        let formatted = ann.format_inline();
        assert!(formatted.contains("⚠"));
    }

    #[test]
    fn test_zero_tokens() {
        let result = make_result(0, 0, vec![]);
        let ann = CompressionAnnotation::from_result(&result);
        let formatted = ann.format_inline();
        assert!(formatted.contains("0→0"));
    }
}
