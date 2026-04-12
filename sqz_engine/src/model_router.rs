use crate::preset::{ModelConfig, Preset};
use crate::types::TaskClassification;

/// Context describing a task to be routed to a model.
#[derive(Debug, Clone)]
pub struct TaskContext {
    pub description: String,
    pub token_count: u32,
    pub file_count: u32,
    pub has_code: bool,
}

/// The result of a routing decision.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub model: ModelConfig,
    pub classification: TaskClassification,
    pub complexity_score: f64,
}

/// Routes tasks to local or remote models based on complexity heuristics.
pub struct ModelRouter {
    complexity_threshold: f64,
    local_model: Option<ModelConfig>,
    remote_model: ModelConfig,
}

impl ModelRouter {
    /// Construct a `ModelRouter` from a `Preset`.
    ///
    /// - `remote_model` is built from `preset.model` (primary field).
    /// - `local_model` is `Some` only when `preset.model.local` is non-empty.
    pub fn new(preset: &Preset) -> Self {
        let remote_model = preset.model.clone();

        let local_model = if preset.model.local.is_empty() {
            None
        } else {
            let mut local = preset.model.clone();
            local.primary = preset.model.local.clone();
            Some(local)
        };

        Self {
            complexity_threshold: preset.model.complexity_threshold,
            local_model,
            remote_model,
        }
    }

    /// Compute a complexity score in [0.0, 1.0] for the given task.
    ///
    /// Heuristics:
    /// - Token contribution : `token_count / 10_000.0`, capped at 0.5
    /// - File count contribution: `file_count * 0.1`, capped at 0.3
    /// - Code presence: +0.1 if `has_code`
    /// - Total capped at 1.0
    pub fn analyze_complexity(&self, task: &TaskContext) -> f64 {
        let token_score = (task.token_count as f64 / 10_000.0).min(0.5);
        let file_score = (task.file_count as f64 * 0.1).min(0.3);
        let code_score = if task.has_code { 0.1 } else { 0.0 };

        (token_score + file_score + code_score).min(1.0)
    }

    /// Route a task to the appropriate model.
    ///
    /// - If `complexity_score < threshold` AND a local model is configured →
    ///   route to local, classify as `Simple`.
    /// - Otherwise → route to remote, classify as `Complex`.
    /// - If no local model is configured, all tasks route to remote.
    ///
    /// Each decision is logged to stderr.
    pub fn route(&self, task: &TaskContext) -> RoutingDecision {
        let complexity_score = self.analyze_complexity(task);

        let (model, classification) =
            if complexity_score < self.complexity_threshold && self.local_model.is_some() {
                let local = self.local_model.clone().unwrap();
                (local, TaskClassification::Simple)
            } else {
                (self.remote_model.clone(), TaskClassification::Complex)
            };

        eprintln!(
            "[ModelRouter] task='{}' score={:.4} threshold={:.4} classification={:?} model='{}'",
            task.description,
            complexity_score,
            self.complexity_threshold,
            classification,
            model.primary,
        );

        RoutingDecision {
            model,
            classification,
            complexity_score,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::Preset;
    use proptest::prelude::*;

    fn make_task(token_count: u32, file_count: u32, has_code: bool) -> TaskContext {
        TaskContext {
            description: "test task".to_string(),
            token_count,
            file_count,
            has_code,
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_analyze_complexity_zero() {
        let preset = Preset::default();
        let router = ModelRouter::new(&preset);
        let task = make_task(0, 0, false);
        assert_eq!(router.analyze_complexity(&task), 0.0);
    }

    #[test]
    fn test_analyze_complexity_token_cap() {
        let preset = Preset::default();
        let router = ModelRouter::new(&preset);
        // 100_000 tokens → token_score = 10.0 → capped at 0.5
        let task = make_task(100_000, 0, false);
        assert!((router.analyze_complexity(&task) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_analyze_complexity_file_cap() {
        let preset = Preset::default();
        let router = ModelRouter::new(&preset);
        // 10 files → file_score = 1.0 → capped at 0.3
        let task = make_task(0, 10, false);
        assert!((router.analyze_complexity(&task) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn test_analyze_complexity_code_bonus() {
        let preset = Preset::default();
        let router = ModelRouter::new(&preset);
        let task = make_task(0, 0, true);
        assert!((router.analyze_complexity(&task) - 0.1).abs() < 1e-9);
    }

    #[test]
    fn test_analyze_complexity_total_cap() {
        let preset = Preset::default();
        let router = ModelRouter::new(&preset);
        // Max everything: 0.5 + 0.3 + 0.1 = 0.9 < 1.0, so no cap needed here.
        let task = make_task(100_000, 10, true);
        assert!((router.analyze_complexity(&task) - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_route_simple_to_local() {
        let preset = Preset::default(); // threshold = 0.4, local = "llama-3.1-8b"
        let router = ModelRouter::new(&preset);
        // score = 0.0 < 0.4 → Simple → local
        let task = make_task(0, 0, false);
        let decision = router.route(&task);
        assert_eq!(decision.classification, TaskClassification::Simple);
        assert_eq!(decision.model.primary, "llama-3.1-8b");
    }

    #[test]
    fn test_route_complex_to_remote() {
        let preset = Preset::default(); // threshold = 0.4
        let router = ModelRouter::new(&preset);
        // score = 0.5 (50k tokens) >= 0.4 → Complex → remote
        let task = make_task(50_000, 0, false);
        let decision = router.route(&task);
        assert_eq!(decision.classification, TaskClassification::Complex);
        assert_eq!(decision.model.primary, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_route_at_threshold_goes_remote() {
        let mut preset = Preset::default();
        preset.model.complexity_threshold = 0.5;
        let router = ModelRouter::new(&preset);
        // score = 0.5 (50k tokens) == threshold → Complex → remote
        let task = make_task(50_000, 0, false);
        let decision = router.route(&task);
        assert_eq!(decision.classification, TaskClassification::Complex);
    }

    #[test]
    fn test_no_local_model_routes_all_to_remote() {
        let mut preset = Preset::default();
        preset.model.local = String::new(); // no local model
        let router = ModelRouter::new(&preset);
        // Even a trivial task should go remote
        let task = make_task(0, 0, false);
        let decision = router.route(&task);
        assert_eq!(decision.classification, TaskClassification::Complex);
        assert_eq!(decision.model.primary, "claude-sonnet-4-20250514");
    }

    // -----------------------------------------------------------------------
    // Property 19: Model routing by complexity threshold
    // Validates: Requirements 12.1, 12.2, 12.3, 12.5
    // -----------------------------------------------------------------------

    /// Strategy: threshold strictly in (0.0, 1.0).
    fn arb_threshold() -> impl Strategy<Value = f64> {
        (1u32..=9999u32).prop_map(|n| n as f64 / 10_000.0)
    }

    /// Strategy: token_count in [0, 200_000].
    fn arb_token_count() -> impl Strategy<Value = u32> {
        0u32..=200_000u32
    }

    /// Strategy: file_count in [0, 20].
    fn arb_file_count() -> impl Strategy<Value = u32> {
        0u32..=20u32
    }

    proptest! {
        /// **Validates: Requirements 12.1, 12.2, 12.3, 12.5**
        ///
        /// Property 19: Model routing by complexity threshold.
        ///
        /// For any task with a computed complexity score, the ModelRouter SHALL
        /// route to the local model if the score is below the configured threshold,
        /// and to the remote model if the score is at or above the threshold.
        /// If no local model is configured, all tasks SHALL route to the remote model.
        #[test]
        fn prop_model_routing_by_complexity_threshold(
            threshold in arb_threshold(),
            token_count in arb_token_count(),
            file_count in arb_file_count(),
            has_code in any::<bool>(),
            has_local in any::<bool>(),
        ) {
            let mut preset = Preset::default();
            preset.model.complexity_threshold = threshold;
            if !has_local {
                preset.model.local = String::new();
            }

            let router = ModelRouter::new(&preset);
            let task = TaskContext {
                description: "prop test task".to_string(),
                token_count,
                file_count,
                has_code,
            };

            let score = router.analyze_complexity(&task);
            let decision = router.route(&task);

            // Score must be in [0.0, 1.0]
            prop_assert!(score >= 0.0 && score <= 1.0,
                "score {} out of range", score);

            if !has_local {
                // No local model → always remote, always Complex
                prop_assert_eq!(
                    decision.classification,
                    TaskClassification::Complex,
                    "no local model: expected Complex"
                );
                prop_assert_eq!(
                    decision.model.primary,
                    preset.model.primary,
                    "no local model: expected remote primary model"
                );
            } else if score < threshold {
                // Below threshold with local model → Simple → local
                prop_assert_eq!(
                    decision.classification,
                    TaskClassification::Simple,
                    "score={} < threshold={}: expected Simple", score, threshold
                );
                prop_assert_eq!(
                    decision.model.primary,
                    preset.model.local,
                    "score={} < threshold={}: expected local model", score, threshold
                );
            } else {
                // At or above threshold → Complex → remote
                prop_assert_eq!(
                    decision.classification,
                    TaskClassification::Complex,
                    "score={} >= threshold={}: expected Complex", score, threshold
                );
                prop_assert_eq!(
                    decision.model.primary,
                    preset.model.primary,
                    "score={} >= threshold={}: expected remote model", score, threshold
                );
            }

            // complexity_score in decision must match analyze_complexity
            prop_assert!(
                (decision.complexity_score - score).abs() < 1e-12,
                "decision.complexity_score {} != analyze_complexity {}", decision.complexity_score, score
            );
        }
    }
}
