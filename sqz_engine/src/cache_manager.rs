use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::delta_encoder::DeltaEncoder;
use crate::error::Result;
use crate::pipeline::{CompressionPipeline, SessionContext};
use crate::preset::Preset;
use crate::session_store::SessionStore;
use crate::types::CompressedContent;

/// Outcome of a cache lookup in [`CacheManager`].
///
/// The cache has three possible outcomes:
/// - **Dedup**: exact match, returns a tiny `§ref:HASH§` token (~13 tokens)
/// - **Delta**: near-duplicate, returns a compact diff against the cached version
/// - **Fresh**: cache miss, returns the full compressed output
pub enum CacheResult {
    /// Previously seen content — returns a short inline reference (~13 tokens).
    Dedup {
        /// Inline token of the form `§ref:<hash_prefix>§`.
        inline_ref: String,
        /// Approximate token cost of the reference (always 13).
        token_cost: u32,
    },
    /// Near-duplicate of cached content — returns a compact delta.
    Delta {
        /// The delta text (header + changed lines).
        delta_text: String,
        /// Approximate token cost of the delta.
        token_cost: u32,
        /// Similarity to the cached version (0.0–1.0).
        similarity: f64,
    },
    /// Content not seen before — full compression result.
    Fresh { output: CompressedContent },
}

/// Tracks when a dedup ref was last sent, so we can detect staleness.
///
/// Historically used for an in-memory per-process turn counter; now kept
/// only for interface compatibility (clear on notify_compaction). Actual
/// staleness is computed from SQLite `accessed_at` timestamps so it works
/// across the shell-hook invocation model where each sqz process is short-
/// lived. See the comment on `is_ref_fresh` for details.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct RefEntry {
    /// The turn number when this ref was last sent to the LLM.
    last_sent_turn: u64,
}

/// SHA-256 content-hash deduplication cache backed by [`SessionStore`],
/// with delta encoding for near-duplicate content and compaction awareness.
///
/// # Freshness model
///
/// A dedup ref is considered fresh (safe to serve instead of the full
/// content) when the cache entry's `accessed_at` timestamp in SQLite is
/// within `max_ref_age` of now. When sqz is invoked from shell hooks each
/// invocation is a short-lived process, so the freshness check must be
/// persistent — in-memory state is gone the moment the process exits.
///
/// The previous turn-counter heuristic was in-memory only and therefore
/// never registered freshness across hook invocations, which silently
/// disabled the dedup feature in production. Issue found April 18 2026.
///
/// Default TTL: 30 minutes. Empirically matches a typical active coding
/// session before a context compaction. Use [`with_ref_age`] to tune.
pub struct CacheManager {
    store: SessionStore,
    max_size_bytes: u64,
    delta_encoder: DeltaEncoder,
    /// Retained for notify_compaction's semantic ("forget all tracked refs"),
    /// but no longer consulted for freshness checks.
    #[allow(dead_code)]
    turn_counter: std::cell::Cell<u64>,
    /// Retained for notify_compaction; cleared on compaction events.
    #[allow(dead_code)]
    ref_tracker: std::cell::RefCell<HashMap<String, RefEntry>>,
    /// Maximum wall-clock age before a dedup ref is considered stale.
    /// After this duration we assume the LLM's context window has rolled
    /// over enough to have dropped the original content, so we re-send the
    /// full version instead of a dangling ref.
    max_ref_age: Duration,
    /// Records the instant at which the in-memory compaction flag was set.
    /// Any cache entry whose `accessed_at` predates this instant is stale.
    /// Reset by [`notify_compaction`].
    compaction_marker: std::cell::Cell<Option<chrono::DateTime<chrono::Utc>>>,
}

impl CacheManager {
    /// Create a new cache manager backed by the given session store.
    ///
    /// `max_size_bytes` controls when LRU eviction kicks in. A good default
    /// is 512 MB (`512 * 1024 * 1024`). Dedup refs go stale after 30 minutes
    /// of wall-clock time by default — use [`with_ref_age`] to tune.
    pub fn new(store: SessionStore, max_size_bytes: u64) -> Self {
        Self::with_ref_age_duration(store, max_size_bytes, Duration::from_secs(30 * 60))
    }

    /// Create a CacheManager with a custom ref staleness threshold measured
    /// in turns. The turn count is converted to wall-clock time by assuming
    /// ~1 minute per turn (a rough approximation; the real freshness check
    /// uses SQLite timestamps). This constructor exists for backward
    /// compatibility with tests that previously advanced a turn counter.
    #[doc(hidden)]
    pub fn with_ref_age(store: SessionStore, max_size_bytes: u64, max_ref_age_turns: u64) -> Self {
        Self::with_ref_age_duration(
            store,
            max_size_bytes,
            Duration::from_secs(max_ref_age_turns.saturating_mul(60)),
        )
    }

    /// Create a CacheManager with an explicit wall-clock ref-age cap.
    pub fn with_ref_age_duration(
        store: SessionStore,
        max_size_bytes: u64,
        max_ref_age: Duration,
    ) -> Self {
        Self {
            store,
            max_size_bytes,
            delta_encoder: DeltaEncoder::new(),
            turn_counter: std::cell::Cell::new(0),
            ref_tracker: std::cell::RefCell::new(HashMap::new()),
            max_ref_age,
            compaction_marker: std::cell::Cell::new(None),
        }
    }

    /// Compute the SHA-256 hex digest of `bytes`.
    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Advance the turn counter. Retained for API compatibility; not used
    /// for freshness. The context_evictor still reads `current_turn` for
    /// LRU scoring during `sqz compact`.
    pub fn advance_turn(&self) {
        self.turn_counter.set(self.turn_counter.get() + 1);
    }

    /// Get the current turn number. Used by the context_evictor for scoring.
    pub fn current_turn(&self) -> u64 {
        self.turn_counter.get()
    }

    /// Notify the cache that a context compaction has occurred.
    ///
    /// Persists a compaction timestamp into the session store so any cache
    /// entry whose `accessed_at` predates the marker is considered stale
    /// by **every subsequent sqz process**, not just this one. The shell-
    /// hook invocation model means this method is typically called from a
    /// short-lived `sqz hook precompact` process, and the check runs in a
    /// different `sqz compress` process milliseconds later.
    ///
    /// Call this when:
    /// - The harness signals a compaction event (PreCompact hook)
    /// - A session is resumed after being idle
    /// - The user runs `sqz compact`
    pub fn notify_compaction(&self) {
        let now = chrono::Utc::now();
        self.compaction_marker.set(Some(now));
        self.ref_tracker.borrow_mut().clear();
        // Persist the marker so other sqz processes see the invalidation.
        // Silently swallow a write error: losing the marker means some
        // refs may survive the compaction and show as dedup hits in the
        // next few calls — annoying, not wrong (the agent still receives
        // valid content; it just sees a short-ref it has to resolve).
        let _ = self
            .store
            .set_metadata("last_compaction_at", &now.to_rfc3339());
    }

    /// Check if a dedup ref for the given hash is still fresh (likely still
    /// in the LLM's context window).
    ///
    /// Uses the SQLite `accessed_at` timestamp rather than the in-memory
    /// turn counter. This works across sqz process invocations: shell hooks
    /// spawn a new sqz process per intercepted command, so any in-memory
    /// counter would reset every time. The database survives.
    ///
    /// The compaction marker is read from SQLite on every check so that
    /// a `sqz hook precompact` call from another process immediately
    /// invalidates refs in the current process. Without the persistent
    /// read, the invalidation would only affect the process that called
    /// notify_compaction — which is never the same process that serves
    /// dedup hits.
    fn is_ref_fresh(&self, hash: &str) -> bool {
        let accessed = match self.store.get_cache_entry_accessed_at(hash) {
            Ok(Some(ts)) => ts,
            _ => return false,
        };
        // In-memory compaction marker (set in this process).
        if let Some(marker) = self.compaction_marker.get() {
            if accessed < marker {
                return false;
            }
        }
        // Persistent compaction marker — set by `sqz hook precompact` in
        // a different process. Without this read the in-memory marker is
        // never consulted because each hook invocation is a fresh process.
        if let Ok(Some(raw)) = self.store.get_metadata("last_compaction_at") {
            if let Ok(marker) = raw.parse::<chrono::DateTime<chrono::Utc>>() {
                if accessed < marker {
                    return false;
                }
            }
        }
        let age = (chrono::Utc::now() - accessed)
            .to_std()
            .unwrap_or(Duration::from_secs(0));
        age < self.max_ref_age
    }

    /// Record that a dedup ref was sent for the given hash. Updates the
    /// persistent `accessed_at` timestamp so subsequent freshness checks
    /// see this send. Silently swallows SQLite errors — losing a touch
    /// means the next call may treat the ref as stale and re-send, which
    /// is strictly worse on tokens but never wrong.
    fn record_ref_sent(&self, hash: &str) {
        let _ = self.store.touch_cache_entry(hash);
    }

    /// Look up `content` in the cache with compaction awareness.
    ///
    /// - On exact dedup with fresh ref: return `CacheResult::Dedup` (~13 tokens).
    /// - On exact dedup with stale ref: re-compress and return `CacheResult::Fresh`
    ///   (the original content may have been compacted out of the LLM's context).
    /// - On near-duplicate: return `CacheResult::Delta` with a compact diff.
    /// - On cache miss: compress via `pipeline`, persist, return `CacheResult::Fresh`.
    pub fn get_or_compress(
        &self,
        _path: &Path,
        content: &[u8],
        pipeline: &CompressionPipeline,
    ) -> Result<CacheResult> {
        let hash = Self::sha256_hex(content);

        // Exact match — check if the ref is still fresh
        // Exact match — probe without touching accessed_at, then check
        // freshness. Touching on the probe would make every ref appear
        // fresh immediately (the timestamp we just wrote is `now`).
        let exists = self.store.cache_entry_exists(&hash)?;
        if exists {
            if self.is_ref_fresh(&hash) {
                // Ref is fresh — the LLM likely still has the original in context
                let hash_prefix = &hash[..16];
                let inline_ref = format!("§ref:{hash_prefix}§");
                // Update the sent timestamp
                self.record_ref_sent(&hash);
                return Ok(CacheResult::Dedup {
                    inline_ref,
                    token_cost: 13,
                });
            } else {
                // Ref is stale — re-send the full compressed content.
                // The original may have been compacted out of the LLM's context.
                let text = String::from_utf8_lossy(content).into_owned();
                let ctx = SessionContext {
                    session_id: "cache".to_string(),
                };
                let preset = Preset::default();
                let compressed = pipeline.compress(&text, &ctx, &preset)?;
                // Record that we re-sent this content
                self.record_ref_sent(&hash);
                return Ok(CacheResult::Fresh { output: compressed });
            }
        }

        // Near-duplicate check: compare against recent cache entries
        let text = String::from_utf8_lossy(content).into_owned();
        if let Some(delta_result) = self.try_delta_encode(&text)? {
            // Store the new content in cache for future exact matches
            let ctx = SessionContext {
                session_id: "cache".to_string(),
            };
            let preset = Preset::default();
            let compressed = pipeline.compress(&text, &ctx, &preset)?;
            self.store.save_cache_entry(&hash, &compressed)?;
            self.record_ref_sent(&hash);

            let token_cost = (delta_result.delta_text.len() / 4) as u32;
            return Ok(CacheResult::Delta {
                delta_text: delta_result.delta_text,
                token_cost: token_cost.max(5),
                similarity: delta_result.similarity,
            });
        }

        let ctx = SessionContext {
            session_id: "cache".to_string(),
        };
        let preset = Preset::default();
        let compressed = pipeline.compress(&text, &ctx, &preset)?;
        self.store.save_cache_entry(&hash, &compressed)?;
        // Record that this content was sent at the current turn
        self.record_ref_sent(&hash);

        Ok(CacheResult::Fresh { output: compressed })
    }

    /// Try to delta-encode content against recent cache entries.
    /// Returns Some(DeltaResult) if a near-duplicate was found.
    fn try_delta_encode(
        &self,
        new_content: &str,
    ) -> Result<Option<crate::delta_encoder::DeltaResult>> {
        let entries = self.store.list_cache_entries_lru()?;

        // Check the most recent entries (up to 10) for near-duplicates
        let check_count = entries.len().min(10);
        for (hash, _) in entries.iter().rev().take(check_count) {
            if let Some(cached) = self.store.get_cache_entry(hash)? {
                let hash_prefix = &hash[..hash.len().min(16)];
                if let Ok(Some(delta)) =
                    self.delta_encoder
                        .encode(&cached.data, new_content, hash_prefix)
                {
                    // Only use delta if it's actually smaller than the full content
                    if delta.delta_text.len() < new_content.len() {
                        return Ok(Some(delta));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check if `content` is already in the persistent cache (dedup lookup only).
    ///
    /// Returns `Some(inline_ref)` if cached AND the ref is still fresh,
    /// `None` if the content is not cached or the ref is stale.
    ///
    /// Unlike [`get_or_compress`], this method does not touch `accessed_at`
    /// until after the freshness check — otherwise every read would make
    /// itself "fresh."
    pub fn check_dedup(&self, content: &[u8]) -> Result<Option<String>> {
        let hash = Self::sha256_hex(content);
        // Probe existence without touching accessed_at.
        let fresh = self.is_ref_fresh(&hash);
        if fresh {
            let hash_prefix = &hash[..16];
            self.record_ref_sent(&hash);
            Ok(Some(format!("§ref:{hash_prefix}§")))
        } else {
            // If the entry exists but is stale, don't return a dangling ref.
            // If it doesn't exist at all, same result: no dedup.
            Ok(None)
        }
    }

    /// Store a compressed result in the persistent cache, keyed by the
    /// SHA-256 hash of the original content.
    ///
    /// Also records the ref as sent at the current turn for compaction tracking.
    pub fn store_compressed(
        &self,
        original_content: &[u8],
        compressed: &CompressedContent,
    ) -> Result<()> {
        let hash = Self::sha256_hex(original_content);
        self.store.save_cache_entry(&hash, compressed)?;
        self.record_ref_sent(&hash);
        Ok(())
    }

    /// Invalidate the cache entry for `path` if its current content is known.
    ///
    /// Reads the file at `path`, computes its hash, and removes the matching
    /// entry from the store.  If the file does not exist the call is a no-op.
    pub fn invalidate(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }
        let bytes = std::fs::read(path)?;
        let hash = Self::sha256_hex(&bytes);
        self.store.delete_cache_entry(&hash)?;
        Ok(())
    }

    /// Evict least-recently-used entries until total cache size is at or below
    /// `max_size_bytes`.
    ///
    /// Returns the number of bytes freed.
    pub fn evict_lru(&self) -> Result<u64> {
        let entries = self.store.list_cache_entries_lru()?;

        // Compute current total size.
        let total: u64 = entries.iter().map(|(_, sz)| sz).sum();
        if total <= self.max_size_bytes {
            return Ok(0);
        }

        let mut freed: u64 = 0;
        let mut remaining = total;

        for (hash, size) in &entries {
            if remaining <= self.max_size_bytes {
                break;
            }
            self.store.delete_cache_entry(hash)?;
            freed += size;
            remaining -= size;
        }

        Ok(freed)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{
        BudgetConfig, CollapseArraysConfig, CompressionConfig, CondenseConfig,
        CustomTransformsConfig, ModelConfig, PresetMeta, StripNullsConfig, TerseModeConfig,
        ToolSelectionConfig, TruncateStringsConfig,
    };
    use crate::session_store::SessionStore;

    fn in_memory_store() -> (SessionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let store = SessionStore::open_or_create(&path).unwrap();
        (store, dir)
    }

    fn test_preset() -> Preset {
        Preset {
            preset: PresetMeta {
                name: "test".into(),
                version: "1.0".into(),
                description: String::new(),
            },
            compression: CompressionConfig {
                stages: vec![],
                keep_fields: None,
                strip_fields: None,
                condense: Some(CondenseConfig {
                    enabled: true,
                    max_repeated_lines: 3,
                }),
                git_diff_fold: None,
                strip_nulls: Some(StripNullsConfig { enabled: true }),
                flatten: None,
                truncate_strings: Some(TruncateStringsConfig {
                    enabled: true,
                    max_length: 500,
                }),
                collapse_arrays: Some(CollapseArraysConfig {
                    enabled: true,
                    max_items: 5,
                    summary_template: "... and {remaining} more items".into(),
                }),
                custom_transforms: Some(CustomTransformsConfig { enabled: true }),
            },
            tool_selection: ToolSelectionConfig {
                max_tools: 5,
                similarity_threshold: 0.7,
                default_tools: vec![],
            },
            budget: BudgetConfig {
                warning_threshold: 0.70,
                ceiling_threshold: 0.85,
                default_window_size: 200_000,
                agents: Default::default(),
            },
            terse_mode: TerseModeConfig {
                enabled: false,
                level: crate::preset::TerseLevel::Moderate,
            },
            model: ModelConfig {
                family: "anthropic".into(),
                primary: "claude-sonnet-4-20250514".into(),
                local: String::new(),
                complexity_threshold: 0.4,
                pricing: None,
            },
        }
    }

    fn make_pipeline() -> CompressionPipeline {
        CompressionPipeline::new(&test_preset())
    }

    #[test]
    fn first_read_is_miss() {
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::new(store, u64::MAX);
        let pipeline = make_pipeline();
        let content = b"hello world";
        let result = cm
            .get_or_compress(Path::new("file.txt"), content, &pipeline)
            .unwrap();
        assert!(matches!(result, CacheResult::Fresh { .. }));
    }

    #[test]
    fn second_read_is_hit() {
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::new(store, u64::MAX);
        let pipeline = make_pipeline();
        let content = b"hello world";
        let path = Path::new("file.txt");

        // First read — miss
        cm.get_or_compress(path, content, &pipeline).unwrap();

        // Second read — hit
        let result = cm.get_or_compress(path, content, &pipeline).unwrap();
        match result {
            CacheResult::Dedup {
                inline_ref,
                token_cost,
            } => {
                assert!(inline_ref.starts_with("§ref:"));
                assert!(inline_ref.ends_with('§'));
                assert_eq!(token_cost, 13);
            }
            CacheResult::Fresh { .. } | CacheResult::Delta { .. } => panic!("expected cache hit"),
        }
    }

    #[test]
    fn different_content_is_miss() {
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::new(store, u64::MAX);
        let pipeline = make_pipeline();
        let path = Path::new("file.txt");

        cm.get_or_compress(path, b"content v1", &pipeline).unwrap();
        let result = cm
            .get_or_compress(path, b"content v2", &pipeline)
            .unwrap();
        assert!(matches!(result, CacheResult::Fresh { .. } | CacheResult::Delta { .. }));
    }

    #[test]
    fn evict_lru_frees_bytes_when_over_limit() {
        let (store, _dir) = in_memory_store();
        // Very small limit so eviction triggers immediately.
        let cm = CacheManager::new(store, 1);
        let pipeline = make_pipeline();
        let path = Path::new("f.txt");

        // Populate cache with a few entries.
        cm.get_or_compress(path, b"entry one", &pipeline).unwrap();
        cm.get_or_compress(path, b"entry two", &pipeline).unwrap();
        cm.get_or_compress(path, b"entry three", &pipeline).unwrap();

        let freed = cm.evict_lru().unwrap();
        assert!(freed > 0, "expected bytes to be freed");
    }

    #[test]
    fn evict_lru_no_op_when_under_limit() {
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::new(store, u64::MAX);
        let pipeline = make_pipeline();

        cm.get_or_compress(Path::new("f.txt"), b"data", &pipeline)
            .unwrap();

        let freed = cm.evict_lru().unwrap();
        assert_eq!(freed, 0);
    }

    #[test]
    fn invalidate_removes_entry() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, b"some content").unwrap();

        let store_path = dir.path().join("store.db");
        let store = SessionStore::open_or_create(&store_path).unwrap();
        let cm = CacheManager::new(store, u64::MAX);
        let pipeline = make_pipeline();

        // Populate cache.
        let content = std::fs::read(&file_path).unwrap();
        cm.get_or_compress(&file_path, &content, &pipeline).unwrap();

        // Verify it's a hit.
        let hit = cm
            .get_or_compress(&file_path, &content, &pipeline)
            .unwrap();
        assert!(matches!(hit, CacheResult::Dedup { .. }));

        cm.invalidate(&file_path).unwrap();

        let miss = cm
            .get_or_compress(&file_path, &content, &pipeline)
            .unwrap();
        assert!(matches!(miss, CacheResult::Fresh { .. }));
    }

    #[test]
    fn invalidate_nonexistent_path_is_noop() {
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::new(store, u64::MAX);
        // Should not error.
        cm.invalidate(Path::new("/nonexistent/path/file.txt"))
            .unwrap();
    }

    // ── Compaction / freshness tests ──────────────────────────────────────
    //
    // These tests used to exercise an in-memory turn counter. Freshness is
    // now computed from SQLite `accessed_at` timestamps so dedup works
    // across the shell-hook model (each hook invocation is a fresh
    // process). The tests below use wall-clock durations instead.

    #[test]
    fn stale_ref_returns_fresh_instead_of_dedup() {
        let (store, _dir) = in_memory_store();
        // Set max_ref_age to 0 — every ref goes stale immediately.
        let cm = CacheManager::with_ref_age_duration(store, u64::MAX, Duration::ZERO);
        let pipeline = make_pipeline();
        let content = b"hello world";
        let path = Path::new("file.txt");

        // First read — miss. accessed_at recorded.
        cm.get_or_compress(path, content, &pipeline).unwrap();

        // Second read — with TTL=0 the ref is already stale, should re-send.
        let result = cm.get_or_compress(path, content, &pipeline).unwrap();
        assert!(
            matches!(result, CacheResult::Fresh { .. }),
            "stale ref (TTL=0) should return Fresh, not Dedup"
        );
    }

    #[test]
    fn fresh_ref_returns_dedup() {
        let (store, _dir) = in_memory_store();
        // Generous TTL: one day. Refs stay fresh for the life of the test.
        let cm = CacheManager::with_ref_age_duration(
            store,
            u64::MAX,
            Duration::from_secs(86_400),
        );
        let pipeline = make_pipeline();
        let content = b"hello world";
        let path = Path::new("file.txt");

        cm.get_or_compress(path, content, &pipeline).unwrap();
        let result = cm.get_or_compress(path, content, &pipeline).unwrap();
        assert!(
            matches!(result, CacheResult::Dedup { .. }),
            "fresh ref should dedup"
        );
    }

    #[test]
    fn notify_compaction_invalidates_all_refs() {
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::with_ref_age_duration(
            store,
            u64::MAX,
            Duration::from_secs(86_400),
        );
        let pipeline = make_pipeline();
        let path = Path::new("file.txt");

        // Populate cache — every subsequent read is a dedup hit.
        cm.get_or_compress(path, b"content A", &pipeline).unwrap();
        cm.get_or_compress(path, b"content B", &pipeline).unwrap();
        assert!(matches!(
            cm.get_or_compress(path, b"content A", &pipeline).unwrap(),
            CacheResult::Dedup { .. }
        ));
        assert!(matches!(
            cm.get_or_compress(path, b"content B", &pipeline).unwrap(),
            CacheResult::Dedup { .. }
        ));

        // Simulate a context compaction. The compaction marker is set to
        // `now`; any cache entry whose accessed_at predates this moment is
        // treated as stale even though the TTL hasn't expired.
        // Sleep 10ms to ensure `now` is strictly after the last touch.
        std::thread::sleep(std::time::Duration::from_millis(10));
        cm.notify_compaction();

        // After compaction, refs predate the marker — re-send full content.
        assert!(matches!(
            cm.get_or_compress(path, b"content A", &pipeline).unwrap(),
            CacheResult::Fresh { .. }
        ));
        assert!(matches!(
            cm.get_or_compress(path, b"content B", &pipeline).unwrap(),
            CacheResult::Fresh { .. }
        ));
    }

    #[test]
    fn ref_refreshed_after_resend() {
        let (store, _dir) = in_memory_store();
        // TTL of 10ms: a fresh send bumps accessed_at, so immediately after
        // the re-send the ref is fresh again.
        let cm = CacheManager::with_ref_age_duration(
            store,
            u64::MAX,
            Duration::from_millis(10),
        );
        let pipeline = make_pipeline();
        let content = b"hello world";
        let path = Path::new("file.txt");

        cm.get_or_compress(path, content, &pipeline).unwrap();
        // Wait past the TTL so the entry is stale.
        std::thread::sleep(std::time::Duration::from_millis(25));

        // Stale — must re-send Fresh. The re-send bumps accessed_at.
        let result = cm.get_or_compress(path, content, &pipeline).unwrap();
        assert!(matches!(result, CacheResult::Fresh { .. }));

        // Immediately read again — the freshly-updated accessed_at is
        // within the 10ms TTL, so the ref is fresh.
        let result = cm.get_or_compress(path, content, &pipeline).unwrap();
        assert!(
            matches!(result, CacheResult::Dedup { .. }),
            "ref should be fresh after re-send"
        );
    }

    #[test]
    fn check_dedup_returns_none_for_stale_ref() {
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::with_ref_age_duration(
            store,
            u64::MAX,
            Duration::from_millis(10),
        );
        let pipeline = make_pipeline();
        let content = b"test content";
        let path = Path::new("file.txt");

        cm.get_or_compress(path, content, &pipeline).unwrap();

        // Immediately fresh.
        assert!(cm.check_dedup(content).unwrap().is_some());

        // Wait past TTL.
        std::thread::sleep(std::time::Duration::from_millis(25));
        assert!(
            cm.check_dedup(content).unwrap().is_none(),
            "stale ref should not be returned by check_dedup"
        );
    }

    #[test]
    fn advance_turn_increments_counter() {
        // The counter is retained for context_evictor compatibility.
        let (store, _dir) = in_memory_store();
        let cm = CacheManager::new(store, u64::MAX);
        assert_eq!(cm.current_turn(), 0);
        cm.advance_turn();
        assert_eq!(cm.current_turn(), 1);
        cm.advance_turn();
        assert_eq!(cm.current_turn(), 2);
    }

    #[test]
    fn dedup_survives_cache_manager_restart() {
        // Regression for the April 18 bug: the turn counter was in-memory
        // only, so every new sqz process saw an empty ref tracker and the
        // dedup feature silently produced Fresh results forever. With
        // accessed_at-based freshness, a fresh CacheManager reading the
        // same SQLite store picks up the dedup correctly.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cache.db");
        let pipeline = make_pipeline();
        let content = b"a substantial chunk of content to dedup";
        let path = Path::new("x.txt");

        // First "process": populate cache.
        {
            let store = SessionStore::open_or_create(&db_path).unwrap();
            let cm = CacheManager::with_ref_age_duration(
                store,
                u64::MAX,
                Duration::from_secs(3600),
            );
            let first = cm.get_or_compress(path, content, &pipeline).unwrap();
            assert!(matches!(first, CacheResult::Fresh { .. }));
        }

        // Second "process": new CacheManager, same DB. Dedup must fire.
        {
            let store = SessionStore::open_or_create(&db_path).unwrap();
            let cm = CacheManager::with_ref_age_duration(
                store,
                u64::MAX,
                Duration::from_secs(3600),
            );
            let second = cm.get_or_compress(path, content, &pipeline).unwrap();
            assert!(
                matches!(second, CacheResult::Dedup { .. }),
                "second-process read must dedup — this was broken before the April 18 fix"
            );
        }
    }

    #[test]
    fn compaction_from_one_process_invalidates_refs_in_another() {
        // Regression for the PreCompact hook wiring: the host harness
        // (e.g. Claude Code) runs `sqz hook precompact` in a short-lived
        // process to signal auto-compaction. The actual dedup serving runs
        // in a DIFFERENT sqz process (the shell hook). notify_compaction
        // must persist through SQLite so the second process sees it.
        //
        // Before the fix, compaction_marker was Cell<Option<DateTime>>
        // in memory only — the precompact process set it, exited, the
        // state was lost. Next shell-hook process started with a clean
        // marker, served stale refs to the agent, and the agent saw a
        // §ref:HASH§ pointing at content no longer in its context.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cache.db");
        let pipeline = make_pipeline();
        let content = b"content that needs stale-marking after compaction";
        let path = Path::new("file.txt");
        let ttl = Duration::from_secs(3600);

        // Process A: populate the cache so the content is dedup-eligible.
        {
            let store = SessionStore::open_or_create(&db_path).unwrap();
            let cm = CacheManager::with_ref_age_duration(store, u64::MAX, ttl);
            cm.get_or_compress(path, content, &pipeline).unwrap();
        }
        // Sleep so the compaction marker is strictly after the touch.
        std::thread::sleep(Duration::from_millis(10));

        // Process B: simulates `sqz hook precompact`. Just calls
        // notify_compaction and exits. No reads.
        {
            let store = SessionStore::open_or_create(&db_path).unwrap();
            let cm = CacheManager::with_ref_age_duration(store, u64::MAX, ttl);
            cm.notify_compaction();
        }

        // Process C: simulates the next `sqz compress` shell-hook call.
        // Reads the same content. MUST re-send Fresh, not return a ref
        // the agent can no longer resolve.
        {
            let store = SessionStore::open_or_create(&db_path).unwrap();
            let cm = CacheManager::with_ref_age_duration(store, u64::MAX, ttl);
            let result = cm.get_or_compress(path, content, &pipeline).unwrap();
            assert!(
                matches!(result, CacheResult::Fresh { .. }),
                "post-compaction read from a fresh process must re-send Fresh; \
                 returning Dedup would be a dangling-ref bug"
            );
        }
    }

    use proptest::prelude::*;

    // ── Property 8: Cache deduplication ──────────────────────────────────────
    // **Validates: Requirements 8.1, 8.2, 18.1, 18.2**
    //
    // For any file content, reading the file twice through the CacheManager
    // (with no content change between reads) SHALL return a cache hit on the
    // second read with a reference token of approximately 13 tokens.

    proptest! {
        /// **Validates: Requirements 8.1, 8.2, 18.1, 18.2**
        ///
        /// For any file content, the second read through CacheManager SHALL be
        /// a cache hit with tokens == 13.
        #[test]
        fn prop_cache_deduplication(
            content in proptest::collection::vec(any::<u8>(), 1..=1000usize),
        ) {
            let (store, _dir) = in_memory_store();
            let cm = CacheManager::new(store, u64::MAX);
            let pipeline = make_pipeline();
            let path = Path::new("file.txt");

            // First read — must be a miss.
            let first = cm.get_or_compress(path, &content, &pipeline).unwrap();
            prop_assert!(
                matches!(first, CacheResult::Fresh { .. }),
                "first read should be a cache miss"
            );

            let second = cm.get_or_compress(path, &content, &pipeline).unwrap();
            match second {
                CacheResult::Dedup { inline_ref, token_cost } => {
                    prop_assert_eq!(
                        token_cost, 13,
                        "cache hit should report ~13 reference tokens"
                    );
                    prop_assert!(
                        inline_ref.starts_with("§ref:"),
                        "reference token should start with §ref:"
                    );
                    prop_assert!(
                        inline_ref.ends_with('§'),
                        "reference token should end with §"
                    );
                }
                CacheResult::Fresh { .. } | CacheResult::Delta { .. } => {
                    prop_assert!(false, "second read should be a cache hit, not a miss");
                }
            }
        }
    }

    // ── Property 9: Cache invalidation on content change ─────────────────────
    // **Validates: Requirements 8.3, 18.3**
    //
    // For any cached file, if the file content changes (producing a different
    // SHA-256 hash), the CacheManager SHALL treat the next read as a cache miss
    // and re-compress the updated content.

    proptest! {
        /// **Validates: Requirements 8.3, 18.3**
        ///
        /// For any two distinct byte sequences, the first read of each is a
        /// cache miss — content change always triggers re-compression.
        #[test]
        fn prop_cache_invalidation_on_content_change(
            content_a in proptest::collection::vec(any::<u8>(), 1..=500usize),
            content_b in proptest::collection::vec(any::<u8>(), 1..=500usize),
        ) {
            // Only meaningful when the two contents differ (different hashes).
            prop_assume!(content_a != content_b);

            let (store, _dir) = in_memory_store();
            let cm = CacheManager::new(store, u64::MAX);
            let pipeline = make_pipeline();
            let path = Path::new("file.txt");

            // Cache content_a.
            let r1 = cm.get_or_compress(path, &content_a, &pipeline).unwrap();
            prop_assert!(
                matches!(r1, CacheResult::Fresh { .. }),
                "first read of content_a should be a miss"
            );

            let r2 = cm.get_or_compress(path, &content_a, &pipeline).unwrap();
            prop_assert!(
                matches!(r2, CacheResult::Dedup { .. }),
                "second read of content_a should be a hit"
            );

            let r3 = cm.get_or_compress(path, &content_b, &pipeline).unwrap();
            prop_assert!(
                matches!(r3, CacheResult::Fresh { .. } | CacheResult::Delta { .. }),
                "read with changed content should be a cache miss or delta"
            );
        }
    }

    // ── Property 10: Cache LRU eviction ──────────────────────────────────────
    // **Validates: Requirements 8.5**
    //
    // For any cache state where total size exceeds the configured maximum, the
    // CacheManager SHALL evict entries in LRU order until total size is at or
    // below the limit.

    proptest! {
        /// **Validates: Requirements 8.5**
        ///
        /// After evict_lru, the total remaining cache size SHALL be at or below
        /// max_size_bytes.
        #[test]
        fn prop_cache_lru_eviction(
            // Generate 2-8 distinct content entries.
            entries in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 10..=200usize),
                2..=8usize,
            ),
        ) {
            // Deduplicate entries so each has a unique hash.
            let mut unique_entries: Vec<Vec<u8>> = Vec::new();
            for e in &entries {
                if !unique_entries.contains(e) {
                    unique_entries.push(e.clone());
                }
            }
            prop_assume!(unique_entries.len() >= 2);

            let (store, _dir) = in_memory_store();
            // Use a very small limit (1 byte) to guarantee eviction is needed.
            let cm = CacheManager::new(store, 1);
            let pipeline = make_pipeline();
            let path = Path::new("f.txt");

            // Populate the cache.
            for entry in &unique_entries {
                cm.get_or_compress(path, entry, &pipeline).unwrap();
            }

            // Evict LRU entries.
            let freed = cm.evict_lru().unwrap();

            // Bytes freed must be > 0 since total > 1 byte.
            prop_assert!(freed > 0, "evict_lru should free bytes when over limit");

            // After eviction, total remaining size must be <= max_size_bytes (1).
            // We verify by checking that evict_lru now returns 0 (nothing left to evict).
            let freed_again = cm.evict_lru().unwrap();
            prop_assert_eq!(
                freed_again, 0,
                "second evict_lru call should free 0 bytes (already at or below limit)"
            );
        }
    }

    // ── Property 34: Cache persistence across sessions ────────────────────────
    // **Validates: Requirements 18.4**
    //
    // For any set of cache entries saved to the SessionStore, reloading the
    // store (opening the same database file) SHALL produce the same cache
    // entries, and a subsequent read with the same content hash SHALL return a
    // cache hit.

    proptest! {
        /// **Validates: Requirements 18.4**
        ///
        /// Cache entries written in one CacheManager instance SHALL survive
        /// a store close/reopen. With the wall-clock freshness model
        /// (introduced April 18 2026), a subsequent CacheManager reading
        /// the same database SHALL see the entry as fresh (within TTL) and
        /// return a Dedup hit on the very first read — this is the whole
        /// point of the cross-process fix. Previous behavior (Fresh on
        /// first read after restart) was a bug that silently disabled the
        /// dedup feature in production.
        #[test]
        fn prop_cache_persistence_across_sessions(
            content in proptest::collection::vec(any::<u8>(), 1..=500usize),
        ) {
            use crate::session_store::SessionStore;

            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("cache.db");
            let path = Path::new("file.txt");

            // Session 1: populate the cache.
            {
                let store = SessionStore::open_or_create(&db_path).unwrap();
                // Explicit long TTL so tests don't race with wall-clock drift.
                let cm = CacheManager::with_ref_age_duration(
                    store,
                    u64::MAX,
                    Duration::from_secs(3600),
                );
                let pipeline = make_pipeline();

                let r = cm.get_or_compress(path, &content, &pipeline).unwrap();
                prop_assert!(
                    matches!(r, CacheResult::Fresh { .. }),
                    "first-ever read should be a miss"
                );
            }

            // Session 2: reopen the same database file.
            {
                let store = SessionStore::open_or_create(&db_path).unwrap();
                let cm = CacheManager::with_ref_age_duration(
                    store,
                    u64::MAX,
                    Duration::from_secs(3600),
                );
                let pipeline = make_pipeline();

                // First read in the new session MUST dedup. The entry was
                // just written (within TTL), so the wall-clock freshness
                // check finds it fresh. This is what makes sqz's dedup
                // actually work across shell-hook invocations.
                let r = cm.get_or_compress(path, &content, &pipeline).unwrap();
                match r {
                    CacheResult::Dedup { token_cost, .. } => {
                        prop_assert_eq!(
                            token_cost, 13,
                            "first read after restart must be a 13-token dedup ref"
                        );
                    }
                    CacheResult::Fresh { .. } | CacheResult::Delta { .. } => {
                        prop_assert!(
                            false,
                            "first read after restart must dedup — this was the \
                             April 18 bug and its fix is the whole reason this \
                             test exists"
                        );
                    }
                }
            }
        }
    }
}
