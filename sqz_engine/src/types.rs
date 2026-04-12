use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Type aliases
pub type SessionId = String;
pub type AgentId = String;
pub type CorrectionId = String;
pub type ToolId = String;

// --- Enums ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentType {
    Json,
    CliOutput { command: String },
    Code { language: String, path: PathBuf },
    PlainText,
    Image { format: ImageFormat },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelFamily {
    AnthropicClaude,
    OpenAiGpt,
    GoogleGemini,
    Local(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    /// 90% cache discount
    Anthropic,
    /// 50% cache discount
    OpenAI,
    /// No cache boundary
    Google,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskClassification {
    Simple,
    Complex,
}

// --- Content types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMetadata {
    pub source: Option<String>,
    pub path: Option<PathBuf>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    pub raw: String,
    pub content_type: ContentType,
    pub metadata: ContentMetadata,
    pub tokens_original: u32,
}

/// Source provenance for a compressed segment — enables reversibility and trust.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedContent {
    pub data: String,
    pub tokens_compressed: u32,
    pub tokens_original: u32,
    pub stages_applied: Vec<String>,
    pub compression_ratio: f64,
    /// Source provenance — where this content came from.
    #[serde(default)]
    pub provenance: Provenance,
    /// Verifier result — None if verify pass was not run.
    #[serde(default)]
    pub verify: Option<VerifyResult>,
}

// --- Session types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub role: Role,
    pub content: String,
    pub tokens: u32,
    pub pinned: bool,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinEntry {
    pub turn_index: usize,
    pub reason: String,
    pub tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Learning {
    pub key: String,
    pub value: String,
    pub source_turn: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetState {
    pub window_size: u32,
    pub consumed: u32,
    pub pinned: u32,
    pub model_family: ModelFamily,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUsageRecord {
    pub tool_name: String,
    pub tokens_input: u32,
    pub tokens_output: u32,
    pub cost_usd: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionEntry {
    pub id: CorrectionId,
    pub timestamp: DateTime<Utc>,
    pub original: String,
    pub correction: String,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CorrectionLog {
    pub entries: Vec<CorrectionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub id: SessionId,
    pub project_dir: PathBuf,
    pub conversation: Vec<ConversationTurn>,
    pub corrections: CorrectionLog,
    pub pins: Vec<PinEntry>,
    pub learnings: Vec<Learning>,
    pub compressed_summary: String,
    pub budget: BudgetState,
    pub tool_usage: Vec<ToolUsageRecord>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Configuration for a single compression stage
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StageConfig {
    pub enabled: bool,
    #[serde(default)]
    pub options: serde_json::Value,
}

/// Result of the two-pass compression verifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    /// Overall pass/fail.
    pub passed: bool,
    /// Confidence score 0.0–1.0 (1.0 = fully verified).
    pub confidence: f64,
    /// Which checks passed.
    pub checks_passed: Vec<String>,
    /// Which checks failed with reason.
    pub checks_failed: Vec<(String, String)>,
    /// Whether the pipeline fell back to a safer preset.
    pub fallback_triggered: bool,
}
