use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SqzError};
use crate::preset::Preset;
use crate::types::ToolId;

/// A tool definition with an id, name, and description.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolDefinition {
    pub id: ToolId,
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    #[serde(default)]
    pub input_schema: serde_json::Value,
    /// Optional JSON Schema describing the structure of a tool's structured
    /// output (see MCP spec `outputSchema`).
    ///
    /// Per the MCP 2025-06-18 specification, when present this schema's root
    /// `type` MUST be `"object"` and servers MUST return `structuredContent`
    /// matching the schema. Leave as `Value::Null` (the default) when the
    /// tool returns plain text via the `content` field — strict clients such
    /// as OpenCode validate this and will disable the whole server if a
    /// scalar-typed schema is advertised (reported in issue #5).
    #[serde(default)]
    pub output_schema: serde_json::Value,
    /// Description of what sqz does to this tool's output.
    #[serde(default)]
    pub compression_transforms: Vec<String>,
}

/// Bag-of-words representation: a set of lowercase word tokens.
type BagOfWords = HashSet<String>;

/// Tokenize a string into a bag of lowercase words, splitting on whitespace and punctuation.
fn tokenize(text: &str) -> BagOfWords {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// Tokenize into a frequency map (term → count) for TF-IDF.
fn tokenize_tf(text: &str) -> HashMap<String, u32> {
    let mut freq = HashMap::new();
    for word in text.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()) {
        *freq.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    freq
}

/// Jaccard similarity between two bags of words: |A ∩ B| / |A ∪ B|.
/// Returns 0.0 if both sets are empty.
fn jaccard(a: &BagOfWords, b: &BagOfWords) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

// ── TF-IDF + Cosine Similarity ────────────────────────────────────────────────

/// Sparse TF-IDF vector: term → weight.
type TfIdfVector = HashMap<String, f64>;

/// Compute TF-IDF weight for a term in a document.
///
/// TF(t,d) = count(t,d) / |d|
/// IDF(t) = ln(N / DF(t))
/// TF-IDF(t,d) = TF(t,d) × IDF(t)
fn compute_tfidf(
    term_freq: &HashMap<String, u32>,
    doc_freq: &HashMap<String, u32>,
    total_docs: u32,
) -> TfIdfVector {
    let doc_len: u32 = term_freq.values().sum();
    if doc_len == 0 {
        return HashMap::new();
    }

    let mut vector = HashMap::new();
    for (term, &count) in term_freq {
        let tf = count as f64 / doc_len as f64;
        let df = doc_freq.get(term).copied().unwrap_or(1).max(1);
        let idf = (total_docs as f64 / df as f64).ln();
        let weight = tf * idf;
        if weight > 0.0 {
            vector.insert(term.clone(), weight);
        }
    }
    vector
}

/// Cosine similarity between two sparse TF-IDF vectors.
///
/// cosine(a, b) = (a · b) / (||a|| × ||b||)
fn cosine_similarity(a: &TfIdfVector, b: &TfIdfVector) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let dot: f64 = a
        .iter()
        .filter_map(|(term, &wa)| b.get(term).map(|&wb| wa * wb))
        .sum();

    let norm_a: f64 = a.values().map(|w| w * w).sum::<f64>().sqrt();
    let norm_b: f64 = b.values().map(|w| w * w).sum::<f64>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

// ── ToolSelector ──────────────────────────────────────────────────────────────

/// Selects 3–5 relevant tools per task using TF-IDF + cosine similarity,
/// with Jaccard as a fallback for very short descriptions.
///
/// # Selection rules
/// - Compute TF-IDF vectors for each tool description at registration time.
/// - At query time, compute the TF-IDF vector for the intent and score via cosine.
/// - Sort tools by descending similarity score.
/// - Return between 3 and min(5, tool_count) tools.
/// - If no tool has similarity > threshold, return the default tool set instead.
pub struct ToolSelector {
    /// Bag-of-words for each registered tool description (kept for backward compat).
    bags: HashMap<ToolId, BagOfWords>,
    /// TF-IDF vectors for each registered tool description.
    tfidf_vectors: HashMap<ToolId, TfIdfVector>,
    /// Document frequency: term → number of tools containing that term.
    doc_freq: HashMap<String, u32>,
    /// Total number of registered tools (for IDF computation).
    total_docs: u32,
    /// Ordered list of registered tool ids (preserves insertion order for determinism).
    tool_ids: Vec<ToolId>,
    /// Similarity threshold below which we fall back to defaults.
    threshold: f64,
    /// Default tool ids returned when confidence is low.
    default_tools: Vec<ToolId>,
    /// Raw term frequencies per tool (needed for recomputing TF-IDF on updates).
    term_freqs: HashMap<ToolId, HashMap<String, u32>>,
}

impl ToolSelector {
    /// Create a new `ToolSelector` from a `Preset`.
    ///
    /// `model_path` is accepted for API compatibility but is unused.
    pub fn new(_model_path: &std::path::Path, preset: &Preset) -> Result<Self> {
        let threshold = preset.tool_selection.similarity_threshold;
        let default_tools = preset.tool_selection.default_tools.clone();
        Ok(Self {
            bags: HashMap::new(),
            tfidf_vectors: HashMap::new(),
            doc_freq: HashMap::new(),
            total_docs: 0,
            tool_ids: Vec::new(),
            threshold,
            default_tools,
            term_freqs: HashMap::new(),
        })
    }

    /// Register a slice of tools, computing TF-IDF vectors for each description.
    pub fn register_tools(&mut self, tools: &[ToolDefinition]) -> Result<()> {
        // First pass: collect term frequencies and update doc_freq
        for tool in tools {
            let bag = tokenize(&tool.description);
            let tf = tokenize_tf(&tool.description);

            // Update document frequency for new terms
            if !self.bags.contains_key(&tool.id) {
                self.tool_ids.push(tool.id.clone());
                self.total_docs += 1;
                for term in tf.keys() {
                    *self.doc_freq.entry(term.clone()).or_insert(0) += 1;
                }
            } else {
                // Tool already registered — update doc_freq (remove old, add new)
                if let Some(old_tf) = self.term_freqs.get(&tool.id) {
                    for term in old_tf.keys() {
                        if let Some(count) = self.doc_freq.get_mut(term) {
                            *count = count.saturating_sub(1);
                        }
                    }
                }
                for term in tf.keys() {
                    *self.doc_freq.entry(term.clone()).or_insert(0) += 1;
                }
            }

            self.bags.insert(tool.id.clone(), bag);
            self.term_freqs.insert(tool.id.clone(), tf);
        }

        // Second pass: recompute all TF-IDF vectors (IDF changed)
        self.recompute_tfidf();
        Ok(())
    }

    /// Recompute TF-IDF vectors for all registered tools.
    fn recompute_tfidf(&mut self) {
        for id in &self.tool_ids {
            if let Some(tf) = self.term_freqs.get(id) {
                let vector = compute_tfidf(tf, &self.doc_freq, self.total_docs);
                self.tfidf_vectors.insert(id.clone(), vector);
            }
        }
    }

    /// Select between 3 and min(5, tool_count) tools whose descriptions best match `intent`.
    ///
    /// Uses TF-IDF + cosine similarity for scoring. Falls back to Jaccard for
    /// very short intents (< 3 words) where TF-IDF has insufficient signal.
    ///
    /// Returns the default tool set when no tool exceeds the similarity threshold.
    pub fn select(&self, intent: &str, max_tools: usize) -> Result<Vec<ToolId>> {
        let tool_count = self.tool_ids.len();
        if tool_count == 0 {
            return Ok(self.default_tools.clone());
        }

        let intent_words: Vec<&str> = intent
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();

        // Use TF-IDF + cosine for intents with enough signal, Jaccard for short ones
        let use_tfidf = intent_words.len() >= 3 && self.total_docs >= 2;

        let mut scored: Vec<(f64, &ToolId)> = if use_tfidf {
            let intent_tf = tokenize_tf(intent);
            let intent_vector = compute_tfidf(&intent_tf, &self.doc_freq, self.total_docs);

            self.tool_ids
                .iter()
                .map(|id| {
                    let score = self
                        .tfidf_vectors
                        .get(id)
                        .map(|v| cosine_similarity(&intent_vector, v))
                        .unwrap_or(0.0);
                    (score, id)
                })
                .collect()
        } else {
            // Fallback to Jaccard for short intents
            let intent_bag = tokenize(intent);
            self.tool_ids
                .iter()
                .map(|id| {
                    let bag = self.bags.get(id).expect("bag must exist for registered tool");
                    let score = jaccard(&intent_bag, bag);
                    (score, id)
                })
                .collect()
        };

        // Sort descending by score, then ascending by id for determinism on ties.
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(b.1))
        });

        // Check whether any tool exceeds the threshold.
        let best_score = scored.first().map(|(s, _)| *s).unwrap_or(0.0);
        if best_score < self.threshold {
            return Ok(self.default_tools.clone());
        }

        // Cardinality: return between 3 and min(max_tools, 5, tool_count) tools.
        let upper = max_tools.min(5).min(tool_count);
        let lower = 3_usize.min(tool_count);
        let count = upper.max(lower);

        let result = scored
            .into_iter()
            .take(count)
            .map(|(_, id)| id.clone())
            .collect();

        Ok(result)
    }

    /// Re-embed a single tool on description change.
    pub fn update_tool(&mut self, tool: &ToolDefinition) -> Result<()> {
        if !self.bags.contains_key(&tool.id) {
            return Err(SqzError::Other(format!(
                "tool '{}' is not registered; use register_tools first",
                tool.id
            )));
        }

        // Update doc_freq: remove old terms, add new
        if let Some(old_tf) = self.term_freqs.get(&tool.id) {
            for term in old_tf.keys() {
                if let Some(count) = self.doc_freq.get_mut(term) {
                    *count = count.saturating_sub(1);
                }
            }
        }

        let bag = tokenize(&tool.description);
        let tf = tokenize_tf(&tool.description);
        for term in tf.keys() {
            *self.doc_freq.entry(term.clone()).or_insert(0) += 1;
        }

        self.bags.insert(tool.id.clone(), bag);
        self.term_freqs.insert(tool.id.clone(), tf);

        // Recompute all vectors since IDF may have changed
        self.recompute_tfidf();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::Path;

    fn make_preset_with_threshold(threshold: f64, default_tools: Vec<String>) -> Preset {
        let mut p = Preset::default();
        p.tool_selection.similarity_threshold = threshold;
        p.tool_selection.default_tools = default_tools;
        p
    }

    fn make_tools(n: usize) -> Vec<ToolDefinition> {
        (0..n)
            .map(|i| ToolDefinition {
                id: format!("tool_{i}"),
                name: format!("Tool {i}"),
                description: format!(
                    "This tool performs operation number {i} for task category alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega {i}"
                ),
                ..Default::default()
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_tokenize_basic() {
        let bag = tokenize("hello world foo");
        assert!(bag.contains("hello"));
        assert!(bag.contains("world"));
        assert!(bag.contains("foo"));
    }

    #[test]
    fn test_tokenize_punctuation() {
        let bag = tokenize("read_file: reads a file.");
        assert!(bag.contains("read"));
        assert!(bag.contains("file"));
        assert!(bag.contains("reads"));
        assert!(bag.contains("a"));
    }

    #[test]
    fn test_jaccard_identical() {
        let a = tokenize("read file");
        let b = tokenize("read file");
        assert!((jaccard(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = tokenize("alpha beta");
        let b = tokenize("gamma delta");
        assert!((jaccard(&a, &b)).abs() < 1e-9);
    }

    #[test]
    fn test_select_returns_between_3_and_5_for_large_set() {
        let preset = make_preset_with_threshold(0.0, vec![]);
        let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
        let tools = make_tools(10);
        selector.register_tools(&tools).unwrap();

        let result = selector.select("operation task alpha beta", 5).unwrap();
        assert!(result.len() >= 3, "expected >= 3, got {}", result.len());
        assert!(result.len() <= 5, "expected <= 5, got {}", result.len());
    }

    #[test]
    fn test_select_returns_at_most_tool_count_for_small_set() {
        let preset = make_preset_with_threshold(0.0, vec![]);
        let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
        let tools = make_tools(2);
        selector.register_tools(&tools).unwrap();

        let result = selector.select("operation task", 5).unwrap();
        assert!(result.len() <= 2, "expected <= 2, got {}", result.len());
    }

    #[test]
    fn test_fallback_to_defaults_on_low_confidence() {
        let defaults = vec!["default_a".to_string(), "default_b".to_string()];
        // threshold = 1.0 means nothing will ever exceed it (Jaccard < 1 unless identical)
        let preset = make_preset_with_threshold(1.0, defaults.clone());
        let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
        let tools = make_tools(5);
        selector.register_tools(&tools).unwrap();

        let result = selector.select("completely unrelated xyz", 5).unwrap();
        assert_eq!(result, defaults);
    }

    #[test]
    fn test_update_tool_changes_embedding() {
        let preset = make_preset_with_threshold(0.0, vec![]);
        let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
        let tools = vec![ToolDefinition {
            id: "t1".to_string(),
            name: "T1".to_string(),
            description: "alpha beta gamma".to_string(),
            ..Default::default()
        }];
        selector.register_tools(&tools).unwrap();

        let updated = ToolDefinition {
            id: "t1".to_string(),
            name: "T1".to_string(),
            description: "delta epsilon zeta".to_string(),
            ..Default::default()
        };
        selector.update_tool(&updated).unwrap();

        let bag = selector.bags.get("t1").unwrap();
        assert!(bag.contains("delta"));
        assert!(!bag.contains("alpha"));
    }

    // ── TF-IDF specific tests ─────────────────────────────────────────────

    #[test]
    fn test_tfidf_discriminative_ranking() {
        // TF-IDF should rank a tool with a rare matching term higher than
        // one with only common terms.
        let preset = make_preset_with_threshold(0.0, vec![]);
        let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();

        let tools = vec![
            ToolDefinition {
                id: "generic".to_string(),
                name: "Generic".to_string(),
                description: "this tool performs common operations on files and data".to_string(),
                ..Default::default()
            },
            ToolDefinition {
                id: "specific".to_string(),
                name: "Specific".to_string(),
                description: "this tool performs kubernetes pod deployment orchestration".to_string(),
                ..Default::default()
            },
            ToolDefinition {
                id: "other".to_string(),
                name: "Other".to_string(),
                description: "this tool handles database migration and schema updates".to_string(),
                ..Default::default()
            },
        ];
        selector.register_tools(&tools).unwrap();

        // "kubernetes deployment" should rank "specific" highest because
        // "kubernetes" and "deployment" are rare (discriminative) terms
        let result = selector
            .select("deploy kubernetes pods to the cluster", 5)
            .unwrap();
        assert_eq!(
            result[0], "specific",
            "TF-IDF should rank the tool with rare matching terms first"
        );
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let mut a = HashMap::new();
        a.insert("hello".to_string(), 1.0);
        a.insert("world".to_string(), 2.0);
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let mut a = HashMap::new();
        a.insert("hello".to_string(), 1.0);
        let mut b = HashMap::new();
        b.insert("world".to_string(), 1.0);
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let a: TfIdfVector = HashMap::new();
        let b: TfIdfVector = HashMap::new();
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_tfidf_vectors_populated() {
        let preset = make_preset_with_threshold(0.0, vec![]);
        let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
        let tools = make_tools(5);
        selector.register_tools(&tools).unwrap();
        assert_eq!(selector.tfidf_vectors.len(), 5);
        assert_eq!(selector.total_docs, 5);
    }

    #[test]
    fn test_update_tool_unregistered_returns_error() {
        let preset = make_preset_with_threshold(0.0, vec![]);
        let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
        let result = selector.update_tool(&ToolDefinition {
            id: "nonexistent".to_string(),
            name: "X".to_string(),
            description: "desc".to_string(),
            ..Default::default()
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_tool_set_returns_defaults() {
        let defaults = vec!["fallback".to_string()];
        let preset = make_preset_with_threshold(0.0, defaults.clone());
        let selector = ToolSelector::new(Path::new(""), &preset).unwrap();
        let result = selector.select("anything", 5).unwrap();
        assert_eq!(result, defaults);
    }

    // -----------------------------------------------------------------------
    // Property 2: Tool selection cardinality
    // Validates: Requirements 3.1, 26.3
    // -----------------------------------------------------------------------

    /// Strategy: generate a tool count in [5, 20] and an intent string.
    fn arb_tool_count_and_intent() -> impl Strategy<Value = (usize, String)> {
        (5usize..=20usize, "[a-z ]{5,40}".prop_map(|s| s.trim().to_string()))
    }

    /// Strategy: generate a small tool count in [1, 4] and an intent string.
    fn arb_small_tool_count_and_intent() -> impl Strategy<Value = (usize, String)> {
        (1usize..=4usize, "[a-z ]{5,40}".prop_map(|s| s.trim().to_string()))
    }

    proptest! {
        /// **Validates: Requirements 3.1, 26.3**
        ///
        /// Property 2: Tool selection cardinality.
        ///
        /// For any intent string and any tool set of size >= 5, the ToolSelector
        /// SHALL return between 3 and 5 tools (inclusive).
        ///
        /// For tool sets smaller than 5, it SHALL return at most the size of the
        /// tool set.
        #[test]
        fn prop_tool_selection_cardinality_large(
            (tool_count, intent) in arb_tool_count_and_intent()
        ) {
            // Use threshold = 0.0 so all tools are eligible (no fallback to defaults).
            let preset = make_preset_with_threshold(0.0, vec![]);
            let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
            let tools = make_tools(tool_count);
            selector.register_tools(&tools).unwrap();

            let result = selector.select(&intent, 5).unwrap();

            prop_assert!(
                result.len() >= 3,
                "expected >= 3 tools, got {} (tool_count={}, intent='{}')",
                result.len(), tool_count, intent
            );
            prop_assert!(
                result.len() <= 5,
                "expected <= 5 tools, got {} (tool_count={}, intent='{}')",
                result.len(), tool_count, intent
            );
        }

        /// **Validates: Requirements 3.1, 26.3**
        ///
        /// Property 2b: For tool sets smaller than 5, ToolSelector returns at most
        /// the size of the tool set.
        #[test]
        fn prop_tool_selection_cardinality_small(
            (tool_count, intent) in arb_small_tool_count_and_intent()
        ) {
            let preset = make_preset_with_threshold(0.0, vec![]);
            let mut selector = ToolSelector::new(Path::new(""), &preset).unwrap();
            let tools = make_tools(tool_count);
            selector.register_tools(&tools).unwrap();

            let result = selector.select(&intent, 5).unwrap();

            prop_assert!(
                result.len() <= tool_count,
                "expected <= {} tools, got {} (intent='{}')",
                tool_count, result.len(), intent
            );
        }
    }
}
