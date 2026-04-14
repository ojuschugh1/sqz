/// Dictionary-based JSON Compression.
///
/// Exploits cross-document redundancy by maintaining a dictionary of common
/// JSON field names, value patterns, and structural elements. Each new JSON
/// document is compressed relative to this shared dictionary, achieving
/// better ratios than compressing in isolation.
///
/// This is a pure-Rust implementation that avoids adding the zstd crate
/// (which would add ~200KB to the binary). Instead, it uses a dictionary
/// substitution approach optimized for JSON payloads.

use std::collections::HashMap;

use crate::error::Result;

/// A dictionary entry mapping a common pattern to a short code.
#[derive(Debug, Clone)]
struct DictEntry {
    /// The original pattern (e.g., a field name or common value).
    pattern: String,
    /// The short replacement code.
    code: String,
    /// How many times this pattern has been seen.
    #[allow(dead_code)]
    frequency: u32,
}

/// Configuration for dictionary compression.
#[derive(Debug, Clone)]
pub struct DictConfig {
    /// Maximum number of dictionary entries.
    /// Default: 256
    pub max_entries: usize,
    /// Minimum pattern length (chars) to consider for dictionary.
    /// Default: 4
    pub min_pattern_length: usize,
    /// Minimum frequency before a pattern is added to the dictionary.
    /// Default: 2
    pub min_frequency: u32,
}

impl Default for DictConfig {
    fn default() -> Self {
        Self {
            max_entries: 256,
            min_pattern_length: 4,
            min_frequency: 2,
        }
    }
}

/// Result of dictionary compression.
#[derive(Debug, Clone)]
pub struct DictCompressResult {
    /// The compressed text.
    pub data: String,
    /// Number of substitutions made.
    pub substitutions: usize,
    /// Bytes saved.
    pub bytes_saved: usize,
    /// Whether the dictionary header was included.
    pub has_dict_header: bool,
}

/// Dictionary-based JSON compressor.
///
/// Maintains a session-level dictionary of common JSON patterns and
/// substitutes them with short codes during compression.
pub struct DictCompressor {
    config: DictConfig,
    /// Pattern → DictEntry mapping.
    entries: HashMap<String, DictEntry>,
    /// Frequency counter for candidate patterns.
    candidates: HashMap<String, u32>,
    /// Next code index for new entries.
    next_code: u16,
    /// Built-in common JSON field names.
    builtin_entries: Vec<DictEntry>,
}

impl DictCompressor {
    pub fn new() -> Self {
        Self::with_config(DictConfig::default())
    }

    pub fn with_config(config: DictConfig) -> Self {
        let builtin = build_builtin_dictionary();
        let mut compressor = Self {
            config,
            entries: HashMap::new(),
            candidates: HashMap::new(),
            next_code: 0,
            builtin_entries: builtin,
        };
        compressor.load_builtins();
        compressor
    }

    /// Load built-in dictionary entries.
    fn load_builtins(&mut self) {
        for entry in &self.builtin_entries.clone() {
            self.entries.insert(entry.pattern.clone(), entry.clone());
        }
        self.next_code = self.entries.len() as u16;
    }

    /// Observe a JSON string to learn common patterns.
    pub fn observe(&mut self, json_str: &str) {
        // Extract field names from JSON
        let fields = extract_json_fields(json_str);
        for field in fields {
            if field.len() >= self.config.min_pattern_length {
                let count = self.candidates.entry(field.clone()).or_insert(0);
                *count += 1;

                // Promote to dictionary if frequency threshold met
                if *count >= self.config.min_frequency
                    && !self.entries.contains_key(&field)
                    && self.entries.len() < self.config.max_entries
                {
                    let code = format!("~{:X}", self.next_code);
                    self.next_code += 1;
                    self.entries.insert(
                        field.clone(),
                        DictEntry {
                            pattern: field,
                            code,
                            frequency: *count,
                        },
                    );
                }
            }
        }
    }

    /// Compress a JSON string using the dictionary.
    ///
    /// Substitutes known field names and patterns with short codes.
    /// Prepends a dictionary header if substitutions were made.
    pub fn compress(&self, json_str: &str) -> Result<DictCompressResult> {
        if self.entries.is_empty() {
            return Ok(DictCompressResult {
                data: json_str.to_string(),
                substitutions: 0,
                bytes_saved: 0,
                has_dict_header: false,
            });
        }

        let mut result = json_str.to_string();
        let mut substitutions = 0usize;
        let mut bytes_saved = 0usize;
        let mut used_entries: Vec<&DictEntry> = Vec::new();

        // Sort entries by pattern length (longest first) to avoid partial matches
        let mut sorted_entries: Vec<&DictEntry> = self.entries.values().collect();
        sorted_entries.sort_by(|a, b| b.pattern.len().cmp(&a.pattern.len()));

        for entry in &sorted_entries {
            // Only substitute in JSON key positions: "field_name":
            let search = format!("\"{}\":", entry.pattern);
            let replace = format!("\"{}\":", entry.code);

            if result.contains(&search) {
                let count = result.matches(&search).count();
                result = result.replace(&search, &replace);
                substitutions += count;
                bytes_saved += count * (search.len() - replace.len());
                used_entries.push(entry);
            }
        }

        // Only add header if we actually made substitutions
        let has_dict_header = !used_entries.is_empty();
        if has_dict_header {
            let header = format_dict_header(&used_entries);
            result = format!("{header}\n{result}");
        }

        Ok(DictCompressResult {
            data: result,
            substitutions,
            bytes_saved,
            has_dict_header,
        })
    }

    /// Get the current dictionary size.
    pub fn dict_size(&self) -> usize {
        self.entries.len()
    }

    /// Reset the dictionary (e.g., on session reset).
    pub fn reset(&mut self) {
        self.entries.clear();
        self.candidates.clear();
        self.next_code = 0;
        self.load_builtins();
    }
}

impl Default for DictCompressor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Built-in dictionary ───────────────────────────────────────────────────────

/// Build the default dictionary of common JSON field names.
fn build_builtin_dictionary() -> Vec<DictEntry> {
    let common_fields = [
        // API response fields
        ("status_code", "~s"),
        ("message", "~m"),
        ("timestamp", "~ts"),
        ("created_at", "~ca"),
        ("updated_at", "~ua"),
        ("deleted_at", "~da"),
        ("description", "~dc"),
        ("metadata", "~md"),
        ("properties", "~pp"),
        ("attributes", "~at"),
        ("parameters", "~pm"),
        ("configuration", "~cf"),
        ("environment", "~ev"),
        ("dependencies", "~dp"),
        ("permissions", "~pr"),
        ("resources", "~rs"),
        ("namespace", "~ns"),
        ("annotations", "~an"),
        // Error fields
        ("error_code", "~ec"),
        ("error_message", "~em"),
        ("stack_trace", "~st"),
        ("exception", "~ex"),
        // Pagination
        ("page_size", "~ps"),
        ("next_token", "~nt"),
        ("total_count", "~tc"),
        // AWS/Cloud common
        ("resource_type", "~rt"),
        ("account_id", "~ai"),
        ("region", "~rg"),
    ];

    common_fields
        .iter()
        .enumerate()
        .map(|(_, &(pattern, code))| DictEntry {
            pattern: pattern.to_string(),
            code: code.to_string(),
            frequency: 100, // built-in entries have high base frequency
        })
        .collect()
}

/// Extract field names from a JSON string (simple regex-free parser).
fn extract_json_fields(json_str: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let bytes = json_str.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Look for "field_name":
        if bytes[i] == b'"' {
            let start = i + 1;
            i += 1;
            // Find closing quote
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1; // skip escaped char
                }
                i += 1;
            }
            if i < len {
                let end = i;
                i += 1;
                // Check if followed by ':'
                // Skip whitespace
                while i < len && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r') {
                    i += 1;
                }
                if i < len && bytes[i] == b':' {
                    if let Ok(field) = std::str::from_utf8(&bytes[start..end]) {
                        fields.push(field.to_string());
                    }
                }
            }
        } else {
            i += 1;
        }
    }

    fields
}

/// Format the dictionary header for compressed output.
fn format_dict_header(entries: &[&DictEntry]) -> String {
    let mut lines = vec!["§dict§".to_string()];
    for entry in entries {
        lines.push(format!("{}={}", entry.code, entry.pattern));
    }
    lines.push("§/dict§".to_string());
    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_has_builtin_entries() {
        let comp = DictCompressor::new();
        assert!(comp.dict_size() > 0);
    }

    #[test]
    fn test_compress_with_known_fields() {
        let comp = DictCompressor::new();
        let json = r#"{"status_code":200,"message":"ok","timestamp":"2024-01-01"}"#;
        let result = comp.compress(json).unwrap();
        assert!(result.substitutions > 0);
        assert!(result.bytes_saved > 0);
        assert!(result.has_dict_header);
        assert!(result.data.contains("§dict§"));
    }

    #[test]
    fn test_compress_no_known_fields() {
        let mut comp = DictCompressor::new();
        comp.entries.clear(); // Remove builtins for this test
        let json = r#"{"x":1,"y":2}"#;
        let result = comp.compress(json).unwrap();
        assert_eq!(result.substitutions, 0);
        assert_eq!(result.bytes_saved, 0);
        assert!(!result.has_dict_header);
        assert_eq!(result.data, json);
    }

    #[test]
    fn test_observe_learns_patterns() {
        let mut comp = DictCompressor::new();
        let initial_size = comp.dict_size();

        // Observe the same JSON twice to meet min_frequency
        let json = r#"{"custom_field_name":1,"another_long_field":2}"#;
        comp.observe(json);
        comp.observe(json);

        assert!(comp.dict_size() > initial_size);
    }

    #[test]
    fn test_observe_ignores_short_fields() {
        let mut comp = DictCompressor::new();
        let initial_size = comp.dict_size();
        let json = r#"{"id":1,"x":2}"#;
        comp.observe(json);
        comp.observe(json);
        // Short fields should not be added
        assert_eq!(comp.dict_size(), initial_size);
    }

    #[test]
    fn test_extract_json_fields() {
        let json = r#"{"name":"Alice","age":30,"nested":{"key":"val"}}"#;
        let fields = extract_json_fields(json);
        assert!(fields.contains(&"name".to_string()));
        assert!(fields.contains(&"age".to_string()));
        assert!(fields.contains(&"nested".to_string()));
        assert!(fields.contains(&"key".to_string()));
    }

    #[test]
    fn test_extract_json_fields_with_escaped_quotes() {
        let json = r#"{"field_with_\"quotes\"":"value","normal":1}"#;
        let fields = extract_json_fields(json);
        assert!(fields.contains(&"normal".to_string()));
    }

    #[test]
    fn test_reset_clears_learned_entries() {
        let mut comp = DictCompressor::new();
        let json = r#"{"custom_field_name":1}"#;
        comp.observe(json);
        comp.observe(json);
        let size_after_learn = comp.dict_size();

        comp.reset();
        // Should be back to just builtins
        assert!(comp.dict_size() <= size_after_learn);
        assert!(comp.candidates.is_empty());
    }

    #[test]
    fn test_dict_header_format() {
        let entry = DictEntry {
            pattern: "status_code".to_string(),
            code: "~s".to_string(),
            frequency: 10,
        };
        let entries = vec![&entry];
        let header = format_dict_header(&entries);
        assert!(header.starts_with("§dict§"));
        assert!(header.ends_with("§/dict§"));
        assert!(header.contains("~s=status_code"));
    }

    // ── Property tests ────────────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// Compression never increases the logical content size
        /// (excluding the dictionary header).
        #[test]
        fn prop_compression_does_not_increase_content(
            field_name in "[a-z_]{5,15}",
            value in "[a-z0-9]{1,20}",
            repeat in 2usize..=5usize,
        ) {
            let mut comp = DictCompressor::new();
            comp.entries.clear(); // Start fresh

            let entry = format!("\"{}\":\"{}\"", field_name, value);
            let json = format!("{{{}}}", std::iter::repeat(entry.as_str())
                .take(repeat)
                .collect::<Vec<_>>()
                .join(","));

            // Observe enough times to learn the pattern
            for _ in 0..3 {
                comp.observe(&json);
            }

            let result = comp.compress(&json).unwrap();

            // If substitutions were made, the content portion should be shorter
            if result.substitutions > 0 {
                prop_assert!(
                    result.bytes_saved > 0,
                    "substitutions made but no bytes saved"
                );
            }
        }
    }
}
