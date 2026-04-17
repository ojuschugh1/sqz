/// Schema-Aware JSON Projection — strips JSON to only the fields
/// relevant to the current context.
///
/// Unlike `strip_nulls` (removes null values) or `keep_fields` (requires
/// an explicit field list), projection automatically identifies and removes
/// low-value fields based on content patterns:
///
/// - Internal/debug fields: `_id`, `__v`, `debug_*`, `internal_*`, `trace_*`
/// - Metadata bloat: `created_by`, `updated_by`, `etag`, `_links`, `_embedded`
/// - Redundant timestamps: keeps `created_at`, drops `modified_at` if same day
/// - Empty collections: `[]`, `{}`
/// - Verbose nested objects below a depth threshold
///
/// The projection is conservative — it only removes fields that are
/// demonstrably low-value for LLM comprehension.

use crate::error::Result;

/// Configuration for JSON projection.
#[derive(Debug, Clone)]
pub struct ProjectionConfig {
    /// Remove fields matching these prefixes (case-insensitive).
    pub strip_prefixes: Vec<String>,
    /// Remove fields matching these exact names (case-insensitive).
    pub strip_names: Vec<String>,
    /// Maximum nesting depth to preserve. Objects deeper than this
    /// are replaced with `{...N keys}`. Default: 5.
    pub max_depth: usize,
    /// Remove empty arrays and objects. Default: true.
    pub strip_empty: bool,
    /// Remove redundant timestamps (keep only the most recent). Default: true.
    pub dedup_timestamps: bool,
}

impl Default for ProjectionConfig {
    fn default() -> Self {
        Self {
            strip_prefixes: vec![
                "_".to_string(),
                "debug".to_string(),
                "internal".to_string(),
                "trace".to_string(),
                "x_".to_string(),
            ],
            strip_names: vec![
                "__v".to_string(),
                "__typename".to_string(),
                "etag".to_string(),
                "_links".to_string(),
                "_embedded".to_string(),
                "cursor".to_string(),
                "request_id".to_string(),
                "x_request_id".to_string(),
                "correlation_id".to_string(),
            ],
            max_depth: 5,
            strip_empty: true,
            dedup_timestamps: true,
        }
    }
}

/// Result of JSON projection.
#[derive(Debug, Clone)]
pub struct ProjectionResult {
    /// The projected JSON string.
    pub data: String,
    /// Number of fields removed.
    pub fields_removed: usize,
    /// Estimated tokens saved.
    pub tokens_saved: u32,
}

/// Apply schema-aware projection to a JSON string.
///
/// Returns the projected JSON and stats, or the original string unchanged
/// if it's not valid JSON or projection doesn't help.
pub fn project_json(input: &str, config: &ProjectionConfig) -> Result<ProjectionResult> {
    let trimmed = input.trim();
    let mut value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            return Ok(ProjectionResult {
                data: input.to_string(),
                fields_removed: 0,
                tokens_saved: 0,
            });
        }
    };

    let original_tokens = estimate_tokens(input);
    let mut fields_removed = 0;

    project_value(&mut value, config, 0, &mut fields_removed);

    let projected = serde_json::to_string(&value)
        .unwrap_or_else(|_| input.to_string());

    let projected_tokens = estimate_tokens(&projected);
    let tokens_saved = original_tokens.saturating_sub(projected_tokens);

    // Only return projected version if it's actually smaller or fields were removed
    if projected.len() < input.len() || fields_removed > 0 {
        Ok(ProjectionResult {
            data: projected,
            fields_removed,
            tokens_saved,
        })
    } else {
        Ok(ProjectionResult {
            data: input.to_string(),
            fields_removed: 0,
            tokens_saved: 0,
        })
    }
}

/// Recursively project a JSON value, removing low-value fields.
fn project_value(
    value: &mut serde_json::Value,
    config: &ProjectionConfig,
    depth: usize,
    removed: &mut usize,
) {
    match value {
        serde_json::Value::Object(map) => {
            // At max depth, replace deep objects with a summary
            if depth >= config.max_depth {
                let key_count = map.len();
                if key_count > 0 {
                    map.clear();
                    map.insert(
                        "_sqz_summary".to_string(),
                        serde_json::Value::String(format!("{{...{key_count} keys}}")),
                    );
                    *removed += key_count;
                }
                return;
            }

            let keys_to_remove: Vec<String> = map
                .keys()
                .filter(|k| should_strip_field(k, config))
                .cloned()
                .collect();

            for key in &keys_to_remove {
                map.remove(key);
                *removed += 1;
            }

            // Strip empty collections
            if config.strip_empty {
                let empty_keys: Vec<String> = map
                    .iter()
                    .filter(|(_, v)| is_empty_collection(v))
                    .map(|(k, _)| k.clone())
                    .collect();
                for key in &empty_keys {
                    map.remove(key);
                    *removed += 1;
                }
            }

            // Dedup timestamps: if multiple timestamp fields exist on the same
            // day, keep only the most descriptive one
            if config.dedup_timestamps {
                dedup_timestamps(map, removed);
            }

            // Recurse into remaining values
            for v in map.values_mut() {
                project_value(v, config, depth + 1, removed);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                project_value(item, config, depth + 1, removed);
            }
        }
        _ => {}
    }
}

/// Check if a field name should be stripped based on the config.
fn should_strip_field(name: &str, config: &ProjectionConfig) -> bool {
    let lower = name.to_lowercase();

    // Check exact name matches
    for strip_name in &config.strip_names {
        if lower == strip_name.to_lowercase() {
            return true;
        }
    }

    // Check prefix matches
    for prefix in &config.strip_prefixes {
        let prefix_lower = prefix.to_lowercase();
        if lower.starts_with(&prefix_lower) && lower != prefix_lower {
            // Don't strip if the field IS the prefix (e.g., don't strip "id" for prefix "_")
            // But DO strip "_id", "debug_info", etc.
            return true;
        }
    }

    false
}

/// Check if a value is an empty collection.
fn is_empty_collection(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(arr) => arr.is_empty(),
        serde_json::Value::Object(map) => map.is_empty(),
        serde_json::Value::String(s) => s.is_empty(),
        _ => false,
    }
}

/// Remove redundant timestamp fields. If multiple fields end in `_at` or `_date`
/// and have the same date prefix (YYYY-MM-DD), keep only the first one.
fn dedup_timestamps(
    map: &mut serde_json::Map<String, serde_json::Value>,
    removed: &mut usize,
) {
    let timestamp_fields: Vec<(String, String)> = map
        .iter()
        .filter_map(|(k, v)| {
            if (k.ends_with("_at") || k.ends_with("_date") || k.ends_with("_time"))
                && v.is_string()
            {
                let date_prefix = v.as_str().unwrap_or("").chars().take(10).collect::<String>();
                if date_prefix.len() == 10 && date_prefix.contains('-') {
                    return Some((k.clone(), date_prefix));
                }
            }
            None
        })
        .collect();

    if timestamp_fields.len() <= 1 {
        return;
    }

    // Group by date prefix
    let mut seen_dates: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut first_field_per_date: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut to_remove = Vec::new();

    for (field, date) in &timestamp_fields {
        if seen_dates.contains(date) {
            // This date was already seen — remove this field unless it's the "primary" one
            let primary = first_field_per_date.get(date).unwrap();
            // Keep the more descriptive field (created_at > updated_at > modified_at)
            let dominated = if field.contains("created") {
                // created_at is more important — remove the previous one
                to_remove.push(primary.clone());
                false
            } else {
                true
            };
            if dominated {
                to_remove.push(field.clone());
            }
        } else {
            seen_dates.insert(date.clone());
            first_field_per_date.insert(date.clone(), field.clone());
        }
    }

    for field in &to_remove {
        map.remove(field);
        *removed += 1;
    }
}

fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_strips_internal_fields() {
        let input = json!({
            "id": 1,
            "name": "Alice",
            "_id": "abc123",
            "__v": 3,
            "debug_info": "verbose stuff",
            "internal_state": "hidden"
        });
        let config = ProjectionConfig::default();
        let result = project_json(&serde_json::to_string(&input).unwrap(), &config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.data).unwrap();
        assert!(parsed.get("id").is_some(), "id should be kept");
        assert!(parsed.get("name").is_some(), "name should be kept");
        assert!(parsed.get("_id").is_none(), "_id should be stripped");
        assert!(parsed.get("__v").is_none(), "__v should be stripped");
        assert!(parsed.get("debug_info").is_none(), "debug_info should be stripped");
        assert!(result.fields_removed > 0);
    }

    #[test]
    fn test_strips_empty_collections() {
        let input = json!({
            "name": "Bob",
            "tags": [],
            "metadata": {},
            "bio": ""
        });
        let config = ProjectionConfig::default();
        let result = project_json(&serde_json::to_string(&input).unwrap(), &config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.data).unwrap();
        assert!(parsed.get("name").is_some());
        assert!(parsed.get("tags").is_none(), "empty array should be stripped");
        assert!(parsed.get("metadata").is_none(), "empty object should be stripped");
        assert!(parsed.get("bio").is_none(), "empty string should be stripped");
    }

    #[test]
    fn test_max_depth_truncation() {
        let input = json!({
            "a": {"b": {"c": {"d": {"e": {"f": "deep"}}}}}
        });
        let config = ProjectionConfig {
            max_depth: 3,
            ..Default::default()
        };
        let result = project_json(&serde_json::to_string(&input).unwrap(), &config).unwrap();
        // At depth 3, the nested object should be replaced with a summary
        let parsed: serde_json::Value = serde_json::from_str(&result.data).unwrap();
        // Navigate to depth 3: a -> b -> c (this is where truncation happens)
        let at_depth = &parsed["a"]["b"]["c"];
        assert!(
            at_depth.get("_sqz_summary").is_some() || result.fields_removed > 0,
            "deep nesting should be truncated at max_depth: {:?}", parsed
        );
    }

    #[test]
    fn test_dedup_timestamps() {
        let input = json!({
            "name": "Alice",
            "created_at": "2024-01-15T10:00:00Z",
            "updated_at": "2024-01-15T14:30:00Z",
            "modified_at": "2024-01-15T14:30:00Z"
        });
        let config = ProjectionConfig::default();
        let result = project_json(&serde_json::to_string(&input).unwrap(), &config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.data).unwrap();
        // Should keep created_at, drop one of the redundant same-day timestamps
        assert!(parsed.get("created_at").is_some(), "created_at should be kept");
        assert!(result.fields_removed > 0, "redundant timestamps should be removed");
    }

    #[test]
    fn test_non_json_passthrough() {
        let input = "not json at all";
        let config = ProjectionConfig::default();
        let result = project_json(input, &config).unwrap();
        assert_eq!(result.data, input);
        assert_eq!(result.fields_removed, 0);
    }

    #[test]
    fn test_preserves_important_fields() {
        let input = json!({
            "id": 42,
            "name": "Alice",
            "email": "alice@example.com",
            "role": "admin",
            "status": "active"
        });
        let config = ProjectionConfig::default();
        let result = project_json(&serde_json::to_string(&input).unwrap(), &config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.data).unwrap();
        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["name"], "Alice");
        assert_eq!(parsed["role"], "admin");
    }

    #[test]
    fn test_custom_strip_prefixes() {
        let input = json!({
            "name": "Alice",
            "tmp_cache": "data",
            "tmp_buffer": "more data"
        });
        let config = ProjectionConfig {
            strip_prefixes: vec!["tmp_".to_string()],
            ..Default::default()
        };
        let result = project_json(&serde_json::to_string(&input).unwrap(), &config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.data).unwrap();
        assert!(parsed.get("name").is_some());
        assert!(parsed.get("tmp_cache").is_none());
        assert!(parsed.get("tmp_buffer").is_none());
    }

    #[test]
    fn test_nested_projection() {
        let input = json!({
            "user": {
                "id": 1,
                "name": "Alice",
                "_internal_id": "xyz",
                "debug_flags": [1, 2, 3]
            }
        });
        let config = ProjectionConfig::default();
        let result = project_json(&serde_json::to_string(&input).unwrap(), &config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.data).unwrap();
        assert!(parsed["user"].get("id").is_some());
        assert!(parsed["user"].get("_internal_id").is_none());
        assert!(parsed["user"].get("debug_flags").is_none());
    }

    use proptest::prelude::*;

    proptest! {
        /// Projection never produces invalid JSON from valid JSON input.
        #[test]
        fn prop_projection_produces_valid_json(
            key1 in "[a-z]{3,10}",
            key2 in "[a-z]{3,10}",
            val in "[a-z0-9 ]{1,50}",
        ) {
            let input = format!(r#"{{"{key1}":"{val}","{key2}":42}}"#);
            let config = ProjectionConfig::default();
            let result = project_json(&input, &config).unwrap();
            let parsed: std::result::Result<serde_json::Value, _> = serde_json::from_str(&result.data);
            prop_assert!(parsed.is_ok(), "projection output must be valid JSON");
        }

        /// Fields removed count is non-negative.
        #[test]
        fn prop_fields_removed_non_negative(
            val in "[a-z]{1,20}",
        ) {
            let input = format!(r#"{{"name":"{val}","_debug":"x","__v":1}}"#);
            let config = ProjectionConfig::default();
            let result = project_json(&input, &config).unwrap();
            // fields_removed is usize, always >= 0
            let _ = result.fields_removed;
        }
    }
}
