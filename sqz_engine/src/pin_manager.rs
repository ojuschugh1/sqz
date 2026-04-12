use chrono::Utc;

use crate::error::{Result, SqzError};
use crate::session_store::SessionStore;
use crate::types::PinEntry;

/// Manages pin/unpin operations for conversation turns within a session.
///
/// Pins mark content as protected from compaction. Pinned turns are excluded
/// from all compaction operations. Unpinning restores compaction eligibility.
/// Pin metadata is persisted in the `SessionStore` so pins survive restarts.
pub struct PinManager {
    store: SessionStore,
}

impl PinManager {
    /// Create a new `PinManager` backed by the given `SessionStore`.
    pub fn new(store: SessionStore) -> Self {
        Self { store }
    }

    /// Pin a conversation turn by index.
    ///
    /// Creates or updates a `PinEntry` for the given `turn_index` in the
    /// session identified by `session_id`. The session's `conversation` entry
    /// at `turn_index` is also marked `pinned = true`. Returns the resulting
    /// `PinEntry`.
    pub fn pin(
        &self,
        session_id: &str,
        turn_index: usize,
        reason: &str,
        tokens: u32,
    ) -> Result<PinEntry> {
        let mut session = self.store.load_session(session_id.to_string())?;

        // Validate turn_index
        if turn_index >= session.conversation.len() {
            return Err(SqzError::Other(format!(
                "turn_index {turn_index} out of range (session has {} turns)",
                session.conversation.len()
            )));
        }

        // Mark the conversation turn as pinned
        session.conversation[turn_index].pinned = true;

        // Upsert the PinEntry
        let entry = PinEntry {
            turn_index,
            reason: reason.to_string(),
            tokens,
        };

        if let Some(existing) = session.pins.iter_mut().find(|p| p.turn_index == turn_index) {
            *existing = entry.clone();
        } else {
            session.pins.push(entry.clone());
        }

        session.updated_at = Utc::now();
        self.store.save_session(&session)?;

        Ok(entry)
    }

    /// Unpin a conversation turn by index.
    ///
    /// Removes the `PinEntry` for `turn_index` and sets `pinned = false` on
    /// the corresponding `ConversationTurn`. After unpinning the turn is
    /// eligible for compaction again.
    pub fn unpin(&self, session_id: &str, turn_index: usize) -> Result<()> {
        let mut session = self.store.load_session(session_id.to_string())?;

        // Remove the PinEntry (if present)
        session.pins.retain(|p| p.turn_index != turn_index);

        // Clear the pinned flag on the conversation turn (if in range)
        if turn_index < session.conversation.len() {
            session.conversation[turn_index].pinned = false;
        }

        session.updated_at = Utc::now();
        self.store.save_session(&session)?;

        Ok(())
    }

    /// Return `true` if the turn at `turn_index` is currently pinned.
    pub fn is_pinned(&self, session_id: &str, turn_index: usize) -> Result<bool> {
        let session = self.store.load_session(session_id.to_string())?;
        Ok(session.pins.iter().any(|p| p.turn_index == turn_index))
    }

    /// Return all `PinEntry` records for the session.
    pub fn get_pins(&self, session_id: &str) -> Result<Vec<PinEntry>> {
        let session = self.store.load_session(session_id.to_string())?;
        Ok(session.pins)
    }

    /// Return `true` if the turn at `turn_index` is eligible for compaction
    /// (i.e., it is **not** pinned).
    pub fn is_compaction_eligible(&self, session_id: &str, turn_index: usize) -> Result<bool> {
        Ok(!self.is_pinned(session_id, turn_index)?)
    }
}

// ── Helpers (test-only) ───────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use crate::types::{BudgetState, ConversationTurn, CorrectionLog, ModelFamily, Role, SessionState};
    use rusqlite::Connection;
    use crate::session_store::apply_schema;

    pub fn in_memory_store() -> SessionStore {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        SessionStore::from_connection(conn)
    }

    pub fn make_session_with_turns(id: &str, num_turns: usize) -> SessionState {
        let now = chrono::Utc::now();
        let conversation = (0..num_turns)
            .map(|i| ConversationTurn {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                content: format!("turn {i}"),
                tokens: 10,
                pinned: false,
                timestamp: now,
            })
            .collect();

        SessionState {
            id: id.to_string(),
            project_dir: std::path::PathBuf::from("/proj"),
            conversation,
            corrections: CorrectionLog::default(),
            pins: vec![],
            learnings: vec![],
            compressed_summary: String::new(),
            budget: BudgetState {
                window_size: 200_000,
                consumed: 0,
                pinned: 0,
                model_family: ModelFamily::AnthropicClaude,
            },
            tool_usage: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;
    use proptest::prelude::*;

    // ── Unit tests ────────────────────────────────────────────────────────────

    #[test]
    fn test_pin_marks_turn_protected() {
        let store = in_memory_store();
        let session = make_session_with_turns("s1", 3);
        store.save_session(&session).unwrap();

        let pm = PinManager::new(store);
        pm.pin("s1", 1, "important", 10).unwrap();

        assert!(pm.is_pinned("s1", 1).unwrap());
        assert!(!pm.is_compaction_eligible("s1", 1).unwrap());
    }

    #[test]
    fn test_unpin_restores_eligibility() {
        let store = in_memory_store();
        let session = make_session_with_turns("s1", 3);
        store.save_session(&session).unwrap();

        let pm = PinManager::new(store);
        pm.pin("s1", 0, "reason", 10).unwrap();
        pm.unpin("s1", 0).unwrap();

        assert!(!pm.is_pinned("s1", 0).unwrap());
        assert!(pm.is_compaction_eligible("s1", 0).unwrap());
    }

    #[test]
    fn test_unpinned_turns_are_compaction_eligible_by_default() {
        let store = in_memory_store();
        let session = make_session_with_turns("s1", 5);
        store.save_session(&session).unwrap();

        let pm = PinManager::new(store);
        for i in 0..5 {
            assert!(pm.is_compaction_eligible("s1", i).unwrap());
        }
    }

    #[test]
    fn test_get_pins_returns_all_pins() {
        let store = in_memory_store();
        let session = make_session_with_turns("s1", 5);
        store.save_session(&session).unwrap();

        let pm = PinManager::new(store);
        pm.pin("s1", 0, "r0", 5).unwrap();
        pm.pin("s1", 2, "r2", 15).unwrap();
        pm.pin("s1", 4, "r4", 20).unwrap();

        let pins = pm.get_pins("s1").unwrap();
        assert_eq!(pins.len(), 3);
        let indices: Vec<usize> = pins.iter().map(|p| p.turn_index).collect();
        assert!(indices.contains(&0));
        assert!(indices.contains(&2));
        assert!(indices.contains(&4));
    }

    #[test]
    fn test_pin_out_of_range_errors() {
        let store = in_memory_store();
        let session = make_session_with_turns("s1", 2);
        store.save_session(&session).unwrap();

        let pm = PinManager::new(store);
        let result = pm.pin("s1", 99, "reason", 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_pin_upserts_existing_entry() {
        let store = in_memory_store();
        let session = make_session_with_turns("s1", 3);
        store.save_session(&session).unwrap();

        let pm = PinManager::new(store);
        pm.pin("s1", 1, "first reason", 10).unwrap();
        pm.pin("s1", 1, "updated reason", 20).unwrap();

        let pins = pm.get_pins("s1").unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].reason, "updated reason");
        assert_eq!(pins[0].tokens, 20);
    }

    // ── Property tests ────────────────────────────────────────────────────────

    /// **Property 13: Pin/unpin compaction round-trip**
    ///
    /// **Validates: Requirements 10.1, 10.4**
    ///
    /// For any turn_index, after pin then unpin, `is_compaction_eligible`
    /// returns `true`.
    proptest! {
        #[test]
        fn prop_pin_unpin_restores_compaction_eligibility(
            num_turns in 1usize..=20usize,
            turn_index in 0usize..20usize,
            tokens in 1u32..=1000u32,
        ) {
            // Clamp turn_index to valid range
            let turn_index = turn_index % num_turns;

            let store = in_memory_store();
            let session = make_session_with_turns("s1", num_turns);
            store.save_session(&session).unwrap();

            let pm = PinManager::new(store);

            // Before pinning: eligible
            prop_assert!(pm.is_compaction_eligible("s1", turn_index).unwrap());

            // After pinning: not eligible
            pm.pin("s1", turn_index, "test reason", tokens).unwrap();
            prop_assert!(!pm.is_compaction_eligible("s1", turn_index).unwrap());

            // After unpinning: eligible again
            pm.unpin("s1", turn_index).unwrap();
            prop_assert!(pm.is_compaction_eligible("s1", turn_index).unwrap());
        }
    }

    /// **Property 14: Pin persistence**
    ///
    /// **Validates: Requirements 10.2**
    ///
    /// For any set of pin entries saved to the Session_Store, reloading the
    /// session SHALL produce the same set of pin entries.
    proptest! {
        #[test]
        fn prop_pins_persist_across_store_reload(
            num_turns in 2usize..=10usize,
            // Indices to pin (as fractions of num_turns, deduplicated)
            raw_indices in proptest::collection::vec(0usize..10usize, 1..=5usize),
        ) {
            let num_turns = num_turns.max(2);
            // Deduplicate and clamp indices
            let mut pin_indices: Vec<usize> = raw_indices
                .into_iter()
                .map(|i| i % num_turns)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            pin_indices.sort_unstable();

            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("store.db");

            // --- Phase 1: save pins ---
            {
                let store = SessionStore::open_or_create(&db_path).unwrap();
                let session = make_session_with_turns("s1", num_turns);
                store.save_session(&session).unwrap();

                let pm = PinManager::new(store);
                for &idx in &pin_indices {
                    pm.pin("s1", idx, "persisted reason", 42).unwrap();
                }
            }

            // --- Phase 2: reopen store and verify pins ---
            {
                let store = SessionStore::open_or_create(&db_path).unwrap();
                let pm = PinManager::new(store);

                let pins = pm.get_pins("s1").unwrap();
                let persisted_indices: std::collections::HashSet<usize> =
                    pins.iter().map(|p| p.turn_index).collect();

                for &idx in &pin_indices {
                    prop_assert!(
                        persisted_indices.contains(&idx),
                        "pin at turn_index {} was not persisted",
                        idx
                    );
                }

                prop_assert_eq!(
                    pins.len(),
                    pin_indices.len(),
                    "expected {} pins, found {}",
                    pin_indices.len(),
                    pins.len()
                );
            }
        }
    }
}
