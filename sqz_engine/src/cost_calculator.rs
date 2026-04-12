use std::collections::HashMap;

use crate::types::{ModelFamily, SessionState};

// ---------------------------------------------------------------------------
// Pricing configuration
// ---------------------------------------------------------------------------

/// Per-model pricing rates.
#[derive(Debug, Clone)]
pub struct ModelPricing {
    /// USD cost per 1 000 input tokens (non-cached).
    pub input_per_1k: f64,
    /// USD cost per 1 000 output tokens.
    pub output_per_1k: f64,
    /// Fraction of the input price saved on cached tokens.
    /// 0.9 → 90 % discount (Anthropic), 0.5 → 50 % discount (OpenAI),
    /// 0.0 → no cache discount (Gemini / Local).
    pub cache_read_discount: f64,
}

/// Top-level pricing configuration passed to `CostCalculator::new`.
#[derive(Debug, Clone, Default)]
pub struct PricingConfig {
    pub models: HashMap<ModelFamily, ModelPricing>,
}

impl PricingConfig {
    /// Returns a `PricingConfig` pre-populated with the default rates for
    /// Anthropic Claude, OpenAI GPT, and Google Gemini.
    pub fn default_pricing() -> Self {
        let mut models = HashMap::new();
        models.insert(
            ModelFamily::AnthropicClaude,
            ModelPricing {
                input_per_1k: 0.003,
                output_per_1k: 0.015,
                cache_read_discount: 0.9,
            },
        );
        models.insert(
            ModelFamily::OpenAiGpt,
            ModelPricing {
                input_per_1k: 0.002,
                output_per_1k: 0.008,
                cache_read_discount: 0.5,
            },
        );
        models.insert(
            ModelFamily::GoogleGemini,
            ModelPricing {
                input_per_1k: 0.001,
                output_per_1k: 0.004,
                cache_read_discount: 0.0,
            },
        );
        PricingConfig { models }
    }
}

// ---------------------------------------------------------------------------
// Token usage
// ---------------------------------------------------------------------------

/// Token counts for a single request / tool call.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Non-cached input tokens.
    pub input: u32,
    /// Output tokens.
    pub output: u32,
    /// Input tokens served from the provider's prompt cache.
    pub cached_input: u32,
}

// ---------------------------------------------------------------------------
// Cost breakdown
// ---------------------------------------------------------------------------

/// Per-tool cost record.
#[derive(Debug, Clone)]
pub struct ToolCost {
    pub tokens_input: u32,
    pub tokens_output: u32,
    pub cost_usd: f64,
}

/// Full cost breakdown for a single `compute_cost` call.
#[derive(Debug, Clone)]
pub struct CostBreakdown {
    /// Total USD cost (after cache discount).
    pub total_usd: f64,
    /// Per-tool breakdown keyed by tool name.
    pub per_tool: HashMap<String, ToolCost>,
    /// USD saved because of prompt-cache hits.
    pub cache_savings_usd: f64,
    /// USD saved because of sqz compression (estimated from token reduction).
    pub compression_savings_usd: f64,
}

/// Session-level cost summary.
#[derive(Debug, Clone)]
pub struct SessionCostSummary {
    pub total_tokens: u32,
    pub total_usd: f64,
    pub cache_savings_usd: f64,
    pub compression_savings_usd: f64,
}

// ---------------------------------------------------------------------------
// CostCalculator
// ---------------------------------------------------------------------------

/// Computes real-time USD cost with per-tool breakdown and cache discount.
pub struct CostCalculator {
    pricing: HashMap<ModelFamily, ModelPricing>,
}

impl CostCalculator {
    /// Create a new `CostCalculator` from the supplied pricing configuration.
    pub fn new(pricing_config: &PricingConfig) -> Self {
        CostCalculator {
            pricing: pricing_config.models.clone(),
        }
    }

    /// Create a `CostCalculator` with the built-in default pricing.
    pub fn with_defaults() -> Self {
        Self::new(&PricingConfig::default_pricing())
    }

    /// Compute the cost for a single `TokenUsage` against the given model.
    ///
    /// If the model is not found in the pricing table the cost is zero.
    pub fn compute_cost(&self, model: &ModelFamily, tokens: &TokenUsage) -> CostBreakdown {
        let pricing = match self.pricing.get(model) {
            Some(p) => p,
            None => {
                return CostBreakdown {
                    total_usd: 0.0,
                    per_tool: HashMap::new(),
                    cache_savings_usd: 0.0,
                    compression_savings_usd: 0.0,
                }
            }
        };

        let input_cost = (tokens.input as f64 / 1_000.0) * pricing.input_per_1k;
        let output_cost = (tokens.output as f64 / 1_000.0) * pricing.output_per_1k;

        // Cached tokens are charged at (1 - discount) × normal input rate.
        let cached_full_cost = (tokens.cached_input as f64 / 1_000.0) * pricing.input_per_1k;
        let cached_actual_cost = cached_full_cost * (1.0 - pricing.cache_read_discount);
        let cache_savings = cached_full_cost - cached_actual_cost;

        let total_usd = input_cost + output_cost + cached_actual_cost;

        CostBreakdown {
            total_usd,
            per_tool: HashMap::new(), // populated by session_summary
            cache_savings_usd: cache_savings,
            compression_savings_usd: 0.0, // set by caller when compression ratio is known
        }
    }

    /// Produce a session-level cost summary from a `SessionState`.
    ///
    /// The session's `budget.model_family` determines which pricing to use.
    /// Per-tool costs are derived from `session.tool_usage`.
    pub fn session_summary(&self, session: &SessionState) -> SessionCostSummary {
        let model = &session.budget.model_family;
        let pricing = self.pricing.get(model);

        let mut total_tokens: u32 = 0;
        let mut total_usd: f64 = 0.0;
        let mut cache_savings_usd: f64 = 0.0;

        for record in &session.tool_usage {
            total_tokens = total_tokens
                .saturating_add(record.tokens_input)
                .saturating_add(record.tokens_output);
            total_usd += record.cost_usd;

            // Estimate cache savings from the record's stored cost vs full price.
            if let Some(p) = pricing {
                let full_input_cost =
                    (record.tokens_input as f64 / 1_000.0) * p.input_per_1k;
                let full_output_cost =
                    (record.tokens_output as f64 / 1_000.0) * p.output_per_1k;
                let full_cost = full_input_cost + full_output_cost;
                if full_cost > record.cost_usd {
                    cache_savings_usd += full_cost - record.cost_usd;
                }
            }
        }

        // Compression savings: tokens saved × input rate.
        // We approximate from the conversation: original tokens vs consumed.
        let compression_savings_usd = if let Some(p) = pricing {
            let original_tokens: u32 = session
                .conversation
                .iter()
                .map(|t| t.tokens)
                .sum();
            let consumed = session.budget.consumed;
            if original_tokens > consumed {
                let saved = original_tokens - consumed;
                (saved as f64 / 1_000.0) * p.input_per_1k
            } else {
                0.0
            }
        } else {
            0.0
        };

        SessionCostSummary {
            total_tokens,
            total_usd,
            cache_savings_usd,
            compression_savings_usd,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolUsageRecord;
    use proptest::prelude::*;

    // -----------------------------------------------------------------------
    // Helpers / strategies
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    fn arb_token_usage() -> impl Strategy<Value = TokenUsage> {
        (0u32..=100_000u32, 0u32..=50_000u32, 0u32..=100_000u32).prop_map(
            |(input, output, cached_input)| TokenUsage {
                input,
                output,
                cached_input,
            },
        )
    }

    fn arb_model() -> impl Strategy<Value = ModelFamily> {
        prop_oneof![
            Just(ModelFamily::AnthropicClaude),
            Just(ModelFamily::OpenAiGpt),
            Just(ModelFamily::GoogleGemini),
        ]
    }

    // -----------------------------------------------------------------------
    // Property 29: Cost calculation per-tool invariant
    // Validates: Requirements 22.1, 22.2, 22.4
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 22.1, 22.2, 22.4**
        ///
        /// Property 29: Cost calculation per-tool invariant.
        ///
        /// For any set of tool usage records in a session, the sum of per-tool
        /// USD costs SHALL equal the total session USD cost.  The session cost
        /// summary SHALL include total tokens, total USD, cache savings, and
        /// compression savings.
        #[test]
        fn prop_cost_per_tool_invariant(
            records in prop::collection::vec(
                (
                    "[a-z_]{1,16}",          // tool name
                    0u32..=10_000u32,        // tokens_input
                    0u32..=5_000u32,         // tokens_output
                ),
                0..=20,
            ),
            model in arb_model(),
        ) {
            use chrono::Utc;
            use crate::types::{BudgetState, CorrectionLog, SessionState};
            use std::path::PathBuf;

            let calc = CostCalculator::with_defaults();
            let pricing = PricingConfig::default_pricing();
            let p = pricing.models.get(&model).unwrap();

            // Build tool usage records with realistic costs.
            let tool_usage: Vec<ToolUsageRecord> = records
                .iter()
                .map(|(name, ti, to)| {
                    let cost = (*ti as f64 / 1_000.0) * p.input_per_1k
                        + (*to as f64 / 1_000.0) * p.output_per_1k;
                    ToolUsageRecord {
                        tool_name: name.clone(),
                        tokens_input: *ti,
                        tokens_output: *to,
                        cost_usd: cost,
                        timestamp: Utc::now(),
                    }
                })
                .collect();

            let expected_total_usd: f64 = tool_usage.iter().map(|r| r.cost_usd).sum();
            let expected_total_tokens: u32 = tool_usage
                .iter()
                .map(|r| r.tokens_input.saturating_add(r.tokens_output))
                .sum();

            let session = SessionState {
                id: "test".to_string(),
                project_dir: PathBuf::from("/tmp"),
                conversation: vec![],
                corrections: CorrectionLog::default(),
                pins: vec![],
                learnings: vec![],
                compressed_summary: String::new(),
                budget: BudgetState {
                    window_size: 200_000,
                    consumed: 0,
                    pinned: 0,
                    model_family: model,
                },
                tool_usage,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            let summary = calc.session_summary(&session);

            // Total tokens must match sum of per-record tokens.
            prop_assert_eq!(summary.total_tokens, expected_total_tokens);

            // Total USD must match sum of per-record costs (within floating-point tolerance).
            prop_assert!(
                (summary.total_usd - expected_total_usd).abs() < 1e-9,
                "total_usd mismatch: {} vs {}",
                summary.total_usd,
                expected_total_usd
            );
        }
    }

    // -----------------------------------------------------------------------
    // Property 4: Cache discount in cost calculation
    // Validates: Requirements 4.3, 22.3
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 4.3, 22.3**
        ///
        /// Property 4: Cache discount in cost calculation.
        ///
        /// For any token usage with an active prompt cache boundary, the
        /// Cost_Calculator SHALL apply the provider-specific discount (90% for
        /// Anthropic, 50% for OpenAI) to cached tokens, resulting in a lower
        /// total cost than the same usage without caching.
        #[test]
        fn prop_cache_discount_lowers_cost(
            input in 1u32..=100_000u32,
            output in 0u32..=50_000u32,
            cached_input in 1u32..=100_000u32,  // at least 1 cached token
            model in arb_model(),
        ) {
            let calc = CostCalculator::with_defaults();

            // Usage WITH cache hits.
            let with_cache = TokenUsage { input, output, cached_input };
            // Same usage but all tokens treated as non-cached.
            let without_cache = TokenUsage {
                input: input + cached_input,
                output,
                cached_input: 0,
            };

            let cost_with = calc.compute_cost(&model, &with_cache);
            let cost_without = calc.compute_cost(&model, &without_cache);

            let pricing = PricingConfig::default_pricing();
            let p = pricing.models.get(&model).unwrap();

            if p.cache_read_discount > 0.0 {
                // With a non-zero discount, cached cost must be strictly lower.
                prop_assert!(
                    cost_with.total_usd < cost_without.total_usd,
                    "expected cost_with ({}) < cost_without ({}) for model {:?}",
                    cost_with.total_usd,
                    cost_without.total_usd,
                    model
                );
                // cache_savings_usd must be positive.
                prop_assert!(
                    cost_with.cache_savings_usd > 0.0,
                    "expected positive cache_savings_usd, got {}",
                    cost_with.cache_savings_usd
                );
            } else {
                // Gemini: no discount → costs are equal.
                prop_assert!(
                    (cost_with.total_usd - cost_without.total_usd).abs() < 1e-12,
                    "expected equal costs for zero-discount model, got {} vs {}",
                    cost_with.total_usd,
                    cost_without.total_usd
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_anthropic_pricing_defaults() {
        let calc = CostCalculator::with_defaults();
        let usage = TokenUsage {
            input: 1_000,
            output: 1_000,
            cached_input: 0,
        };
        let breakdown = calc.compute_cost(&ModelFamily::AnthropicClaude, &usage);
        // 1k input @ $0.003 + 1k output @ $0.015 = $0.018
        assert!((breakdown.total_usd - 0.018).abs() < 1e-9);
        assert_eq!(breakdown.cache_savings_usd, 0.0);
    }

    #[test]
    fn test_anthropic_cache_discount() {
        let calc = CostCalculator::with_defaults();
        let usage = TokenUsage {
            input: 0,
            output: 0,
            cached_input: 1_000,
        };
        let breakdown = calc.compute_cost(&ModelFamily::AnthropicClaude, &usage);
        // 1k cached @ $0.003 × (1 - 0.9) = $0.0003
        assert!((breakdown.total_usd - 0.0003).abs() < 1e-9);
        // savings = $0.003 - $0.0003 = $0.0027
        assert!((breakdown.cache_savings_usd - 0.0027).abs() < 1e-9);
    }

    #[test]
    fn test_openai_cache_discount() {
        let calc = CostCalculator::with_defaults();
        let usage = TokenUsage {
            input: 0,
            output: 0,
            cached_input: 1_000,
        };
        let breakdown = calc.compute_cost(&ModelFamily::OpenAiGpt, &usage);
        // 1k cached @ $0.002 × (1 - 0.5) = $0.001
        assert!((breakdown.total_usd - 0.001).abs() < 1e-9);
        assert!((breakdown.cache_savings_usd - 0.001).abs() < 1e-9);
    }

    #[test]
    fn test_gemini_no_cache_discount() {
        let calc = CostCalculator::with_defaults();
        let usage = TokenUsage {
            input: 1_000,
            output: 0,
            cached_input: 1_000,
        };
        let breakdown = calc.compute_cost(&ModelFamily::GoogleGemini, &usage);
        // No discount: 2k input @ $0.001 = $0.002
        assert!((breakdown.total_usd - 0.002).abs() < 1e-9);
        assert_eq!(breakdown.cache_savings_usd, 0.0);
    }

    #[test]
    fn test_unknown_model_returns_zero() {
        let calc = CostCalculator::with_defaults();
        let usage = TokenUsage {
            input: 1_000,
            output: 1_000,
            cached_input: 0,
        };
        let breakdown = calc.compute_cost(&ModelFamily::Local("custom".to_string()), &usage);
        assert_eq!(breakdown.total_usd, 0.0);
    }
}
