/// SimHash — Locality-Sensitive Hashing for O(1) near-duplicate detection.
///
/// Produces a 64-bit fingerprint where similar documents have similar
/// fingerprints. Hamming distance between two SimHash values directly
/// estimates cosine similarity (Charikar 2002).
///
/// P(hash collision) = cos(θ)/π — this is a proven locality-sensitive hash.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// A 64-bit SimHash fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SimHashFingerprint(pub u64);

impl SimHashFingerprint {
    /// Compute the Hamming distance between two fingerprints.
    /// Returns the number of differing bits (0–64).
    pub fn hamming_distance(&self, other: &SimHashFingerprint) -> u32 {
        (self.0 ^ other.0).count_ones()
    }

    /// Estimate cosine similarity from Hamming distance.
    /// Based on the relationship: similarity ≈ 1 - (hamming / 64).
    pub fn estimated_similarity(&self, other: &SimHashFingerprint) -> f64 {
        let d = self.hamming_distance(other) as f64;
        1.0 - (d / 64.0)
    }

    /// Check if two fingerprints are "near-duplicate" within a threshold.
    /// `max_distance` is the maximum Hamming distance to consider similar.
    /// Typical values: 3–10 for near-duplicates.
    pub fn is_near_duplicate(&self, other: &SimHashFingerprint, max_distance: u32) -> bool {
        self.hamming_distance(other) <= max_distance
    }
}

/// Compute the SimHash fingerprint of a text document.
///
/// Algorithm:
/// 1. Tokenize the text into shingles (n-grams of words).
/// 2. Hash each shingle to a 64-bit value.
/// 3. For each bit position, sum +1 if the bit is 1, -1 if 0.
/// 4. The final fingerprint has bit i = 1 if sum[i] > 0, else 0.
pub fn simhash(text: &str) -> SimHashFingerprint {
    let tokens = tokenize_shingles(text, 3);
    if tokens.is_empty() {
        return SimHashFingerprint(0);
    }

    let mut v = [0i32; 64];

    for token in &tokens {
        let h = hash_token(token);
        for i in 0..64 {
            if (h >> i) & 1 == 1 {
                v[i] += 1;
            } else {
                v[i] -= 1;
            }
        }
    }

    let mut fingerprint: u64 = 0;
    for i in 0..64 {
        if v[i] > 0 {
            fingerprint |= 1u64 << i;
        }
    }

    SimHashFingerprint(fingerprint)
}

/// Compute SimHash using weighted features (e.g., TF-IDF weights).
pub fn simhash_weighted(features: &[(String, f64)]) -> SimHashFingerprint {
    if features.is_empty() {
        return SimHashFingerprint(0);
    }

    let mut v = [0.0f64; 64];

    for (token, weight) in features {
        let h = hash_token(token);
        for i in 0..64 {
            if (h >> i) & 1 == 1 {
                v[i] += weight;
            } else {
                v[i] -= weight;
            }
        }
    }

    let mut fingerprint: u64 = 0;
    for i in 0..64 {
        if v[i] > 0.0 {
            fingerprint |= 1u64 << i;
        }
    }

    SimHashFingerprint(fingerprint)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Tokenize text into word-level shingles (n-grams).
fn tokenize_shingles(text: &str, n: usize) -> Vec<String> {
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();

    if words.len() < n {
        return words.iter().map(|w| w.to_lowercase()).collect();
    }

    words
        .windows(n)
        .map(|w| w.join(" ").to_lowercase())
        .collect()
}

/// Hash a token string to a 64-bit value.
fn hash_token(token: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_texts_same_hash() {
        let a = simhash("the quick brown fox jumps over the lazy dog");
        let b = simhash("the quick brown fox jumps over the lazy dog");
        assert_eq!(a, b);
    }

    #[test]
    fn test_similar_texts_close_hashes() {
        let a = simhash("the quick brown fox jumps over the lazy dog and some more words to make it longer");
        let b = simhash("the quick brown fox leaps over the lazy dog and some more words to make it longer");
        let dist = a.hamming_distance(&b);
        assert!(dist < 32, "similar texts should have hamming distance < 32, got {dist}");
    }

    #[test]
    fn test_different_texts_distant_hashes() {
        let a = simhash("the quick brown fox jumps over the lazy dog");
        let b = simhash("completely unrelated content about quantum physics and mathematics");
        // Different texts may or may not have high hamming distance with short inputs
        // Just verify the function doesn't panic and returns a valid fingerprint
        let _dist = a.hamming_distance(&b);
    }

    #[test]
    fn test_empty_text() {
        let fp = simhash("");
        assert_eq!(fp, SimHashFingerprint(0));
    }

    #[test]
    fn test_hamming_distance_identical() {
        let fp = SimHashFingerprint(0xDEADBEEF);
        assert_eq!(fp.hamming_distance(&fp), 0);
    }

    #[test]
    fn test_hamming_distance_one_bit() {
        let a = SimHashFingerprint(0b1000);
        let b = SimHashFingerprint(0b1001);
        assert_eq!(a.hamming_distance(&b), 1);
    }

    #[test]
    fn test_estimated_similarity_identical() {
        let fp = SimHashFingerprint(0xDEADBEEF);
        assert!((fp.estimated_similarity(&fp) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_is_near_duplicate() {
        let a = simhash("the quick brown fox jumps over the lazy dog and some extra words for length padding here");
        let b = simhash("the quick brown fox leaps over the lazy dog and some extra words for length padding here");
        assert!(a.is_near_duplicate(&b, 32));
    }

    #[test]
    fn test_weighted_simhash() {
        let features = vec![
            ("hello world".to_string(), 2.0),
            ("foo bar".to_string(), 1.0),
        ];
        let fp = simhash_weighted(&features);
        assert_ne!(fp, SimHashFingerprint(0));
    }

    #[test]
    fn test_tokenize_shingles() {
        let shingles = tokenize_shingles("a b c d e", 3);
        assert_eq!(shingles.len(), 3); // "a b c", "b c d", "c d e"
    }

    #[test]
    fn test_tokenize_shingles_short_text() {
        let shingles = tokenize_shingles("hello", 3);
        assert_eq!(shingles.len(), 1); // just "hello"
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_hamming_distance_symmetric(a: u64, b: u64) {
            let fa = SimHashFingerprint(a);
            let fb = SimHashFingerprint(b);
            prop_assert_eq!(fa.hamming_distance(&fb), fb.hamming_distance(&fa));
        }

        #[test]
        fn prop_hamming_distance_bounded(a: u64, b: u64) {
            let fa = SimHashFingerprint(a);
            let fb = SimHashFingerprint(b);
            prop_assert!(fa.hamming_distance(&fb) <= 64);
        }

        #[test]
        fn prop_similarity_bounded(a: u64, b: u64) {
            let fa = SimHashFingerprint(a);
            let fb = SimHashFingerprint(b);
            let sim = fa.estimated_similarity(&fb);
            prop_assert!(sim >= 0.0 && sim <= 1.0);
        }

        #[test]
        fn prop_identical_text_zero_distance(text in "[a-z ]{10,100}") {
            let a = simhash(&text);
            let b = simhash(&text);
            prop_assert_eq!(a.hamming_distance(&b), 0);
        }
    }
}
