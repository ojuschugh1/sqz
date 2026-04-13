use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::ast_parser::AstParser;
use crate::budget_tracker::{BudgetTracker, UsageReport};
use crate::cache_manager::CacheManager;
use crate::confidence_router::ConfidenceRouter;
use crate::cost_calculator::{CostCalculator, SessionCostSummary};
use crate::ctx_format::CtxFormat;
use crate::error::{Result, SqzError};
use crate::model_router::ModelRouter;
use crate::pin_manager::PinManager;
use crate::pipeline::CompressionPipeline;
use crate::plugin_api::PluginLoader;
use crate::preset::{Preset, PresetParser};
use crate::session_store::{SessionStore, SessionSummary};
use crate::terse_mode::TerseMode;
use crate::types::{CompressedContent, PinEntry, Provenance, SessionId};
use crate::verifier::Verifier;

/// Top-level facade that wires all sqz_engine modules together.
///
/// # Concurrency design
///
/// `SqzEngine` is designed for single-threaded use on the main thread.
/// The only cross-thread sharing happens during preset hot-reload: the
/// file-watcher callback runs on a background thread and needs to update
/// the preset, pipeline, and model router. These three fields are wrapped
/// in `Arc<Mutex<>>` specifically for that purpose. All other fields are
/// owned directly — no unnecessary synchronization.
pub struct SqzEngine {
    // --- Hot-reloadable state (shared with file-watcher thread) ---
    preset: Arc<Mutex<Preset>>,
    pipeline: Arc<Mutex<CompressionPipeline>>,
    model_router: Arc<Mutex<ModelRouter>>,

    // --- Single-owner state (no cross-thread sharing needed) ---
    session_store: SessionStore,
    #[allow(dead_code)] // used internally by compress pipeline; public API pending
    cache_manager: CacheManager,
    budget_tracker: BudgetTracker,
    cost_calculator: CostCalculator,
    ast_parser: AstParser,
    terse_mode: TerseMode,
    pin_manager: PinManager,
    confidence_router: ConfidenceRouter,
    _plugin_loader: PluginLoader,
}

impl SqzEngine {
    /// Create a new engine with the default preset and a persistent session store.
    ///
    /// Sessions are stored in `~/.sqz/sessions.db` for cross-session continuity.
    /// Falls back to a temp-file store if the home directory is unavailable.
    pub fn new() -> Result<Self> {
        let preset = Preset::default();
        let store_path = Self::default_store_path();
        Self::with_preset_and_store(preset, &store_path)
    }

    /// Resolve the default session store path: `~/.sqz/sessions.db`.
    /// Falls back to a temp-file path if home dir is unavailable.
    fn default_store_path() -> std::path::PathBuf {
        if let Some(home) = dirs_next::home_dir() {
            let sqz_dir = home.join(".sqz");
            if std::fs::create_dir_all(&sqz_dir).is_ok() {
                return sqz_dir.join("sessions.db");
            }
        }
        // Fallback: temp dir with unique name
        let dir = std::env::temp_dir();
        dir.join(format!(
            "sqz_session_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    /// Create with a custom preset and a file-backed session store.
    ///
    /// Opens a single SQLite connection for the session store. The cache
    /// manager and pin manager share the same store via separate connections
    /// (SQLite WAL mode supports concurrent readers).
    pub fn with_preset_and_store(preset: Preset, store_path: &Path) -> Result<Self> {
        let pipeline = CompressionPipeline::new(&preset);
        let window_size = preset.budget.default_window_size;

        // One connection per consumer. SQLite WAL mode handles concurrency.
        let session_store = SessionStore::open_or_create(store_path)?;
        let cache_store = SessionStore::open_or_create(store_path)?;
        let pin_store = SessionStore::open_or_create(store_path)?;

        Ok(SqzEngine {
            preset: Arc::new(Mutex::new(preset.clone())),
            pipeline: Arc::new(Mutex::new(pipeline)),
            model_router: Arc::new(Mutex::new(ModelRouter::new(&preset))),
            session_store,
            cache_manager: CacheManager::new(cache_store, 512 * 1024 * 1024),
            budget_tracker: BudgetTracker::new(window_size, &preset),
            cost_calculator: CostCalculator::with_defaults(),
            ast_parser: AstParser::new(),
            terse_mode: TerseMode,
            pin_manager: PinManager::new(pin_store),
            confidence_router: ConfidenceRouter::new(),
            _plugin_loader: PluginLoader::new(Path::new("plugins")),
        })
    }

    /// Compress input text using the current preset.
    ///
    /// Two-pass pipeline:
    /// 1. Route to compression mode based on content entropy and risk patterns.
    /// 2. Compress using the pipeline (safe preset for Safe mode, default otherwise).
    /// 3. Verify invariants (error lines, JSON keys, diff hunks, etc.).
    /// 4. If verification confidence is low, fall back to safe mode and re-compress.
    pub fn compress(&self, input: &str) -> Result<CompressedContent> {
        let preset = self.preset.lock()
            .map_err(|_| SqzError::Other("preset lock poisoned".into()))?;
        let pipeline = self.pipeline.lock()
            .map_err(|_| SqzError::Other("pipeline lock poisoned".into()))?;
        let ctx = crate::pipeline::SessionContext {
            session_id: "engine".to_string(),
        };

        // Step 1: Route — check content risk before compressing
        let mode = self.confidence_router.route(input);

        // Step 2: If Safe mode, skip aggressive pipeline and go straight to safe compress
        if mode == crate::confidence_router::CompressionMode::Safe {
            eprintln!("[sqz] fallback: safe mode — content classified as high-risk (stack trace / migration / secret)");
            return self.compress_safe(input, &pipeline, &ctx);
        }

        // Step 3: Compress with the configured pipeline
        let mut result = pipeline.compress(input, &ctx, &preset)?;

        // Step 4: Verify invariants
        let verify = Verifier::verify(input, &result.data);
        let fallback = verify.fallback_triggered;
        result.verify = Some(verify);

        // Step 5: If verifier signals low confidence, re-compress with safe settings
        if fallback && result.data != input {
            eprintln!("[sqz] fallback: verifier confidence {:.2} below threshold — re-compressing in safe mode",
                result.verify.as_ref().map(|v| v.confidence).unwrap_or(0.0));
            let safe_result = self.compress_safe(input, &pipeline, &ctx)?;
            return Ok(safe_result);
        }

        Ok(result)
    }

    /// Compress with explicit mode override, bypassing the confidence router.
    ///
    /// - `CompressionMode::Safe` → safe pipeline only (ANSI strip + condense)
    /// - `CompressionMode::Default` → standard pipeline
    /// - `CompressionMode::Aggressive` → standard pipeline (aggressive preset TBD)
    pub fn compress_with_mode(&self, input: &str, mode: crate::confidence_router::CompressionMode) -> Result<CompressedContent> {
        let pipeline = self.pipeline.lock()
            .map_err(|_| SqzError::Other("pipeline lock poisoned".into()))?;
        let ctx = crate::pipeline::SessionContext {
            session_id: "engine".to_string(),
        };

        match mode {
            crate::confidence_router::CompressionMode::Safe => {
                self.compress_safe(input, &pipeline, &ctx)
            }
            _ => {
                // Default and Aggressive: run normal pipeline + verify
                drop(pipeline); // release lock before calling compress()
                self.compress(input)
            }
        }
    }

    /// Safe-mode compression: minimal transforms only (ANSI strip + condense).
    fn compress_safe(
        &self,
        input: &str,
        pipeline: &crate::pipeline::CompressionPipeline,
        ctx: &crate::pipeline::SessionContext,
    ) -> Result<CompressedContent> {
        use crate::preset::{
            CompressionConfig, CondenseConfig, CustomTransformsConfig, BudgetConfig,
            ModelConfig, PresetMeta, TerseModeConfig, TerseLevel, ToolSelectionConfig,
        };

        let safe_preset = Preset {
            preset: PresetMeta {
                name: "safe".to_string(),
                version: "1.0".to_string(),
                description: "Safe fallback — minimal compression".to_string(),
            },
            compression: CompressionConfig {
                stages: vec!["condense".to_string()],
                keep_fields: None,
                strip_fields: None,
                condense: Some(CondenseConfig { enabled: true, max_repeated_lines: 3 }),
                git_diff_fold: None,
                strip_nulls: None,
                flatten: None,
                truncate_strings: None,
                collapse_arrays: None,
                custom_transforms: Some(CustomTransformsConfig { enabled: false }),
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
            terse_mode: TerseModeConfig { enabled: false, level: TerseLevel::Moderate },
            model: ModelConfig {
                family: "anthropic".to_string(),
                primary: String::new(),
                local: String::new(),
                complexity_threshold: 0.4,
                pricing: None,
            },
        };

        let mut result = pipeline.compress(input, ctx, &safe_preset)?;
        let verify = Verifier::verify(input, &result.data);
        result.verify = Some(verify);
        result.provenance = Provenance {
            label: Some("safe-fallback".to_string()),
            ..Default::default()
        };
        Ok(result)
    }

    /// Compress with explicit provenance metadata attached to the result.
    pub fn compress_with_provenance(
        &self,
        input: &str,
        provenance: Provenance,
    ) -> Result<CompressedContent> {
        let mut result = self.compress(input)?;
        result.provenance = provenance;
        Ok(result)
    }

    /// Export a session to CTX format.
    pub fn export_ctx(&self, session_id: &str) -> Result<String> {
        let session = self.session_store.load_session(session_id.to_string())?;
        CtxFormat::serialize(&session)
    }

    /// Import a CTX string and save as a new session.
    pub fn import_ctx(&self, ctx: &str) -> Result<SessionId> {
        let session = CtxFormat::deserialize(ctx)?;
        self.session_store.save_session(&session)
    }

    /// Pin a conversation turn.
    pub fn pin(&self, session_id: &str, turn_index: usize, reason: &str, tokens: u32) -> Result<PinEntry> {
        self.pin_manager.pin(session_id, turn_index, reason, tokens)
    }

    /// Unpin a conversation turn.
    pub fn unpin(&self, session_id: &str, turn_index: usize) -> Result<()> {
        self.pin_manager.unpin(session_id, turn_index)
    }

    /// Search sessions by keyword.
    pub fn search_sessions(&self, query: &str) -> Result<Vec<SessionSummary>> {
        self.session_store.search(query)
    }

    /// Get usage report for an agent.
    pub fn usage_report(&self, agent_id: &str) -> UsageReport {
        self.budget_tracker.usage_report(agent_id.to_string())
    }

    /// Get cost summary for a session.
    pub fn cost_summary(&self, session_id: &str) -> Result<SessionCostSummary> {
        let session = self.session_store.load_session(session_id.to_string())?;
        Ok(self.cost_calculator.session_summary(&session))
    }

    /// Reload the preset from a TOML string (hot-reload support).
    pub fn reload_preset(&mut self, toml: &str) -> Result<()> {
        let new_preset = PresetParser::parse(toml)?;
        if let Ok(mut pipeline) = self.pipeline.lock() {
            pipeline.reload_preset(&new_preset)?;
        }
        if let Ok(mut router) = self.model_router.lock() {
            *router = ModelRouter::new(&new_preset);
        }
        if let Ok(mut preset) = self.preset.lock() {
            *preset = new_preset;
        }
        Ok(())
    }

    /// Spawn a background thread that watches `path` for preset file changes.
    ///
    /// Only the preset, pipeline, and model_router are shared with the watcher
    /// thread (via `Arc<Mutex<>>`). All other engine state stays on the main thread.
    pub fn watch_preset_file(&self, path: &Path) -> Result<notify::RecommendedWatcher> {
        use notify::{Event, EventKind, RecursiveMode, Watcher};

        let preset_arc = Arc::clone(&self.preset);
        let pipeline_arc = Arc::clone(&self.pipeline);
        let router_arc = Arc::clone(&self.model_router);
        let watched_path = path.to_owned();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    match std::fs::read_to_string(&watched_path) {
                        Ok(toml_str) => match PresetParser::parse(&toml_str) {
                            Ok(new_preset) => {
                                if let Ok(mut p) = pipeline_arc.lock() {
                                    let _ = p.reload_preset(&new_preset);
                                }
                                if let Ok(mut r) = router_arc.lock() {
                                    *r = ModelRouter::new(&new_preset);
                                }
                                if let Ok(mut pr) = preset_arc.lock() {
                                    *pr = new_preset;
                                }
                            }
                            Err(e) => eprintln!("[sqz] invalid preset: {e}"),
                        },
                        Err(e) => eprintln!("[sqz] preset read error: {e}"),
                    }
                }
            }
        })
        .map_err(|e| SqzError::Other(format!("watcher error: {e}")))?;

        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|e| SqzError::Other(format!("watch error: {e}")))?;

        Ok(watcher)
    }

    /// Access the underlying `SessionStore`.
    pub fn session_store(&self) -> &SessionStore {
        &self.session_store
    }

    /// Access the `CacheManager` for persistent dedup.
    pub fn cache_manager(&self) -> &CacheManager {
        &self.cache_manager
    }

    /// Access the `AstParser`.
    pub fn ast_parser(&self) -> &AstParser {
        &self.ast_parser
    }

    /// Access the `TerseMode` helper.
    pub fn terse_mode(&self) -> &TerseMode {
        &self.terse_mode
    }

    /// Reorder context sections using the LITM positioner to mitigate
    /// the "Lost In The Middle" attention bias in long-context models.
    ///
    /// Places highest-priority sections at the beginning and end of the
    /// context window, lowest-priority in the middle.
    pub fn reorder_context(
        &self,
        sections: &mut Vec<crate::litm_positioner::ContextSection>,
        strategy: crate::litm_positioner::LitmStrategy,
    ) {
        let positioner = crate::litm_positioner::LitmPositioner::new(strategy);
        positioner.reorder(sections);
    }

    /// Route content to the appropriate compression mode based on entropy
    /// and risk pattern analysis.
    pub fn route_compression_mode(&self, content: &str) -> crate::confidence_router::CompressionMode {
        self.confidence_router.route(content)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BudgetState, CorrectionLog, ModelFamily, SessionState};
    use chrono::Utc;
    use std::path::PathBuf;

    fn make_session(id: &str) -> SessionState {
        let now = Utc::now();
        SessionState {
            id: id.to_string(),
            project_dir: PathBuf::from("/tmp/test"),
            conversation: vec![],
            corrections: CorrectionLog::default(),
            pins: vec![],
            learnings: vec![],
            compressed_summary: "test session".to_string(),
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

    #[test]
    fn test_engine_new() {
        let engine = SqzEngine::new();
        assert!(engine.is_ok(), "SqzEngine::new() should succeed");
    }

    #[test]
    fn test_compress_plain_text() {
        let engine = SqzEngine::new().unwrap();
        let result = engine.compress("hello world");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().data, "hello world");
    }

    #[test]
    fn test_compress_json_applies_toon() {
        let engine = SqzEngine::new().unwrap();
        let result = engine.compress(r#"{"name":"Alice","age":30}"#).unwrap();
        assert!(result.data.starts_with("TOON:"), "JSON should be TOON-encoded");
    }

    #[test]
    fn test_export_import_ctx_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store.db");
        let engine = SqzEngine::with_preset_and_store(Preset::default(), &store_path).unwrap();

        let session = make_session("sess-rt");
        engine.session_store().save_session(&session).unwrap();

        let ctx = engine.export_ctx("sess-rt").unwrap();
        let imported_id = engine.import_ctx(&ctx).unwrap();
        assert_eq!(imported_id, "sess-rt");
    }

    #[test]
    fn test_search_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store.db");
        let engine = SqzEngine::with_preset_and_store(Preset::default(), &store_path).unwrap();

        let mut session = make_session("sess-search");
        session.compressed_summary = "authentication refactor".to_string();
        engine.session_store().save_session(&session).unwrap();

        let results = engine.search_sessions("authentication").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "sess-search");
    }

    #[test]
    fn test_usage_report_starts_at_zero() {
        let engine = SqzEngine::new().unwrap();
        let report = engine.usage_report("default");
        assert_eq!(report.consumed, 0);
        assert_eq!(report.available, report.allocated);
    }

    #[test]
    fn test_cost_summary() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store.db");
        let engine = SqzEngine::with_preset_and_store(Preset::default(), &store_path).unwrap();

        let session = make_session("sess-cost");
        engine.session_store().save_session(&session).unwrap();

        let summary = engine.cost_summary("sess-cost").unwrap();
        assert_eq!(summary.total_tokens, 0);
        assert!((summary.total_usd - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reload_preset_updates_state() {
        let mut engine = SqzEngine::new().unwrap();
        let toml = r#"
[preset]
name = "reloaded"
version = "2.0"

[compression]
stages = []

[tool_selection]
max_tools = 5
similarity_threshold = 0.7

[budget]
warning_threshold = 0.70
ceiling_threshold = 0.85
default_window_size = 200000

[terse_mode]
enabled = false
level = "moderate"

[model]
family = "anthropic"
primary = "claude-sonnet-4-20250514"
complexity_threshold = 0.4
"#;
        assert!(engine.reload_preset(toml).is_ok());
        // Verify the preset was actually updated
        let preset = engine.preset.lock().unwrap();
        assert_eq!(preset.preset.name, "reloaded");
    }

    #[test]
    fn test_reload_invalid_preset_returns_error() {
        let mut engine = SqzEngine::new().unwrap();
        let result = engine.reload_preset("not valid toml [[[");
        assert!(result.is_err(), "invalid TOML should return error");
    }

    #[test]
    fn test_export_nonexistent_session_returns_error() {
        let engine = SqzEngine::new().unwrap();
        let result = engine.export_ctx("does-not-exist");
        assert!(result.is_err());
    }

    #[test]
    fn test_import_invalid_ctx_returns_error() {
        let engine = SqzEngine::new().unwrap();
        let result = engine.import_ctx("not valid json {{{");
        assert!(result.is_err());
    }
}
