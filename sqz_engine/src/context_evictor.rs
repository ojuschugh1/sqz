/// Proactive Context Eviction — gives the agent explicit control over
/// context management by summarizing and evicting stale content.
///
/// Unlike passive compaction (which the LLM harness does unpredictably),
/// proactive eviction lets sqz decide what to keep and what to summarize
/// based on recency, access frequency, and content importance.
///
/// The agent can call `sqz compact` to trigger eviction, or the engine
/// can suggest eviction when the budget approaches the ceiling.

use crate::error::Result;

/// Configuration for context eviction.
#[derive(Debug, Clone)]
pub struct EvictionConfig {
    /// Maximum age (in turns) before content is eligible for eviction.
    /// Default: 30.
    pub max_age_turns: u64,
    /// Fraction of context to evict when triggered (0.0–1.0).
    /// Default: 0.3 (evict the oldest 30%).
    pub eviction_ratio: f64,
    /// Minimum number of items to always keep (most recent).
    /// Default: 5.
    pub min_keep: usize,
}

impl Default for EvictionConfig {
    fn default() -> Self {
        Self {
            max_age_turns: 30,
            eviction_ratio: 0.3,
            min_keep: 5,
        }
    }
}

/// A tracked context item with metadata for eviction decisions.
#[derive(Debug, Clone)]
pub struct ContextItem {
    /// Unique identifier (e.g., file path or content hash prefix).
    pub id: String,
    /// The content (or a summary of it).
    pub content: String,
    /// Turn number when this item was last accessed.
    pub last_accessed_turn: u64,
    /// Number of times this item has been accessed.
    pub access_count: u32,
    /// Estimated token count.
    pub tokens: u32,
    /// Whether this item is pinned (protected from eviction).
    pub pinned: bool,
}

/// Result of an eviction pass.
#[derive(Debug, Clone)]
pub struct EvictionResult {
    /// Items that were kept (most recent / most accessed / pinned).
    pub kept: Vec<ContextItem>,
    /// Items that were evicted (summarized to one-line references).
    pub evicted: Vec<EvictedItem>,
    /// Total tokens before eviction.
    pub tokens_before: u32,
    /// Total tokens after eviction.
    pub tokens_after: u32,
    /// A compact summary of what was evicted, suitable for injecting
    /// into the context so the agent knows what's no longer available.
    pub eviction_summary: String,
}

/// A summarized evicted item.
#[derive(Debug, Clone)]
pub struct EvictedItem {
    /// Original item id.
    pub id: String,
    /// One-line summary of the evicted content.
    pub summary: String,
    /// Original token count.
    pub original_tokens: u32,
}

/// Run a proactive eviction pass on a set of context items.
///
/// Items are scored by a combination of recency and access frequency.
/// The lowest-scoring items (up to `eviction_ratio` of the total) are
/// evicted and replaced with one-line summaries.
///
/// Pinned items are never evicted.
pub fn evict(
    items: &[ContextItem],
    current_turn: u64,
    config: &EvictionConfig,
) -> Result<EvictionResult> {
    if items.is_empty() {
        return Ok(EvictionResult {
            kept: vec![],
            evicted: vec![],
            tokens_before: 0,
            tokens_after: 0,
            eviction_summary: String::new(),
        });
    }

    let tokens_before: u32 = items.iter().map(|i| i.tokens).sum();

    // Score each item: higher = more valuable = keep
    let mut scored: Vec<(f64, usize)> = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let score = compute_retention_score(item, current_turn, config);
            (score, idx)
        })
        .collect();

    // Sort by score ascending (lowest score = evict first)
    scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // Determine how many to evict
    let evict_count = ((items.len() as f64 * config.eviction_ratio).ceil() as usize)
        .min(items.len().saturating_sub(config.min_keep));

    let mut kept = Vec::new();
    let mut evicted = Vec::new();

    for (rank, &(_, idx)) in scored.iter().enumerate() {
        let item = &items[idx];

        if item.pinned || rank >= evict_count {
            kept.push(item.clone());
        } else {
            evicted.push(EvictedItem {
                id: item.id.clone(),
                summary: summarize_for_eviction(&item.content),
                original_tokens: item.tokens,
            });
        }
    }

    let tokens_after: u32 = kept.iter().map(|i| i.tokens).sum::<u32>()
        + evicted.iter().map(|e| estimate_tokens(&e.summary)).sum::<u32>();

    let eviction_summary = if evicted.is_empty() {
        String::new()
    } else {
        format_eviction_summary(&evicted)
    };

    Ok(EvictionResult {
        kept,
        evicted,
        tokens_before,
        tokens_after,
        eviction_summary,
    })
}

/// Suggest whether eviction should be triggered based on current usage.
pub fn should_evict(
    total_tokens: u32,
    budget_ceiling: u32,
    warning_threshold: f64,
) -> bool {
    if budget_ceiling == 0 {
        return false;
    }
    let usage_ratio = total_tokens as f64 / budget_ceiling as f64;
    usage_ratio >= warning_threshold
}

// ── Scoring ───────────────────────────────────────────────────────────────

/// Compute a retention score for an item. Higher = more valuable.
///
/// Score = recency_weight × recency + frequency_weight × frequency
///
/// Recency: 1.0 for current turn, decays toward 0.0 as age increases.
/// Frequency: log2(access_count + 1) normalized to [0, 1].
fn compute_retention_score(
    item: &ContextItem,
    current_turn: u64,
    config: &EvictionConfig,
) -> f64 {
    // Pinned items get maximum score
    if item.pinned {
        return f64::MAX;
    }

    let age = current_turn.saturating_sub(item.last_accessed_turn) as f64;
    let max_age = config.max_age_turns as f64;

    // Recency: exponential decay
    let recency = (-age / max_age.max(1.0)).exp();

    // Frequency: logarithmic scaling
    let frequency = ((item.access_count as f64 + 1.0).ln()) / 5.0_f64.ln();
    let frequency = frequency.min(1.0);

    // Combined score (recency weighted 70%, frequency 30%)
    0.7 * recency + 0.3 * frequency
}

/// Create a one-line summary of content for eviction.
fn summarize_for_eviction(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("");
    let truncated = if first_line.len() > 80 {
        format!("{}...", &first_line[..77])
    } else {
        first_line.to_string()
    };
    let line_count = content.lines().count();
    let char_count = content.len();
    format!("[evicted: {} lines, {} chars] {}", line_count, char_count, truncated)
}

/// Format a summary of all evicted items.
fn format_eviction_summary(evicted: &[EvictedItem]) -> String {
    let mut lines = vec![format!("[sqz compact: {} items evicted]", evicted.len())];
    for item in evicted {
        lines.push(format!("  {} — {}", item.id, item.summary));
    }
    let total_tokens: u32 = evicted.iter().map(|e| e.original_tokens).sum();
    lines.push(format!("  ({} tokens freed)", total_tokens));
    lines.join("\n")
}

fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(id: &str, tokens: u32, last_turn: u64, access_count: u32) -> ContextItem {
        ContextItem {
            id: id.to_string(),
            content: format!("Content of {id} with some text to make it realistic"),
            last_accessed_turn: last_turn,
            access_count,
            tokens,
            pinned: false,
        }
    }

    #[test]
    fn test_empty_items() {
        let result = evict(&[], 10, &EvictionConfig::default()).unwrap();
        assert!(result.kept.is_empty());
        assert!(result.evicted.is_empty());
        assert_eq!(result.tokens_before, 0);
    }

    #[test]
    fn test_evicts_oldest_items() {
        let items = vec![
            make_item("old1", 100, 0, 1),
            make_item("old2", 100, 1, 1),
            make_item("recent1", 100, 8, 1),
            make_item("recent2", 100, 9, 1),
            make_item("recent3", 100, 10, 1),
        ];
        let config = EvictionConfig {
            eviction_ratio: 0.4,
            min_keep: 3,
            ..Default::default()
        };
        let result = evict(&items, 10, &config).unwrap();
        assert_eq!(result.evicted.len(), 2, "should evict 2 oldest items");
        assert!(result.evicted.iter().any(|e| e.id == "old1"));
        assert!(result.evicted.iter().any(|e| e.id == "old2"));
        assert_eq!(result.kept.len(), 3);
    }

    #[test]
    fn test_pinned_items_never_evicted() {
        let mut items = vec![
            make_item("old_pinned", 100, 0, 1),
            make_item("old_unpinned", 100, 0, 1),
            make_item("recent", 100, 10, 1),
        ];
        items[0].pinned = true;

        let config = EvictionConfig {
            eviction_ratio: 0.5,
            min_keep: 1,
            ..Default::default()
        };
        let result = evict(&items, 10, &config).unwrap();
        assert!(
            result.kept.iter().any(|k| k.id == "old_pinned"),
            "pinned item should be kept even though it's old"
        );
    }

    #[test]
    fn test_frequently_accessed_items_retained() {
        let items = vec![
            make_item("old_frequent", 100, 2, 50),  // old but accessed 50 times
            make_item("old_rare", 100, 2, 1),        // old and rarely accessed
            make_item("recent", 100, 10, 1),
        ];
        let config = EvictionConfig {
            eviction_ratio: 0.4,
            min_keep: 2,
            ..Default::default()
        };
        let result = evict(&items, 10, &config).unwrap();
        // The frequently accessed old item should be retained over the rare one
        assert!(
            result.kept.iter().any(|k| k.id == "old_frequent"),
            "frequently accessed item should be retained"
        );
    }

    #[test]
    fn test_min_keep_respected() {
        let items = vec![
            make_item("a", 100, 0, 1),
            make_item("b", 100, 0, 1),
            make_item("c", 100, 0, 1),
        ];
        let config = EvictionConfig {
            eviction_ratio: 1.0, // try to evict everything
            min_keep: 2,
            ..Default::default()
        };
        let result = evict(&items, 10, &config).unwrap();
        assert!(result.kept.len() >= 2, "should keep at least min_keep items");
    }

    #[test]
    fn test_eviction_summary_format() {
        let items = vec![
            make_item("old", 500, 0, 1),
            make_item("recent", 100, 10, 1),
        ];
        let config = EvictionConfig {
            eviction_ratio: 0.5,
            min_keep: 1,
            ..Default::default()
        };
        let result = evict(&items, 10, &config).unwrap();
        if !result.evicted.is_empty() {
            assert!(result.eviction_summary.contains("[sqz compact:"));
            assert!(result.eviction_summary.contains("tokens freed"));
        }
    }

    #[test]
    fn test_should_evict_threshold() {
        assert!(!should_evict(50000, 200000, 0.7));  // 25% usage
        assert!(should_evict(150000, 200000, 0.7));   // 75% usage
        assert!(should_evict(200000, 200000, 0.7));   // 100% usage
        assert!(!should_evict(0, 0, 0.7));             // zero budget
    }

    #[test]
    fn test_tokens_after_less_than_before() {
        let items = vec![
            make_item("a", 500, 0, 1),
            make_item("b", 500, 1, 1),
            make_item("c", 100, 10, 1),
        ];
        let config = EvictionConfig {
            eviction_ratio: 0.5,
            min_keep: 1,
            ..Default::default()
        };
        let result = evict(&items, 10, &config).unwrap();
        assert!(
            result.tokens_after <= result.tokens_before,
            "eviction should not increase token count: {} vs {}",
            result.tokens_after, result.tokens_before
        );
    }

    #[test]
    fn test_summarize_for_eviction() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let summary = summarize_for_eviction(content);
        assert!(summary.contains("[evicted:"));
        assert!(summary.contains("fn main()"));
    }

    use proptest::prelude::*;

    proptest! {
        /// Eviction never increases total token count.
        #[test]
        fn prop_eviction_reduces_tokens(
            n in 2usize..=10usize,
            current_turn in 5u64..=50u64,
        ) {
            let items: Vec<ContextItem> = (0..n)
                .map(|i| make_item(&format!("item_{i}"), 100 + (i as u32 * 50), i as u64, 1))
                .collect();
            let config = EvictionConfig {
                eviction_ratio: 0.3,
                min_keep: 1,
                ..Default::default()
            };
            let result = evict(&items, current_turn, &config).unwrap();
            prop_assert!(
                result.tokens_after <= result.tokens_before,
                "tokens_after ({}) should be <= tokens_before ({})",
                result.tokens_after, result.tokens_before
            );
        }

        /// Kept + evicted = original count.
        #[test]
        fn prop_eviction_accounting(
            n in 2usize..=10usize,
        ) {
            let items: Vec<ContextItem> = (0..n)
                .map(|i| make_item(&format!("item_{i}"), 100, i as u64, 1))
                .collect();
            let config = EvictionConfig::default();
            let result = evict(&items, 50, &config).unwrap();
            prop_assert_eq!(
                result.kept.len() + result.evicted.len(),
                items.len(),
                "kept + evicted should equal original count"
            );
        }
    }
}
