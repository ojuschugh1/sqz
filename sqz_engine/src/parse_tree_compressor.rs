//! Parse Tree Guided Code Compression — uses tree-sitter ASTs to identify
//! low-entropy subtrees and collapse them to their signature line.
//!
//! For each node in the parse tree, we compute the Shannon entropy of its
//! text content (character-level). Nodes with entropy below the median are
//! considered "low-entropy" (boilerplate, repetitive code) and are collapsed
//! to just their first line (the signature). High-entropy nodes are preserved
//! in full.
//!
//! # Supported languages
//! - Rust
//! - Python
//! - JavaScript

use tree_sitter::{Language, Parser, Node};
use crate::error::{Result, SqzError};

/// Compress source code by collapsing low-entropy parse tree subtrees.
///
/// Parses `code` using tree-sitter for the given `language`, computes
/// per-node character entropy, and collapses subtrees below the median
/// entropy to their signature line (first line).
///
/// # Arguments
/// * `code` — source code to compress
/// * `language` — one of `"rust"`, `"python"`, `"javascript"`
///
/// # Returns
/// Compressed code string with low-entropy subtrees collapsed.
pub fn compress_code(code: &str, language: &str) -> Result<String> {
    let ts_lang = get_language(language)?;
    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .map_err(|e| SqzError::Other(format!("tree-sitter language error: {e}")))?;

    let tree = parser
        .parse(code, None)
        .ok_or_else(|| SqzError::Other("tree-sitter parse failed".into()))?;

    let root = tree.root_node();

    // Collect top-level nodes with their entropy
    let mut node_entropies: Vec<(usize, usize, f64)> = Vec::new(); // (start_byte, end_byte, entropy)
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        let text = &code[child.byte_range()];
        let entropy = char_entropy(text);
        node_entropies.push((child.start_byte(), child.end_byte(), entropy));
    }

    if node_entropies.is_empty() {
        return Ok(code.to_string());
    }

    // Compute median entropy
    let median = compute_median_entropy(&node_entropies);

    // Build output: preserve high-entropy nodes, collapse low-entropy ones
    let mut result = String::new();
    let mut last_end = 0;

    for (start, end, entropy) in &node_entropies {
        // Preserve whitespace/comments between nodes
        if *start > last_end {
            result.push_str(&code[last_end..*start]);
        }

        let node_text = &code[*start..*end];
        if *entropy < median {
            // Collapse to signature line
            let sig = signature_line(node_text);
            result.push_str(&sig);
            result.push_str(" /* … */");
        } else {
            result.push_str(node_text);
        }

        last_end = *end;
    }

    // Preserve trailing content
    if last_end < code.len() {
        result.push_str(&code[last_end..]);
    }

    Ok(result)
}

/// Compute Shannon entropy (bits per character) of a string.
///
/// Uses character frequency distribution. Returns 0.0 for empty strings.
pub fn char_entropy(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }

    let mut freq = [0u32; 256];
    let mut total = 0u32;

    for byte in text.bytes() {
        freq[byte as usize] += 1;
        total += 1;
    }

    if total == 0 {
        return 0.0;
    }

    let total_f = total as f64;
    let mut entropy = 0.0;
    for &count in &freq {
        if count > 0 {
            let p = count as f64 / total_f;
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Extract the first line of a node's text as its "signature".
fn signature_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .to_string()
}

/// Compute the median entropy from a list of (start, end, entropy) tuples.
fn compute_median_entropy(entries: &[(usize, usize, f64)]) -> f64 {
    let mut entropies: Vec<f64> = entries.iter().map(|(_, _, e)| *e).collect();
    entropies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = entropies.len() / 2;
    if entropies.len() % 2 == 0 && entropies.len() >= 2 {
        (entropies[mid - 1] + entropies[mid]) / 2.0
    } else {
        entropies[mid]
    }
}

/// Resolve a language name to a tree-sitter `Language`.
fn get_language(language: &str) -> Result<Language> {
    match language {
        "rust" => Ok(tree_sitter_rust::language()),
        "python" => Ok(tree_sitter_python::language()),
        "javascript" | "js" => Ok(tree_sitter_javascript::language()),
        other => Err(SqzError::UnsupportedLanguage(other.to_string())),
    }
}

/// Recursively collect entropy for all named children of a node.
/// Returns a flat list of (node_kind, start_byte, end_byte, entropy).
#[allow(dead_code)]
fn collect_node_entropies<'a>(
    node: &Node<'a>,
    source: &str,
) -> Vec<(String, usize, usize, f64)> {
    let mut results = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            let text = &source[child.byte_range()];
            let entropy = char_entropy(text);
            results.push((
                child.kind().to_string(),
                child.start_byte(),
                child.end_byte(),
                entropy,
            ));
        }
    }
    results
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_entropy_empty() {
        assert!((char_entropy("") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_char_entropy_single_char() {
        // All same character → entropy = 0
        assert!((char_entropy("aaaa") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_char_entropy_two_chars() {
        // Equal distribution of 2 chars → entropy = 1.0 bit
        let entropy = char_entropy("abab");
        assert!(
            (entropy - 1.0).abs() < 0.01,
            "expected ~1.0 bit, got {entropy}"
        );
    }

    #[test]
    fn test_compress_rust_code() {
        let code = r#"use std::collections::HashMap;

/// A simple struct.
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
"#;
        let compressed = compress_code(code, "rust").unwrap();
        // Compressed output should not be empty and should contain recognizable content
        assert!(!compressed.is_empty());
        // Should still contain some recognizable content
        assert!(compressed.contains("pub"));
    }

    #[test]
    fn test_compress_python_code() {
        let code = r#"import os
import sys

class MyClass:
    def __init__(self):
        self.data = []
        self.count = 0
        self.name = "default"

    def process(self, item):
        self.data.append(item)
        self.count += 1
        return self.count

def helper_function(x, y):
    result = x + y
    temp = result * 2
    return temp - x
"#;
        let compressed = compress_code(code, "python").unwrap();
        assert!(!compressed.is_empty());
    }

    #[test]
    fn test_compress_javascript_code() {
        let code = r#"import { useState } from 'react';

function App() {
    const [count, setCount] = useState(0);
    const increment = () => setCount(count + 1);
    const decrement = () => setCount(count - 1);
    return { count, increment, decrement };
}

class DataProcessor {
    constructor(data) {
        this.data = data;
        this.processed = false;
    }

    process() {
        this.data = this.data.map(x => x * 2);
        this.processed = true;
        return this.data;
    }
}
"#;
        let compressed = compress_code(code, "javascript").unwrap();
        assert!(!compressed.is_empty());
    }

    #[test]
    fn test_unsupported_language() {
        let result = compress_code("code", "cobol");
        assert!(matches!(result, Err(SqzError::UnsupportedLanguage(_))));
    }

    #[test]
    fn test_empty_code() {
        let compressed = compress_code("", "rust").unwrap();
        assert!(compressed.is_empty() || compressed.trim().is_empty());
    }

    // ── Property-based tests ──────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Entropy is always non-negative.
        #[test]
        fn prop_entropy_non_negative(text in "\\PC{0,200}") {
            let e = char_entropy(&text);
            prop_assert!(e >= 0.0, "entropy should be >= 0, got {e}");
        }

        /// Entropy of a single repeated byte is always 0.
        #[test]
        fn prop_entropy_uniform_is_zero(c in 0x20u8..0x7F, n in 1usize..100) {
            let text: String = std::iter::repeat(c as char).take(n).collect();
            let e = char_entropy(&text);
            prop_assert!(
                e.abs() < f64::EPSILON,
                "uniform text should have entropy 0, got {e}"
            );
        }

        /// Compressed output is never longer than original + overhead.
        /// (The "/* … */" markers add a small constant per collapsed node.)
        #[test]
        fn prop_compress_rust_bounded(
            n_fns in 1usize..=4,
            body_lines in 3usize..=10,
        ) {
            let mut code = String::from("use std::io;\n\n");
            for i in 0..n_fns {
                code.push_str(&format!("fn func_{i}(x: i32) -> i32 {{\n"));
                for j in 0..body_lines {
                    code.push_str(&format!("    let _v{j} = x + {j};\n"));
                }
                code.push_str("    x\n}\n\n");
            }
            let compressed = compress_code(&code, "rust").unwrap();
            // Compressed should not be dramatically larger than original
            // Allow 20% overhead for markers
            let max_len = (code.len() as f64 * 1.2) as usize + 100;
            prop_assert!(
                compressed.len() <= max_len,
                "compressed ({}) should be <= {} (original {} + overhead)",
                compressed.len(),
                max_len,
                code.len()
            );
        }
    }
}
