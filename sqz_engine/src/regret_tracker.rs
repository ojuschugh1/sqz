//! Compression Regret Tracker — learns from compression mistakes to improve
//! future decisions per-file and per-content-type.
//!
//! When the LLM re-reads a file that was served from dedup cache, or when
//! the verifier triggers a safe-mode fallback, that's a "regret event."
//! The tracker records these and adjusts compression aggressiveness per file.

use std::collections::HashMap;

/// A single regret event.
#[derive(Debug, Clone)]
pub struct RegretEvent {
    /// The file path or content identifier.
    pub content_id: String,
    /// What happened.
    pub kind: RegretKind,
    /// Turn number when the regret occurred.
    pub turn: u64,
}

/// Types of compression regret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegretKind {
    /// LLM re-read a file that was served from dedup cache.
    DedupReRead,
    /// Verifier triggered safe-mode fallback (compression was too aggressive).
    VerifierFallback,
    /// LLM asked about content that was compressed away.
    InformationLoss,
}

/// Per-file compression profile learned from regret events.
#[derive(Debug, Clone)]
pub struct FileProfile {
    /// Number of times this file triggered a regret event.
    pub regret_count: u32,
    /// Recommended compression aggressiveness (0.0 = safe, 1.0 = aggressive).
    pub aggressiveness: f64,
    /// Last turn this file was accessed.
    pub last_access_turn: u64,
}

impl Default for FileProfile {
    fn default() -> Self {
        Self {
            regret_count: 0,
            aggressiveness: 0.5, // default: balanced
            last_access_turn: 0,
        }
    }
}

/// Tracks compression regret events and learns per-file compression profiles.
pub struct RegretTracker {
    /// Per-file profiles.
    profiles: HashMap<String, FileProfile>,
    /// All regret events (for analysis).
    events: Vec<RegretEvent>,
    /// How much to reduce aggressiveness per regret event.
    decay_rate: f64,
    /// Minimum aggressiveness (never go below this).
    min_aggressiveness: f64,
}

impl Default for RegretTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl RegretTracker {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
            events: Vec::new(),
            decay_rate: 0.15,
            min_aggressiveness: 0.1,
        }
    }

    /// Record a regret event and update the file's compression profile.
    pub fn record_regret(&mut self, event: RegretEvent) {
        let profile = self.profiles
            .entry(event.content_id.clone())
            .or_default();

        profile.regret_count += 1;
        profile.last_access_turn = event.turn;

        // Reduce aggressiveness based on regret type
        let penalty = match event.kind {
            RegretKind::DedupReRead => self.decay_rate * 0.5,
            RegretKind::VerifierFallback => self.decay_rate,
            RegretKind::InformationLoss => self.decay_rate * 2.0,
        };

        profile.aggressiveness = (profile.aggressiveness - penalty)
            .max(self.min_aggressiveness);

        self.events.push(event);
    }

    /// Record a successful compression (no regret) — slowly increase aggressiveness.
    pub fn record_success(&mut self, content_id: &str, turn: u64) {
        let profile = self.profiles
            .entry(content_id.to_string())
            .or_default();

        profile.last_access_turn = turn;

        // Slowly recover aggressiveness after successful compressions
        profile.aggressiveness = (profile.aggressiveness + 0.02).min(1.0);
    }

    /// Get the recommended compression aggressiveness for a file.
    /// Returns 0.0-1.0 where 0.0 = safe mode, 1.0 = maximum compression.
    pub fn recommended_aggressiveness(&self, content_id: &str) -> f64 {
        self.profiles
            .get(content_id)
            .map(|p| p.aggressiveness)
            .unwrap_or(0.5) // default: balanced
    }

    /// Get the profile for a specific file.
    pub fn get_profile(&self, content_id: &str) -> Option<&FileProfile> {
        self.profiles.get(content_id)
    }

    /// Get total regret event count.
    pub fn total_regrets(&self) -> usize {
        self.events.len()
    }

    /// Get a summary of the most regretted files.
    pub fn most_regretted(&self, top_n: usize) -> Vec<(&str, &FileProfile)> {
        let mut sorted: Vec<_> = self.profiles.iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        sorted.sort_by(|a, b| b.1.regret_count.cmp(&a.1.regret_count));
        sorted.truncate(top_n);
        sorted
    }

    /// Format a human-readable regret report.
    pub fn format_report(&self) -> String {
        let mut out = format!("sqz regret tracker: {} events\n", self.events.len());

        let top = self.most_regretted(5);
        if top.is_empty() {
            out.push_str("  No regret events recorded.\n");
            return out;
        }

        out.push_str("  Most regretted files:\n");
        for (path, profile) in &top {
            out.push_str(&format!(
                "    {} — {} regrets, aggressiveness: {:.2}\n",
                path, profile.regret_count, profile.aggressiveness
            ));
        }
        out
    }

    /// Reset all profiles (e.g., on project change).
    pub fn reset(&mut self) {
        self.profiles.clear();
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tracker_empty() {
        let tracker = RegretTracker::new();
        assert_eq!(tracker.total_regrets(), 0);
        assert_eq!(tracker.recommended_aggressiveness("any.rs"), 0.5);
    }

    #[test]
    fn test_record_regret_reduces_aggressiveness() {
        let mut tracker = RegretTracker::new();
        let initial = tracker.recommended_aggressiveness("auth.rs");

        tracker.record_regret(RegretEvent {
            content_id: "auth.rs".into(),
            kind: RegretKind::VerifierFallback,
            turn: 1,
        });

        let after = tracker.recommended_aggressiveness("auth.rs");
        assert!(after < initial, "aggressiveness should decrease after regret");
    }

    #[test]
    fn test_multiple_regrets_compound() {
        let mut tracker = RegretTracker::new();

        for i in 0..5 {
            tracker.record_regret(RegretEvent {
                content_id: "config.yaml".into(),
                kind: RegretKind::InformationLoss,
                turn: i,
            });
        }

        let agg = tracker.recommended_aggressiveness("config.yaml");
        assert_eq!(agg, 0.1, "should hit minimum aggressiveness after many regrets");
    }

    #[test]
    fn test_success_recovers_aggressiveness() {
        let mut tracker = RegretTracker::new();

        tracker.record_regret(RegretEvent {
            content_id: "lib.rs".into(),
            kind: RegretKind::DedupReRead,
            turn: 1,
        });

        let after_regret = tracker.recommended_aggressiveness("lib.rs");

        for i in 2..20 {
            tracker.record_success("lib.rs", i);
        }

        let after_recovery = tracker.recommended_aggressiveness("lib.rs");
        assert!(after_recovery > after_regret, "aggressiveness should recover after successes");
    }

    #[test]
    fn test_most_regretted() {
        let mut tracker = RegretTracker::new();

        for _ in 0..3 {
            tracker.record_regret(RegretEvent {
                content_id: "a.rs".into(),
                kind: RegretKind::DedupReRead,
                turn: 1,
            });
        }
        tracker.record_regret(RegretEvent {
            content_id: "b.rs".into(),
            kind: RegretKind::DedupReRead,
            turn: 2,
        });

        let top = tracker.most_regretted(2);
        assert_eq!(top[0].0, "a.rs");
        assert_eq!(top[0].1.regret_count, 3);
    }

    #[test]
    fn test_format_report() {
        let mut tracker = RegretTracker::new();
        tracker.record_regret(RegretEvent {
            content_id: "test.rs".into(),
            kind: RegretKind::VerifierFallback,
            turn: 1,
        });
        let report = tracker.format_report();
        assert!(report.contains("test.rs"));
        assert!(report.contains("1 regrets"));
    }

    #[test]
    fn test_reset_clears_all() {
        let mut tracker = RegretTracker::new();
        tracker.record_regret(RegretEvent {
            content_id: "x.rs".into(),
            kind: RegretKind::DedupReRead,
            turn: 1,
        });
        tracker.reset();
        assert_eq!(tracker.total_regrets(), 0);
        assert_eq!(tracker.recommended_aggressiveness("x.rs"), 0.5);
    }
}
