use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::pipeline::{CompressionPipeline, SessionContext};
use crate::preset::Preset;
use crate::session_store::SessionStore;
use crate::types::CompressedContent;

/// Outcome of a cache lookup in [`CacheManager`].
pub enum CacheResult {
    /// Previously seen content — returns a short inline reference (~13 tokens).
    Dedup {
        /// Inline token of the form `§ref:<hash_prefix>§`.
        inline_ref: String,
        /// Approximate token cost of the reference (always 13).
        token_cost: u32,
    },
    /// Content not seen before — full compression result.
    Fresh { output: CompressedContent },
}

/// SHA-256 content-hash deduplication cache backed by [`SessionStore`].
pub struct CacheManager {
    store: SessionStore,
    max_size_bytes: u64,
}

impl CacheManager {
    pub fn new(store: SessionStore, max_size_bytes: u64) -> Self {
        Self {
            store,
            max_size_bytes,
        }
    }

    /// Compute the SHA-256 hex digest of `bytes`.
    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Look up `content` in the cache.
    ///
    /// - On dedup: return `CacheResult::Dedup` with a compact reference token.
    /// - On fresh: compress via `pipeline`, persist to store, return
    ///   `CacheResult::Fresh`.
    pub fn get_or_compress(
        &self,
        _path: &Path,
        content: &[u8],
        pipeline: &CompressionPipeline,
    ) -> Result<CacheResult> {
        let hash = Self::sha256_hex(content);

        if self.store.get_cache_entry(&hash)?.is_some() {
            let hash_prefix = &hash[..16];
            let inline_ref = format!("§ref:{hash_prefix}§");
            return Ok(CacheResult::Dedup {
                inline_ref,
                token_cost: 13,
            });
        }

        let text = String::from_utf8_lossy(content).into_owned();
        let ctx = SessionContext {
            session_id: "cache".to_string(),
        };
        let preset = Preset::default();
        let compressed = pipeline.compress(&text, &ctx, &preset)?;
        self.store.save_cache_entry(&hash, &compressed)?;

        Ok(CacheResult::Fresh { output: compressed })
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
            CacheResult::Fresh { .. } => panic!("expected cache hit"),
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
        assert!(matches!(result, CacheResult::Fresh { .. }));
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

    // ── Property-based tests ──────────────────────────────────────────────────

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
                CacheResult::Fresh { .. } => {
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
                matches!(r3, CacheResult::Fresh { .. }),
                "read with changed content should be a cache miss"
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
        /// Cache entries written in one CacheManager instance SHALL survive a
        /// store close/reopen and produce cache hits in a new instance.
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
                let cm = CacheManager::new(store, u64::MAX);
                let pipeline = make_pipeline();

                let r = cm.get_or_compress(path, &content, &pipeline).unwrap();
                prop_assert!(
                    matches!(r, CacheResult::Fresh { .. }),
                    "first read should be a miss"
                );
            }
            // Store is dropped here — connection closed.

            // Session 2: reopen the same database file.
            {
                let store = SessionStore::open_or_create(&db_path).unwrap();
                let cm = CacheManager::new(store, u64::MAX);
                let pipeline = make_pipeline();

                // Same content should now be a hit.
                let r = cm.get_or_compress(path, &content, &pipeline).unwrap();
                match r {
                    CacheResult::Dedup { token_cost, .. } => {
                        prop_assert_eq!(
                            token_cost, 13,
                            "persisted cache hit should report 13 tokens"
                        );
                    }
                    CacheResult::Fresh { .. } => {
                        prop_assert!(
                            false,
                            "cache entry should persist across store reopen"
                        );
                    }
                }
            }
        }
    }
}
