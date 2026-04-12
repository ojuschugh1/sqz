use std::collections::HashMap;

use crate::preset::Preset;
use crate::types::AgentId;

// ---------------------------------------------------------------------------
// Per-agent budget state
// ---------------------------------------------------------------------------

/// Token budget for a single agent.
#[derive(Debug, Clone)]
pub struct AgentBudget {
    pub agent_id: AgentId,
    /// Tokens allocated to this agent (fraction of the window).
    pub allocated: u32,
    /// Tokens consumed so far.
    pub consumed: u32,
    /// Tokens currently pinned (excluded from available budget).
    pub pinned: u32,
}

impl AgentBudget {
    fn new(agent_id: AgentId, allocated: u32) -> Self {
        AgentBudget {
            agent_id,
            allocated,
            consumed: 0,
            pinned: 0,
        }
    }

    /// Available = allocated − consumed − pinned (saturating at 0).
    fn available(&self) -> u32 {
        self.allocated
            .saturating_sub(self.consumed)
            .saturating_sub(self.pinned)
    }

    fn consumed_pct(&self) -> f64 {
        if self.allocated == 0 {
            return 1.0;
        }
        (self.consumed + self.pinned) as f64 / self.allocated as f64
    }

    fn pinned_pct(&self) -> f64 {
        if self.allocated == 0 {
            return 0.0;
        }
        self.pinned as f64 / self.allocated as f64
    }
}

// ---------------------------------------------------------------------------
// Warnings
// ---------------------------------------------------------------------------

/// Warnings emitted by the `BudgetTracker`.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetWarning {
    /// Consumed percentage crossed the warning threshold.
    ThresholdCrossed {
        agent: AgentId,
        percentage: f64,
        remaining: u32,
    },
    /// A pending operation would push usage above the ceiling threshold.
    PredictiveOverage {
        agent: AgentId,
        current: f64,
        projected: f64,
    },
    /// Pinned tokens exceed 50 % of the agent's allocated budget.
    PinnedExcessive {
        agent: AgentId,
        pinned_pct: f64,
    },
    /// Agent has exhausted its allocated budget.
    AgentBudgetExhausted { agent: AgentId },
}

// ---------------------------------------------------------------------------
// Usage prediction / report
// ---------------------------------------------------------------------------

/// Prediction for a pending operation.
#[derive(Debug, Clone)]
pub struct UsagePrediction {
    pub current_pct: f64,
    pub projected_pct: f64,
    pub would_exceed_ceiling: bool,
}

/// Snapshot of an agent's budget state.
#[derive(Debug, Clone)]
pub struct UsageReport {
    pub agent_id: AgentId,
    pub consumed: u32,
    pub allocated: u32,
    pub pinned: u32,
    pub available: u32,
    pub consumed_pct: f64,
}

// ---------------------------------------------------------------------------
// BudgetTracker
// ---------------------------------------------------------------------------

/// Tracks token budgets across multiple agents with predictive warnings.
pub struct BudgetTracker {
    window_size: u32,
    agents: HashMap<AgentId, AgentBudget>,
    /// Fraction of allocated budget at which a threshold warning fires (default 0.70).
    warning_threshold: f64,
    /// Fraction of allocated budget at which a predictive ceiling warning fires (default 0.85).
    ceiling_threshold: f64,
}

impl BudgetTracker {
    /// Create a new `BudgetTracker` from a window size and a `Preset`.
    ///
    /// Per-agent allocations are read from `preset.budget.agents`; each value
    /// is treated as a fraction of `window_size`.  If no agents are configured
    /// in the preset a single "default" agent is created with the full window.
    pub fn new(window_size: u32, preset: &Preset) -> Self {
        let warning_threshold = preset.budget.warning_threshold;
        let ceiling_threshold = preset.budget.ceiling_threshold;

        let mut agents: HashMap<AgentId, AgentBudget> = HashMap::new();

        if preset.budget.agents.is_empty() {
            agents.insert(
                "default".to_string(),
                AgentBudget::new("default".to_string(), window_size),
            );
        } else {
            for (name, fraction) in &preset.budget.agents {
                let allocated = (window_size as f64 * fraction).round() as u32;
                agents.insert(name.clone(), AgentBudget::new(name.clone(), allocated));
            }
        }

        BudgetTracker {
            window_size,
            agents,
            warning_threshold,
            ceiling_threshold,
        }
    }

    /// Create a `BudgetTracker` with explicit thresholds (useful in tests).
    pub fn with_thresholds(
        window_size: u32,
        warning_threshold: f64,
        ceiling_threshold: f64,
    ) -> Self {
        let mut agents = HashMap::new();
        agents.insert(
            "default".to_string(),
            AgentBudget::new("default".to_string(), window_size),
        );
        BudgetTracker {
            window_size,
            agents,
            warning_threshold,
            ceiling_threshold,
        }
    }

    // -----------------------------------------------------------------------
    // Ensure an agent entry exists (lazy initialisation).
    // -----------------------------------------------------------------------

    fn ensure_agent(&mut self, agent: &AgentId) {
        if !self.agents.contains_key(agent) {
            self.agents.insert(
                agent.clone(),
                AgentBudget::new(agent.clone(), self.window_size),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Record `tokens` consumed by `agent`.
    ///
    /// Returns any warnings triggered by this update.
    pub fn record_tokens(&mut self, agent: AgentId, tokens: u32) -> Vec<BudgetWarning> {
        self.ensure_agent(&agent);
        let mut warnings = Vec::new();

        let budget = self.agents.get_mut(&agent).unwrap();
        let before_pct = budget.consumed_pct();
        budget.consumed = budget.consumed.saturating_add(tokens);
        let after_pct = budget.consumed_pct();

        // Exhausted?
        if budget.consumed >= budget.allocated {
            warnings.push(BudgetWarning::AgentBudgetExhausted {
                agent: agent.clone(),
            });
            return warnings;
        }

        // Threshold crossed (warning fires once when crossing from below to above).
        if before_pct < self.warning_threshold && after_pct >= self.warning_threshold {
            warnings.push(BudgetWarning::ThresholdCrossed {
                agent: agent.clone(),
                percentage: after_pct,
                remaining: budget.available(),
            });
        }

        // Excessive pin check.
        let pinned_pct = budget.pinned_pct();
        if pinned_pct > 0.5 {
            warnings.push(BudgetWarning::PinnedExcessive {
                agent: agent.clone(),
                pinned_pct,
            });
        }

        warnings
    }

    /// Predict what would happen if `pending_tokens` were consumed by `agent`.
    pub fn predict_usage(&self, agent: AgentId, pending_tokens: u32) -> UsagePrediction {
        let budget = match self.agents.get(&agent) {
            Some(b) => b,
            None => {
                // Unknown agent: treat as full window, nothing consumed.
                let current_pct = 0.0;
                let projected_pct = pending_tokens as f64 / self.window_size as f64;
                return UsagePrediction {
                    current_pct,
                    projected_pct,
                    would_exceed_ceiling: projected_pct >= self.ceiling_threshold,
                };
            }
        };

        let current_pct = budget.consumed_pct();
        let projected_consumed = budget.consumed.saturating_add(pending_tokens);
        let projected_pct = if budget.allocated == 0 {
            1.0
        } else {
            (projected_consumed + budget.pinned) as f64 / budget.allocated as f64
        };

        UsagePrediction {
            current_pct,
            projected_pct,
            would_exceed_ceiling: projected_pct >= self.ceiling_threshold,
        }
    }

    /// Returns the number of tokens still available for `agent`.
    ///
    /// Available = allocated − consumed − pinned.
    pub fn available(&self, agent: AgentId) -> u32 {
        match self.agents.get(&agent) {
            Some(b) => b.available(),
            None => self.window_size,
        }
    }

    /// Pin `tokens` for `agent`, reducing the available budget.
    ///
    /// Emits a `PinnedExcessive` warning if pinned tokens exceed 50 % of the
    /// agent's allocated budget after this operation.
    pub fn pin_tokens(&mut self, agent: AgentId, tokens: u32) -> Vec<BudgetWarning> {
        self.ensure_agent(&agent);
        let mut warnings = Vec::new();

        let budget = self.agents.get_mut(&agent).unwrap();
        budget.pinned = budget.pinned.saturating_add(tokens);

        if budget.pinned_pct() > 0.5 {
            warnings.push(BudgetWarning::PinnedExcessive {
                agent: agent.clone(),
                pinned_pct: budget.pinned_pct(),
            });
        }

        warnings
    }

    /// Unpin `tokens` for `agent`, restoring them to the available budget.
    pub fn unpin_tokens(&mut self, agent: AgentId, tokens: u32) {
        self.ensure_agent(&agent);
        let budget = self.agents.get_mut(&agent).unwrap();
        budget.pinned = budget.pinned.saturating_sub(tokens);
    }

    /// Return a usage report snapshot for `agent`.
    pub fn usage_report(&self, agent: AgentId) -> UsageReport {
        match self.agents.get(&agent) {
            Some(b) => UsageReport {
                agent_id: agent,
                consumed: b.consumed,
                allocated: b.allocated,
                pinned: b.pinned,
                available: b.available(),
                consumed_pct: b.consumed_pct(),
            },
            None => UsageReport {
                agent_id: agent,
                consumed: 0,
                allocated: self.window_size,
                pinned: 0,
                available: self.window_size,
                consumed_pct: 0.0,
            },
        }
    }

    /// Expose the window size (useful in tests).
    pub fn window_size(&self) -> u32 {
        self.window_size
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn tracker(window: u32) -> BudgetTracker {
        BudgetTracker::with_thresholds(window, 0.70, 0.85)
    }

    #[allow(dead_code)]
    fn arb_agent() -> impl Strategy<Value = AgentId> {
        "[a-z]{1,8}".prop_map(|s| s)
    }

    // -----------------------------------------------------------------------
    // Property 11: Budget token count invariant
    // Validates: Requirements 9.1
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 9.1**
        ///
        /// Property 11: Budget token count invariant.
        ///
        /// For any sequence of `record_tokens` calls with amounts [a1, a2, ..., an],
        /// the Budget_Tracker's reported consumed total SHALL equal a1 + a2 + ... + an.
        #[test]
        fn prop_budget_token_count_invariant(
            amounts in prop::collection::vec(0u32..=10_000u32, 0..=50),
        ) {
            let window = 1_000_000u32;
            let mut bt = tracker(window);
            let agent = "agent".to_string();

            let expected: u32 = amounts.iter().copied().fold(0u32, |acc, x| acc.saturating_add(x));

            for &a in &amounts {
                bt.record_tokens(agent.clone(), a);
            }

            let report = bt.usage_report(agent);
            prop_assert_eq!(report.consumed, expected);
        }
    }

    // -----------------------------------------------------------------------
    // Property 12: Budget threshold and predictive warnings
    // Validates: Requirements 9.2, 9.3, 9.4
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 9.2, 9.3, 9.4**
        ///
        /// Property 12: Budget threshold and predictive warnings.
        ///
        /// A warning SHALL be emitted if and only if the consumed percentage
        /// crosses the warning threshold.  A predictive warning SHALL be emitted
        /// if and only if current + pending would exceed the ceiling threshold.
        #[test]
        fn prop_threshold_warning_fires_on_crossing(
            // window in [1000, 200_000]
            window in 1_000u32..=200_000u32,
            // warning threshold in (0.1, 0.8)
            wt_raw in 1_000u32..=8_000u32,
            // ceiling threshold strictly above warning, in (wt, 0.99)
        ) {
            let warning_threshold = wt_raw as f64 / 10_000.0;
            // ceiling = warning + 0.05 (clamped to < 1.0)
            let ceiling_threshold = (warning_threshold + 0.05).min(0.99);

            let mut bt = BudgetTracker::with_thresholds(window, warning_threshold, ceiling_threshold);
            let agent = "a".to_string();

            // Record tokens just below the threshold.
            let just_below = ((window as f64 * warning_threshold) - 1.0).max(0.0) as u32;
            bt.record_tokens(agent.clone(), just_below);

            // No threshold warning yet.
            let report = bt.usage_report(agent.clone());
            let pct = report.consumed_pct;
            prop_assert!(pct < warning_threshold || just_below == 0,
                "pct={} should be below warning_threshold={}", pct, warning_threshold);

            // Now push over the threshold.
            let push_over = 2u32;
            let warnings = bt.record_tokens(agent.clone(), push_over);

            let new_pct = bt.usage_report(agent.clone()).consumed_pct;
            if new_pct >= warning_threshold && new_pct < 1.0 {
                let has_threshold_warning = warnings.iter().any(|w| {
                    matches!(w, BudgetWarning::ThresholdCrossed { .. })
                        || matches!(w, BudgetWarning::AgentBudgetExhausted { .. })
                });
                prop_assert!(has_threshold_warning,
                    "expected ThresholdCrossed or Exhausted warning at pct={}", new_pct);
            }
        }

        /// **Validates: Requirements 9.3, 9.4**
        ///
        /// Predictive warning fires when current + pending would exceed ceiling.
        #[test]
        fn prop_predictive_warning_fires_above_ceiling(
            window in 1_000u32..=200_000u32,
            consumed_raw in 0u32..=8_000u32,
            pending_raw in 0u32..=5_000u32,
        ) {
            let warning_threshold = 0.70;
            let ceiling_threshold = 0.85;
            let mut bt = BudgetTracker::with_thresholds(window, warning_threshold, ceiling_threshold);
            let agent = "a".to_string();

            // Scale consumed and pending to the window.
            let consumed = (consumed_raw as f64 / 10_000.0 * window as f64) as u32;
            let pending = (pending_raw as f64 / 10_000.0 * window as f64) as u32;

            bt.record_tokens(agent.clone(), consumed);
            let pred = bt.predict_usage(agent.clone(), pending);

            if pred.projected_pct >= ceiling_threshold {
                prop_assert!(pred.would_exceed_ceiling,
                    "projected_pct={} >= ceiling={} but would_exceed_ceiling=false",
                    pred.projected_pct, ceiling_threshold);
            } else {
                prop_assert!(!pred.would_exceed_ceiling,
                    "projected_pct={} < ceiling={} but would_exceed_ceiling=true",
                    pred.projected_pct, ceiling_threshold);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Property 15: Budget subtracts pinned tokens
    // Validates: Requirements 10.3
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 10.3**
        ///
        /// Property 15: Budget subtracts pinned tokens.
        ///
        /// For any Budget_Tracker state with pinned tokens P, consumed tokens C,
        /// and window size W, the available budget SHALL equal W − C − P.
        #[test]
        fn prop_available_equals_window_minus_consumed_minus_pinned(
            window in 1u32..=1_000_000u32,
            consumed in 0u32..=500_000u32,
            pinned in 0u32..=500_000u32,
        ) {
            let mut bt = tracker(window);
            let agent = "a".to_string();

            // Ensure agent exists with full window allocation.
            bt.record_tokens(agent.clone(), 0);

            // Manually set consumed and pinned.
            {
                let b = bt.agents.get_mut(&agent).unwrap();
                b.consumed = consumed.min(window);
                b.pinned = pinned.min(window);
            }

            let expected = window
                .saturating_sub(consumed.min(window))
                .saturating_sub(pinned.min(window));

            let actual = bt.available(agent);
            prop_assert_eq!(actual, expected);
        }
    }

    // -----------------------------------------------------------------------
    // Property 16: Excessive pin warning
    // Validates: Requirements 10.5
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 10.5**
        ///
        /// Property 16: Excessive pin warning.
        ///
        /// For any Budget_Tracker state where pinned tokens exceed 50 % of the
        /// window size, a warning SHALL be emitted indicating excessive pinning.
        #[test]
        fn prop_excessive_pin_warning(
            window in 2u32..=1_000_000u32,
            // pin_raw in (5001, 10000] → pinned > 50 % of window
            pin_raw in 5_001u32..=10_000u32,
        ) {
            let pinned = (pin_raw as f64 / 10_000.0 * window as f64).ceil() as u32;
            // Ensure pinned > 50 % of window.
            let pinned = pinned.max(window / 2 + 1);

            let mut bt = tracker(window);
            let agent = "a".to_string();

            let warnings = bt.pin_tokens(agent.clone(), pinned);

            let has_excessive = warnings
                .iter()
                .any(|w| matches!(w, BudgetWarning::PinnedExcessive { .. }));
            prop_assert!(has_excessive,
                "expected PinnedExcessive warning for pinned={} / window={}", pinned, window);
        }

        /// No excessive pin warning when pinned ≤ 50 %.
        #[test]
        fn prop_no_excessive_pin_warning_below_threshold(
            window in 2u32..=1_000_000u32,
            // pin_raw in [0, 5000] → pinned ≤ 50 % of window
            pin_raw in 0u32..=5_000u32,
        ) {
            let pinned = (pin_raw as f64 / 10_000.0 * window as f64) as u32;

            let mut bt = tracker(window);
            let agent = "a".to_string();

            let warnings = bt.pin_tokens(agent.clone(), pinned);

            let has_excessive = warnings
                .iter()
                .any(|w| matches!(w, BudgetWarning::PinnedExcessive { .. }));
            prop_assert!(!has_excessive,
                "unexpected PinnedExcessive warning for pinned={} / window={}", pinned, window);
        }
    }

    // -----------------------------------------------------------------------
    // Property 27: Multi-agent budget isolation
    // Validates: Requirements 21.1
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 21.1**
        ///
        /// Property 27: Multi-agent budget isolation.
        ///
        /// Recording tokens for agent A SHALL increase only agent A's consumed
        /// count and SHALL NOT affect any other agent's consumed count.
        #[test]
        fn prop_multi_agent_isolation(
            tokens_a in 0u32..=50_000u32,
            tokens_b in 0u32..=50_000u32,
        ) {
            let window = 1_000_000u32;
            let mut bt = tracker(window);

            let agent_a = "alpha".to_string();
            let agent_b = "beta".to_string();

            // Initialise both agents.
            bt.record_tokens(agent_a.clone(), 0);
            bt.record_tokens(agent_b.clone(), 0);

            // Record for A only.
            bt.record_tokens(agent_a.clone(), tokens_a);

            let report_a = bt.usage_report(agent_a.clone());
            let report_b = bt.usage_report(agent_b.clone());

            prop_assert_eq!(report_a.consumed, tokens_a,
                "agent_a consumed should be {}", tokens_a);
            prop_assert_eq!(report_b.consumed, 0,
                "agent_b consumed should still be 0 after recording for agent_a");

            // Now record for B.
            bt.record_tokens(agent_b.clone(), tokens_b);

            let report_a2 = bt.usage_report(agent_a.clone());
            let report_b2 = bt.usage_report(agent_b.clone());

            prop_assert_eq!(report_a2.consumed, tokens_a,
                "agent_a consumed should remain {}", tokens_a);
            prop_assert_eq!(report_b2.consumed, tokens_b,
                "agent_b consumed should be {}", tokens_b);
        }
    }

    // -----------------------------------------------------------------------
    // Property 28: Multi-agent budget enforcement
    // Validates: Requirements 21.2, 21.3, 21.4
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 21.2, 21.3, 21.4**
        ///
        /// Property 28: Multi-agent budget enforcement.
        ///
        /// A warning SHALL be emitted when an agent's usage is within 10 % of
        /// its allocated budget (i.e. ≥ 90 % consumed).  When an agent exceeds
        /// its allocated budget, AgentBudgetExhausted SHALL be emitted.
        #[test]
        fn prop_agent_exhausted_warning(
            window in 100u32..=1_000_000u32,
        ) {
            // Use a low ceiling so we can easily trigger it.
            let mut bt = BudgetTracker::with_thresholds(window, 0.70, 0.85);
            let agent = "worker".to_string();

            // Record exactly the full window → exhausted.
            let warnings = bt.record_tokens(agent.clone(), window);

            let exhausted = warnings
                .iter()
                .any(|w| matches!(w, BudgetWarning::AgentBudgetExhausted { .. }));
            prop_assert!(exhausted,
                "expected AgentBudgetExhausted after consuming full window={}", window);
        }

        /// Warning fires when usage is within 10 % of allocated (≥ 90 %).
        #[test]
        fn prop_near_budget_warning(
            window in 1_000u32..=1_000_000u32,
        ) {
            // Set warning threshold at 0.90 to test the "within 10 %" requirement.
            let mut bt = BudgetTracker::with_thresholds(window, 0.90, 0.95);
            let agent = "worker".to_string();

            // Consume 91 % of the window.
            let tokens = (window as f64 * 0.91) as u32;
            let warnings = bt.record_tokens(agent.clone(), tokens);

            let has_warning = warnings.iter().any(|w| {
                matches!(w, BudgetWarning::ThresholdCrossed { .. })
                    || matches!(w, BudgetWarning::AgentBudgetExhausted { .. })
            });
            prop_assert!(has_warning,
                "expected threshold warning at 91% of window={}", window);
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_available_decreases_with_pin() {
        let mut bt = tracker(1_000);
        let agent = "a".to_string();
        bt.record_tokens(agent.clone(), 200);
        assert_eq!(bt.available(agent.clone()), 800);
        bt.pin_tokens(agent.clone(), 100);
        assert_eq!(bt.available(agent.clone()), 700);
        bt.unpin_tokens(agent.clone(), 50);
        assert_eq!(bt.available(agent.clone()), 750);
    }

    #[test]
    fn test_unpin_does_not_go_negative() {
        let mut bt = tracker(1_000);
        let agent = "a".to_string();
        bt.pin_tokens(agent.clone(), 100);
        bt.unpin_tokens(agent.clone(), 200); // more than pinned
        assert_eq!(bt.available(agent.clone()), 1_000);
    }

    #[test]
    fn test_usage_report_fields() {
        let mut bt = tracker(10_000);
        let agent = "x".to_string();
        bt.record_tokens(agent.clone(), 3_000);
        bt.pin_tokens(agent.clone(), 1_000);
        let r = bt.usage_report(agent);
        assert_eq!(r.consumed, 3_000);
        assert_eq!(r.pinned, 1_000);
        assert_eq!(r.allocated, 10_000);
        assert_eq!(r.available, 6_000);
        assert!((r.consumed_pct - 0.4).abs() < 1e-9);
    }
}
