/// TextRank — graph-based extractive compression for prose content.
///
/// Applies the TextRank algorithm (Mihalcea & Tarau 2004, same math as
/// Google's PageRank) to rank sentences by importance. Builds a graph
/// where sentences are nodes and edges are weighted by word overlap
/// similarity. The stationary distribution of a random walk gives each
/// sentence an importance score. Keeps the top-K sentences, drops the rest.
///
/// Convergence guarantee: Perron-Frobenius theorem ensures a unique
/// stationary distribution for any connected graph with positive weights.

use crate::error::Result;

/// Configuration for TextRank compression.
#[derive(Debug, Clone)]
pub struct TextRankConfig {
    /// Fraction of sentences to keep (0.0–1.0). Default: 0.5 (keep top 50%).
    pub keep_ratio: f64,
    /// Damping factor for the random walk (same as PageRank). Default: 0.85.
    pub damping: f64,
    /// Maximum iterations for convergence. Default: 50.
    pub max_iterations: usize,
    /// Convergence threshold. Default: 1e-4.
    pub epsilon: f64,
    /// Minimum number of sentences to keep regardless of ratio. Default: 2.
    pub min_sentences: usize,
}

impl Default for TextRankConfig {
    fn default() -> Self {
        Self {
            keep_ratio: 0.5,
            damping: 0.85,
            max_iterations: 50,
            epsilon: 1e-4,
            min_sentences: 2,
        }
    }
}

/// Result of TextRank compression.
#[derive(Debug, Clone)]
pub struct TextRankResult {
    /// The compressed text (top-K sentences in original order).
    pub text: String,
    /// Number of sentences kept.
    pub sentences_kept: usize,
    /// Number of sentences dropped.
    pub sentences_dropped: usize,
    /// The importance scores for each original sentence.
    pub scores: Vec<f64>,
}

/// Compress prose text using TextRank extractive summarization.
pub fn textrank_compress(text: &str, config: &TextRankConfig) -> Result<TextRankResult> {
    let sentences = split_sentences(text);

    if sentences.len() <= config.min_sentences {
        return Ok(TextRankResult {
            text: text.to_string(),
            sentences_kept: sentences.len(),
            sentences_dropped: 0,
            scores: vec![1.0; sentences.len()],
        });
    }

    // Build word sets for each sentence
    let word_sets: Vec<std::collections::HashSet<String>> = sentences
        .iter()
        .map(|s| tokenize_words(s))
        .collect();

    // Build similarity matrix
    let n = sentences.len();
    let mut sim_matrix = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let sim = word_overlap_similarity(&word_sets[i], &word_sets[j]);
            sim_matrix[i][j] = sim;
            sim_matrix[j][i] = sim;
        }
    }

    // Run PageRank iteration
    let scores = pagerank(&sim_matrix, config.damping, config.max_iterations, config.epsilon);

    // Determine how many sentences to keep
    let keep_count = ((sentences.len() as f64 * config.keep_ratio).ceil() as usize)
        .max(config.min_sentences)
        .min(sentences.len());

    // Rank sentences by score, keep top-K
    let mut ranked: Vec<(usize, f64)> = scores.iter().copied().enumerate().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut keep_indices: Vec<usize> = ranked[..keep_count].iter().map(|(i, _)| *i).collect();
    // Sort by original position to preserve reading order
    keep_indices.sort();

    let kept_sentences: Vec<&str> = keep_indices.iter().map(|&i| sentences[i]).collect();
    let result_text = kept_sentences.join(" ");

    Ok(TextRankResult {
        text: result_text,
        sentences_kept: keep_count,
        sentences_dropped: sentences.len() - keep_count,
        scores,
    })
}

// ── PageRank ──────────────────────────────────────────────────────────────

/// Run the PageRank algorithm on a similarity matrix.
/// Returns a score vector where higher = more important.
fn pagerank(sim_matrix: &[Vec<f64>], damping: f64, max_iter: usize, epsilon: f64) -> Vec<f64> {
    let n = sim_matrix.len();
    if n == 0 {
        return vec![];
    }

    // Normalize rows (outgoing edge weights)
    let row_sums: Vec<f64> = sim_matrix
        .iter()
        .map(|row| row.iter().sum::<f64>().max(1e-10))
        .collect();

    let mut scores = vec![1.0 / n as f64; n];

    for _ in 0..max_iter {
        let mut new_scores = vec![(1.0 - damping) / n as f64; n];

        for i in 0..n {
            let mut sum = 0.0;
            for j in 0..n {
                if i != j && sim_matrix[j][i] > 0.0 {
                    sum += sim_matrix[j][i] / row_sums[j] * scores[j];
                }
            }
            new_scores[i] += damping * sum;
        }

        // Check convergence
        let diff: f64 = scores
            .iter()
            .zip(new_scores.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();

        scores = new_scores;

        if diff < epsilon {
            break;
        }
    }

    scores
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Split text into sentences using simple heuristics.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;

    for (i, ch) in text.char_indices() {
        if (ch == '.' || ch == '!' || ch == '?') && i + 1 < text.len() {
            let next = text[i + 1..].chars().next();
            if next == Some(' ') || next == Some('\n') {
                let sentence = text[start..=i].trim();
                if !sentence.is_empty() && sentence.split_whitespace().count() >= 3 {
                    sentences.push(sentence);
                }
                start = i + 1;
            }
        }
    }

    // Add remaining text as last sentence
    let remaining = text[start..].trim();
    if !remaining.is_empty() && remaining.split_whitespace().count() >= 3 {
        sentences.push(remaining);
    }

    sentences
}

/// Tokenize a sentence into a set of lowercase words.
fn tokenize_words(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2) // skip short words
        .map(|s| s.to_lowercase())
        .collect()
}

/// Compute word overlap similarity between two word sets.
/// Returns |A ∩ B| / (log|A| + log|B|) to normalize for sentence length.
fn word_overlap_similarity(
    a: &std::collections::HashSet<String>,
    b: &std::collections::HashSet<String>,
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let denominator = (a.len() as f64).ln() + (b.len() as f64).ln();
    if denominator <= 0.0 {
        0.0
    } else {
        intersection / denominator
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_text_unchanged() {
        let config = TextRankConfig::default();
        let result = textrank_compress("Hello world.", &config).unwrap();
        assert_eq!(result.sentences_dropped, 0);
    }

    #[test]
    fn test_compresses_long_prose() {
        let config = TextRankConfig { keep_ratio: 0.5, ..Default::default() };
        let text = "The system architecture is modular. Each component operates independently. \
                    The database layer handles persistence. The API layer handles routing. \
                    Error handling is centralized. Logging is structured. \
                    The deployment pipeline is automated. Tests run on every commit.";
        let result = textrank_compress(text, &config).unwrap();
        assert!(result.sentences_dropped > 0, "should drop some sentences");
        assert!(result.sentences_kept >= 2, "should keep at least min_sentences");
        assert!(result.text.len() < text.len(), "output should be shorter");
    }

    #[test]
    fn test_preserves_sentence_order() {
        let config = TextRankConfig { keep_ratio: 0.6, ..Default::default() };
        let text = "First sentence here. Second sentence here. Third sentence here. \
                    Fourth sentence here. Fifth sentence here.";
        let result = textrank_compress(text, &config).unwrap();
        // Kept sentences should be in original order
        let kept: Vec<&str> = result.text.split(". ").collect();
        for window in kept.windows(2) {
            let pos_a = text.find(window[0]).unwrap_or(0);
            let pos_b = text.find(window[1]).unwrap_or(0);
            assert!(pos_a <= pos_b, "sentences should be in original order");
        }
    }

    #[test]
    fn test_split_sentences() {
        let text = "Hello world today. This is a test. Another sentence here.";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 3);
    }

    #[test]
    fn test_split_sentences_skips_short() {
        let text = "Hi. Ok. This is a real sentence with enough words.";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 1); // only the long one
    }

    #[test]
    fn test_word_overlap_similarity() {
        let a: std::collections::HashSet<String> = ["hello", "world", "test"]
            .iter().map(|s| s.to_string()).collect();
        let b: std::collections::HashSet<String> = ["hello", "world", "other"]
            .iter().map(|s| s.to_string()).collect();
        let sim = word_overlap_similarity(&a, &b);
        assert!(sim > 0.0, "overlapping sets should have positive similarity");
    }

    #[test]
    fn test_word_overlap_disjoint() {
        let a: std::collections::HashSet<String> = ["aaa", "bbb"]
            .iter().map(|s| s.to_string()).collect();
        let b: std::collections::HashSet<String> = ["ccc", "ddd"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(word_overlap_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_pagerank_converges() {
        let sim = vec![
            vec![0.0, 0.5, 0.3],
            vec![0.5, 0.0, 0.2],
            vec![0.3, 0.2, 0.0],
        ];
        let scores = pagerank(&sim, 0.85, 50, 1e-4);
        assert_eq!(scores.len(), 3);
        // All scores should be positive
        for s in &scores {
            assert!(*s > 0.0, "score should be positive: {s}");
        }
    }

    #[test]
    fn test_pagerank_empty() {
        let scores = pagerank(&[], 0.85, 50, 1e-4);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_min_sentences_respected() {
        let config = TextRankConfig {
            keep_ratio: 0.1, // would keep ~1 sentence
            min_sentences: 3,
            ..Default::default()
        };
        let text = "First important sentence here. Second important sentence here. \
                    Third important sentence here. Fourth important sentence here. \
                    Fifth important sentence here.";
        let result = textrank_compress(text, &config).unwrap();
        assert!(result.sentences_kept >= 3, "should keep at least min_sentences");
    }

    use proptest::prelude::*;

    proptest! {
        /// TextRank never produces empty output from non-empty input.
        #[test]
        fn prop_textrank_non_empty(
            text in "[A-Z][a-z]{5,20}\\. [A-Z][a-z]{5,20}\\. [A-Z][a-z]{5,20}\\."
        ) {
            let config = TextRankConfig::default();
            let result = textrank_compress(&text, &config).unwrap();
            prop_assert!(!result.text.is_empty());
        }

        /// Scores are always non-negative.
        #[test]
        fn prop_scores_non_negative(
            text in "[A-Z][a-z]{5,20}\\. [A-Z][a-z]{5,20}\\. [A-Z][a-z]{5,20}\\."
        ) {
            let config = TextRankConfig::default();
            let result = textrank_compress(&text, &config).unwrap();
            for s in &result.scores {
                prop_assert!(*s >= 0.0, "score should be non-negative: {s}");
            }
        }
    }
}
