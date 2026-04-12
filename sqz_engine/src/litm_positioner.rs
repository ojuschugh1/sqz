use serde::{Deserialize, Serialize};

/// Strategy for LITM (Lost In The Middle) context reordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LitmStrategy {
    /// Default: place high-priority content at edges of the context window.
    Enabled,
    /// Preserve original section order — no reordering.
    Disabled,
    /// Extreme edge bias: high-priority at edges, drop lowest-priority middle sections.
    Aggressive,
}

impl Default for LitmStrategy {
    fn default() -> Self {
        Self::Enabled
    }
}

/// The type of a context section, used to assign default priority scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionType {
    /// User corrections — highest priority.
    Correction,
    /// Pinned content — high priority.
    Pin,
    /// Recent conversation turns — high priority.
    RecentTurn,
    /// System prompt — high priority.
    SystemPrompt,
    /// Older conversation history — lower priority.
    OlderHistory,
    /// Background/reference material — lowest priority.
    Background,
}

/// A section of context to be positioned in the context window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSection {
    pub content: String,
    pub priority: f64,
    pub section_type: SectionType,
}

/// Reorders context sections to mitigate the "Lost In The Middle" problem.
///
/// LLMs pay less attention to content in the middle of the context window.
/// The positioner places high-priority sections at the beginning and end (edges)
/// and lower-priority sections in the middle.
pub struct LitmPositioner {
    strategy: LitmStrategy,
}

impl LitmPositioner {
    pub fn new(strategy: LitmStrategy) -> Self {
        Self { strategy }
    }

    pub fn strategy(&self) -> LitmStrategy {
        self.strategy
    }

    /// Reorder sections in place according to the configured strategy.
    ///
    /// - **Disabled**: no-op, sections remain in their original order.
    /// - **Enabled**: sort by priority, then interleave highest-priority sections
    ///   at the beginning and end, lowest-priority in the middle.
    /// - **Aggressive**: same as Enabled, but drops the lowest-priority middle
    ///   sections entirely (bottom 30% by count after sorting).
    pub fn reorder(&self, sections: &mut Vec<ContextSection>) {
        match self.strategy {
            LitmStrategy::Disabled => {}
            LitmStrategy::Enabled => Self::reorder_edges(sections, false),
            LitmStrategy::Aggressive => Self::reorder_edges(sections, true),
        }
    }

    /// Core algorithm: sort by priority descending, then distribute to edges.
    ///
    /// After sorting highest-priority first, we alternate placement:
    ///   - 1st highest → beginning
    ///   - 2nd highest → end
    ///   - 3rd highest → beginning
    ///   - …and so on
    ///
    /// This ensures the top-priority items land at the edges of the window.
    /// In aggressive mode, the bottom 30% of sections (by count) are dropped.
    fn reorder_edges(sections: &mut Vec<ContextSection>, aggressive: bool) {
        if sections.len() <= 1 {
            return;
        }

        // Sort by priority descending (highest first).
        sections.sort_by(|a, b| {
            b.priority
                .partial_cmp(&a.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if aggressive {
            // Drop the bottom 30% of sections.
            let keep = (sections.len() as f64 * 0.7).ceil() as usize;
            let keep = keep.max(1); // always keep at least one
            sections.truncate(keep);
        }

        // Distribute to edges: alternate front / back.
        let sorted: Vec<ContextSection> = sections.drain(..).collect();
        let mut front = Vec::new();
        let mut back = Vec::new();

        for (i, section) in sorted.into_iter().enumerate() {
            if i % 2 == 0 {
                front.push(section);
            } else {
                back.push(section);
            }
        }

        // Back items were added highest-priority-first, but they should appear
        // with the highest priority at the very end of the window, so reverse.
        back.reverse();

        sections.extend(front);
        sections.extend(back);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(content: &str, priority: f64, section_type: SectionType) -> ContextSection {
        ContextSection {
            content: content.to_string(),
            priority,
            section_type,
        }
    }

    #[test]
    fn disabled_preserves_order() {
        let positioner = LitmPositioner::new(LitmStrategy::Disabled);
        let mut sections = vec![
            section("low", 1.0, SectionType::Background),
            section("high", 10.0, SectionType::Correction),
            section("mid", 5.0, SectionType::OlderHistory),
        ];
        let original = sections.clone();
        positioner.reorder(&mut sections);
        assert_eq!(sections, original);
    }

    #[test]
    fn enabled_places_highest_at_edges() {
        let positioner = LitmPositioner::new(LitmStrategy::Enabled);
        let mut sections = vec![
            section("bg", 1.0, SectionType::Background),
            section("old", 3.0, SectionType::OlderHistory),
            section("pin", 8.0, SectionType::Pin),
            section("corr", 10.0, SectionType::Correction),
            section("recent", 7.0, SectionType::RecentTurn),
        ];
        positioner.reorder(&mut sections);

        // Highest priority (10.0) should be first.
        assert_eq!(sections.first().unwrap().priority, 10.0);
        // Second highest (8.0) should be last.
        assert_eq!(sections.last().unwrap().priority, 8.0);
        // Lowest priority should be in the middle, not at edges.
        let lowest_idx = sections
            .iter()
            .position(|s| s.priority == 1.0)
            .unwrap();
        assert!(lowest_idx > 0 && lowest_idx < sections.len() - 1);
    }

    #[test]
    fn aggressive_drops_low_priority() {
        let positioner = LitmPositioner::new(LitmStrategy::Aggressive);
        let mut sections = vec![
            section("bg1", 1.0, SectionType::Background),
            section("bg2", 2.0, SectionType::Background),
            section("bg3", 3.0, SectionType::Background),
            section("old", 4.0, SectionType::OlderHistory),
            section("old2", 5.0, SectionType::OlderHistory),
            section("recent", 7.0, SectionType::RecentTurn),
            section("pin", 8.0, SectionType::Pin),
            section("sys", 9.0, SectionType::SystemPrompt),
            section("corr1", 10.0, SectionType::Correction),
            section("corr2", 9.5, SectionType::Correction),
        ];
        let original_len = sections.len();
        positioner.reorder(&mut sections);

        // Should keep ceil(10 * 0.7) = 7 sections.
        assert_eq!(sections.len(), 7);
        assert!(sections.len() < original_len);

        // Dropped sections should be the lowest-priority ones.
        let priorities: Vec<f64> = sections.iter().map(|s| s.priority).collect();
        // None of the dropped priorities (1.0, 2.0, 3.0) should remain.
        assert!(!priorities.contains(&1.0));
        assert!(!priorities.contains(&2.0));
        assert!(!priorities.contains(&3.0));
    }

    #[test]
    fn empty_sections_no_panic() {
        let positioner = LitmPositioner::new(LitmStrategy::Enabled);
        let mut sections: Vec<ContextSection> = vec![];
        positioner.reorder(&mut sections);
        assert!(sections.is_empty());
    }

    #[test]
    fn single_section_unchanged() {
        let positioner = LitmPositioner::new(LitmStrategy::Enabled);
        let mut sections = vec![section("only", 5.0, SectionType::Pin)];
        positioner.reorder(&mut sections);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].content, "only");
    }

    #[test]
    fn two_sections_highest_at_edges() {
        let positioner = LitmPositioner::new(LitmStrategy::Enabled);
        let mut sections = vec![
            section("low", 1.0, SectionType::Background),
            section("high", 10.0, SectionType::Correction),
        ];
        positioner.reorder(&mut sections);
        assert_eq!(sections[0].priority, 10.0);
        assert_eq!(sections[1].priority, 1.0);
    }

    #[test]
    fn aggressive_keeps_at_least_one() {
        let positioner = LitmPositioner::new(LitmStrategy::Aggressive);
        let mut sections = vec![section("only", 1.0, SectionType::Background)];
        positioner.reorder(&mut sections);
        assert_eq!(sections.len(), 1);
    }

    #[test]
    fn default_strategy_is_enabled() {
        assert_eq!(LitmStrategy::default(), LitmStrategy::Enabled);
    }

    #[test]
    fn enabled_all_sections_preserved() {
        let positioner = LitmPositioner::new(LitmStrategy::Enabled);
        let mut sections = vec![
            section("a", 1.0, SectionType::Background),
            section("b", 5.0, SectionType::OlderHistory),
            section("c", 10.0, SectionType::Correction),
        ];
        positioner.reorder(&mut sections);
        // All 3 sections should still be present.
        assert_eq!(sections.len(), 3);
        let contents: Vec<&str> = sections.iter().map(|s| s.content.as_str()).collect();
        assert!(contents.contains(&"a"));
        assert!(contents.contains(&"b"));
        assert!(contents.contains(&"c"));
    }

    // -----------------------------------------------------------------------
    // Property-based tests
    // -----------------------------------------------------------------------

    use proptest::prelude::*;

    /// Arbitrary `SectionType` generator.
    fn arb_section_type() -> impl Strategy<Value = SectionType> {
        prop_oneof![
            Just(SectionType::Correction),
            Just(SectionType::Pin),
            Just(SectionType::RecentTurn),
            Just(SectionType::SystemPrompt),
            Just(SectionType::OlderHistory),
            Just(SectionType::Background),
        ]
    }

    /// Generate a `ContextSection` with a priority in [0.0, 100.0].
    fn arb_section() -> impl Strategy<Value = ContextSection> {
        (any::<u16>(), 0.0f64..100.0f64, arb_section_type()).prop_map(
            |(id, priority, section_type)| ContextSection {
                content: format!("section_{id}"),
                priority,
                section_type,
            },
        )
    }

    /// Generate a vec of sections where every priority is distinct.
    fn arb_distinct_priority_sections(
        min: usize,
        max: usize,
    ) -> impl Strategy<Value = Vec<ContextSection>> {
        proptest::collection::vec(arb_section(), min..=max).prop_map(|mut sections| {
            // Ensure distinct priorities by assigning index-based offsets.
            for (i, s) in sections.iter_mut().enumerate() {
                s.priority = (i as f64) * 1.1 + s.priority * 0.001;
            }
            sections
        })
    }

    proptest! {
        /// **Validates: Requirements 34.1, 34.2**
        ///
        /// Property 39: LITM positioning places priority content at edges.
        ///
        /// For any set of sections (N >= 3) with distinct priorities, after
        /// reordering with the Enabled strategy:
        ///   1. The highest-priority section is at position 0 or position N-1.
        ///   2. The lowest-priority section is never at position 0 or N-1.
        #[test]
        fn prop39_litm_edge_positioning(
            sections in arb_distinct_priority_sections(3, 20)
        ) {
            let positioner = LitmPositioner::new(LitmStrategy::Enabled);
            let mut reordered = sections.clone();
            positioner.reorder(&mut reordered);

            let n = reordered.len();

            // Find the highest and lowest priorities from the original set.
            let max_priority = sections
                .iter()
                .map(|s| s.priority)
                .fold(f64::NEG_INFINITY, f64::max);
            let min_priority = sections
                .iter()
                .map(|s| s.priority)
                .fold(f64::INFINITY, f64::min);

            // 1. Highest-priority section must be at an edge (pos 0 or N-1).
            let first_priority = reordered[0].priority;
            let last_priority = reordered[n - 1].priority;
            prop_assert!(
                (first_priority - max_priority).abs() < f64::EPSILON
                    || (last_priority - max_priority).abs() < f64::EPSILON,
                "Highest-priority section ({max_priority}) must be at position 0 \
                 (got {first_priority}) or position {} (got {last_priority})",
                n - 1
            );

            // 2. Lowest-priority section must NOT be at an edge.
            prop_assert!(
                (first_priority - min_priority).abs() >= f64::EPSILON,
                "Lowest-priority section ({min_priority}) must not be at position 0"
            );
            prop_assert!(
                (last_priority - min_priority).abs() >= f64::EPSILON,
                "Lowest-priority section ({min_priority}) must not be at position {}",
                n - 1
            );
        }
    }
}
