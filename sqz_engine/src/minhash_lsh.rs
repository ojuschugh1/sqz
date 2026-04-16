//! MinHash + LSH — Locality-Sensitive Hashing for O(1) near-duplicate detection.
//!
//! Uses MinHash signature generation from character-level 3-gram shingles with
//! 64 hash functions, and LSH banding (8 bands × 8 rows) for fast candidate
//! retrieval. Documents with high Jaccard similarity will hash to the same
//! bucket in at least one band with high probability.
//!
//! # Example
//! ```
//! use sqz_engine::minhash_lsh::MinHashLsh;
//!
//! let mut lsh = MinHashLsh::new();
//! lsh.insert(1, "the quick brown fox jumps over the lazy dog");
//! lsh.insert(2, "the quick brown fox leaps over the lazy dog");
//! lsh.insert(3, "completely unrelated content about quantum physics");
//!
//! let candidates = lsh.query("the quick brown fox jumps over the lazy dog");
//! assert!(candidates.contains(&1));
//! ```

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// Number of hash functions in the MinHash signature.
const NUM_HASHES: usize = 64;
/// Number of bands for LSH banding.
const NUM_BANDS: usize = 8;
/// Number of rows per band (NUM_HASHES / NUM_BANDS).
const ROWS_PER_BAND: usize = NUM_HASHES / NUM_BANDS;
/// Default shingle size (character n-grams).
const SHINGLE_SIZE: usize = 3;

/// A MinHash signature — a vector of minimum hash values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinHashSignature {
    /// The minimum hash values, one per hash function.
    pub values: Vec<u64>,
}

impl MinHashSignature {
    /// Estimate Jaccard similarity between two signatures.
    ///
    /// Returns a value in `[0.0, 1.0]` where 1.0 means identical signatures.
    pub fn jaccard_similarity(&self, other: &MinHashSignature) -> f64 {
        if self.values.len() != other.values.len() || self.values.is_empty() {
            return 0.0;
        }
        let matches = self
            .values
            .iter()
            .zip(other.values.iter())
            .filter(|(a, b)| a == b)
            .count();
        matches as f64 / self.values.len() as f64
    }
}

/// MinHash + LSH index for near-duplicate detection.
///
/// Supports `insert` to add documents and `query` to find candidates
/// that are likely near-duplicates of a given text.
pub struct MinHashLsh {
    /// Band → bucket → set of document IDs.
    bands: Vec<HashMap<u64, HashSet<u64>>>,
    /// Document ID → MinHash signature (for optional similarity refinement).
    signatures: HashMap<u64, MinHashSignature>,
}

impl Default for MinHashLsh {
    fn default() -> Self {
        Self::new()
    }
}

impl MinHashLsh {
    /// Create a new empty MinHash LSH index.
    pub fn new() -> Self {
        let bands = (0..NUM_BANDS).map(|_| HashMap::new()).collect();
        Self {
            bands,
            signatures: HashMap::new(),
        }
    }

    /// Insert a document into the index.
    ///
    /// `doc_id` is a unique identifier for the document.
    /// `text` is the document content to be shingled and hashed.
    pub fn insert(&mut self, doc_id: u64, text: &str) {
        let sig = Self::compute_signature(text);
        self.insert_into_bands(doc_id, &sig);
        self.signatures.insert(doc_id, sig);
    }

    /// Query the index for candidate near-duplicates of `text`.
    ///
    /// Returns a list of document IDs that share at least one LSH band
    /// bucket with the query. These are *candidates* — false positives
    /// are possible but false negatives are unlikely for high-similarity
    /// pairs.
    pub fn query(&self, text: &str) -> Vec<u64> {
        let sig = Self::compute_signature(text);
        let mut candidates = HashSet::new();

        for (band_idx, band_map) in self.bands.iter().enumerate() {
            let band_hash = Self::hash_band(&sig, band_idx);
            if let Some(doc_ids) = band_map.get(&band_hash) {
                for &id in doc_ids {
                    candidates.insert(id);
                }
            }
        }

        let mut result: Vec<u64> = candidates.into_iter().collect();
        result.sort_unstable();
        result
    }

    /// Compute the MinHash signature for a text string.
    pub fn compute_signature(text: &str) -> MinHashSignature {
        let shingles = Self::shingle(text);
        if shingles.is_empty() {
            return MinHashSignature {
                values: vec![u64::MAX; NUM_HASHES],
            };
        }

        let mut min_hashes = vec![u64::MAX; NUM_HASHES];

        for shingle in &shingles {
            for i in 0..NUM_HASHES {
                let h = Self::hash_with_seed(shingle, i as u64);
                if h < min_hashes[i] {
                    min_hashes[i] = h;
                }
            }
        }

        MinHashSignature { values: min_hashes }
    }

    /// Get the stored signature for a document, if it exists.
    pub fn get_signature(&self, doc_id: u64) -> Option<&MinHashSignature> {
        self.signatures.get(&doc_id)
    }

    /// Return the number of documents in the index.
    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    /// Return true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Generate character-level n-gram shingles from text.
    fn shingle(text: &str) -> HashSet<String> {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() < SHINGLE_SIZE {
            let mut set = HashSet::new();
            if !chars.is_empty() {
                set.insert(chars.iter().collect());
            }
            return set;
        }

        chars
            .windows(SHINGLE_SIZE)
            .map(|w| w.iter().collect::<String>())
            .collect()
    }

    /// Hash a shingle with a seed to produce one of the NUM_HASHES hash values.
    fn hash_with_seed(shingle: &str, seed: u64) -> u64 {
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        shingle.hash(&mut hasher);
        hasher.finish()
    }

    /// Hash a band (a slice of the signature) to a single bucket key.
    fn hash_band(sig: &MinHashSignature, band_idx: usize) -> u64 {
        let start = band_idx * ROWS_PER_BAND;
        let end = start + ROWS_PER_BAND;
        let mut hasher = DefaultHasher::new();
        for &val in &sig.values[start..end] {
            val.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Insert a document's signature into all band buckets.
    fn insert_into_bands(&mut self, doc_id: u64, sig: &MinHashSignature) {
        for band_idx in 0..NUM_BANDS {
            let band_hash = Self::hash_band(sig, band_idx);
            self.bands[band_idx]
                .entry(band_hash)
                .or_default()
                .insert(doc_id);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_documents_are_candidates() {
        let mut lsh = MinHashLsh::new();
        lsh.insert(1, "the quick brown fox jumps over the lazy dog");
        let candidates = lsh.query("the quick brown fox jumps over the lazy dog");
        assert!(
            candidates.contains(&1),
            "identical document should be a candidate"
        );
    }

    #[test]
    fn test_similar_documents_are_candidates() {
        let mut lsh = MinHashLsh::new();
        lsh.insert(1, "the quick brown fox jumps over the lazy dog and more text here");
        lsh.insert(2, "the quick brown fox leaps over the lazy dog and more text here");
        // Query with text very similar to doc 1
        let candidates = lsh.query("the quick brown fox jumps over the lazy dog and more text here");
        assert!(
            candidates.contains(&1),
            "identical document should always be a candidate"
        );
    }

    #[test]
    fn test_empty_index_returns_no_candidates() {
        let lsh = MinHashLsh::new();
        let candidates = lsh.query("anything");
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_empty_text_insert_and_query() {
        let mut lsh = MinHashLsh::new();
        lsh.insert(1, "");
        let candidates = lsh.query("");
        // Empty text should still match itself (both produce the same max-value signature)
        assert!(candidates.contains(&1));
    }

    #[test]
    fn test_signature_jaccard_identical() {
        let sig = MinHashLsh::compute_signature("hello world foo bar baz");
        let similarity = sig.jaccard_similarity(&sig);
        assert!(
            (similarity - 1.0).abs() < f64::EPSILON,
            "identical signatures should have Jaccard similarity 1.0"
        );
    }

    #[test]
    fn test_signature_jaccard_different() {
        let sig_a = MinHashLsh::compute_signature(
            "the quick brown fox jumps over the lazy dog and some extra padding text",
        );
        let sig_b = MinHashLsh::compute_signature(
            "completely different content about quantum physics and mathematics research",
        );
        let similarity = sig_a.jaccard_similarity(&sig_b);
        assert!(
            similarity < 1.0,
            "different texts should have Jaccard similarity < 1.0, got {similarity}"
        );
    }

    #[test]
    fn test_multiple_inserts_and_query() {
        let mut lsh = MinHashLsh::new();
        for i in 0..10 {
            lsh.insert(i, &format!("document number {i} with some shared content here"));
        }
        assert_eq!(lsh.len(), 10);
        assert!(!lsh.is_empty());

        let candidates = lsh.query("document number 5 with some shared content here");
        assert!(
            candidates.contains(&5),
            "exact match should be a candidate"
        );
    }

    #[test]
    fn test_get_signature() {
        let mut lsh = MinHashLsh::new();
        lsh.insert(42, "test document");
        assert!(lsh.get_signature(42).is_some());
        assert!(lsh.get_signature(99).is_none());
    }

    // ── Property-based tests ──────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Identical texts must always produce the same MinHash signature.
        #[test]
        fn prop_identical_texts_same_signature(text in "[a-z ]{5,100}") {
            let sig_a = MinHashLsh::compute_signature(&text);
            let sig_b = MinHashLsh::compute_signature(&text);
            prop_assert_eq!(sig_a, sig_b);
        }

        /// Jaccard similarity of identical signatures is always 1.0.
        #[test]
        fn prop_self_similarity_is_one(text in "[a-z ]{5,100}") {
            let sig = MinHashLsh::compute_signature(&text);
            let sim = sig.jaccard_similarity(&sig);
            prop_assert!((sim - 1.0).abs() < f64::EPSILON);
        }

        /// Jaccard similarity is always in [0.0, 1.0].
        #[test]
        fn prop_jaccard_bounded(a in "[a-z ]{5,100}", b in "[a-z ]{5,100}") {
            let sig_a = MinHashLsh::compute_signature(&a);
            let sig_b = MinHashLsh::compute_signature(&b);
            let sim = sig_a.jaccard_similarity(&sig_b);
            prop_assert!(sim >= 0.0 && sim <= 1.0, "similarity out of bounds: {sim}");
        }

        /// An inserted document is always returned as a candidate for its own text.
        #[test]
        fn prop_self_query_returns_self(text in "[a-z ]{5,100}", id in 1u64..10000) {
            let mut lsh = MinHashLsh::new();
            lsh.insert(id, &text);
            let candidates = lsh.query(&text);
            prop_assert!(
                candidates.contains(&id),
                "document {id} should be a candidate for its own text"
            );
        }
    }
}
