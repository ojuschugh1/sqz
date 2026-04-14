pub mod advanced_search;
pub mod ansi_strip;
pub mod ast_parser;
pub mod benchmarks;
pub mod bpe_compressor;
pub mod cmd_formatters;
pub mod compression_quality;
pub mod confidence_router;
pub mod crp_engine;
pub mod dashboard;
pub mod delta_encoder;
pub mod dependency_mapper;
pub mod dict_compressor;
pub mod entropy_analyzer;
pub mod entropy_truncator;
pub mod file_reader;
pub mod image_compressor;
pub mod litm_positioner;
pub mod ngram_abbreviator;
pub mod rle_compressor;
pub mod simhash;
pub mod token_pruner;
pub mod tool_selector;
pub mod engine;
pub mod hook_manager;
pub mod budget_tracker;
pub mod cache_manager;
pub mod correction_log;
pub mod cost_calculator;
pub mod ctx_format;
pub mod error;
pub mod model_router;
pub mod pipeline;
pub mod pin_manager;
pub mod plugin_api;
pub mod preset;
pub mod progressive_throttle;
pub mod prompt_cache;
pub mod sandbox_executor;
pub mod session_continuity;
pub mod session_store;
pub mod stages;
pub mod tee_mode;
pub mod terse_mode;
pub mod token_counter;
pub mod toon;
pub mod types;
pub mod url_indexer;
pub mod verifier;

pub use advanced_search::{AdvancedSearch, SearchResult};
pub use ansi_strip::AnsiStripper;
pub use ast_parser::{AstParser, ClassDefinition, CodeSummary, FunctionSignature, ImportDecl, TypeDeclaration};
pub use bpe_compressor::{bpe_compress, BpeConfig, BpeResult};
pub use compression_quality::{measure_quality, format_quality_report, CompressionQuality, QualityGrade};
pub use confidence_router::{ConfidenceRouter, CompressionMode};
pub use cmd_formatters::format_command;
pub use delta_encoder::{DeltaConfig, DeltaEncoder, DeltaResult};
pub use dependency_mapper::DependencyMapper;
pub use dict_compressor::{DictCompressor, DictConfig, DictCompressResult};
pub use entropy_analyzer::{EntropyAnalyzer, InfoLevel, AnalyzedBlock};
pub use entropy_truncator::{EntropyTruncator, EntropyTruncConfig, EntropyTruncResult, EntropyTruncArrayResult};
pub use file_reader::{FileReadMode, FileReader, ReadResult, BlockEntropy, compute_entropy, analyze_block_entropies};
pub use image_compressor::{ImageCompressor, ImageDescription};
pub use litm_positioner::{ContextSection, LitmPositioner, LitmStrategy, SectionType};
pub use ngram_abbreviator::{NgramAbbreviator, AbbreviatorConfig, AbbreviationResult};
pub use rle_compressor::{rle_compress, sliding_window_dedup, RleResult, SlidingWindowResult};
pub use simhash::{simhash, SimHashFingerprint};
pub use token_pruner::{TokenPruner, PrunerConfig, PruneResult};
pub use tool_selector::{ToolDefinition, ToolSelector};
pub use budget_tracker::{
    AgentBudget, BudgetTracker, BudgetWarning, UsagePrediction, UsageReport,
};
pub use cache_manager::{CacheManager, CacheResult};
pub use correction_log::ContextWindow;
pub use crp_engine::{CrpEngine, CrpLevel};
pub use cost_calculator::{
    CostBreakdown, CostCalculator, ModelPricing, PricingConfig, SessionCostSummary, TokenUsage,
    ToolCost,
};
pub use ctx_format::{CtxEnvelope, CtxFormat, CtxMetadata};
pub use error::{Result, SqzError, SourceLocation};
pub use model_router::{ModelRouter, RoutingDecision, TaskContext};
pub use pipeline::{CompressionPipeline, SessionContext};
pub use pin_manager::PinManager;
pub use plugin_api::{PluginLoader, PluginManifest, PluginSource, SqzPlugin};
pub use prompt_cache::{CacheBoundary, Message, PromptCacheDetector};
pub use session_store::{CompressionStats, DailyGain, SessionStore, SessionSummary};
pub use session_continuity::{
    SessionContinuityManager, SessionGuide, Snapshot, SnapshotEvent, SnapshotEventType,
};
pub use toon::ToonEncoder;
pub use tee_mode::{TeeMode, TeeManager, TeeEntry};
pub use terse_mode::TerseMode;
pub use token_counter::TokenCounter;
pub use types::*;
pub use preset::{
    BudgetConfig, CollapseArraysConfig, CompressionConfig, CondenseConfig, CustomTransformsConfig,
    FlattenConfig, GitDiffFoldConfig, KeepFieldsConfig, ModelConfig, ModelPricingConfig, Preset,
    PresetHeader, PresetMeta, PresetParser, StripFieldsConfig, StripNullsConfig, TerseLevel,
    TerseModeConfig, ToolSelectionConfig, TruncateStringsConfig,
};
pub use progressive_throttle::{ProgressiveThrottler, ThrottleConfig, ThrottleLevel};
pub use dashboard::{
    CommandBreakdown, DashboardConfig, DashboardHtml, DashboardMetrics, DashboardServer,
    SessionHistoryEntry, ToolBreakdown,
};
pub use engine::SqzEngine;
pub use hook_manager::{
    generate_platform_config, known_platforms, Hook, HookAction, HookContext, HookManager, HookType,
};
pub use sandbox_executor::{SandboxExecutor, SandboxResult, RuntimeInfo, FilteredOutput};
pub use url_indexer::{ContentFetcher, IndexedChunk, IndexResult, UrlIndexer};
pub use verifier::Verifier;
