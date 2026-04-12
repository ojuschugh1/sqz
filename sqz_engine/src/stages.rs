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

/// For JSON content, if an array has more than `max_items` elements, keep the
/// first `max_items` and replace the rest with a summary string element.
/// Config options:
///   - `max_items` (u32, default 5)
///   - `summary_template` (string, default "... and {remaining} more items")
/// Non-JSON content passes through unchanged.
pub struct CollapseArraysStage;

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
            // Then collapse if needed
            if arr.len() > max_items {
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
        // Quick check: must contain diff markers
        if !content.raw.contains("\n+") && !content.raw.contains("\n-") {
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
}
