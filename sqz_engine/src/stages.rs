use crate::error::{Result, SqzError};
use crate::toon::ToonEncoder;
use crate::types::{Content, ContentType, StageConfig};

/// A single compression stage in the pipeline.
///
/// Each stage transforms `Content` in place according to its `StageConfig`.
/// Stages must check `config.enabled` and return early (no-op) when disabled.
pub trait CompressionStage: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> u32;
    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Helper: parse raw as JSON, apply a transform, serialize back
// ---------------------------------------------------------------------------

fn with_json<F>(content: &mut Content, f: F) -> Result<()>
where
    F: FnOnce(&mut serde_json::Value) -> Result<()>,
{
    if !ToonEncoder::is_json(&content.raw) {
        return Ok(());
    }
    let mut value: serde_json::Value = serde_json::from_str(&content.raw)?;
    f(&mut value)?;
    content.raw = serde_json::to_string(&value)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Stage 1: keep_fields
// ---------------------------------------------------------------------------

/// For JSON content, keep only the specified top-level fields; drop all others.
/// Config options: `fields` — array of field name strings.
/// Non-JSON content passes through unchanged.
pub struct KeepFieldsStage;

impl CompressionStage for KeepFieldsStage {
    fn name(&self) -> &str {
        "keep_fields"
    }

    fn priority(&self) -> u32 {
        10
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        let fields: Vec<String> = match config.options.get("fields") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| SqzError::Other(format!("keep_fields: invalid fields option: {e}")))?,
            None => return Ok(()),
        };
        if fields.is_empty() {
            return Ok(());
        }
        with_json(content, |value| {
            if let serde_json::Value::Object(map) = value {
                map.retain(|k, _| fields.contains(k));
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 2: strip_fields
// ---------------------------------------------------------------------------

/// For JSON content, remove specified fields by key name.
/// Supports dot-notation for nested fields (e.g. "metadata.internal_id").
/// Config options: `fields` — array of field path strings.
/// Non-JSON content passes through unchanged.
pub struct StripFieldsStage;

fn strip_field_path(value: &mut serde_json::Value, path: &[&str]) {
    if path.is_empty() {
        return;
    }
    if let serde_json::Value::Object(map) = value {
        if path.len() == 1 {
            map.remove(path[0]);
        } else {
            if let Some(child) = map.get_mut(path[0]) {
                strip_field_path(child, &path[1..]);
            }
        }
    }
}

impl CompressionStage for StripFieldsStage {
    fn name(&self) -> &str {
        "strip_fields"
    }

    fn priority(&self) -> u32 {
        20
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        let fields: Vec<String> = match config.options.get("fields") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| SqzError::Other(format!("strip_fields: invalid fields option: {e}")))?,
            None => return Ok(()),
        };
        if fields.is_empty() {
            return Ok(());
        }
        with_json(content, |value| {
            for field in &fields {
                let parts: Vec<&str> = field.split('.').collect();
                strip_field_path(value, &parts);
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 3: condense
// ---------------------------------------------------------------------------

/// For plain text / CLI output, collapse runs of repeated identical lines
/// down to at most `max_repeated_lines`.
/// Config options: `max_repeated_lines` (u32, default 3).
/// Non-plain-text content passes through unchanged.
pub struct CondenseStage;

impl CompressionStage for CondenseStage {
    fn name(&self) -> &str {
        "condense"
    }

    fn priority(&self) -> u32 {
        30
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        // Only apply to plain text and CLI output
        match &content.content_type {
            ContentType::PlainText | ContentType::CliOutput { .. } => {}
            _ => return Ok(()),
        }

        let max_repeated: u32 = config
            .options
            .get("max_repeated_lines")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(3);

        let mut result = Vec::new();
        let mut current_line: Option<&str> = None;
        let mut run_count: u32 = 0;

        for line in content.raw.lines() {
            match current_line {
                Some(prev) if prev == line => {
                    run_count += 1;
                    if run_count <= max_repeated {
                        result.push(line);
                    }
                }
                _ => {
                    current_line = Some(line);
                    run_count = 1;
                    result.push(line);
                }
            }
        }

        // Preserve trailing newline if original had one
        let trailing_newline = content.raw.ends_with('\n');
        content.raw = result.join("\n");
        if trailing_newline {
            content.raw.push('\n');
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Stage 4: strip_nulls
// ---------------------------------------------------------------------------

/// For JSON content, recursively remove all null-valued fields from objects.
/// Arrays keep their null elements.
/// Config options: `enabled` (bool).
pub struct StripNullsStage;

fn strip_nulls_recursive(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for v in map.values_mut() {
                strip_nulls_recursive(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                strip_nulls_recursive(item);
            }
        }
        _ => {}
    }
}

impl CompressionStage for StripNullsStage {
    fn name(&self) -> &str {
        "strip_nulls"
    }

    fn priority(&self) -> u32 {
        40
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        with_json(content, |value| {
            strip_nulls_recursive(value);
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 5: flatten
// ---------------------------------------------------------------------------

/// For JSON content, flatten nested objects up to `max_depth` levels using
/// dot-notation for flattened keys (e.g. `{"a":{"b":1}}` → `{"a.b":1}`).
/// Config options: `max_depth` (u32, default 3).
/// Non-JSON content passes through unchanged.
pub struct FlattenStage;

fn flatten_value(
    value: &serde_json::Value,
    prefix: &str,
    depth: u32,
    max_depth: u32,
    out: &mut serde_json::Map<String, serde_json::Value>,
) {
    if let serde_json::Value::Object(map) = value {
        if depth < max_depth {
            for (k, v) in map {
                let new_key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_value(v, &new_key, depth + 1, max_depth, out);
            }
            return;
        }
    }
    out.insert(prefix.to_owned(), value.clone());
}

impl CompressionStage for FlattenStage {
    fn name(&self) -> &str {
        "flatten"
    }

    fn priority(&self) -> u32 {
        50
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        let max_depth: u32 = config
            .options
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(3);

        with_json(content, |value| {
            if let serde_json::Value::Object(map) = value {
                let mut out = serde_json::Map::new();
                for (k, v) in map.iter() {
                    flatten_value(v, k, 1, max_depth, &mut out);
                }
                *map = out;
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 6: truncate_strings
// ---------------------------------------------------------------------------

/// For JSON content, truncate string values longer than `max_length` chars,
/// appending "..." to indicate truncation.
/// Config options: `max_length` (u32, default 500).
/// Non-JSON content passes through unchanged.
pub struct TruncateStringsStage;

fn truncate_strings_recursive(value: &mut serde_json::Value, max_length: usize) {
    match value {
        serde_json::Value::String(s) => {
            if s.chars().count() > max_length {
                let truncated: String = s.chars().take(max_length).collect();
                *s = format!("{truncated}...");
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                truncate_strings_recursive(v, max_length);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                truncate_strings_recursive(item, max_length);
            }
        }
        _ => {}
    }
}

impl CompressionStage for TruncateStringsStage {
    fn name(&self) -> &str {
        "truncate_strings"
    }

    fn priority(&self) -> u32 {
        60
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        let max_length: usize = config
            .options
            .get("max_length")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(500);

        with_json(content, |value| {
            truncate_strings_recursive(value, max_length);
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 7: collapse_arrays
// ---------------------------------------------------------------------------

/// For JSON content, if an array has more than `max_items` elements:
/// 1. First, try tabular encoding: if all elements are objects with the same
///    keys, encode as a header row + data rows for maximum compression.
/// 2. Otherwise, keep the first `max_items` and replace the rest with a
///    summary string element.
///
/// Config options:
///   - `max_items` (u32, default 5)
///   - `summary_template` (string, default "... and {remaining} more items")
/// Non-JSON content passes through unchanged.
pub struct CollapseArraysStage;

/// Check if all elements in an array are objects with the same set of keys.
/// Returns the shared keys in a stable order if uniform, None otherwise.
fn detect_uniform_array(arr: &[serde_json::Value]) -> Option<Vec<String>> {
    if arr.len() < 2 {
        return None;
    }

    let first_keys: Vec<String> = match &arr[0] {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return None;
            }
            map.keys().cloned().collect()
        }
        _ => return None,
    };

    // Check that every element is an object with exactly the same keys
    for item in &arr[1..] {
        match item {
            serde_json::Value::Object(map) => {
                if map.len() != first_keys.len() {
                    return None;
                }
                for key in &first_keys {
                    if !map.contains_key(key) {
                        return None;
                    }
                }
            }
            _ => return None,
        }
    }

    Some(first_keys)
}

/// Encode a uniform array of objects as a compact tabular string:
/// `[headers] | col1 | col2 | ... \n val1 | val2 | ...`
fn encode_tabular(arr: &[serde_json::Value], keys: &[String]) -> String {
    let mut lines = Vec::with_capacity(arr.len() + 1);

    // Header row
    lines.push(keys.join(" | "));

    // Data rows
    for item in arr {
        if let serde_json::Value::Object(map) = item {
            let row: Vec<String> = keys
                .iter()
                .map(|k| value_to_compact_string(map.get(k).unwrap_or(&serde_json::Value::Null)))
                .collect();
            lines.push(row.join(" | "));
        }
    }

    lines.join("\n")
}

/// Convert a JSON value to a compact single-line string for tabular display.
fn value_to_compact_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            if s.len() > 50 {
                format!("{}...", &s[..47])
            } else {
                s.clone()
            }
        }
        serde_json::Value::Array(a) => format!("[{} items]", a.len()),
        serde_json::Value::Object(m) => format!("{{{} keys}}", m.len()),
    }
}

fn collapse_arrays_recursive(
    value: &mut serde_json::Value,
    max_items: usize,
    summary_template: &str,
) {
    match value {
        serde_json::Value::Array(arr) => {
            // First recurse into existing items
            for item in arr.iter_mut() {
                collapse_arrays_recursive(item, max_items, summary_template);
            }

            // Try tabular encoding for uniform arrays
            if arr.len() > max_items {
                if let Some(keys) = detect_uniform_array(arr) {
                    let table = encode_tabular(arr, &keys);
                    let count = arr.len();
                    arr.clear();
                    arr.push(serde_json::Value::String(
                        format!("[table: {count} rows]\n{table}"),
                    ));
                    return;
                }

                // Fallback: simple truncation with summary
                let remaining = arr.len() - max_items;
                arr.truncate(max_items);
                let summary = summary_template.replace("{remaining}", &remaining.to_string());
                arr.push(serde_json::Value::String(summary));
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                collapse_arrays_recursive(v, max_items, summary_template);
            }
        }
        _ => {}
    }
}

impl CompressionStage for CollapseArraysStage {
    fn name(&self) -> &str {
        "collapse_arrays"
    }

    fn priority(&self) -> u32 {
        70
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        let max_items: usize = config
            .options
            .get("max_items")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(5);
        let summary_template = config
            .options
            .get("summary_template")
            .and_then(|v| v.as_str())
            .unwrap_or("... and {remaining} more items")
            .to_owned();

        with_json(content, |value| {
            collapse_arrays_recursive(value, max_items, &summary_template);
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 7a: word_abbreviate
// ---------------------------------------------------------------------------

/// For plain text / CLI output, replace common long words with standard
/// abbreviations (e.g. "implementation" → "impl", "configuration" → "config").
///
/// Only replaces whole words (not substrings) and only in prose-like content.
/// Config options: `enabled` (bool).
pub struct WordAbbreviateStage;

/// Built-in abbreviation table: (long_form, short_form).
/// Only includes unambiguous, widely-understood abbreviations.
const WORD_ABBREVIATIONS: &[(&str, &str)] = &[
    ("implementation", "impl"),
    ("implementations", "impls"),
    ("configuration", "config"),
    ("configurations", "configs"),
    ("authentication", "auth"),
    ("authorization", "authz"),
    ("application", "app"),
    ("applications", "apps"),
    ("environment", "env"),
    ("environments", "envs"),
    ("development", "dev"),
    ("production", "prod"),
    ("repository", "repo"),
    ("repositories", "repos"),
    ("dependency", "dep"),
    ("dependencies", "deps"),
    ("documentation", "docs"),
    ("information", "info"),
    ("directory", "dir"),
    ("directories", "dirs"),
    ("parameter", "param"),
    ("parameters", "params"),
    ("argument", "arg"),
    ("arguments", "args"),
    ("function", "fn"),
    ("functions", "fns"),
    ("reference", "ref"),
    ("references", "refs"),
    ("specification", "spec"),
    ("specifications", "specs"),
    ("temporary", "tmp"),
    ("administrator", "admin"),
    ("administrators", "admins"),
    ("database", "db"),
    ("databases", "dbs"),
    ("message", "msg"),
    ("messages", "msgs"),
    ("response", "resp"),
    ("request", "req"),
    ("requests", "reqs"),
    ("attribute", "attr"),
    ("attributes", "attrs"),
    ("expression", "expr"),
    ("expressions", "exprs"),
    ("operation", "op"),
    ("operations", "ops"),
    ("maximum", "max"),
    ("minimum", "min"),
    ("boolean", "bool"),
    ("integer", "int"),
    ("previous", "prev"),
    ("current", "curr"),
    ("original", "orig"),
    ("synchronize", "sync"),
    ("asynchronous", "async"),
    ("initialize", "init"),
    ("allocation", "alloc"),
    ("allocations", "allocs"),
    ("generation", "gen"),
    ("miscellaneous", "misc"),
    ("statistics", "stats"),
    ("connection", "conn"),
    ("connections", "conns"),
    ("transaction", "txn"),
    ("transactions", "txns"),
    ("management", "mgmt"),
    ("notification", "notif"),
    ("notifications", "notifs"),
    ("permission", "perm"),
    ("permissions", "perms"),
    ("distribution", "distro"),
    ("distributions", "distros"),
    ("architecture", "arch"),
    ("infrastructure", "infra"),
    ("kubernetes", "k8s"),
    ("namespace", "ns"),
    ("namespaces", "nses"),
    ("container", "ctr"),
    ("containers", "ctrs"),
    ("microservice", "svc"),
    ("microservices", "svcs"),
];

impl CompressionStage for WordAbbreviateStage {
    fn name(&self) -> &str {
        "word_abbreviate"
    }

    fn priority(&self) -> u32 {
        25 // After strip_fields (20), before condense (30)
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        // Only apply to plain text and CLI output
        match &content.content_type {
            ContentType::PlainText | ContentType::CliOutput { .. } => {}
            _ => return Ok(()),
        }

        let mut result = content.raw.clone();
        for &(long, short) in WORD_ABBREVIATIONS {
            // Replace whole words only (case-insensitive for the check,
            // but preserve surrounding context)
            result = replace_whole_word(&result, long, short);
        }

        content.raw = result;
        Ok(())
    }
}

/// Apply word abbreviation to a plain text string.
///
/// This is a convenience function for callers that want to abbreviate
/// outside the pipeline stage system (e.g. CLI proxy post-processing).
pub fn abbreviate_words(text: &str) -> String {
    let mut result = text.to_string();
    for &(long, short) in WORD_ABBREVIATIONS {
        result = replace_whole_word(&result, long, short);
    }
    result
}

/// Replace whole-word occurrences of `word` with `replacement`.
/// A "whole word" is bounded by non-alphanumeric characters or string edges.
fn replace_whole_word(text: &str, word: &str, replacement: &str) -> String {
    if text.is_empty() || word.is_empty() {
        return text.to_string();
    }

    let lower = text.to_lowercase();
    let word_lower = word.to_lowercase();
    let word_len = word.len();
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    let text_bytes = text.as_bytes();

    for (start, _) in lower.match_indices(&word_lower) {
        let end = start + word_len;

        // Check word boundary before
        let before_ok = start == 0
            || !text_bytes[start - 1].is_ascii_alphanumeric();
        // Check word boundary after
        let after_ok = end >= text.len()
            || !text_bytes[end].is_ascii_alphanumeric();

        if before_ok && after_ok {
            result.push_str(&text[last_end..start]);
            result.push_str(replacement);
            last_end = end;
        }
    }

    result.push_str(&text[last_end..]);
    result
}

// ---------------------------------------------------------------------------
// Stage 7b: git_diff_fold
// ---------------------------------------------------------------------------

/// For git diff output, fold consecutive unchanged context lines (lines
/// starting with a space) into a compact `[N unchanged lines]` marker.
/// This preserves all changed lines (+/-) and hunk headers (@@) while
/// dramatically reducing noise from context lines.
///
/// Config options:
///   - `max_context_lines` (u32, default 2) — keep this many context lines
///     before/after each changed block before folding the rest.
pub struct GitDiffFoldStage;

impl CompressionStage for GitDiffFoldStage {
    fn name(&self) -> &str {
        "git_diff_fold"
    }

    fn priority(&self) -> u32 {
        35
    }

    fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        // Only apply to plain text / CLI output that looks like a diff
        match &content.content_type {
            ContentType::PlainText | ContentType::CliOutput { .. } => {}
            _ => return Ok(()),
        }

        // Real diff detection: require strong structural signals, not just
        // lines starting with +/-. The old check (`contains("\n+") || contains("\n-")`)
        // false-positived on ls -l output (regular files start with -rw-),
        // Markdown bullet lists, CSV with negative numbers, etc.
        let looks_like_diff = content.raw.starts_with("diff --git ")
            || content.raw.starts_with("diff -")
            || content.raw.contains("\n@@ ")       // hunk header
            || content.raw.contains("\n--- a/")    // unified diff file header
            || content.raw.contains("\n+++ b/");   // unified diff file header

        if !looks_like_diff {
            return Ok(());
        }

        let max_ctx: usize = config
            .options
            .get("max_context_lines")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(2);

        let lines: Vec<&str> = content.raw.lines().collect();
        let n = lines.len();

        // Mark which lines are "changed" (added, removed, or hunk headers)
        let is_changed: Vec<bool> = lines
            .iter()
            .map(|l| {
                l.starts_with('+')
                    || l.starts_with('-')
                    || l.starts_with("@@")
                    || l.starts_with("diff ")
                    || l.starts_with("index ")
                    || l.starts_with("--- ")
                    || l.starts_with("+++ ")
            })
            .collect();

        // For each context line, determine if it's within max_ctx of a changed line
        let mut keep = vec![false; n];
        for i in 0..n {
            if is_changed[i] {
                keep[i] = true;
                // Keep max_ctx lines before
                for j in i.saturating_sub(max_ctx)..i {
                    keep[j] = true;
                }
                // Keep max_ctx lines after
                for j in (i + 1)..n.min(i + 1 + max_ctx) {
                    keep[j] = true;
                }
            }
        }

        // Build output, folding consecutive non-kept lines
        let mut result = Vec::new();
        let mut fold_count = 0usize;

        for i in 0..n {
            if keep[i] {
                if fold_count > 0 {
                    result.push(format!("[{fold_count} unchanged lines]"));
                    fold_count = 0;
                }
                result.push(lines[i].to_owned());
            } else {
                fold_count += 1;
            }
        }
        if fold_count > 0 {
            result.push(format!("[{fold_count} unchanged lines]"));
        }

        let trailing_newline = content.raw.ends_with('\n');
        content.raw = result.join("\n");
        if trailing_newline {
            content.raw.push('\n');
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Stage 8: custom_transforms
// ---------------------------------------------------------------------------

/// No-op stage that serves as the insertion point for plugin stages.
/// Passes content through unchanged.
pub struct CustomTransformsStage;

impl CompressionStage for CustomTransformsStage {
    fn name(&self) -> &str {
        "custom_transforms"
    }

    fn priority(&self) -> u32 {
        80
    }

    fn process(&self, _content: &mut Content, config: &StageConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        // No-op: plugin stages are inserted here by the pipeline
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentMetadata, ContentType};
    use serde_json::json;

    fn json_content(raw: &str) -> Content {
        Content {
            raw: raw.to_owned(),
            content_type: ContentType::Json,
            metadata: ContentMetadata {
                source: None,
                path: None,
                language: None,
            },
            tokens_original: 0,
        }
    }

    fn text_content(raw: &str) -> Content {
        Content {
            raw: raw.to_owned(),
            content_type: ContentType::PlainText,
            metadata: ContentMetadata {
                source: None,
                path: None,
                language: None,
            },
            tokens_original: 0,
        }
    }

    fn enabled_config(options: serde_json::Value) -> StageConfig {
        StageConfig {
            enabled: true,
            options,
        }
    }

    fn disabled_config() -> StageConfig {
        StageConfig {
            enabled: false,
            options: json!({}),
        }
    }

    // --- keep_fields ---

    #[test]
    fn keep_fields_retains_specified() {
        let mut c = json_content(r#"{"id":1,"name":"Alice","debug":"x"}"#);
        let cfg = enabled_config(json!({"fields": ["id", "name"]}));
        KeepFieldsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v, json!({"id":1,"name":"Alice"}));
    }

    #[test]
    fn keep_fields_disabled_passthrough() {
        let raw = r#"{"id":1,"name":"Alice"}"#;
        let mut c = json_content(raw);
        KeepFieldsStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    #[test]
    fn keep_fields_non_json_passthrough() {
        let raw = "not json at all";
        let mut c = text_content(raw);
        let cfg = enabled_config(json!({"fields": ["id"]}));
        KeepFieldsStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- strip_fields ---

    #[test]
    fn strip_fields_removes_top_level() {
        let mut c = json_content(r#"{"id":1,"debug":"x","name":"Bob"}"#);
        let cfg = enabled_config(json!({"fields": ["debug"]}));
        StripFieldsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v, json!({"id":1,"name":"Bob"}));
    }

    #[test]
    fn strip_fields_dot_notation() {
        let mut c = json_content(r#"{"metadata":{"internal_id":"x","public":"y"},"name":"Bob"}"#);
        let cfg = enabled_config(json!({"fields": ["metadata.internal_id"]}));
        StripFieldsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v, json!({"metadata":{"public":"y"},"name":"Bob"}));
    }

    #[test]
    fn strip_fields_disabled_passthrough() {
        let raw = r#"{"id":1}"#;
        let mut c = json_content(raw);
        StripFieldsStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- condense ---

    #[test]
    fn condense_collapses_repeated_lines() {
        let raw = "a\na\na\na\na\nb\n";
        let mut c = text_content(raw);
        let cfg = enabled_config(json!({"max_repeated_lines": 3}));
        CondenseStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, "a\na\na\nb\n");
    }

    #[test]
    fn condense_keeps_up_to_max() {
        let raw = "x\nx\nx\n";
        let mut c = text_content(raw);
        let cfg = enabled_config(json!({"max_repeated_lines": 3}));
        CondenseStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, "x\nx\nx\n");
    }

    #[test]
    fn condense_disabled_passthrough() {
        let raw = "a\na\na\na\n";
        let mut c = text_content(raw);
        CondenseStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    #[test]
    fn condense_skips_json() {
        let raw = r#"{"a":1}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({"max_repeated_lines": 1}));
        CondenseStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- strip_nulls ---

    #[test]
    fn strip_nulls_removes_null_fields() {
        let mut c = json_content(r#"{"a":1,"b":null,"c":"x"}"#);
        let cfg = enabled_config(json!({}));
        StripNullsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v, json!({"a":1,"c":"x"}));
    }

    #[test]
    fn strip_nulls_recursive() {
        let mut c = json_content(r#"{"a":{"b":null,"c":1}}"#);
        let cfg = enabled_config(json!({}));
        StripNullsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v, json!({"a":{"c":1}}));
    }

    #[test]
    fn strip_nulls_keeps_null_in_arrays() {
        let mut c = json_content(r#"{"arr":[1,null,2]}"#);
        let cfg = enabled_config(json!({}));
        StripNullsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v, json!({"arr":[1,null,2]}));
    }

    #[test]
    fn strip_nulls_disabled_passthrough() {
        let raw = r#"{"a":null}"#;
        let mut c = json_content(raw);
        StripNullsStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- flatten ---

    #[test]
    fn flatten_nested_object() {
        let mut c = json_content(r#"{"a":{"b":{"c":1}}}"#);
        let cfg = enabled_config(json!({"max_depth": 3}));
        FlattenStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v, json!({"a.b.c":1}));
    }

    #[test]
    fn flatten_respects_max_depth() {
        let mut c = json_content(r#"{"a":{"b":{"c":1}}}"#);
        let cfg = enabled_config(json!({"max_depth": 1}));
        FlattenStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        // At max_depth=1, top-level values are not descended into
        assert_eq!(v, json!({"a":{"b":{"c":1}}}));
    }

    #[test]
    fn flatten_disabled_passthrough() {
        let raw = r#"{"a":{"b":1}}"#;
        let mut c = json_content(raw);
        FlattenStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- truncate_strings ---

    #[test]
    fn truncate_strings_long_value() {
        let long = "a".repeat(600);
        let raw = format!(r#"{{"key":"{}"}}"#, long);
        let mut c = json_content(&raw);
        let cfg = enabled_config(json!({"max_length": 500}));
        TruncateStringsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        let s = v["key"].as_str().unwrap();
        assert!(s.ends_with("..."));
        assert_eq!(s.chars().count(), 503); // 500 + "..."
    }

    #[test]
    fn truncate_strings_short_value_unchanged() {
        let raw = r#"{"key":"hello"}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({"max_length": 500}));
        TruncateStringsStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v["key"].as_str().unwrap(), "hello");
    }

    #[test]
    fn truncate_strings_disabled_passthrough() {
        let long = "a".repeat(600);
        let raw = format!(r#"{{"key":"{}"}}"#, long);
        let mut c = json_content(&raw);
        TruncateStringsStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- collapse_arrays ---

    #[test]
    fn collapse_arrays_truncates_long_array() {
        let mut c = json_content(r#"{"items":[1,2,3,4,5,6,7]}"#);
        let cfg = enabled_config(json!({
            "max_items": 5,
            "summary_template": "... and {remaining} more items"
        }));
        CollapseArraysStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        let arr = v["items"].as_array().unwrap();
        assert_eq!(arr.len(), 6); // 5 kept + 1 summary
        assert_eq!(arr[5].as_str().unwrap(), "... and 2 more items");
    }

    #[test]
    fn collapse_arrays_short_array_unchanged() {
        let raw = r#"{"items":[1,2,3]}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({"max_items": 5}));
        CollapseArraysStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn collapse_arrays_disabled_passthrough() {
        let raw = r#"{"items":[1,2,3,4,5,6,7]}"#;
        let mut c = json_content(raw);
        CollapseArraysStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- git_diff_fold ---

    #[test]
    fn git_diff_fold_folds_unchanged_lines() {
        // Use a realistic diff with many unchanged context lines
        let diff = concat!(
            "diff --git a/src/main.rs b/src/main.rs\n",
            "--- a/src/main.rs\n",
            "+++ b/src/main.rs\n",
            "@@ -1,12 +1,12 @@\n",
            " line1\n",
            " line2\n",
            " line3\n",
            " line4\n",
            " line5\n",
            " line6\n",
            "-old line\n",
            "+new line\n",
            " line7\n",
            " line8\n",
            " line9\n",
            " line10\n",
            " line11\n",
            " line12\n",
        );
        let mut c = text_content(diff);
        let cfg = enabled_config(serde_json::json!({"max_context_lines": 2}));
        GitDiffFoldStage.process(&mut c, &cfg).unwrap();
        // Changed lines must be preserved
        assert!(c.raw.contains("-old line"), "output: {}", c.raw);
        assert!(c.raw.contains("+new line"), "output: {}", c.raw);
        // Hunk header must be preserved
        assert!(c.raw.contains("@@ -1,12"), "output: {}", c.raw);
        // Output should be shorter (folded lines 1-4 and 9-12)
        assert!(c.raw.len() < diff.len(), "output should be shorter, got:\n{}", c.raw);
        // Should contain fold markers
        assert!(c.raw.contains("unchanged lines"), "expected fold markers in:\n{}", c.raw);
    }

    #[test]
    fn git_diff_fold_preserves_hunk_headers() {
        let diff = "@@ -1,5 +1,5 @@\n unchanged\n-old\n+new\n unchanged\n";
        let mut c = text_content(diff);
        let cfg = enabled_config(serde_json::json!({"max_context_lines": 1}));
        GitDiffFoldStage.process(&mut c, &cfg).unwrap();
        assert!(c.raw.contains("@@ -1,5 +1,5 @@"), "output: {}", c.raw);
    }

    #[test]
    fn git_diff_fold_skips_non_diff_text() {
        let raw = "just some plain text\nno diff markers here\n";
        let mut c = text_content(raw);
        let cfg = enabled_config(serde_json::json!({"max_context_lines": 2}));
        GitDiffFoldStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, raw);
    }

    #[test]
    fn git_diff_fold_disabled_passthrough() {
        let diff = "diff --git a/f b/f\n-old\n+new\n unchanged\n unchanged\n unchanged\n";
        let mut c = text_content(diff);
        GitDiffFoldStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, diff);
    }

    // --- git_diff_fold false-positive regression tests ---
    // https://github.com/ojuschugh1/sqz/issues/1 (Reddit report)
    //
    // ls -l output contains lines starting with - (regular file permissions:
    // -rw-r--r--) which the old diff detector treated as diff deletions.
    // Directory entries were silently dropped from the model's view.

    #[test]
    fn git_diff_fold_does_not_fold_ls_output() {
        let ls_output = concat!(
            "total 24\n",
            "drwxr-xr-x  6 user user  192 Apr 18 10:00 packages\n",
            "drwxr-xr-x  3 user user   96 Apr 18 10:00 configuration\n",
            "drwxr-xr-x  4 user user  128 Apr 18 10:00 documentation\n",
            "drwxr-xr-x  2 user user   64 Apr 18 10:00 environment\n",
            "-rw-r--r--  1 user user 1024 Apr 18 10:00 README.md\n",
        );
        let mut c = text_content(ls_output);
        let cfg = enabled_config(serde_json::json!({"max_context_lines": 2}));
        GitDiffFoldStage.process(&mut c, &cfg).unwrap();
        // ALL directory entries must be preserved — none should be folded
        assert!(c.raw.contains("packages"), "packages must survive: {}", c.raw);
        assert!(c.raw.contains("configuration"), "configuration must survive: {}", c.raw);
        assert!(c.raw.contains("documentation"), "documentation must survive: {}", c.raw);
        assert!(c.raw.contains("environment"), "environment must survive: {}", c.raw);
        assert!(c.raw.contains("README.md"), "README.md must survive: {}", c.raw);
        assert!(!c.raw.contains("unchanged lines"), "no folding should occur: {}", c.raw);
    }

    #[test]
    fn git_diff_fold_does_not_fold_markdown_bullets() {
        let markdown = concat!(
            "# Features\n",
            "\n",
            "- First feature\n",
            "- Second feature\n",
            "- Third feature\n",
            "+ Added bonus\n",
            "\n",
            "## Details\n",
        );
        let mut c = text_content(markdown);
        let cfg = enabled_config(serde_json::json!({"max_context_lines": 2}));
        GitDiffFoldStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, markdown, "markdown should pass through unchanged");
    }

    #[test]
    fn git_diff_fold_still_works_on_real_diffs() {
        // Verify the fix didn't break actual diff folding
        let diff = concat!(
            "diff --git a/src/main.rs b/src/main.rs\n",
            "--- a/src/main.rs\n",
            "+++ b/src/main.rs\n",
            "@@ -1,10 +1,10 @@\n",
            " line1\n",
            " line2\n",
            " line3\n",
            " line4\n",
            " line5\n",
            "-old line\n",
            "+new line\n",
            " line6\n",
            " line7\n",
            " line8\n",
            " line9\n",
            " line10\n",
        );
        let mut c = text_content(diff);
        let cfg = enabled_config(serde_json::json!({"max_context_lines": 2}));
        GitDiffFoldStage.process(&mut c, &cfg).unwrap();
        // Changed lines must be preserved
        assert!(c.raw.contains("-old line"), "removed line preserved: {}", c.raw);
        assert!(c.raw.contains("+new line"), "added line preserved: {}", c.raw);
        // Should fold some context lines
        assert!(c.raw.contains("unchanged lines"), "should fold context: {}", c.raw);
        // Output should have fewer lines (fold markers replace multiple lines)
        assert!(
            c.raw.lines().count() < diff.lines().count(),
            "output should have fewer lines: {} vs {}",
            c.raw.lines().count(), diff.lines().count()
        );
    }

    // --- custom_transforms ---

    #[test]
    fn custom_transforms_is_noop() {
        let raw = r#"{"a":1}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({}));
        CustomTransformsStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, raw);
    }

    #[test]
    fn custom_transforms_disabled_passthrough() {
        let raw = "some text";
        let mut c = text_content(raw);
        CustomTransformsStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    // --- tabular encoding (in collapse_arrays) ---

    #[test]
    fn collapse_arrays_tabular_encoding_uniform_objects() {
        // Array of objects with identical keys → should produce tabular output
        let raw = r#"{"users":[
            {"id":1,"name":"Alice","role":"admin"},
            {"id":2,"name":"Bob","role":"user"},
            {"id":3,"name":"Carol","role":"user"},
            {"id":4,"name":"Dave","role":"admin"},
            {"id":5,"name":"Eve","role":"user"},
            {"id":6,"name":"Frank","role":"user"}
        ]}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({
            "max_items": 3,
            "summary_template": "... and {remaining} more items"
        }));
        CollapseArraysStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        let arr = v["users"].as_array().unwrap();
        // Should be collapsed to a single tabular string element
        assert_eq!(arr.len(), 1, "uniform array should be encoded as single table element");
        let table_str = arr[0].as_str().unwrap();
        assert!(table_str.contains("[table: 6 rows]"), "should contain row count: {}", table_str);
        assert!(table_str.contains("Alice"), "should contain data: {}", table_str);
        assert!(table_str.contains("Frank"), "should contain all rows: {}", table_str);
    }

    #[test]
    fn collapse_arrays_mixed_objects_falls_back_to_truncation() {
        // Array of objects with DIFFERENT keys → should fall back to truncation
        let raw = r#"{"items":[
            {"id":1,"name":"Alice"},
            {"x":2,"y":3},
            {"id":3,"name":"Carol"},
            {"x":4,"y":5},
            {"id":5,"name":"Eve"},
            {"x":6,"y":7}
        ]}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({
            "max_items": 3,
            "summary_template": "... and {remaining} more items"
        }));
        CollapseArraysStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        let arr = v["items"].as_array().unwrap();
        // Should fall back to truncation: 3 kept + 1 summary
        assert_eq!(arr.len(), 4);
        assert!(arr[3].as_str().unwrap().contains("3 more items"));
    }

    #[test]
    fn collapse_arrays_small_uniform_array_unchanged() {
        // Uniform array but under max_items → no collapse
        let raw = r#"{"users":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({"max_items": 5}));
        CollapseArraysStage.process(&mut c, &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.raw).unwrap();
        assert_eq!(v["users"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn detect_uniform_array_returns_keys_for_uniform() {
        let arr = vec![
            json!({"a": 1, "b": 2}),
            json!({"a": 3, "b": 4}),
        ];
        let keys = detect_uniform_array(&arr);
        assert!(keys.is_some());
        let keys = keys.unwrap();
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b".to_string()));
    }

    #[test]
    fn detect_uniform_array_returns_none_for_mixed() {
        let arr = vec![
            json!({"a": 1, "b": 2}),
            json!({"x": 3, "y": 4}),
        ];
        assert!(detect_uniform_array(&arr).is_none());
    }

    #[test]
    fn detect_uniform_array_returns_none_for_non_objects() {
        let arr = vec![json!(1), json!(2), json!(3)];
        assert!(detect_uniform_array(&arr).is_none());
    }

    #[test]
    fn detect_uniform_array_returns_none_for_single_element() {
        let arr = vec![json!({"a": 1})];
        assert!(detect_uniform_array(&arr).is_none());
    }

    #[test]
    fn value_to_compact_string_truncates_long_strings() {
        let long = "a".repeat(100);
        let v = serde_json::Value::String(long);
        let s = value_to_compact_string(&v);
        assert!(s.len() <= 53); // 47 chars + "..."
        assert!(s.ends_with("..."));
    }

    #[test]
    fn value_to_compact_string_short_string_unchanged() {
        let v = serde_json::Value::String("hello".to_string());
        assert_eq!(value_to_compact_string(&v), "hello");
    }

    #[test]
    fn value_to_compact_string_nested_types() {
        assert_eq!(value_to_compact_string(&json!(null)), "null");
        assert_eq!(value_to_compact_string(&json!(true)), "true");
        assert_eq!(value_to_compact_string(&json!(42)), "42");
        assert_eq!(value_to_compact_string(&json!([1, 2, 3])), "[3 items]");
        assert_eq!(value_to_compact_string(&json!({"a": 1})), "{1 keys}");
    }

    // --- word_abbreviate ---

    #[test]
    fn word_abbreviate_replaces_known_words() {
        let raw = "The implementation of the configuration is complete.";
        let mut c = text_content(raw);
        let cfg = enabled_config(json!({}));
        WordAbbreviateStage.process(&mut c, &cfg).unwrap();
        assert!(c.raw.contains("impl"), "should abbreviate 'implementation': {}", c.raw);
        assert!(c.raw.contains("config"), "should abbreviate 'configuration': {}", c.raw);
        assert!(!c.raw.contains("implementation"), "original word should be gone: {}", c.raw);
    }

    #[test]
    fn word_abbreviate_preserves_partial_matches() {
        // "implement" should NOT be abbreviated — only "implementation" is in the table
        let raw = "We need to implement this feature.";
        let mut c = text_content(raw);
        let cfg = enabled_config(json!({}));
        WordAbbreviateStage.process(&mut c, &cfg).unwrap();
        assert!(c.raw.contains("implement"), "partial match should be preserved: {}", c.raw);
    }

    #[test]
    fn word_abbreviate_disabled_passthrough() {
        let raw = "The implementation is complete.";
        let mut c = text_content(raw);
        WordAbbreviateStage.process(&mut c, &disabled_config()).unwrap();
        assert_eq!(c.raw, raw);
    }

    #[test]
    fn word_abbreviate_skips_json() {
        let raw = r#"{"implementation":"value"}"#;
        let mut c = json_content(raw);
        let cfg = enabled_config(json!({}));
        WordAbbreviateStage.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, raw, "JSON content should pass through unchanged");
    }

    #[test]
    fn word_abbreviate_case_insensitive() {
        let raw = "The Implementation and CONFIGURATION are ready.";
        let mut c = text_content(raw);
        let cfg = enabled_config(json!({}));
        WordAbbreviateStage.process(&mut c, &cfg).unwrap();
        assert!(c.raw.contains("impl"), "should handle mixed case: {}", c.raw);
        assert!(c.raw.contains("config"), "should handle uppercase: {}", c.raw);
    }

    #[test]
    fn replace_whole_word_basic() {
        assert_eq!(
            replace_whole_word("the implementation is done", "implementation", "impl"),
            "the impl is done"
        );
    }

    #[test]
    fn replace_whole_word_no_partial() {
        // "implementations" contains "implementation" but shouldn't match
        // because the 's' after makes it not a word boundary
        let result = replace_whole_word("multiple implementations exist", "implementation", "impl");
        // The word "implementations" has "implementation" followed by 's' which is alphanumeric,
        // so it should NOT be replaced
        assert_eq!(result, "multiple implementations exist");
    }

    #[test]
    fn replace_whole_word_at_boundaries() {
        assert_eq!(
            replace_whole_word("implementation", "implementation", "impl"),
            "impl"
        );
        assert_eq!(
            replace_whole_word("(implementation)", "implementation", "impl"),
            "(impl)"
        );
    }

    #[test]
    fn replace_whole_word_empty_inputs() {
        assert_eq!(replace_whole_word("", "word", "w"), "");
        assert_eq!(replace_whole_word("text", "", "w"), "text");
    }
}
