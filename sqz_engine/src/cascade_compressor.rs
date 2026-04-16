//! Compression Cascades — multi-level compression that degrades gracefully
//! as content ages out of relevance.
//!
//! Level 0 (fresh):   Full compressed content
//! Level 1 (aging):   Signatures + changed lines only
//! Level 2 (old):     File name + public API summary
//! Level 3 (ancient): One-line reference with metadata
//!
//! Each level is a lossy compression of the previous level, but sqz controls
//! what's lost (unlike the LLM's compaction which is unpredictable).


/// Compression cascade level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CascadeLevel {
    /// Full compressed content (current behavior).
    Fresh = 0,
    /// Signatures + changed lines only.
    Aging = 1,
    /// File name + public API summary.
    Old = 2,
    /// One-line reference.
    Ancient = 3,
}

impl CascadeLevel {
    /// Determine the cascade level based on age (turns since last access).
    pub fn from_age(age: u64, thresholds: &CascadeThresholds) -> Self {
        if age < thresholds.aging {
            CascadeLevel::Fresh
        } else if age < thresholds.old {
            CascadeLevel::Aging
        } else if age < thresholds.ancient {
            CascadeLevel::Old
        } else {
            CascadeLevel::Ancient
        }
    }
}

/// Configurable thresholds for cascade levels (in turns).
#[derive(Debug, Clone)]
pub struct CascadeThresholds {
    /// Turns before content moves from Fresh to Aging.
    pub aging: u64,
    /// Turns before content moves from Aging to Old.
    pub old: u64,
    /// Turns before content moves from Old to Ancient.
    pub ancient: u64,
}

impl Default for CascadeThresholds {
    fn default() -> Self {
        Self {
            aging: 10,
            old: 20,
            ancient: 30,
        }
    }
}

/// Result of cascade compression.
#[derive(Debug, Clone)]
pub struct CascadeResult {
    pub text: String,
    pub level: CascadeLevel,
    pub tokens_original: u32,
    pub tokens_compressed: u32,
}

/// Apply cascade compression to content based on its age.
///
/// - Fresh: return content as-is (already compressed by the pipeline)
/// - Aging: extract function/class signatures + any changed lines
/// - Old: file name + public API count
/// - Ancient: one-line reference
pub fn cascade_compress(
    content: &str,
    file_path: &str,
    age: u64,
    thresholds: &CascadeThresholds,
) -> CascadeResult {
    let level = CascadeLevel::from_age(age, thresholds);
    let tokens_original = approx_tokens(content);

    let text = match level {
        CascadeLevel::Fresh => content.to_string(),
        CascadeLevel::Aging => compress_aging(content, file_path),
        CascadeLevel::Old => compress_old(content, file_path),
        CascadeLevel::Ancient => compress_ancient(content, file_path),
    };

    let tokens_compressed = approx_tokens(&text);

    CascadeResult {
        text,
        level,
        tokens_original,
        tokens_compressed,
    }
}

/// Level 1: Extract signatures and key lines.
fn compress_aging(content: &str, file_path: &str) -> String {
    let mut result = Vec::new();
    result.push(format!("// {file_path} (signatures)"));

    for line in content.lines() {
        let trimmed = line.trim();
        // Keep function/class/struct/impl signatures
        if trimmed.starts_with("pub fn ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub struct ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("pub enum ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("pub trait ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("def ")
            || trimmed.starts_with("function ")
            || trimmed.starts_with("export ")
            || trimmed.starts_with("import ")
            || trimmed.starts_with("use ")
        {
            result.push(trimmed.to_string());
        }
        // Keep error/warning lines
        if trimmed.contains("error") || trimmed.contains("Error")
            || trimmed.contains("TODO") || trimmed.contains("FIXME")
        {
            result.push(trimmed.to_string());
        }
    }

    if result.len() <= 1 {
        // No signatures found — keep first and last 3 lines
        let lines: Vec<&str> = content.lines().collect();
        result.push("// (no signatures detected, showing head/tail)".into());
        for line in lines.iter().take(3) {
            result.push(line.to_string());
        }
        if lines.len() > 6 {
            result.push(format!("// ... {} lines omitted ...", lines.len() - 6));
        }
        for line in lines.iter().rev().take(3).rev() {
            result.push(line.to_string());
        }
    }

    result.join("\n")
}

/// Level 2: File name + public API count.
fn compress_old(content: &str, file_path: &str) -> String {
    let pub_count = content.lines()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("pub fn ") || t.starts_with("pub struct ")
                || t.starts_with("pub enum ") || t.starts_with("pub trait ")
                || t.starts_with("export ") || t.starts_with("def ")
        })
        .count();

    let line_count = content.lines().count();
    let token_count = approx_tokens(content);

    format!(
        "[{file_path}: {line_count} lines, {pub_count} public items, ~{token_count} tokens]"
    )
}

/// Level 3: One-line reference.
fn compress_ancient(_content: &str, file_path: &str) -> String {
    format!("[{file_path}]")
}

fn approx_tokens(s: &str) -> u32 {
    ((s.len() as f64) / 4.0).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CODE: &str = r#"use std::collections::HashMap;

pub struct Config {
    pub name: String,
    pub value: i32,
}

impl Config {
    pub fn new(name: &str, value: i32) -> Self {
        Self { name: name.to_string(), value }
    }

    pub fn validate(&self) -> bool {
        !self.name.is_empty() && self.value > 0
    }
}

fn helper(x: i32) -> i32 {
    x * 2
}
"#;

    #[test]
    fn test_fresh_returns_unchanged() {
        let result = cascade_compress(SAMPLE_CODE, "config.rs", 0, &CascadeThresholds::default());
        assert_eq!(result.level, CascadeLevel::Fresh);
        assert_eq!(result.text, SAMPLE_CODE);
    }

    #[test]
    fn test_aging_extracts_signatures() {
        let result = cascade_compress(SAMPLE_CODE, "config.rs", 15, &CascadeThresholds::default());
        assert_eq!(result.level, CascadeLevel::Aging);
        assert!(result.text.contains("pub struct Config"));
        assert!(result.text.contains("pub fn new"));
        assert!(result.text.contains("pub fn validate"));
        assert!(result.tokens_compressed < result.tokens_original);
    }

    #[test]
    fn test_old_shows_summary() {
        let result = cascade_compress(SAMPLE_CODE, "config.rs", 25, &CascadeThresholds::default());
        assert_eq!(result.level, CascadeLevel::Old);
        assert!(result.text.contains("config.rs"));
        assert!(result.text.contains("public items"));
        assert!(result.tokens_compressed < result.tokens_original);
    }

    #[test]
    fn test_ancient_one_line() {
        let result = cascade_compress(SAMPLE_CODE, "config.rs", 35, &CascadeThresholds::default());
        assert_eq!(result.level, CascadeLevel::Ancient);
        assert_eq!(result.text, "[config.rs]");
        assert!(result.tokens_compressed <= 5);
    }

    #[test]
    fn test_custom_thresholds() {
        let thresholds = CascadeThresholds {
            aging: 5,
            old: 10,
            ancient: 15,
        };
        let result = cascade_compress(SAMPLE_CODE, "x.rs", 7, &thresholds);
        assert_eq!(result.level, CascadeLevel::Aging);
    }

    #[test]
    fn test_cascade_level_ordering() {
        assert!(CascadeLevel::Fresh < CascadeLevel::Aging);
        assert!(CascadeLevel::Aging < CascadeLevel::Old);
        assert!(CascadeLevel::Old < CascadeLevel::Ancient);
    }

    #[test]
    fn test_empty_content() {
        let result = cascade_compress("", "empty.rs", 15, &CascadeThresholds::default());
        assert_eq!(result.level, CascadeLevel::Aging);
        assert!(!result.text.is_empty()); // at least the header
    }

    #[test]
    fn test_tokens_decrease_with_level() {
        let thresholds = CascadeThresholds::default();
        let fresh = cascade_compress(SAMPLE_CODE, "x.rs", 0, &thresholds);
        let aging = cascade_compress(SAMPLE_CODE, "x.rs", 15, &thresholds);
        let old = cascade_compress(SAMPLE_CODE, "x.rs", 25, &thresholds);
        let ancient = cascade_compress(SAMPLE_CODE, "x.rs", 35, &thresholds);

        assert!(aging.tokens_compressed <= fresh.tokens_compressed);
        assert!(old.tokens_compressed <= aging.tokens_compressed);
        assert!(ancient.tokens_compressed <= old.tokens_compressed);
    }
}
