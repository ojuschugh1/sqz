use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Type aliases

/// Unique identifier for a compression session.
pub type SessionId = String;

/// Unique identifier for an agent (budget partition).
pub type AgentId = String;

/// Unique identifier for a correction entry.
pub type CorrectionId = String;

/// Unique identifier for a tool definition.
pub type ToolId = String;

// --- Enums ---

/// Supported image formats for the image compressor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
}

/// Conversation turn role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

/// What kind of content we're compressing. The pipeline uses this to pick
/// the right compression strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentType {
    Json,
    CliOutput { command: String },
    Code { language: String, path: PathBuf },
    PlainText,
    Image { format: ImageFormat },
}

/// Which LLM family we're targeting. Affects token counting and cache
/// boundary placement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelFamily {
    AnthropicClaude,
    OpenAiGpt,
    GoogleGemini,
    /// A local model identified by name (e.g. "llama-3.1-8b").
    Local(String),
}

/// API provider — determines cache discount rates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    /// 90% cache discount
    Anthropic,
    /// 50% cache discount
    OpenAI,
    /// No cache boundary
    Google,
}

/// Whether a task is simple (can use a smaller/cheaper model) or complex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskClassification {
    Simple,
    Complex,
}

// --- Content types ---

/// Metadata about where content came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMetadata {
    /// Source identifier (e.g. "stdin", "tool:read_file").
    pub source: Option<String>,
    /// File path, if the content came from a file.
    pub path: Option<PathBuf>,
    /// Programming language, if detected.
    pub language: Option<String>,
}

/// Raw content flowing through the compression pipeline. Stages mutate
/// the `raw` field in place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    /// The text content being compressed.
    pub raw: String,
    /// Detected content type (JSON, code, CLI output, etc.).
    pub content_type: ContentType,
    /// Where this content came from.
    pub metadata: ContentMetadata,
    /// Token count before any compression.
    pub tokens_original: u32,
}

/// Source provenance for a compressed segment — tracks where content came
/// from so you can trace back to the original.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Provenance {
    /// File path the content originated from, if known.
    pub file: Option<PathBuf>,
    /// Line range within the file (start inclusive, end exclusive).
    pub line_range: Option<std::ops::Range<usize>>,
    /// SHA-256 hex digest of the original content.
    pub content_hash: Option<String>,
    /// Tool call ID that produced this content, if applicable.
    pub tool_call_id: Option<String>,
    /// Human-readable source label (e.g. "git diff", "cargo test output").
    pub label: Option<String>,
}

/// The result of compressing content through the pipeline. This is what
/// you get back from [`crate::engine::SqzEngine::compress`].
///
/// ```rust
/// # use sqz_engine::SqzEngine;
/// let engine = SqzEngine::new().unwrap();
/// let result = engine.compress("hello world").unwrap();
///
/// println!("output: {}", result.data);
/// println!("ratio: {:.0}%", (1.0 - result.compression_ratio) * 100.0);
/// println!("stages: {:?}", result.stages_applied);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedContent {
    /// The compressed output text.
    pub data: String,
    /// Token count after compression.
    pub tokens_compressed: u32,
    /// Token count before compression.
    pub tokens_original: u32,
    /// Names of pipeline stages that ran (e.g. "condense", "toon_encode").
    pub stages_applied: Vec<String>,
    /// Ratio of compressed to original tokens (0.0 = perfect, 1.0 = no change).
    pub compression_ratio: f64,
    /// Where this content came from.
    #[serde(default)]
    pub provenance: Provenance,
    /// Verifier result — `None` if the verify pass was skipped.
    #[serde(default)]
    pub verify: Option<VerifyResult>,
}

// --- Session types ---

/// A single conversation turn (user message, assistant response, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub content: String,
    pub tokens: u32,
    pub pinned: bool,
    pub timestamp: DateTime<Utc>,
}

/// Alias for backward compatibility.
pub type ConversationTurn = Turn;

/// A pinned segment — a conversation turn that should never be evicted
/// from the context window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedSegment {
    pub turn_index: usize,
    pub reason: String,
    pub tokens: u32,
}

/// Alias for backward compatibility.
pub type PinEntry = PinnedSegment;

/// A key-value fact extracted from conversation (e.g. "preferred language" → "Rust").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvFact {
    pub key: String,
    pub value: String,
    pub source_turn: usize,
}

/// Alias for backward compatibility.
pub type Learning = KvFact;

/// Current token budget state for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowUsage {
    /// Total context window size in tokens.
    pub window_size: u32,
    /// Tokens consumed so far.
    pub consumed: u32,
    /// Tokens reserved by pinned segments.
    pub pinned: u32,
    /// Which model family determines token counting.
    pub model_family: ModelFamily,
}

/// Alias for backward compatibility.
pub type BudgetState = WindowUsage;

/// Record of a single tool call with cost tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub tokens_input: u32,
    pub tokens_output: u32,
    pub cost_usd: f64,
    pub timestamp: DateTime<Utc>,
}

/// Alias for backward compatibility.
pub type ToolUsageRecord = ToolCall;

/// A single correction (edit) made during a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditRecord {
    pub id: CorrectionId,
    pub timestamp: DateTime<Utc>,
    pub original: String,
    pub correction: String,
    pub context: String,
}

/// Alias for backward compatibility.
pub type CorrectionEntry = EditRecord;

/// History of corrections made during a session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EditHistory {
    pub entries: Vec<EditRecord>,
}

/// Alias for backward compatibility.
pub type CorrectionLog = EditHistory;

/// Active compression session — the main state object that tracks an entire
/// conversation, its budget, tool usage, pins, and learnings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub project_dir: PathBuf,
    pub conversation: Vec<Turn>,
    pub corrections: EditHistory,
    pub pins: Vec<PinnedSegment>,
    pub learnings: Vec<KvFact>,
    pub compressed_summary: String,
    pub budget: WindowUsage,
    pub tool_usage: Vec<ToolCall>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Alias for backward compatibility.
pub type SessionState = Session;

/// Configuration for a single compression stage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StageConfig {
    /// Whether this stage is enabled.
    pub enabled: bool,
    /// Stage-specific options as a JSON value.
    #[serde(default)]
    pub options: serde_json::Value,
}

/// Result of the two-pass compression verifier. Checks that important
/// content (error lines, JSON keys, diff hunks) survived compression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    /// Overall pass/fail.
    pub passed: bool,
    /// Confidence score 0.0–1.0 (1.0 = fully verified).
    pub confidence: f64,
    /// Which checks passed.
    pub checks_passed: Vec<String>,
    /// Which checks failed, with reason.
    pub checks_failed: Vec<(String, String)>,
    /// Whether the pipeline fell back to a safer preset.
    pub fallback_triggered: bool,
}
