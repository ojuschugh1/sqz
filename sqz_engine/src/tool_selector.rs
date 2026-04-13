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
    /// JSON Schema describing the structure of the compressed output.
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

/// Selects 3–5 relevant tools per task using TF-IDF-style word-overlap (Jaccard) similarity.
///
/// # Selection rules
/// - Compute Jaccard similarity between the intent query and each tool description.
/// - Sort tools by descending similarity score.
/// - Return between 3 and min(5, tool_count) tools.
/// - If no tool has similarity > threshold, return the default tool set instead.
pub struct ToolSelector {
    /// Bag-of-words for each registered tool description.
    bags: HashMap<ToolId, BagOfWords>,
    /// Ordered list of registered tool ids (preserves insertion order for determinism).
    tool_ids: Vec<ToolId>,
    /// Similarity threshold below which we fall back to defaults.
    threshold: f64,
    /// Default tool ids returned when confidence is low.
    default_tools: Vec<ToolId>,
}

impl ToolSelector {
    /// Create a new `ToolSelector` from a `Preset`.
    ///
    /// `model_path` is accepted for API compatibility but is unused because we use
    /// a bag-of-words Jaccard similarity approach rather than a neural embedding model.
    pub fn new(_model_path: &std::path::Path, preset: &Preset) -> Result<Self> {
        let threshold = preset.tool_selection.similarity_threshold;
        let default_tools = preset.tool_selection.default_tools.clone();
        Ok(Self {
            bags: HashMap::new(),
            tool_ids: Vec::new(),
            threshold,
            default_tools,
        })
    }

    /// Register a slice of tools, computing bag-of-words for each description.
    pub fn register_tools(&mut self, tools: &[ToolDefinition]) -> Result<()> {
        for tool in tools {
            let bag = tokenize(&tool.description);
            if !self.bags.contains_key(&tool.id) {
                self.tool_ids.push(tool.id.clone());
            }
            self.bags.insert(tool.id.clone(), bag);
        }
        Ok(())
    }

    /// Select between 3 and min(5, tool_count) tools whose descriptions best match `intent`.
    ///
    /// Returns the default tool set when no tool exceeds the similarity threshold.
    pub fn select(&self, intent: &str, max_tools: usize) -> Result<Vec<ToolId>> {
        let tool_count = self.tool_ids.len();
        if tool_count == 0 {
            return Ok(self.default_tools.clone());
        }

        let intent_bag = tokenize(intent);

        // Score every registered tool.
        let mut scored: Vec<(f64, &ToolId)> = self
            .tool_ids
            .iter()
            .map(|id| {
                let bag = self.bags.get(id).expect("bag must exist for registered tool");
                let score = jaccard(&intent_bag, bag);
                (score, id)
            })
            .collect();

        // Sort descending by score, then ascending by id for determinism on ties.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(b.1)));

        // Check whether any tool exceeds the threshold.
        // We use strict less-than so that a score equal to the threshold is still
        // considered "confident enough" (threshold = 0.0 means always select).
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
        let bag = tokenize(&tool.description);
        self.bags.insert(tool.id.clone(), bag);
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
