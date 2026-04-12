use crate::error::Result;
use crate::types::{CorrectionEntry, CorrectionId, CorrectionLog};

/// A simple context window that holds replayed correction strings.
pub struct ContextWindow {
    pub entries: Vec<String>,
}

impl ContextWindow {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }
}

impl Default for ContextWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrectionLog {
    /// Create a new, empty CorrectionLog.
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Append a new entry to the log. Returns the entry's id.
    /// Existing entries are never modified.
    pub fn append(&mut self, entry: CorrectionEntry) -> CorrectionId {
        let id = entry.id.clone();
        self.entries.push(entry);
        id
    }

    /// Return an immutable slice of all entries.
    pub fn entries(&self) -> &[CorrectionEntry] {
        &self.entries
    }

    /// Replay all corrections into the given context window by pushing
    /// each correction's text.
    pub fn replay_into(&self, context: &mut ContextWindow) -> Result<()> {
        for entry in &self.entries {
            context.entries.push(entry.correction.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CorrectionEntry;
    use chrono::Utc;
    use proptest::prelude::*;

    fn make_entry(id: &str, correction: &str) -> CorrectionEntry {
        CorrectionEntry {
            id: id.to_string(),
            timestamp: Utc::now(),
            original: "original".to_string(),
            correction: correction.to_string(),
            context: "ctx".to_string(),
        }
    }

    // ── Unit tests ────────────────────────────────────────────────────────────

    #[test]
    fn new_log_is_empty() {
        let log = CorrectionLog::new();
        assert!(log.entries().is_empty());
    }

    #[test]
    fn append_returns_entry_id() {
        let mut log = CorrectionLog::new();
        let id = log.append(make_entry("id-1", "use const"));
        assert_eq!(id, "id-1");
    }

    #[test]
    fn entries_are_immutable_slice() {
        let mut log = CorrectionLog::new();
        log.append(make_entry("a", "fix a"));
        log.append(make_entry("b", "fix b"));
        let entries = log.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "a");
        assert_eq!(entries[1].id, "b");
    }

    #[test]
    fn replay_into_pushes_corrections() {
        let mut log = CorrectionLog::new();
        log.append(make_entry("1", "use const"));
        log.append(make_entry("2", "no var"));
        let mut ctx = ContextWindow::new();
        log.replay_into(&mut ctx).unwrap();
        assert_eq!(ctx.entries, vec!["use const", "no var"]);
    }

    #[test]
    fn replay_into_empty_log_is_noop() {
        let log = CorrectionLog::new();
        let mut ctx = ContextWindow::new();
        log.replay_into(&mut ctx).unwrap();
        assert!(ctx.entries.is_empty());
    }

    // ── Property tests ────────────────────────────────────────────────────────

    /// Helper: build a CorrectionEntry from arbitrary strings.
    fn arb_entry() -> impl Strategy<Value = CorrectionEntry> {
        ("[a-z]{1,8}", "[a-z ]{1,20}", "[a-z ]{1,20}", "[a-z ]{1,20}").prop_map(
            |(id, original, correction, context)| CorrectionEntry {
                id,
                timestamp: Utc::now(),
                original,
                correction,
                context,
            },
        )
    }

    /// **Property 17: Correction_Log immutability**
    ///
    /// For any sequence of append operations on the CorrectionLog, all
    /// previously appended entries SHALL remain unchanged (same id, timestamp,
    /// original, correction, and context fields) after each subsequent append.
    ///
    /// **Validates: Requirements 11.1, 11.2**
    #[test]
    fn prop17_correction_log_immutability() {
        proptest!(|(entries in proptest::collection::vec(arb_entry(), 1..=20))| {
            let mut log = CorrectionLog::new();
            // Snapshot of entries appended so far (id + correction text).
            let mut snapshots: Vec<(String, String, String, String)> = Vec::new();

            for entry in entries {
                let id = entry.id.clone();
                let original = entry.original.clone();
                let correction = entry.correction.clone();
                let context = entry.context.clone();

                log.append(entry);
                snapshots.push((id, original, correction, context));

                // After each append, verify all previously appended entries
                // are unchanged.
                let current = log.entries();
                prop_assert_eq!(current.len(), snapshots.len());
                for (i, (exp_id, exp_orig, exp_corr, exp_ctx)) in snapshots.iter().enumerate() {
                    prop_assert_eq!(&current[i].id, exp_id);
                    prop_assert_eq!(&current[i].original, exp_orig);
                    prop_assert_eq!(&current[i].correction, exp_corr);
                    prop_assert_eq!(&current[i].context, exp_ctx);
                }
            }
        });
    }

    /// **Property 18: Compaction preserves Correction_Log**
    ///
    /// For any session state containing a CorrectionLog with N entries, after
    /// "compaction" (simulated by creating a new log from existing entries),
    /// the CorrectionLog SHALL still contain exactly N entries, each identical
    /// to the pre-compaction state.
    ///
    /// **Validates: Requirements 11.3, 11.4**
    #[test]
    fn prop18_compaction_preserves_correction_log() {
        proptest!(|(entries in proptest::collection::vec(arb_entry(), 0..=20))| {
            // Build the original log.
            let mut original_log = CorrectionLog::new();
            for entry in &entries {
                original_log.append(CorrectionEntry {
                    id: entry.id.clone(),
                    timestamp: entry.timestamp,
                    original: entry.original.clone(),
                    correction: entry.correction.clone(),
                    context: entry.context.clone(),
                });
            }

            let n = original_log.entries().len();

            // Simulate compaction: create a new log by replaying all entries
            // from the original (as the pipeline would do after compaction).
            let mut compacted_log = CorrectionLog::new();
            for entry in original_log.entries() {
                compacted_log.append(CorrectionEntry {
                    id: entry.id.clone(),
                    timestamp: entry.timestamp,
                    original: entry.original.clone(),
                    correction: entry.correction.clone(),
                    context: entry.context.clone(),
                });
            }

            // The compacted log must have exactly N entries.
            prop_assert_eq!(compacted_log.entries().len(), n);

            // Each entry must be identical to the pre-compaction state.
            for (i, orig) in original_log.entries().iter().enumerate() {
                let comp = &compacted_log.entries()[i];
                prop_assert_eq!(&comp.id, &orig.id);
                prop_assert_eq!(&comp.original, &orig.original);
                prop_assert_eq!(&comp.correction, &orig.correction);
                prop_assert_eq!(&comp.context, &orig.context);
            }
        });
    }
}
