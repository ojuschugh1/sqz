use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Throttle level applied to a tool call based on repetition count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThrottleLevel {
    /// Calls within the normal range — full results returned.
    Normal,
    /// Calls within the reduced range — result size is cut (e.g. 2 → 1).
    Reduced,
    /// Calls at or above the blocked threshold — redirect to batch.
    Blocked,
}

/// Configurable thresholds for progressive throttling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleConfig {
    /// Maximum call count that stays in the Normal band (inclusive).
    /// Default: 3
    #[serde(default = "default_normal_limit")]
    pub normal_limit: u32,
    /// Maximum call count that stays in the Reduced band (inclusive).
    /// Default: 8
    #[serde(default = "default_reduced_limit")]
    pub reduced_limit: u32,
}

fn default_normal_limit() -> u32 {
    3
}

fn default_reduced_limit() -> u32 {
    8
}

impl Default for ThrottleConfig {
    fn default() -> Self {
        Self {
            normal_limit: default_normal_limit(),
            reduced_limit: default_reduced_limit(),
        }
    }
}

/// Composite key: (tool_name, params_hash).
type CallKey = (String, u64);

/// Tracks per-tool call counts and applies progressive throttling.
///
/// Calls 1..=normal_limit  → Normal
/// Calls (normal_limit+1)..=reduced_limit → Reduced
/// Calls > reduced_limit   → Blocked
#[derive(Debug, Clone)]
pub struct ProgressiveThrottler {
    config: ThrottleConfig,
    counts: HashMap<CallKey, u32>,
}

impl ProgressiveThrottler {
    /// Create a new throttler with the given configuration.
    pub fn new(config: ThrottleConfig) -> Self {
        Self {
            config,
            counts: HashMap::new(),
        }
    }

    /// Record a call and return the resulting throttle level *after* incrementing.
    pub fn record_call(&mut self, tool_name: &str, params_hash: u64) -> ThrottleLevel {
        let key = (tool_name.to_string(), params_hash);
        let count = self.counts.entry(key).or_insert(0);
        *count += 1;
        let c = *count;
        self.level_for_count(c)
    }

    /// Check the current throttle level for a tool+params without recording a call.
    /// Returns `Normal` if no calls have been recorded yet.
    pub fn get_level(&self, tool_name: &str, params_hash: u64) -> ThrottleLevel {
        let key = (tool_name.to_string(), params_hash);
        let count = self.counts.get(&key).copied().unwrap_or(0);
        self.level_for_count(count)
    }

    /// Reset all counters (e.g. on task context change or new user input).
    pub fn reset(&mut self) {
        self.counts.clear();
    }

    /// Reset counters for a specific tool (all param hashes).
    pub fn reset_tool(&mut self, tool_name: &str) {
        self.counts.retain(|(name, _), _| name != tool_name);
    }

    /// Return the current call count for a given key.
    pub fn call_count(&self, tool_name: &str, params_hash: u64) -> u32 {
        let key = (tool_name.to_string(), params_hash);
        self.counts.get(&key).copied().unwrap_or(0)
    }

    // ---- internal ----

    fn level_for_count(&self, count: u32) -> ThrottleLevel {
        if count <= self.config.normal_limit {
            ThrottleLevel::Normal
        } else if count <= self.config.reduced_limit {
            ThrottleLevel::Reduced
        } else {
            ThrottleLevel::Blocked
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn default_throttler() -> ProgressiveThrottler {
        ProgressiveThrottler::new(ThrottleConfig::default())
    }

    #[test]
    fn normal_band_for_first_three_calls() {
        let mut t = default_throttler();
        assert_eq!(t.record_call("read_file", 42), ThrottleLevel::Normal);
        assert_eq!(t.record_call("read_file", 42), ThrottleLevel::Normal);
        assert_eq!(t.record_call("read_file", 42), ThrottleLevel::Normal);
    }

    #[test]
    fn reduced_band_for_calls_four_through_eight() {
        let mut t = default_throttler();
        for _ in 0..3 {
            t.record_call("read_file", 1);
        }
        for i in 4..=8 {
            let level = t.record_call("read_file", 1);
            assert_eq!(level, ThrottleLevel::Reduced, "call {i} should be Reduced");
        }
    }

    #[test]
    fn blocked_after_eight_calls() {
        let mut t = default_throttler();
        for _ in 0..8 {
            t.record_call("search", 99);
        }
        assert_eq!(t.record_call("search", 99), ThrottleLevel::Blocked);
        assert_eq!(t.record_call("search", 99), ThrottleLevel::Blocked);
    }

    #[test]
    fn different_params_hash_tracked_independently() {
        let mut t = default_throttler();
        for _ in 0..8 {
            t.record_call("read_file", 1);
        }
        // Same tool, different params hash — should still be Normal
        assert_eq!(t.record_call("read_file", 2), ThrottleLevel::Normal);
    }

    #[test]
    fn different_tools_tracked_independently() {
        let mut t = default_throttler();
        for _ in 0..8 {
            t.record_call("read_file", 1);
        }
        assert_eq!(t.record_call("write_file", 1), ThrottleLevel::Normal);
    }

    #[test]
    fn get_level_without_recording() {
        let mut t = default_throttler();
        // No calls yet → Normal
        assert_eq!(t.get_level("read_file", 1), ThrottleLevel::Normal);

        for _ in 0..4 {
            t.record_call("read_file", 1);
        }
        // 4 calls recorded → Reduced
        assert_eq!(t.get_level("read_file", 1), ThrottleLevel::Reduced);
        // get_level should not increment
        assert_eq!(t.call_count("read_file", 1), 4);
    }

    #[test]
    fn reset_clears_all_counters() {
        let mut t = default_throttler();
        for _ in 0..5 {
            t.record_call("read_file", 1);
            t.record_call("search", 2);
        }
        t.reset();
        assert_eq!(t.get_level("read_file", 1), ThrottleLevel::Normal);
        assert_eq!(t.get_level("search", 2), ThrottleLevel::Normal);
        assert_eq!(t.call_count("read_file", 1), 0);
    }

    #[test]
    fn reset_tool_clears_only_that_tool() {
        let mut t = default_throttler();
        for _ in 0..5 {
            t.record_call("read_file", 1);
            t.record_call("read_file", 99);
            t.record_call("search", 2);
        }
        t.reset_tool("read_file");
        assert_eq!(t.call_count("read_file", 1), 0);
        assert_eq!(t.call_count("read_file", 99), 0);
        assert_eq!(t.call_count("search", 2), 5);
    }

    #[test]
    fn custom_thresholds() {
        let config = ThrottleConfig {
            normal_limit: 1,
            reduced_limit: 2,
        };
        let mut t = ProgressiveThrottler::new(config);
        assert_eq!(t.record_call("t", 0), ThrottleLevel::Normal);   // call 1
        assert_eq!(t.record_call("t", 0), ThrottleLevel::Reduced);  // call 2
        assert_eq!(t.record_call("t", 0), ThrottleLevel::Blocked);  // call 3
    }

    #[test]
    fn zero_count_is_normal() {
        let t = default_throttler();
        assert_eq!(t.get_level("nonexistent", 0), ThrottleLevel::Normal);
    }

    #[test]
    fn call_count_tracks_correctly() {
        let mut t = default_throttler();
        assert_eq!(t.call_count("x", 1), 0);
        t.record_call("x", 1);
        assert_eq!(t.call_count("x", 1), 1);
        t.record_call("x", 1);
        t.record_call("x", 1);
        assert_eq!(t.call_count("x", 1), 3);
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        /// **Validates: Requirements 40.1, 40.2**
        ///
        /// Property 42: Progressive throttling enforces limits
        ///
        /// For any sequence of N identical tool calls with arbitrary config
        /// thresholds, calls 1..=normal_limit are Normal, calls
        /// (normal_limit+1)..=reduced_limit are Reduced, and calls
        /// > reduced_limit are Blocked. After reset(), all levels return
        /// to Normal.
        proptest! {
            #[test]
            fn progressive_throttling_enforces_limits(
                normal_limit in 1u32..=20,
                gap in 1u32..=20,
                extra_calls in 1u32..=10,
                tool_name in "[a-z_]{1,12}",
                params_hash in any::<u64>(),
            ) {
                let reduced_limit = normal_limit + gap;
                let total_calls = reduced_limit + extra_calls;

                let config = ThrottleConfig {
                    normal_limit,
                    reduced_limit,
                };
                let mut throttler = ProgressiveThrottler::new(config);

                for call_num in 1..=total_calls {
                    let level = throttler.record_call(&tool_name, params_hash);

                    if call_num <= normal_limit {
                        prop_assert_eq!(
                            level,
                            ThrottleLevel::Normal,
                            "call {} should be Normal (normal_limit={})",
                            call_num,
                            normal_limit,
                        );
                    } else if call_num <= reduced_limit {
                        prop_assert_eq!(
                            level,
                            ThrottleLevel::Reduced,
                            "call {} should be Reduced (normal_limit={}, reduced_limit={})",
                            call_num,
                            normal_limit,
                            reduced_limit,
                        );
                    } else {
                        prop_assert_eq!(
                            level,
                            ThrottleLevel::Blocked,
                            "call {} should be Blocked (reduced_limit={})",
                            call_num,
                            reduced_limit,
                        );
                    }
                }

                // After reset, all levels return to Normal
                throttler.reset();
                prop_assert_eq!(
                    throttler.get_level(&tool_name, params_hash),
                    ThrottleLevel::Normal,
                    "after reset(), level should be Normal",
                );
                prop_assert_eq!(
                    throttler.call_count(&tool_name, params_hash),
                    0,
                    "after reset(), call count should be 0",
                );
            }
        }
    }
}
