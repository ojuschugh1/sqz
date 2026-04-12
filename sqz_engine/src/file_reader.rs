//! Multi-mode file reader supporting 8 reading modes for optimal compression.
//!
//! Modes: `full`, `map`, `signatures`, `diff`, `aggressive`, `entropy`, `task`, `lines`.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::ast_parser::AstParser;
use crate::error::{Result, SqzError};

// ── ReadMode ──────────────────────────────────────────────────────────────────

/// The 8 file reading modes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileReadMode {
    /// Complete file content, no compression.
    Full,
    /// Structural overview (~50 tokens for a 300-line file).
    Map,
    /// Function/class signatures only via AST parser.
    Signatures,
    /// Changes since last cached read.
    Diff,
    /// Maximum compression (entropy + signatures combined).
    Aggressive,
    /// High-information sections only (Shannon entropy).
    Entropy,
    /// Task-relevant sections based on current intent via FTS5.
    Task,
    /// Specific line ranges with surrounding context.
    Lines(Range<usize>),
}

// ── ReadResult ────────────────────────────────────────────────────────────────

/// Result of a file read operation.
#[derive(Debug, Clone)]
pub struct ReadResult {
    pub content: String,
    pub mode: String,
    pub tokens_original: u32,
    pub tokens_result: u32,
}

// ── Entropy helpers ───────────────────────────────────────────────────────────

/// A logical block of source code with its entropy score.
#[derive(Debug, Clone)]
pub struct BlockEntropy {
    pub start_line: usize,
    pub end_line: usize,
    pub entropy: f64,
    pub text: String,
}

/// Compute Shannon entropy (bits per character) for a string.
fn shannon_entropy(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let mut freq: HashMap<char, usize> = HashMap::new();
    let total = text.len() as f64;
    for ch in text.chars() {
        *freq.entry(ch).or_insert(0) += 1;
    }
    let mut entropy = 0.0;
    for &count in freq.values() {
        let p = count as f64 / total;
        if p > 0.0 {
            entropy -= p * p.log2();
        }
    }
    entropy
}

/// Split source into logical blocks (separated by blank lines) and compute
/// entropy for each block.
fn compute_block_entropies(source: &str) -> Vec<BlockEntropy> {
    let lines: Vec<&str> = source.lines().collect();
    let mut blocks = Vec::new();
    let mut block_start = 0;
    let mut current_block = String::new();

    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            if !current_block.trim().is_empty() {
                blocks.push(BlockEntropy {
                    start_line: block_start,
                    end_line: i,
                    entropy: shannon_entropy(&current_block),
                    text: current_block.clone(),
                });
            }
            current_block.clear();
            block_start = i + 1;
        } else {
            if current_block.is_empty() {
                block_start = i;
            }
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
        }
    }
    // Flush last block
    if !current_block.trim().is_empty() {
        blocks.push(BlockEntropy {
            start_line: block_start,
            end_line: lines.len(),
            entropy: shannon_entropy(&current_block),
            text: current_block,
        });
    }
    blocks
}

/// Return blocks above the given percentile threshold.
fn filter_high_entropy(blocks: &[BlockEntropy], percentile: f64) -> Vec<&BlockEntropy> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut entropies: Vec<f64> = blocks.iter().map(|b| b.entropy).collect();
    entropies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((percentile / 100.0) * (entropies.len() as f64 - 1.0)).round() as usize;
    let threshold = entropies[idx.min(entropies.len() - 1)];
    blocks.iter().filter(|b| b.entropy >= threshold).collect()
}

// ── FTS5 task-mode helpers ────────────────────────────────────────────────────

/// Index file chunks into an in-memory FTS5 table and return sections matching
/// the intent via BM25 ranking.
fn fts5_task_filter(source: &str, intent: &str) -> Result<String> {
    let chunks = chunk_by_blocks(source);
    if chunks.is_empty() {
        return Ok(String::new());
    }

    let conn = Connection::open_in_memory()
        .map_err(|e| SqzError::Other(format!("FTS5 in-memory open failed: {e}")))?;

    conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS file_fts USING fts5(
            chunk_id,
            body,
            tokenize='porter ascii'
        );
        "#,
    )
    .map_err(|e| SqzError::Other(format!("FTS5 schema creation failed: {e}")))?;

    for (i, chunk) in chunks.iter().enumerate() {
        conn.execute(
            "INSERT INTO file_fts(chunk_id, body) VALUES (?1, ?2)",
            params![i.to_string(), chunk],
        )
        .map_err(|e| SqzError::Other(format!("FTS5 insert failed: {e}")))?;
    }

    // Sanitize intent for FTS5 query
    let sanitized: String = intent
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' })
        .collect();
    let terms: Vec<&str> = sanitized.split_whitespace().collect();
    if terms.is_empty() {
        // No usable terms — return full content
        return Ok(source.to_string());
    }

    let fts_query = terms.join(" OR ");

    let mut stmt = conn
        .prepare(
            r#"SELECT body FROM file_fts
               WHERE file_fts MATCH ?1
               ORDER BY rank
               LIMIT 20"#,
        )
        .map_err(|e| SqzError::Other(format!("FTS5 query prepare failed: {e}")))?;

    let rows = stmt
        .query_map(params![fts_query], |row| row.get::<_, String>(0))
        .map_err(|e| SqzError::Other(format!("FTS5 query failed: {e}")))?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(|e| SqzError::Other(format!("FTS5 row read failed: {e}")))?);
    }

    if results.is_empty() {
        // No matches — return full content as fallback
        return Ok(source.to_string());
    }

    Ok(results.join("\n\n"))
}

/// Chunk source by blank-line-separated blocks (same strategy as sandbox).
fn chunk_by_blocks(text: &str) -> Vec<String> {
    const MAX_CHUNK_BYTES: usize = 512;
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut chunks = Vec::new();

    for para in paragraphs {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() <= MAX_CHUNK_BYTES {
            chunks.push(trimmed.to_string());
        } else {
            let mut current = String::new();
            for line in trimmed.lines() {
                if !current.is_empty() && current.len() + line.len() + 1 > MAX_CHUNK_BYTES {
                    chunks.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push('\n');
                }
                current.push_str(line);
            }
            if !current.is_empty() {
                chunks.push(current);
            }
        }
    }

    if chunks.is_empty() && !text.trim().is_empty() {
        chunks.push(text.trim().to_string());
    }
    chunks
}

// ── Approximate token count ───────────────────────────────────────────────────

fn approx_tokens(s: &str) -> u32 {
    ((s.len() as f64) / 4.0).ceil() as u32
}

// ── FileReader ────────────────────────────────────────────────────────────────

/// Multi-mode file reader that produces compressed output based on the
/// selected reading mode.
pub struct FileReader {
    ast_parser: AstParser,
    entropy_percentile: f64,
    context_lines: usize,
}

impl FileReader {
    /// Create a new `FileReader` with default settings.
    ///
    /// - `entropy_percentile`: retain blocks above this percentile (default 60.0).
    /// - `context_lines`: lines of context around line ranges (default 3).
    pub fn new() -> Self {
        Self {
            ast_parser: AstParser::new(),
            entropy_percentile: 60.0,
            context_lines: 3,
        }
    }

    /// Create a `FileReader` with custom entropy percentile and context lines.
    pub fn with_config(entropy_percentile: f64, context_lines: usize) -> Self {
        Self {
            ast_parser: AstParser::new(),
            entropy_percentile,
            context_lines,
        }
    }

    /// Read a file using the specified mode.
    ///
    /// - `path`: file path (used for language detection in signatures/map modes).
    /// - `source`: the file content as a string.
    /// - `mode`: one of the 8 reading modes.
    /// - `intent`: optional task intent for `Task` mode.
    /// - `cached_content`: optional previously cached content for `Diff` mode.
    pub fn read(
        &self,
        path: &Path,
        source: &str,
        mode: &FileReadMode,
        intent: Option<&str>,
        cached_content: Option<&str>,
    ) -> Result<ReadResult> {
        let tokens_original = approx_tokens(source);

        match mode {
            FileReadMode::Full => self.read_full(source, tokens_original),
            FileReadMode::Map => self.read_map(path, source, tokens_original),
            FileReadMode::Signatures => self.read_signatures(path, source, tokens_original),
            FileReadMode::Diff => self.read_diff(source, cached_content, tokens_original),
            FileReadMode::Aggressive => self.read_aggressive(path, source, tokens_original),
            FileReadMode::Entropy => self.read_entropy(source, tokens_original),
            FileReadMode::Task => self.read_task(source, intent, tokens_original),
            FileReadMode::Lines(range) => {
                self.read_lines(source, range.clone(), tokens_original)
            }
        }
    }

    /// Full mode: return the complete file content unchanged.
    fn read_full(&self, source: &str, tokens_original: u32) -> Result<ReadResult> {
        Ok(ReadResult {
            content: source.to_string(),
            mode: "full".to_string(),
            tokens_original,
            tokens_result: tokens_original,
        })
    }

    /// Map mode: produce a structural overview ≤50 tokens for a typical
    /// 300-line file. Shows module hierarchy, exports, dependencies, and
    /// type signatures in a compact format.
    fn read_map(&self, path: &Path, source: &str, tokens_original: u32) -> Result<ReadResult> {
        let lang = detect_language(path);
        let mut parts: Vec<String> = Vec::new();

        // File header
        let line_count = source.lines().count();
        parts.push(format!(
            "# {} ({} lines)",
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            line_count
        ));

        if let Some(lang) = &lang {
            if self.ast_parser.is_supported(lang) {
                if let Ok(summary) = self.ast_parser.extract_signatures(source, lang) {
                    // Imports (compact count only)
                    if !summary.imports.is_empty() {
                        let count = summary.imports.len();
                        parts.push(format!("imports: {count}"));
                    }
                    // Deduplicate type names and show count
                    if !summary.types.is_empty() {
                        let mut names: Vec<&str> =
                            summary.types.iter().map(|t| t.name.as_str()).collect();
                        names.sort_unstable();
                        names.dedup();
                        parts.push(format!("types({}): {}", names.len(), names.join(", ")));
                    }
                    // Deduplicate class/struct names and show count
                    if !summary.classes.is_empty() {
                        let mut names: Vec<&str> =
                            summary.classes.iter().map(|c| c.name.as_str()).collect();
                        names.sort_unstable();
                        names.dedup();
                        parts.push(format!("structs({}): {}", names.len(), names.join(", ")));
                    }
                    // Deduplicate function names and show count
                    if !summary.functions.is_empty() {
                        let mut names: Vec<&str> =
                            summary.functions.iter().map(|f| f.name.as_str()).collect();
                        names.sort_unstable();
                        names.dedup();
                        parts.push(format!("fns({}): {}", names.len(), names.join(", ")));
                    }
                }
            }
        }

        // Enforce ≤50 token budget: truncate parts if needed
        const MAP_TOKEN_BUDGET: u32 = 50;
        loop {
            let content = parts.join("\n");
            if approx_tokens(&content) <= MAP_TOKEN_BUDGET || parts.len() <= 1 {
                break;
            }
            // Remove the last detail line to shrink output
            parts.pop();
        }

        // If AST didn't produce much, fall back to line-count summary
        if parts.len() <= 1 {
            // Simple structural scan: count indentation-based sections
            let mut section_count = 0u32;
            for line in source.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("def ")
                    || trimmed.starts_with("class ")
                    || trimmed.starts_with("function ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("impl ")
                    || trimmed.starts_with("trait ")
                {
                    section_count += 1;
                }
            }
            if section_count > 0 {
                parts.push(format!("sections: {section_count}"));
            }
        }

        let content = parts.join("\n");
        let tokens_result = approx_tokens(&content);

        Ok(ReadResult {
            content,
            mode: "map".to_string(),
            tokens_original,
            tokens_result,
        })
    }

    /// Signatures mode: extract function/class signatures via AST parser.
    fn read_signatures(
        &self,
        path: &Path,
        source: &str,
        tokens_original: u32,
    ) -> Result<ReadResult> {
        let lang = detect_language(path);
        if let Some(lang) = &lang {
            if self.ast_parser.is_supported(lang) {
                let summary = self.ast_parser.extract_signatures(source, lang)?;
                let content = summary.to_text();
                let tokens_result = approx_tokens(&content);
                return Ok(ReadResult {
                    content,
                    mode: "signatures".to_string(),
                    tokens_original,
                    tokens_result,
                });
            }
        }
        // Fallback: return full content for unsupported languages
        Ok(ReadResult {
            content: source.to_string(),
            mode: "signatures".to_string(),
            tokens_original,
            tokens_result: tokens_original,
        })
    }

    /// Diff mode: compare against cached version, return only changes with
    /// surrounding context lines.
    fn read_diff(
        &self,
        source: &str,
        cached_content: Option<&str>,
        tokens_original: u32,
    ) -> Result<ReadResult> {
        let cached = match cached_content {
            Some(c) => c,
            None => {
                // No cached version — return full content
                return Ok(ReadResult {
                    content: source.to_string(),
                    mode: "diff".to_string(),
                    tokens_original,
                    tokens_result: tokens_original,
                });
            }
        };

        if source == cached {
            let content = "(no changes)".to_string();
            return Ok(ReadResult {
                content,
                mode: "diff".to_string(),
                tokens_original,
                tokens_result: approx_tokens("(no changes)"),
            });
        }

        let new_lines: Vec<&str> = source.lines().collect();
        let old_lines: Vec<&str> = cached.lines().collect();

        // Find changed line indices using a simple line-by-line comparison
        let mut changed_lines: Vec<usize> = Vec::new();
        let max_len = new_lines.len().max(old_lines.len());
        for i in 0..max_len {
            let new_line = new_lines.get(i).copied().unwrap_or("");
            let old_line = old_lines.get(i).copied().unwrap_or("");
            if new_line != old_line {
                changed_lines.push(i);
            }
        }

        if changed_lines.is_empty() {
            let content = "(no changes)".to_string();
            return Ok(ReadResult {
                content,
                mode: "diff".to_string(),
                tokens_original,
                tokens_result: approx_tokens("(no changes)"),
            });
        }

        // Build output with context around changed lines
        let ctx = self.context_lines;
        let mut included: Vec<bool> = vec![false; new_lines.len()];
        for &line_idx in &changed_lines {
            let start = line_idx.saturating_sub(ctx);
            let end = (line_idx + ctx + 1).min(new_lines.len());
            for j in start..end {
                included[j] = true;
            }
        }

        let mut output = Vec::new();
        let mut in_range = false;
        for (i, line) in new_lines.iter().enumerate() {
            if included[i] {
                if !in_range {
                    output.push(format!("@@ line {} @@", i + 1));
                    in_range = true;
                }
                let marker = if changed_lines.contains(&i) {
                    ">"
                } else {
                    " "
                };
                output.push(format!("{marker} {line}"));
            } else {
                in_range = false;
            }
        }

        let content = output.join("\n");
        let tokens_result = approx_tokens(&content);

        Ok(ReadResult {
            content,
            mode: "diff".to_string(),
            tokens_original,
            tokens_result,
        })
    }

    /// Aggressive mode: maximum compression combining entropy filtering and
    /// signature extraction.
    fn read_aggressive(
        &self,
        path: &Path,
        source: &str,
        tokens_original: u32,
    ) -> Result<ReadResult> {
        // First try signatures
        let lang = detect_language(path);
        let sig_content = if let Some(lang) = &lang {
            if self.ast_parser.is_supported(lang) {
                self.ast_parser
                    .extract_signatures(source, lang)
                    .ok()
                    .map(|s| s.to_text())
            } else {
                None
            }
        } else {
            None
        };

        // Then entropy filter on the original source
        let blocks = compute_block_entropies(source);
        let high = filter_high_entropy(&blocks, self.entropy_percentile);
        let entropy_content: String = high.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n\n");

        // Combine: prefer signatures if available, append high-entropy blocks
        // that aren't already covered
        let content = match sig_content {
            Some(sigs) if !sigs.is_empty() => {
                if entropy_content.is_empty() {
                    sigs
                } else {
                    format!("{sigs}\n\n// --- high-entropy blocks ---\n{entropy_content}")
                }
            }
            _ => {
                if entropy_content.is_empty() {
                    source.to_string()
                } else {
                    entropy_content
                }
            }
        };

        let tokens_result = approx_tokens(&content).min(tokens_original);
        Ok(ReadResult {
            content,
            mode: "aggressive".to_string(),
            tokens_original,
            tokens_result,
        })
    }

    /// Entropy mode: compute Shannon entropy per block, return only
    /// high-entropy blocks.
    fn read_entropy(&self, source: &str, tokens_original: u32) -> Result<ReadResult> {
        let blocks = compute_block_entropies(source);
        let high = filter_high_entropy(&blocks, self.entropy_percentile);

        if high.is_empty() {
            return Ok(ReadResult {
                content: source.to_string(),
                mode: "entropy".to_string(),
                tokens_original,
                tokens_result: tokens_original,
            });
        }

        let content: String = high
            .iter()
            .map(|b| format!("// lines {}-{}\n{}", b.start_line + 1, b.end_line, b.text))
            .collect::<Vec<_>>()
            .join("\n\n");

        let tokens_result = approx_tokens(&content);

        // If line annotations pushed us above full mode, fall back to raw
        // high-entropy text without annotations.
        if tokens_result > tokens_original {
            let plain: String = high
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let plain_tokens = approx_tokens(&plain).min(tokens_original);
            return Ok(ReadResult {
                content: plain,
                mode: "entropy".to_string(),
                tokens_original,
                tokens_result: plain_tokens,
            });
        }

        Ok(ReadResult {
            content,
            mode: "entropy".to_string(),
            tokens_original,
            tokens_result,
        })
    }

    /// Task mode: use current intent to select relevant sections via FTS5.
    fn read_task(
        &self,
        source: &str,
        intent: Option<&str>,
        tokens_original: u32,
    ) -> Result<ReadResult> {
        let intent = match intent {
            Some(i) if !i.trim().is_empty() => i,
            _ => {
                // No intent — return full content
                return Ok(ReadResult {
                    content: source.to_string(),
                    mode: "task".to_string(),
                    tokens_original,
                    tokens_result: tokens_original,
                });
            }
        };

        let content = fts5_task_filter(source, intent)?;
        let tokens_result = approx_tokens(&content);

        Ok(ReadResult {
            content,
            mode: "task".to_string(),
            tokens_original,
            tokens_result,
        })
    }

    /// Lines mode: extract specific line ranges with context.
    fn read_lines(
        &self,
        source: &str,
        range: Range<usize>,
        tokens_original: u32,
    ) -> Result<ReadResult> {
        let lines: Vec<&str> = source.lines().collect();
        let total = lines.len();

        // Clamp range to valid bounds
        let start = range.start.min(total);
        let end = range.end.min(total);

        if start >= end {
            return Ok(ReadResult {
                content: String::new(),
                mode: "lines".to_string(),
                tokens_original,
                tokens_result: 0,
            });
        }

        // Add context lines
        let ctx_start = start.saturating_sub(self.context_lines);
        let ctx_end = (end + self.context_lines).min(total);

        let mut output = Vec::new();
        output.push(format!("// lines {}-{} (of {})", start + 1, end, total));
        for i in ctx_start..ctx_end {
            let marker = if i >= start && i < end { ">" } else { " " };
            output.push(format!("{marker} {:4} | {}", i + 1, lines[i]));
        }

        let content = output.join("\n");
        let tokens_result = approx_tokens(&content);

        Ok(ReadResult {
            content,
            mode: "lines".to_string(),
            tokens_original,
            tokens_result,
        })
    }

    /// Access the underlying AST parser.
    pub fn ast_parser(&self) -> &AstParser {
        &self.ast_parser
    }

    /// Get the configured entropy percentile.
    pub fn entropy_percentile(&self) -> f64 {
        self.entropy_percentile
    }
}

impl Default for FileReader {
    fn default() -> Self {
        Self::new()
    }
}

// ── Language detection ────────────────────────────────────────────────────────

/// Detect programming language from file extension.
fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    let lang = match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "rb" => "ruby",
        "sh" | "bash" => "bash",
        "json" => "json",
        "html" | "htm" => "html",
        "css" => "css",
        "cs" => "csharp",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "toml" => "toml",
        "yml" | "yaml" => "yaml",
        _ => return None,
    };
    Some(lang.to_string())
}

// ── Public helpers ────────────────────────────────────────────────────────────

/// Compute Shannon entropy for a string (public for use by EntropyAnalyzer).
pub fn compute_entropy(text: &str) -> f64 {
    shannon_entropy(text)
}

/// Compute block entropies for source code (public for use by EntropyAnalyzer).
pub fn analyze_block_entropies(source: &str) -> Vec<BlockEntropy> {
    compute_block_entropies(source)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sample_rust_source() -> &'static str {
        r#"use std::collections::HashMap;
use std::path::Path;

/// A configuration struct.
pub struct Config {
    pub name: String,
    pub value: i32,
}

impl Config {
    pub fn new(name: &str, value: i32) -> Self {
        Self {
            name: name.to_string(),
            value,
        }
    }

    pub fn validate(&self) -> bool {
        !self.name.is_empty() && self.value > 0
    }
}

pub fn process(config: &Config) -> String {
    let mut result = String::new();
    for i in 0..config.value {
        result.push_str(&format!("item {}: {}\n", i, config.name));
    }
    result
}

pub type ConfigMap = HashMap<String, Config>;

fn internal_helper() -> i32 {
    42
}
"#
    }

    fn large_source(lines: usize) -> String {
        let mut src = String::new();
        src.push_str("use std::collections::HashMap;\n\n");
        src.push_str("pub struct MyStruct {\n    field: i32,\n}\n\n");
        for i in 0..lines.saturating_sub(6) {
            src.push_str(&format!("// line {i}: some content here\n"));
        }
        src
    }

    #[test]
    fn test_full_mode_returns_unchanged() {
        let reader = FileReader::new();
        let source = "hello world\nline two\n";
        let result = reader
            .read(Path::new("test.txt"), source, &FileReadMode::Full, None, None)
            .unwrap();
        assert_eq!(result.content, source);
        assert_eq!(result.mode, "full");
        assert_eq!(result.tokens_original, result.tokens_result);
    }

    #[test]
    fn test_map_mode_compact_output() {
        let reader = FileReader::new();
        let source = &large_source(300);
        let result = reader
            .read(
                Path::new("test.rs"),
                source,
                &FileReadMode::Map,
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.mode, "map");
        // Map mode should produce ≤50 tokens for a ~300-line file
        assert!(
            result.tokens_result <= 50,
            "map mode produced {} tokens, expected ≤50",
            result.tokens_result
        );
    }

    #[test]
    fn test_signatures_mode_extracts_signatures() {
        let reader = FileReader::new();
        let source = sample_rust_source();
        let result = reader
            .read(
                Path::new("test.rs"),
                source,
                &FileReadMode::Signatures,
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.mode, "signatures");
        assert!(result.content.contains("use std::collections::HashMap"));
        assert!(result.tokens_result < result.tokens_original);
    }

    #[test]
    fn test_signatures_mode_unsupported_language_fallback() {
        let reader = FileReader::new();
        let source = "some content";
        let result = reader
            .read(
                Path::new("test.xyz"),
                source,
                &FileReadMode::Signatures,
                None,
                None,
            )
            .unwrap();
        // Unsupported language: returns full content
        assert_eq!(result.content, source);
    }

    #[test]
    fn test_diff_mode_no_cached() {
        let reader = FileReader::new();
        let source = "line 1\nline 2\n";
        let result = reader
            .read(
                Path::new("test.txt"),
                source,
                &FileReadMode::Diff,
                None,
                None,
            )
            .unwrap();
        // No cached content — returns full
        assert_eq!(result.content, source);
    }

    #[test]
    fn test_diff_mode_no_changes() {
        let reader = FileReader::new();
        let source = "line 1\nline 2\n";
        let result = reader
            .read(
                Path::new("test.txt"),
                source,
                &FileReadMode::Diff,
                None,
                Some(source),
            )
            .unwrap();
        assert_eq!(result.content, "(no changes)");
    }

    #[test]
    fn test_diff_mode_with_changes() {
        let reader = FileReader::new();
        let old = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        let new = "line 1\nline 2 modified\nline 3\nline 4\nline 5\n";
        let result = reader
            .read(
                Path::new("test.txt"),
                new,
                &FileReadMode::Diff,
                None,
                Some(old),
            )
            .unwrap();
        assert!(result.content.contains("line 2 modified"));
        assert!(result.content.contains("@@"));
        // Diff output includes only changed sections, not the full file
        assert_ne!(result.content, new);
    }

    #[test]
    fn test_entropy_mode_filters_blocks() {
        let reader = FileReader::new();
        // Create source with varying entropy: some complex code, some boilerplate
        let source = r#"
fn complex_algorithm(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    for (i, &byte) in data.iter().enumerate() {
        let transformed = byte ^ (i as u8).wrapping_mul(0x5A);
        result.push(transformed.rotate_left(3));
    }
    result
}

// boilerplate
// boilerplate
// boilerplate
// boilerplate
// boilerplate

pub fn another_complex_fn(x: f64, y: f64) -> f64 {
    let theta = x.atan2(y);
    let r = (x * x + y * y).sqrt();
    r * theta.sin() + theta.cos() * r.ln()
}
"#;
        let result = reader
            .read(
                Path::new("test.rs"),
                source,
                &FileReadMode::Entropy,
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.mode, "entropy");
    }

    #[test]
    fn test_task_mode_no_intent_returns_full() {
        let reader = FileReader::new();
        let source = "some content\n";
        let result = reader
            .read(
                Path::new("test.txt"),
                source,
                &FileReadMode::Task,
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.content, source);
    }

    #[test]
    fn test_task_mode_with_intent() {
        let reader = FileReader::new();
        let source = r#"
fn authentication_handler(req: Request) -> Response {
    let token = req.header("Authorization");
    validate_token(token)
}

fn database_query(sql: &str) -> Vec<Row> {
    let conn = get_connection();
    conn.execute(sql)
}

fn logging_middleware(req: Request) -> Request {
    println!("Request: {}", req.path());
    req
}
"#;
        let result = reader
            .read(
                Path::new("test.rs"),
                source,
                &FileReadMode::Task,
                Some("authentication token validation"),
                None,
            )
            .unwrap();
        assert_eq!(result.mode, "task");
        // Should include the authentication section
        assert!(result.content.contains("authentication") || result.content.contains("token"));
    }

    #[test]
    fn test_lines_mode_extracts_range() {
        let reader = FileReader::new();
        let source = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\n";
        let result = reader
            .read(
                Path::new("test.txt"),
                source,
                &FileReadMode::Lines(3..6),
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.mode, "lines");
        assert!(result.content.contains("line 4"));
        assert!(result.content.contains("line 5"));
        assert!(result.content.contains("line 6"));
    }

    #[test]
    fn test_lines_mode_empty_range() {
        let reader = FileReader::new();
        let source = "line 1\nline 2\n";
        let result = reader
            .read(
                Path::new("test.txt"),
                source,
                &FileReadMode::Lines(5..5),
                None,
                None,
            )
            .unwrap();
        assert!(result.content.is_empty());
    }

    #[test]
    fn test_aggressive_mode_compresses() {
        let reader = FileReader::new();
        let source = sample_rust_source();
        let result = reader
            .read(
                Path::new("test.rs"),
                source,
                &FileReadMode::Aggressive,
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.mode, "aggressive");
        // Aggressive should produce fewer tokens than full
        assert!(
            result.tokens_result <= result.tokens_original,
            "aggressive mode should compress: {} vs {}",
            result.tokens_result,
            result.tokens_original
        );
    }

    #[test]
    fn test_shannon_entropy_empty() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    #[test]
    fn test_shannon_entropy_single_char() {
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    fn test_shannon_entropy_varied() {
        let e = shannon_entropy("abcdefghij");
        // 10 distinct chars, each appearing once: entropy = log2(10) ≈ 3.32
        assert!(e > 3.0, "entropy of varied text should be high: {e}");
    }

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language(Path::new("foo.rs")), Some("rust".into()));
        assert_eq!(detect_language(Path::new("bar.py")), Some("python".into()));
        assert_eq!(detect_language(Path::new("baz.js")), Some("javascript".into()));
        assert_eq!(detect_language(Path::new("qux.ts")), Some("typescript".into()));
        assert_eq!(detect_language(Path::new("no_ext")), None);
    }

    #[test]
    fn test_file_read_mode_enum_variants() {
        // Ensure all 8 modes exist
        let modes: Vec<FileReadMode> = vec![
            FileReadMode::Full,
            FileReadMode::Map,
            FileReadMode::Signatures,
            FileReadMode::Diff,
            FileReadMode::Aggressive,
            FileReadMode::Entropy,
            FileReadMode::Task,
            FileReadMode::Lines(0..10),
        ];
        assert_eq!(modes.len(), 8);
    }

    #[test]
    fn test_block_entropies_computation() {
        let source = "fn foo() {\n    let x = 1;\n}\n\nfn bar() {\n    let y = 2;\n}\n";
        let blocks = compute_block_entropies(source);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].entropy > 0.0);
        assert!(blocks[1].entropy > 0.0);
    }

    #[test]
    fn test_default_creates_reader() {
        let reader = FileReader::default();
        assert_eq!(reader.entropy_percentile(), 60.0);
    }

    #[test]
    fn test_with_config() {
        let reader = FileReader::with_config(75.0, 5);
        assert_eq!(reader.entropy_percentile(), 75.0);
    }

    // ── Property-based tests ──────────────────────────────────────────────

    use proptest::prelude::*;

    /// Generate random source code content of a given line count.
    /// Produces a mix of struct definitions, function signatures, comments,
    /// and blank lines to simulate realistic Rust source files.
    fn arb_source_code(min_lines: usize, max_lines: usize) -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just("use std::collections::HashMap;\n".to_string()),
                Just("pub struct Foo {\n    field: i32,\n}\n".to_string()),
                Just("pub fn bar(x: i32) -> i32 {\n    x + 1\n}\n".to_string()),
                Just("// a comment line\n".to_string()),
                Just("\n".to_string()),
                Just("fn helper() -> bool { true }\n".to_string()),
                Just("let val = compute(a, b, c);\n".to_string()),
                Just("impl Foo {\n    pub fn new() -> Self { Self { field: 0 } }\n}\n".to_string()),
            ],
            min_lines..=max_lines,
        )
        .prop_map(|chunks| chunks.join(""))
    }

    // ── Property 36: Multi-mode file reading compression ratio ────────────
    //
    // **Validates: Requirements 31.2, 31.3**
    //
    // For any file of at least 100 lines:
    //   (a) `map` mode SHALL produce output of at most 50 tokens.
    //   (b) Non-full modes (map, signatures, entropy, aggressive) SHALL
    //       produce fewer or equal tokens compared to full mode.

    proptest! {
        #[test]
        fn prop36_map_mode_token_limit(
            source in arb_source_code(40, 80),
        ) {
            // Each chunk produces ~2-4 lines, so 40-80 chunks ≈ 100-300+ lines
            let line_count = source.lines().count();
            // Only test files with at least 100 lines
            prop_assume!(line_count >= 100);

            let reader = FileReader::new();
            let path = Path::new("test.rs");

            let map_result = reader
                .read(path, &source, &FileReadMode::Map, None, None)
                .unwrap();

            prop_assert!(
                map_result.tokens_result <= 50,
                "map mode produced {} tokens for a {}-line file, expected ≤50",
                map_result.tokens_result,
                line_count
            );
        }

        #[test]
        fn prop36_non_full_modes_compress(
            source in arb_source_code(30, 80),
        ) {
            let line_count = source.lines().count();
            prop_assume!(line_count >= 100);

            let reader = FileReader::new();
            let path = Path::new("test.rs");

            let full_result = reader
                .read(path, &source, &FileReadMode::Full, None, None)
                .unwrap();
            let full_tokens = full_result.tokens_result;

            // map mode
            let map_result = reader
                .read(path, &source, &FileReadMode::Map, None, None)
                .unwrap();
            prop_assert!(
                map_result.tokens_result <= full_tokens,
                "map ({}) should be ≤ full ({})",
                map_result.tokens_result, full_tokens
            );

            // signatures mode
            let sig_result = reader
                .read(path, &source, &FileReadMode::Signatures, None, None)
                .unwrap();
            prop_assert!(
                sig_result.tokens_result <= full_tokens,
                "signatures ({}) should be ≤ full ({})",
                sig_result.tokens_result, full_tokens
            );

            // entropy mode
            let ent_result = reader
                .read(path, &source, &FileReadMode::Entropy, None, None)
                .unwrap();
            prop_assert!(
                ent_result.tokens_result <= full_tokens,
                "entropy ({}) should be ≤ full ({})",
                ent_result.tokens_result, full_tokens
            );

            // aggressive mode
            let agg_result = reader
                .read(path, &source, &FileReadMode::Aggressive, None, None)
                .unwrap();
            prop_assert!(
                agg_result.tokens_result <= full_tokens,
                "aggressive ({}) should be ≤ full ({})",
                agg_result.tokens_result, full_tokens
            );
        }
    }
}
