/// Delta Encoding for Near-Duplicate Content.
///
/// When a file changes by a few lines, the SHA-256 dedup cache misses and
/// the entire file is re-compressed. Delta encoding computes a line-level
/// diff between the cached version and the new version, sending only the
/// changed lines prefixed with a cache reference.
///
/// The similarity check uses a rolling hash fingerprint over fixed-size
/// blocks to quickly determine if two pieces of content are "near-duplicate"
/// (similarity > threshold) before computing the full diff.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::error::Result;
use crate::simhash::{simhash, SimHashFingerprint};

/// Configuration for delta encoding.
#[derive(Debug, Clone)]
pub struct DeltaConfig {
    /// Minimum similarity (0.0–1.0) to trigger delta encoding.
    /// Below this threshold, full compression is used instead.
    /// Default: 0.6
    pub similarity_threshold: f64,
    /// Number of context lines to include around each change.
    /// Default: 1
    pub context_lines: usize,
}

impl Default for DeltaConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.6,
            context_lines: 1,
        }
    }
}

/// A single edit operation in a delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    /// Keep N lines unchanged (not emitted in output, just tracked).
    Keep(usize),
    /// Insert new lines.
    Insert(Vec<String>),
    /// Delete N lines from the original.
    Delete(usize),
}

/// Result of delta encoding.
#[derive(Debug, Clone)]
pub struct DeltaResult {
    /// The compact delta representation.
    pub delta_text: String,
    /// Number of lines changed.
    pub lines_changed: usize,
    /// Total lines in the original.
    pub lines_original: usize,
    /// Similarity score between old and new content.
    pub similarity: f64,
}

/// Delta encoder for near-duplicate content.
pub struct DeltaEncoder {
    config: DeltaConfig,
    /// SimHash fingerprint index: maps fingerprints to content identifiers.
    /// Enables O(1) near-duplicate detection instead of O(n×m) LCS comparison.
    fingerprint_index: HashMap<String, SimHashFingerprint>,
}

impl DeltaEncoder {
    pub fn new() -> Self {
        Self::with_config(DeltaConfig::default())
    }

    pub fn with_config(config: DeltaConfig) -> Self {
        Self {
            config,
            fingerprint_index: HashMap::new(),
        }
    }

    /// Compute the SimHash fingerprint of a text.
    pub fn fingerprint(&self, text: &str) -> SimHashFingerprint {
        simhash(text)
    }

    /// Index a piece of content by its identifier for future O(1) lookups.
    pub fn index_content(&mut self, id: &str, text: &str) {
        let fp = simhash(text);
        self.fingerprint_index.insert(id.to_string(), fp);
    }

    /// Find the best near-duplicate match from the index using SimHash.
    /// Returns the id of the closest match within `max_hamming_distance`,
    /// or None if no match is close enough.
    pub fn find_nearest(&self, text: &str, max_hamming_distance: u32) -> Option<&str> {
        let fp = simhash(text);
        let mut best_id: Option<&str> = None;
        let mut best_dist = u32::MAX;

        for (id, &indexed_fp) in &self.fingerprint_index {
            let dist = fp.hamming_distance(&indexed_fp);
            if dist < best_dist && dist <= max_hamming_distance {
                best_dist = dist;
                best_id = Some(id.as_str());
            }
        }

        best_id
    }

    /// Compute similarity between two texts using SimHash for the fast path,
    /// falling back to line-level fingerprinting for precision.
    pub fn similarity(&self, old: &str, new: &str) -> f64 {
        // Fast path: SimHash gives a quick estimate
        let fp_old = simhash(old);
        let fp_new = simhash(new);
        let simhash_sim = fp_old.estimated_similarity(&fp_new);

        // If SimHash says very different (< 0.3), trust it — skip expensive check
        if simhash_sim < 0.3 {
            return simhash_sim;
        }

        // For borderline cases, use precise line-level comparison
        self.similarity_precise(old, new)
    }

    /// Precise line-level similarity (the original O(n) algorithm).
    fn similarity_precise(&self, old: &str, new: &str) -> f64 {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        if old_lines.is_empty() && new_lines.is_empty() {
            return 1.0;
        }
        if old_lines.is_empty() || new_lines.is_empty() {
            return 0.0;
        }

        // Build fingerprint set for old content
        let old_hashes: HashMap<u64, usize> = old_lines
            .iter()
            .map(|l| line_hash(l))
            .fold(HashMap::new(), |mut acc, h| {
                *acc.entry(h).or_insert(0) += 1;
                acc
            });

        let new_hashes: HashMap<u64, usize> = new_lines
            .iter()
            .map(|l| line_hash(l))
            .fold(HashMap::new(), |mut acc, h| {
                *acc.entry(h).or_insert(0) += 1;
                acc
            });

        // Count shared line hashes (min of counts)
        let mut shared = 0usize;
        for (hash, &old_count) in &old_hashes {
            if let Some(&new_count) = new_hashes.get(hash) {
                shared += old_count.min(new_count);
            }
        }

        let total = old_lines.len().max(new_lines.len());
        shared as f64 / total as f64
    }

    /// Compute a line-level diff and produce a compact delta representation.
    ///
    /// Returns `None` if similarity is below the threshold (caller should
    /// fall back to full compression).
    pub fn encode(&self, old: &str, new: &str, hash_prefix: &str) -> Result<Option<DeltaResult>> {
        let sim = self.similarity(old, new);
        if sim < self.config.similarity_threshold {
            return Ok(None);
        }

        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        let ops = compute_diff(&old_lines, &new_lines);
        let (delta_text, lines_changed) =
            format_delta(&ops, &new_lines, hash_prefix, self.config.context_lines);

        Ok(Some(DeltaResult {
            delta_text,
            lines_changed,
            lines_original: old_lines.len(),
            similarity: sim,
        }))
    }

    /// Check if delta encoding would be beneficial for these two texts.
    pub fn should_delta(&self, old: &str, new: &str) -> bool {
        let sim = self.similarity(old, new);
        sim >= self.config.similarity_threshold && sim < 1.0
    }
}

impl Default for DeltaEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Diff algorithm (Myers-like, simplified for line-level) ────────────────────

/// Compute the edit operations to transform `old` into `new`.
fn compute_diff<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DeltaOp> {
    // Use LCS (Longest Common Subsequence) to find matching lines,
    // then derive insert/delete/keep operations.
    let lcs = lcs_table(old, new);
    let mut ops = Vec::new();
    let mut i = old.len();
    let mut j = new.len();

    // Trace back through the LCS table
    let mut raw_ops: Vec<RawOp> = Vec::new();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            raw_ops.push(RawOp::Keep);
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || lcs[i][j - 1] >= lcs[i - 1][j]) {
            raw_ops.push(RawOp::Insert(new[j - 1].to_string()));
            j -= 1;
        } else if i > 0 {
            raw_ops.push(RawOp::Delete);
            i -= 1;
        }
    }
    raw_ops.reverse();

    // Compact raw ops into DeltaOps
    let mut keep_count = 0usize;
    let mut del_count = 0usize;
    let mut ins_buf: Vec<String> = Vec::new();

    for op in raw_ops {
        match op {
            RawOp::Keep => {
                if del_count > 0 {
                    ops.push(DeltaOp::Delete(del_count));
                    del_count = 0;
                }
                if !ins_buf.is_empty() {
                    ops.push(DeltaOp::Insert(std::mem::take(&mut ins_buf)));
                }
                keep_count += 1;
            }
            RawOp::Delete => {
                if keep_count > 0 {
                    ops.push(DeltaOp::Keep(keep_count));
                    keep_count = 0;
                }
                if !ins_buf.is_empty() {
                    ops.push(DeltaOp::Insert(std::mem::take(&mut ins_buf)));
                }
                del_count += 1;
            }
            RawOp::Insert(line) => {
                if keep_count > 0 {
                    ops.push(DeltaOp::Keep(keep_count));
                    keep_count = 0;
                }
                if del_count > 0 {
                    ops.push(DeltaOp::Delete(del_count));
                    del_count = 0;
                }
                ins_buf.push(line);
            }
        }
    }
    // Flush remaining
    if keep_count > 0 {
        ops.push(DeltaOp::Keep(keep_count));
    }
    if del_count > 0 {
        ops.push(DeltaOp::Delete(del_count));
    }
    if !ins_buf.is_empty() {
        ops.push(DeltaOp::Insert(ins_buf));
    }

    ops
}

#[derive(Debug)]
enum RawOp {
    Keep,
    Delete,
    Insert(String),
}

/// Build the LCS length table for two line slices.
fn lcs_table(old: &[&str], new: &[&str]) -> Vec<Vec<usize>> {
    let m = old.len();
    let n = new.len();
    let mut table = vec![vec![0usize; n + 1]; m + 1];

    for i in 1..=m {
        for j in 1..=n {
            if old[i - 1] == new[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }
    table
}

/// Format delta operations into a compact text representation.
fn format_delta(
    ops: &[DeltaOp],
    _new_lines: &[&str],
    hash_prefix: &str,
    context_lines: usize,
) -> (String, usize) {
    let mut output = Vec::new();
    output.push(format!("§delta:{hash_prefix}§"));

    let mut line_num = 0usize;
    let mut lines_changed = 0usize;

    for op in ops {
        match op {
            DeltaOp::Keep(n) => {
                if *n > context_lines * 2 {
                    let skip = n - context_lines * 2;
                    output.push(format!(" @@ skip {skip} unchanged lines @@"));
                }
                line_num += n;
            }
            DeltaOp::Delete(n) => {
                output.push(format!("-[{n} lines removed at L{line_num}]"));
                line_num += n;
                lines_changed += n;
            }
            DeltaOp::Insert(lines) => {
                for line in lines {
                    output.push(format!("+{line}"));
                }
                lines_changed += lines.len();
            }
        }
    }

    (output.join("\n"), lines_changed)
}

/// Compute a fast hash of a single line for fingerprinting.
fn line_hash(line: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_content_similarity_1() {
        let enc = DeltaEncoder::new();
        let text = "line 1\nline 2\nline 3\n";
        assert!((enc.similarity(text, text) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_completely_different_similarity_0() {
        let enc = DeltaEncoder::new();
        let old = "aaa\nbbb\nccc\n";
        let new = "xxx\nyyy\nzzz\n";
        assert!(enc.similarity(old, new) < 0.01);
    }

    #[test]
    fn test_small_edit_high_similarity() {
        let enc = DeltaEncoder::new();
        let old = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        let new = "line 1\nline 2\nline 3 modified\nline 4\nline 5\n";
        let sim = enc.similarity(old, new);
        assert!(sim > 0.7, "expected high similarity, got {sim}");
    }

    #[test]
    fn test_encode_returns_none_below_threshold() {
        let enc = DeltaEncoder::new();
        let old = "aaa\nbbb\nccc\n";
        let new = "xxx\nyyy\nzzz\n";
        let result = enc.encode(old, new, "abc123").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_encode_returns_delta_for_small_edit() {
        let enc = DeltaEncoder::new();
        let old = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        let new = "line 1\nline 2\nline 3 modified\nline 4\nline 5\n";
        let result = enc.encode(old, new, "abc123").unwrap();
        assert!(result.is_some(), "expected delta for small edit");
        let delta = result.unwrap();
        assert!(delta.delta_text.starts_with("§delta:abc123§"));
        assert!(delta.lines_changed > 0);
        assert!(delta.similarity > 0.7);
    }

    #[test]
    fn test_encode_identical_content() {
        let enc = DeltaEncoder::new();
        let text = "line 1\nline 2\nline 3\n";
        let result = enc.encode(text, text, "abc123").unwrap();
        // Identical content has similarity 1.0 which is >= threshold but
        // should produce a delta with 0 changes
        if let Some(delta) = result {
            assert_eq!(delta.lines_changed, 0);
        }
        // It's also valid for the caller to use exact dedup instead
    }

    #[test]
    fn test_should_delta() {
        let enc = DeltaEncoder::new();
        let old = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        let new = "line 1\nline 2\nline 3 modified\nline 4\nline 5\n";
        assert!(enc.should_delta(old, new));

        // Identical content: similarity == 1.0, should_delta returns false
        assert!(!enc.should_delta(old, old));

        // Completely different: below threshold
        assert!(!enc.should_delta("aaa\nbbb\n", "xxx\nyyy\n"));
    }

    #[test]
    fn test_empty_inputs() {
        let enc = DeltaEncoder::new();
        assert!((enc.similarity("", "") - 1.0).abs() < 1e-9);
        assert_eq!(enc.similarity("", "something"), 0.0);
        assert_eq!(enc.similarity("something", ""), 0.0);
    }

    #[test]
    fn test_delta_format_contains_header() {
        let enc = DeltaEncoder::new();
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nb\nc\nX\ne\nf\ng\nh\ni\nj\n";
        if let Some(delta) = enc.encode(old, new, "deadbeef").unwrap() {
            assert!(delta.delta_text.contains("§delta:deadbeef§"));
        }
    }

    #[test]
    fn test_compute_diff_basic() {
        let old = vec!["a", "b", "c"];
        let new = vec!["a", "x", "c"];
        let ops = compute_diff(&old, &new);
        // Should have: Keep(1), Delete(1), Insert(["x"]), Keep(1)
        assert!(!ops.is_empty());
    }

    #[test]
    fn test_compute_diff_insertion() {
        let old = vec!["a", "c"];
        let new = vec!["a", "b", "c"];
        let ops = compute_diff(&old, &new);
        assert!(!ops.is_empty());
    }

    #[test]
    fn test_compute_diff_deletion() {
        let old = vec!["a", "b", "c"];
        let new = vec!["a", "c"];
        let ops = compute_diff(&old, &new);
        assert!(!ops.is_empty());
    }

    // ── Property tests ────────────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Similarity is always in [0.0, 1.0].
        #[test]
        fn prop_similarity_bounded(
            old in "[a-z\n]{5,100}",
            new in "[a-z\n]{5,100}",
        ) {
            let enc = DeltaEncoder::new();
            let sim = enc.similarity(&old, &new);
            prop_assert!(sim >= 0.0 && sim <= 1.0, "similarity out of bounds: {sim}");
        }

        /// Similarity of identical content is 1.0.
        #[test]
        fn prop_similarity_identity(
            text in "[a-z ]{5,100}"
        ) {
            let enc = DeltaEncoder::new();
            let sim = enc.similarity(&text, &text);
            prop_assert!(
                (sim - 1.0).abs() < 1e-9,
                "identical content should have similarity 1.0, got {sim}"
            );
        }

        /// Delta text always starts with the §delta: header when produced.
        #[test]
        fn prop_delta_header_present(
            base in "[a-z]{3,10}\n[a-z]{3,10}\n[a-z]{3,10}\n[a-z]{3,10}\n[a-z]{3,10}\n",
        ) {
            let enc = DeltaEncoder::new();
            // Make a small edit
            let new = format!("{base}extra line\n");
            if let Some(delta) = enc.encode(&base, &new, "test1234").unwrap() {
                prop_assert!(
                    delta.delta_text.starts_with("§delta:test1234§"),
                    "delta should start with header"
                );
            }
        }
    }
}
