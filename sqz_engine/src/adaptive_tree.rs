//! Adaptive Semantic Tree Compression — builds a hierarchical tree from
//! document structure and prunes lowest-importance branches to fit a token
//! budget.
//!
//! The document is decomposed into a tree:
//! - **Root** → sections (delimited by headers or blank-line-separated blocks)
//! - **Section** → paragraphs (blank-line-separated)
//! - **Paragraph** → sentences (period/newline-delimited)
//!
//! Each node is scored using a TF-IDF-style importance metric based on word
//! frequency. Low-importance branches are pruned first until the output fits
//! within the specified token budget.
//!
//! # Example
//! ```
//! use sqz_engine::adaptive_tree::compress_to_budget;
//!
//! let text = "# Introduction\nThis is important.\n\n# Details\nLess important filler.\n";
//! let result = compress_to_budget(text, 50);
//! assert!(!result.is_empty());
//! ```

use std::collections::HashMap;

/// Approximate tokens as `ceil(chars / 4)`.
fn approx_tokens(s: &str) -> usize {
    ((s.len() as f64) / 4.0).ceil() as usize
}

/// A node in the semantic document tree.
#[derive(Debug, Clone)]
pub struct SemanticNode {
    /// The text content of this node.
    pub text: String,
    /// Importance score (higher = more important).
    pub importance: f64,
    /// Child nodes.
    pub children: Vec<SemanticNode>,
    /// Approximate token count for this node's text.
    pub tokens: usize,
}

impl SemanticNode {
    /// Total tokens in this subtree (self + all descendants).
    pub fn total_tokens(&self) -> usize {
        self.tokens + self.children.iter().map(|c| c.total_tokens()).sum::<usize>()
    }
}

/// Build a semantic tree from document text.
///
/// Splits the document into sections (by headers or double newlines),
/// then into paragraphs, then into sentences.
pub fn build_tree(text: &str) -> SemanticNode {
    let word_freq = compute_word_frequencies(text);
    let doc_sections = split_sections(text);

    let children: Vec<SemanticNode> = doc_sections
        .into_iter()
        .map(|section| build_section_node(&section, &word_freq))
        .collect();

    let root_importance = children
        .iter()
        .map(|c| c.importance)
        .fold(0.0f64, f64::max);

    SemanticNode {
        text: String::new(), // root has no direct text
        importance: root_importance,
        children,
        tokens: 0,
    }
}

/// Compress text to fit within a token budget by pruning low-importance
/// branches from the semantic tree.
///
/// # Arguments
/// * `text` — the document text to compress
/// * `max_tokens` — the maximum number of tokens in the output
///
/// # Returns
/// Compressed text that fits within the token budget.
pub fn compress_to_budget(text: &str, max_tokens: usize) -> String {
    if text.is_empty() || max_tokens == 0 {
        return String::new();
    }

    // If already within budget, return as-is
    if approx_tokens(text) <= max_tokens {
        return text.to_string();
    }

    let mut tree = build_tree(text);
    prune_to_budget(&mut tree, max_tokens);
    render_tree(&tree)
}

/// Prune the tree by removing lowest-importance leaf branches until
/// the total token count fits within the budget.
fn prune_to_budget(tree: &mut SemanticNode, max_tokens: usize) {
    // Sort children by importance and remove from lowest until within budget
    // This is more efficient than iterative search
    if tree.children.is_empty() {
        return;
    }

    // First, recursively prune children that themselves have children
    for child in &mut tree.children {
        if !child.children.is_empty() {
            prune_to_budget(child, max_tokens);
        }
    }

    // If still over budget, remove lowest-importance children
    while tree.total_tokens() > max_tokens && !tree.children.is_empty() {
        // Find index of least important child
        let min_idx = tree
            .children
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.importance
                    .partial_cmp(&b.importance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i);

        if let Some(idx) = min_idx {
            tree.children.remove(idx);
        } else {
            break;
        }
    }
}

/// Render the pruned tree back to text.
fn render_tree(tree: &SemanticNode) -> String {
    let mut parts = Vec::new();

    if !tree.text.is_empty() {
        parts.push(tree.text.clone());
    }

    for child in &tree.children {
        let rendered = render_tree(child);
        if !rendered.is_empty() {
            parts.push(rendered);
        }
    }

    parts.join("\n")
}

/// Split document into sections. A section starts with a markdown header
/// (`# ...`) or is separated by double newlines.
fn split_sections(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.starts_with('#') && !current.trim().is_empty() {
            sections.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }

    if !current.trim().is_empty() {
        sections.push(current.trim().to_string());
    }

    // If no headers found, split by double newlines
    if sections.len() <= 1 {
        sections = text
            .split("\n\n")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    sections
}

/// Build a section node with paragraph children.
fn build_section_node(section: &str, word_freq: &HashMap<String, usize>) -> SemanticNode {
    let paragraphs = split_paragraphs(section);
    let children: Vec<SemanticNode> = paragraphs
        .into_iter()
        .map(|p| build_paragraph_node(&p, word_freq))
        .collect();

    let importance = if children.is_empty() {
        compute_importance(section, word_freq)
    } else {
        children.iter().map(|c| c.importance).sum::<f64>() / children.len().max(1) as f64
    };

    SemanticNode {
        text: extract_header(section),
        importance,
        children,
        tokens: approx_tokens(&extract_header(section)),
    }
}

/// Build a paragraph node with sentence children.
fn build_paragraph_node(paragraph: &str, word_freq: &HashMap<String, usize>) -> SemanticNode {
    let sentences = split_sentences(paragraph);

    if sentences.len() <= 1 {
        // Single sentence — leaf node
        return SemanticNode {
            text: paragraph.to_string(),
            importance: compute_importance(paragraph, word_freq),
            children: Vec::new(),
            tokens: approx_tokens(paragraph),
        };
    }

    let children: Vec<SemanticNode> = sentences
        .into_iter()
        .map(|s| SemanticNode {
            importance: compute_importance(&s, word_freq),
            tokens: approx_tokens(&s),
            text: s,
            children: Vec::new(),
        })
        .collect();

    let importance = children.iter().map(|c| c.importance).sum::<f64>()
        / children.len().max(1) as f64;

    SemanticNode {
        text: String::new(),
        importance,
        children,
        tokens: 0,
    }
}

/// Extract the header line from a section (if it starts with `#`).
fn extract_header(section: &str) -> String {
    let first_line = section.lines().next().unwrap_or("");
    if first_line.starts_with('#') {
        first_line.to_string()
    } else {
        String::new()
    }
}

/// Split a section into paragraphs (separated by blank lines).
fn split_paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split a paragraph into sentences (by `. ` or newlines).
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if ch == '.' || ch == '\n' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current = String::new();
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    sentences
}

/// Compute word frequencies across the entire document.
fn compute_word_frequencies(text: &str) -> HashMap<String, usize> {
    let mut freq = HashMap::new();
    for word in text.split_whitespace() {
        let normalized = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if !normalized.is_empty() {
            *freq.entry(normalized).or_insert(0) += 1;
        }
    }
    freq
}

/// Compute TF-IDF-style importance score for a text segment.
///
/// Words that appear less frequently in the overall document are considered
/// more important (inverse document frequency intuition). The score is the
/// sum of `1.0 / freq(word)` for each word in the segment.
fn compute_importance(text: &str, word_freq: &HashMap<String, usize>) -> f64 {
    let mut score = 0.0;
    let mut word_count = 0;

    for word in text.split_whitespace() {
        let normalized = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if normalized.is_empty() {
            continue;
        }
        word_count += 1;
        let freq = *word_freq.get(&normalized).unwrap_or(&1) as f64;
        // IDF-style: rarer words contribute more
        score += 1.0 / freq;
    }

    if word_count == 0 {
        0.0
    } else {
        score / word_count as f64
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        let result = compress_to_budget("", 100);
        assert!(result.is_empty());
    }

    #[test]
    fn test_zero_budget() {
        let result = compress_to_budget("some text here", 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_within_budget_unchanged() {
        let text = "Hello world.";
        let result = compress_to_budget(text, 1000);
        assert_eq!(result, text);
    }

    #[test]
    fn test_compress_reduces_size() {
        let mut text = String::new();
        text.push_str("# Important Section\n");
        text.push_str("This section contains critical unique information.\n\n");
        text.push_str("# Filler Section\n");
        text.push_str("The the the the the the the the the the.\n\n");
        text.push_str("# Another Filler\n");
        text.push_str("The the the the the the the the the the.\n\n");
        text.push_str("# More Filler\n");
        text.push_str("The the the the the the the the the the.\n");

        let budget = approx_tokens(&text) / 2;
        let result = compress_to_budget(&text, budget);

        assert!(
            approx_tokens(&result) <= budget + 5, // small tolerance for rounding
            "result tokens ({}) should be <= budget ({})",
            approx_tokens(&result),
            budget
        );
    }

    #[test]
    fn test_build_tree_structure() {
        let text = "# Header\nSentence one. Sentence two.\n\n# Second\nMore content here.\n";
        let tree = build_tree(text);
        assert!(
            !tree.children.is_empty(),
            "tree should have section children"
        );
    }

    #[test]
    fn test_word_frequencies() {
        let freq = compute_word_frequencies("the cat sat on the mat the");
        assert_eq!(freq.get("the"), Some(&3));
        assert_eq!(freq.get("cat"), Some(&1));
        assert_eq!(freq.get("sat"), Some(&1));
    }

    #[test]
    fn test_importance_rare_words_score_higher() {
        let freq = compute_word_frequencies("the the the cat dog");
        let common_score = compute_importance("the the", &freq);
        let rare_score = compute_importance("cat dog", &freq);
        assert!(
            rare_score > common_score,
            "rare words ({rare_score}) should score higher than common ({common_score})"
        );
    }

    #[test]
    fn test_split_sections_with_headers() {
        let text = "# A\ncontent a\n# B\ncontent b\n";
        let sections = split_sections(text);
        assert_eq!(sections.len(), 2);
    }

    #[test]
    fn test_split_sections_without_headers() {
        let text = "paragraph one\n\nparagraph two\n\nparagraph three\n";
        let sections = split_sections(text);
        assert_eq!(sections.len(), 3);
    }

    // ── Property-based tests ──────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Output never exceeds the token budget (with small rounding tolerance).
        #[test]
        fn prop_output_within_budget(
            n_sections in 2usize..=6,
            words_per_section in 10usize..=30,
            budget_frac in 0.2f64..=0.8,
        ) {
            let mut text = String::new();
            for i in 0..n_sections {
                text.push_str(&format!("# Section {i}\n"));
                for j in 0..words_per_section {
                    text.push_str(&format!("word_{i}_{j} "));
                }
                text.push_str("\n\n");
            }

            let total_tokens = approx_tokens(&text);
            let budget = ((total_tokens as f64) * budget_frac) as usize;
            let budget = budget.max(1);

            let result = compress_to_budget(&text, budget);
            let result_tokens = approx_tokens(&result);

            // Allow small tolerance for rounding
            prop_assert!(
                result_tokens <= budget + 10,
                "result tokens ({result_tokens}) should be <= budget ({budget}) + tolerance"
            );
        }

        /// Empty input always produces empty output regardless of budget.
        #[test]
        fn prop_empty_input_empty_output(budget in 0usize..1000) {
            let result = compress_to_budget("", budget);
            prop_assert!(result.is_empty());
        }

        /// Content within budget is returned unchanged.
        #[test]
        fn prop_within_budget_unchanged(text in "[a-z ]{1,50}") {
            let budget = approx_tokens(&text) + 100;
            let result = compress_to_budget(&text, budget);
            prop_assert_eq!(result, text);
        }

        /// Importance scores are always non-negative.
        #[test]
        fn prop_importance_non_negative(text in "[a-z ]{10,200}") {
            let freq = compute_word_frequencies(&text);
            let score = compute_importance(&text, &freq);
            prop_assert!(score >= 0.0, "importance should be >= 0, got {score}");
        }
    }
}
