//! # sqz-wasm
//!
//! Browser-compatible WASM build of the sqz compression engine. This is a
//! self-contained subset of `sqz_engine` compiled to `wasm32-unknown-unknown` —
//! no tree-sitter, no SQLite, no filesystem. Everything runs in memory.
//!
//! ## What's included
//!
//! - TOON encoding for JSON (null stripping, array collapsing, tabular encoding)
//! - Content classification (JSON, code, logs, prose)
//! - Text compression with phrase abbreviation and word shortening
//! - Log condensing (repeated lines collapsed)
//! - Code compression (comment stripping, blank line collapsing)
//! - In-memory dedup cache (hash-based, no persistence)
//! - Three compression presets: minimal, default, aggressive
//!
//! ## Browser usage
//!
//! Build with `wasm-pack`:
//!
//! ```text
//! wasm-pack build sqz-wasm --target web
//! ```
//!
//! Then in JavaScript:
//!
//! ```text
//! import init, { SqzWasm } from './pkg/sqz_wasm.js';
//!
//! await init();
//! const sqz = new SqzWasm("default");
//!
//! // Compress JSON — gets TOON-encoded, nulls stripped
//! const result = sqz.compress('{"name": "Alice", "debug": null}');
//! console.log(result); // TOON:{name:"Alice"}
//!
//! // Estimate token count
//! const tokens = sqz.estimate_tokens(result);
//!
//! // Session export/import for persistence
//! const ctx = sqz.export_ctx();
//! sqz.import_ctx(ctx);
//! ```
//!
//! ## Presets
//!
//! - `"minimal"` — TOON encoding only, no stripping
//! - `"default"` — strip nulls + condense + TOON + word abbreviation
//! - `"aggressive"` — all of default + collapse arrays at 5 items

use std::collections::HashMap;
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------------
// Minimal TOON encoder (pure Rust, WASM-compatible)
// ---------------------------------------------------------------------------

const TOON_PREFIX: &str = "TOON:";

fn encode_value(v: &serde_json::Value, buf: &mut String) {
    match v {
        serde_json::Value::Null => buf.push_str("null"),
        serde_json::Value::Bool(b) => buf.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => {
            buf.push_str(&serde_json::to_string(&serde_json::Value::Number(n.clone()))
                .unwrap_or_else(|_| n.to_string()));
        }
        serde_json::Value::String(s) => encode_string(s, buf),
        serde_json::Value::Array(arr) => {
            buf.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                encode_value(item, buf);
            }
            buf.push(']');
        }
        serde_json::Value::Object(map) => {
            buf.push('{');
            for (i, (k, val)) in map.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                if is_simple_key(k) {
                    buf.push_str(k);
                } else {
                    encode_string(k, buf);
                }
                buf.push(':');
                encode_value(val, buf);
            }
            buf.push('}');
        }
    }
}

fn encode_string(s: &str, buf: &mut String) {
    buf.push('"');
    for ch in s.chars() {
        match ch {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                buf.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if (c as u32) > 0x7E => {
                let cp = c as u32;
                if cp <= 0xFFFF {
                    buf.push_str(&format!("\\u{:04x}", cp));
                } else {
                    let cp = cp - 0x10000;
                    let high = 0xD800 + (cp >> 10);
                    let low = 0xDC00 + (cp & 0x3FF);
                    buf.push_str(&format!("\\u{:04x}\\u{:04x}", high, low));
                }
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

fn is_simple_key(k: &str) -> bool {
    if k.is_empty() {
        return false;
    }
    let mut chars = k.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    !matches!(k, "true" | "false" | "null")
}

// ---------------------------------------------------------------------------
// JSON preprocessing: strip nulls, collapse arrays
// ---------------------------------------------------------------------------

/// Recursively remove null-valued fields from JSON objects.
/// Arrays keep their null elements (positional data).
fn strip_nulls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for v in map.values_mut() {
                strip_nulls(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                strip_nulls(item);
            }
        }
        _ => {}
    }
}

/// Collapse arrays with more than `max_items` elements.
/// For uniform arrays (all objects with same keys), use tabular encoding.
fn collapse_arrays(value: &mut serde_json::Value, max_items: usize) {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                collapse_arrays(item, max_items);
            }
            if arr.len() > max_items {
                // Check for uniform array → tabular encoding
                if let Some(keys) = detect_uniform_keys(arr) {
                    let table = encode_table(arr, &keys);
                    let count = arr.len();
                    arr.clear();
                    arr.push(serde_json::Value::String(
                        format!("[table: {} rows]\n{}", count, table),
                    ));
                } else {
                    let remaining = arr.len() - max_items;
                    arr.truncate(max_items);
                    arr.push(serde_json::Value::String(
                        format!("... and {} more items", remaining),
                    ));
                }
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                collapse_arrays(v, max_items);
            }
        }
        _ => {}
    }
}

fn detect_uniform_keys(arr: &[serde_json::Value]) -> Option<Vec<String>> {
    if arr.len() < 2 {
        return None;
    }
    let first_keys: Vec<String> = match &arr[0] {
        serde_json::Value::Object(map) if !map.is_empty() => map.keys().cloned().collect(),
        _ => return None,
    };
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

fn encode_table(arr: &[serde_json::Value], keys: &[String]) -> String {
    let mut lines = Vec::with_capacity(arr.len() + 1);
    lines.push(keys.join(" | "));
    for item in arr {
        if let serde_json::Value::Object(map) = item {
            let row: Vec<String> = keys
                .iter()
                .map(|k| compact_value(map.get(k).unwrap_or(&serde_json::Value::Null)))
                .collect();
            lines.push(row.join(" | "));
        }
    }
    lines.join("\n")
}

fn compact_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            if s.len() > 50 { format!("{}...", &s[..47]) } else { s.clone() }
        }
        serde_json::Value::Array(a) => format!("[{} items]", a.len()),
        serde_json::Value::Object(m) => format!("{{{} keys}}", m.len()),
    }
}

// ---------------------------------------------------------------------------
// Content classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentType {
    Json,
    Code,
    Log,
    Prose,
}

fn classify(input: &str) -> ContentType {
    let trimmed = input.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
            return ContentType::Json;
        }
    }
    let lines: Vec<&str> = trimmed.lines().take(20).collect();
    let mut code_score = 0;
    let mut log_score = 0;
    for line in &lines {
        let l = line.trim();
        if l.ends_with('{') || l.ends_with('}') || l.ends_with(';')
            || l.starts_with("fn ") || l.starts_with("def ")
            || l.starts_with("class ") || l.starts_with("import ")
            || l.contains("->") || l.contains("::")
        {
            code_score += 1;
        }
        if l.contains("[INFO]") || l.contains("[ERROR]") || l.contains("[WARN]")
            || l.contains("[DEBUG]") || l.starts_with("20")
        {
            log_score += 1;
        }
    }
    if code_score > lines.len() / 2 { return ContentType::Code; }
    if log_score > lines.len() / 3 { return ContentType::Log; }
    ContentType::Prose
}

// ---------------------------------------------------------------------------
// Plain text compression
// ---------------------------------------------------------------------------

fn compress_text(input: &str) -> String {
    let mut result = String::with_capacity(input.len());

    // Pass 1: Normalize whitespace and collapse blank lines
    let mut prev_blank = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank {
                result.push('\n');
                prev_blank = true;
            }
            continue;
        }
        prev_blank = false;
        let mut prev_space = false;
        for ch in trimmed.chars() {
            if ch == ' ' || ch == '\t' {
                if !prev_space {
                    result.push(' ');
                    prev_space = true;
                }
            } else {
                result.push(ch);
                prev_space = false;
            }
        }
        result.push('\n');
    }

    // Pass 2: Phrase abbreviations
    let result = result
        .replace("for example", "e.g.")
        .replace("For example", "E.g.")
        .replace("that is to say", "i.e.")
        .replace("in order to", "to")
        .replace("In order to", "To")
        .replace("as well as", "&")
        .replace("such as", "e.g.")
        .replace("a number of", "several")
        .replace("A number of", "Several")
        .replace("due to the fact that", "because")
        .replace("Due to the fact that", "Because")
        .replace("in the event that", "if")
        .replace("In the event that", "If")
        .replace("at this point in time", "now")
        .replace("At this point in time", "Now")
        .replace("it is important to note that ", "")
        .replace("It is important to note that ", "")
        .replace("it should be noted that ", "")
        .replace("It should be noted that ", "")
        .replace("the fact that ", "")
        .replace("there is a need to", "must")
        .replace("There is a need to", "Must")
        .replace("in terms of", "regarding")
        .replace("In terms of", "Regarding")
        .replace("with respect to", "regarding")
        .replace("With respect to", "Regarding")
        .replace("on the other hand", "conversely")
        .replace("On the other hand", "Conversely")
        .replace("in addition to", "besides")
        .replace("In addition to", "Besides")
        .replace("as a result of", "from")
        .replace("As a result of", "From")
        .replace("  ", " ");

    // Pass 3: Word abbreviations — intentionally NOT called on this path.
    //
    // Single-word substitutions like "configuration" → "config" or
    // "repository" → "repo" silently rewrite content the model may need
    // to dereference (directory names, URL segments, identifiers). The
    // classifier's "Prose" bucket is a fallback that catches `ls -l`,
    // stack traces, and other mixed-content pastes — not just pure prose.
    //
    // The CLI proxy removed its equivalent call in fd4603d after a Reddit
    // user reported "packages → pkgs" breaking every tool call. Keeping
    // the behavior in the extension's WASM path would reintroduce the
    // same class of bug behind the preview gate.
    //
    // `abbreviate_words()` is retained below for callers that know their
    // input is pure prose (no paths, no identifiers).

    let result = result.trim_end().to_string();
    if result.len() < input.len() { result } else { input.to_string() }
}

/// Collapse repeated consecutive lines (condense stage).
fn condense(input: &str, max_repeated: usize) -> String {
    let mut result = Vec::new();
    let mut current_line: Option<&str> = None;
    let mut run_count: usize = 0;

    for line in input.lines() {
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

    let trailing = input.ends_with('\n');
    let mut out = result.join("\n");
    if trailing { out.push('\n'); }
    out
}

/// Compress log output: condense repeated lines, keep errors/warnings.
fn compress_log(input: &str) -> String {
    condense(input, 2)
}

/// Compress code: strip single-line comments, condense blank lines.
fn compress_code(input: &str) -> String {
    let mut result = Vec::new();
    let mut prev_blank = false;
    for line in input.lines() {
        let trimmed = line.trim();
        // Strip single-line comments (but keep doc comments)
        if (trimmed.starts_with("//") && !trimmed.starts_with("///"))
            || (trimmed.starts_with('#') && !trimmed.starts_with("#!") && !trimmed.starts_with("#["))
        {
            continue;
        }
        if trimmed.is_empty() {
            if !prev_blank {
                result.push("");
                prev_blank = true;
            }
            continue;
        }
        prev_blank = false;
        result.push(line);
    }
    let trailing = input.ends_with('\n');
    let mut out = result.join("\n");
    if trailing { out.push('\n'); }
    if out.len() < input.len() { out } else { input.to_string() }
}

/// Word abbreviation table (subset of the Rust engine's 100+ entries).
///
/// NOTE: this function is NOT called from the WASM compress pipeline. Word-level
/// substitution on free-form text silently rewrites path segments and
/// identifiers (the failure mode reported in issue #1). It is retained here
/// for callers that can prove their input is pure prose — no paths, no
/// identifiers, no URLs, no filenames.
#[allow(dead_code)]
fn abbreviate_words(text: &str) -> String {
    let abbrevs: &[(&str, &str)] = &[
        ("implementation", "impl"),
        ("configuration", "config"),
        ("authentication", "auth"),
        ("authorization", "authz"),
        ("application", "app"),
        ("environment", "env"),
        ("development", "dev"),
        ("production", "prod"),
        ("repository", "repo"),
        ("dependency", "dep"),
        ("dependencies", "deps"),
        ("documentation", "docs"),
        ("information", "info"),
        ("directory", "dir"),
        ("parameter", "param"),
        ("parameters", "params"),
        ("function", "fn"),
        ("reference", "ref"),
        ("specification", "spec"),
        ("administrator", "admin"),
        ("database", "db"),
        ("infrastructure", "infra"),
        ("kubernetes", "k8s"),
        ("namespace", "ns"),
        ("management", "mgmt"),
        ("notification", "notif"),
        ("permission", "perm"),
        ("permissions", "perms"),
    ];
    let mut result = text.to_string();
    for &(long, short) in abbrevs {
        // Simple whole-word replacement (case-insensitive)
        result = replace_word_ci(&result, long, short);
    }
    result
}

fn replace_word_ci(text: &str, word: &str, replacement: &str) -> String {
    let lower = text.to_lowercase();
    let word_lower = word.to_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    let bytes = text.as_bytes();
    for (start, _) in lower.match_indices(&word_lower) {
        let end = start + word.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end >= text.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            result.push_str(&text[last..start]);
            result.push_str(replacement);
            last = end;
        }
    }
    result.push_str(&text[last..]);
    result
}

// ---------------------------------------------------------------------------
// In-memory dedup cache
// ---------------------------------------------------------------------------

fn content_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// In-memory session store (no SQLite — WASM-compatible)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SessionState {
    turns: Vec<String>,
    corrections: Vec<String>,
}

// ---------------------------------------------------------------------------
// Compression preset
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preset {
    /// TOON only, no stripping.
    Minimal,
    /// Strip nulls + condense + TOON + word abbreviation.
    Default,
    /// All of Default + collapse arrays + aggressive text compression.
    Aggressive,
}

impl Preset {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "minimal" => Preset::Minimal,
            "aggressive" => Preset::Aggressive,
            _ => Preset::Default,
        }
    }
}

// ---------------------------------------------------------------------------
// WasmEngine — full-featured subset for browser use
// ---------------------------------------------------------------------------

struct WasmEngine {
    session: SessionState,
    preset: Preset,
    /// In-memory dedup cache: hash → compressed output.
    dedup_cache: HashMap<u64, String>,
}

impl WasmEngine {
    fn new(preset: Preset) -> Self {
        Self {
            session: SessionState::default(),
            preset,
            dedup_cache: HashMap::new(),
        }
    }

    fn compress(&mut self, input: &str) -> String {
        // Dedup check: if we've seen this exact content before, return ref
        let hash = content_hash(input);
        if let Some(cached) = self.dedup_cache.get(&hash) {
            return cached.clone();
        }

        let result = self.compress_inner(input);

        // Cache the result for future dedup
        self.dedup_cache.insert(hash, result.clone());
        result
    }

    fn compress_inner(&self, input: &str) -> String {
        let content_type = classify(input);

        match content_type {
            ContentType::Json => self.compress_json(input),
            ContentType::Code => compress_code(input),
            ContentType::Log => compress_log(input),
            ContentType::Prose => compress_text(input),
        }
    }

    fn compress_json(&self, input: &str) -> String {
        let parsed = match serde_json::from_str::<serde_json::Value>(input.trim()) {
            Ok(v) => v,
            Err(_) => return input.to_string(),
        };

        let mut value = parsed;

        // Strip nulls (unless Minimal preset)
        if self.preset != Preset::Minimal {
            strip_nulls(&mut value);
        }

        // Collapse arrays (Aggressive only, or Default with large arrays)
        match self.preset {
            Preset::Aggressive => collapse_arrays(&mut value, 5),
            Preset::Default => collapse_arrays(&mut value, 10),
            Preset::Minimal => {}
        }

        // TOON encode
        let mut buf = String::with_capacity(input.len());
        buf.push_str(TOON_PREFIX);
        encode_value(&value, &mut buf);

        if buf.len() < input.len() { buf } else { input.to_string() }
    }

    fn estimate_tokens(&self, input: &str) -> u32 {
        (input.len() as f64 / 4.0).ceil() as u32
    }

    fn export_ctx(&self) -> Result<String, String> {
        serde_json::to_string(&self.session).map_err(|e| e.to_string())
    }

    fn import_ctx(&mut self, ctx: &str) -> Result<(), String> {
        self.session = serde_json::from_str(ctx).map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public WASM bindings
// ---------------------------------------------------------------------------

/// Browser-facing WASM wrapper for the sqz compression engine.
///
/// Provides content-aware compression (JSON, code, logs, prose), TOON
/// encoding, word abbreviation, in-memory dedup caching, and session
/// export/import. No filesystem or network access — everything runs in memory.
#[wasm_bindgen]
pub struct SqzWasm {
    engine: WasmEngine,
}

#[wasm_bindgen]
impl SqzWasm {
    /// Create a new SqzWasm instance.
    /// `preset_json` selects the compression preset:
    /// - `"minimal"` — TOON encoding only
    /// - `"default"` — strip nulls + condense + TOON + word abbreviation
    /// - `"aggressive"` — all of default + collapse arrays at 5 items
    #[wasm_bindgen(constructor)]
    pub fn new(preset_json: &str) -> Result<SqzWasm, JsValue> {
        let preset = Preset::from_str(preset_json);
        Ok(SqzWasm {
            engine: WasmEngine::new(preset),
        })
    }

    /// Compress `input`. Routes to JSON/code/log/prose compressor based on
    /// content classification. Returns cached result on duplicate input.
    pub fn compress(&mut self, input: &str) -> Result<JsValue, JsValue> {
        let compressed = self.engine.compress(input);
        Ok(JsValue::from_str(&compressed))
    }

    /// Estimate the token count for `input` using the GPT-style approximation
    /// (chars / 4, rounded up).
    pub fn estimate_tokens(&self, input: &str) -> u32 {
        self.engine.estimate_tokens(input)
    }

    /// Serialize the current session state to a JSON string.
    pub fn export_ctx(&self) -> Result<String, JsValue> {
        self.engine.export_ctx().map_err(|e| JsValue::from_str(&e))
    }

    /// Deserialize session state from a JSON string produced by `export_ctx`.
    pub fn import_ctx(&mut self, ctx: &str) -> Result<(), JsValue> {
        self.engine
            .import_ctx(ctx)
            .map_err(|e| JsValue::from_str(&e))
    }
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn should_show_preview(token_count: u32) -> bool {
        token_count > 100
    }

    fn estimate_tokens(input: &str) -> u32 {
        (input.len() as f64 / 4.0).ceil() as u32
    }

    proptest! {
        #[test]
        fn prop_browser_compression_preview_threshold(input in ".*") {
            let tokens = estimate_tokens(&input);
            let preview = should_show_preview(tokens);
            if tokens > 100 {
                prop_assert!(preview, "expected preview for {} tokens", tokens);
            } else {
                prop_assert!(!preview, "expected no preview for {} tokens", tokens);
            }
        }
    }

    #[test]
    fn estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_four_chars() {
        assert_eq!(estimate_tokens("abcd"), 1);
    }

    #[test]
    fn estimate_tokens_five_chars() {
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn preview_threshold_boundary() {
        assert!(!should_show_preview(100));
        assert!(should_show_preview(101));
    }

    #[test]
    fn compress_json_applies_toon() {
        let mut engine = WasmEngine::new(Preset::Default);
        let json = r#"{ "key":  "value",  "another_key":  "another_value" }"#;
        let result = engine.compress(json);
        assert!(result.starts_with("TOON:"), "result: {result}");
        assert!(result.len() < json.len(), "TOON should be shorter");
    }

    #[test]
    fn compress_json_strips_nulls() {
        let mut engine = WasmEngine::new(Preset::Default);
        let json = r#"{"a": 1, "b": null, "c": "hello"}"#;
        let result = engine.compress(json);
        assert!(result.starts_with("TOON:"), "result: {result}");
        assert!(!result.contains("null"), "nulls should be stripped: {result}");
    }

    #[test]
    fn compress_json_minimal_preserves_nulls() {
        let mut engine = WasmEngine::new(Preset::Minimal);
        let json = r#"{"a": 1, "b": null}"#;
        let result = engine.compress(json);
        assert!(result.contains("null"), "minimal preset should preserve nulls: {result}");
    }

    #[test]
    fn compress_non_json_passthrough_short() {
        let mut engine = WasmEngine::new(Preset::Default);
        let plain = "hello world";
        assert_eq!(engine.compress(plain), plain);
    }

    #[test]
    fn compress_text_normalizes_whitespace() {
        let input = "hello   world\n\n\n\nfoo   bar";
        let result = compress_text(input);
        assert_eq!(result, "hello world\n\nfoo bar");
    }

    #[test]
    fn compress_text_abbreviates_phrases() {
        let input = "in order to do this, for example, we need a number of things";
        let result = compress_text(input);
        assert!(result.len() < input.len(), "result: {result}");
        assert!(result.contains("to do this"), "result: {result}");
        assert!(result.contains("e.g."), "result: {result}");
        assert!(result.contains("several"), "result: {result}");
    }

    #[test]
    fn compress_text_does_not_abbreviate_words() {
        // Regression: word abbreviation used to run on all prose, which
        // includes any content the classifier couldn't place elsewhere
        // (ls -l output, stack traces, mixed docs). It silently rewrote
        // directory names and identifiers. We keep the text as-is and
        // leave `abbreviate_words()` available only to callers that can
        // prove their input contains no paths/identifiers.
        let input = "The implementation of the configuration requires authentication";
        let result = compress_text(input);
        assert!(result.contains("implementation"),
            "word 'implementation' must be preserved: {result}");
        assert!(result.contains("configuration"),
            "word 'configuration' must be preserved: {result}");
        assert!(result.contains("authentication"),
            "word 'authentication' must be preserved: {result}");
    }

    #[test]
    fn compress_text_preserves_paths_in_prose() {
        // Direct repro for the Reddit failure mode, at the WASM layer.
        // A paste containing a path segment must not have the path
        // silently rewritten by `compress_text`.
        let input = "Please check the file at /etc/myapp/configuration/default.yml \
                     which lives in the main repository at \
                     github.com/example/repository for the current environment setup.";
        let result = compress_text(input);
        assert!(result.contains("/etc/myapp/configuration/default.yml"),
            "path must survive compression: {result}");
        assert!(result.contains("github.com/example/repository"),
            "URL must survive compression: {result}");
        assert!(result.contains("environment"),
            "identifier must survive compression: {result}");
    }

    #[test]
    fn condense_collapses_repeated_lines() {
        let input = "ok\nok\nok\nok\nok\ndone\n";
        let result = condense(input, 2);
        assert_eq!(result, "ok\nok\ndone\n");
    }

    #[test]
    fn compress_log_condenses() {
        let input = "[INFO] Connected\n[INFO] Connected\n[INFO] Connected\n[ERROR] Timeout\n";
        let result = compress_log(input);
        assert!(result.contains("[ERROR] Timeout"), "errors preserved: {result}");
        let info_count = result.matches("[INFO] Connected").count();
        assert!(info_count <= 2, "repeated lines condensed: {result}");
    }

    #[test]
    fn compress_code_strips_comments() {
        let input = "fn main() {\n    // this is a comment\n    let x = 42;\n}\n";
        let result = compress_code(input);
        assert!(!result.contains("// this is a comment"), "comment stripped: {result}");
        assert!(result.contains("let x = 42"), "code preserved: {result}");
    }

    #[test]
    fn compress_code_preserves_doc_comments() {
        let input = "/// Documentation comment\nfn main() {}\n";
        let result = compress_code(input);
        assert!(result.contains("/// Documentation"), "doc comment preserved: {result}");
    }

    #[test]
    fn classify_json() {
        assert_eq!(classify(r#"{"key": "value"}"#), ContentType::Json);
        assert_eq!(classify(r#"[1, 2, 3]"#), ContentType::Json);
    }

    #[test]
    fn classify_code() {
        let code = "fn main() {\n    let x = 42;\n    println!(\"hello\");\n}\n";
        assert_eq!(classify(code), ContentType::Code);
    }

    #[test]
    fn classify_log() {
        let log = "2024-01-01 [INFO] Started\n2024-01-01 [INFO] Connected\n2024-01-01 [ERROR] Failed\n";
        assert_eq!(classify(log), ContentType::Log);
    }

    #[test]
    fn classify_prose() {
        let prose = "This is a normal sentence about something interesting.";
        assert_eq!(classify(prose), ContentType::Prose);
    }

    #[test]
    fn dedup_cache_returns_same_result() {
        let mut engine = WasmEngine::new(Preset::Default);
        let input = r#"{"key": "value", "other": null}"#;
        let first = engine.compress(input);
        let second = engine.compress(input);
        assert_eq!(first, second, "dedup should return cached result");
    }

    #[test]
    fn strip_nulls_recursive() {
        let mut v = serde_json::json!({"a": 1, "b": null, "c": {"d": null, "e": 2}});
        strip_nulls(&mut v);
        assert_eq!(v, serde_json::json!({"a": 1, "c": {"e": 2}}));
    }

    #[test]
    fn collapse_arrays_uniform() {
        let mut v = serde_json::json!([
            {"id": 1, "name": "Alice"},
            {"id": 2, "name": "Bob"},
            {"id": 3, "name": "Carol"},
            {"id": 4, "name": "Dave"},
            {"id": 5, "name": "Eve"},
            {"id": 6, "name": "Frank"}
        ]);
        collapse_arrays(&mut v, 3);
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 1, "uniform array should be tabular-encoded");
        let table = arr[0].as_str().unwrap();
        assert!(table.contains("[table: 6 rows]"), "table: {table}");
    }

    #[test]
    fn preset_from_str() {
        assert_eq!(Preset::from_str("minimal"), Preset::Minimal);
        assert_eq!(Preset::from_str("aggressive"), Preset::Aggressive);
        assert_eq!(Preset::from_str("default"), Preset::Default);
        assert_eq!(Preset::from_str("anything_else"), Preset::Default);
    }

    #[test]
    fn export_import_roundtrip() {
        let mut engine = WasmEngine::new(Preset::Default);
        let exported = engine.export_ctx().expect("export should succeed");
        engine.import_ctx(&exported).expect("import should succeed");
        let re_exported = engine.export_ctx().expect("re-export should succeed");
        assert_eq!(exported, re_exported);
    }

    #[test]
    fn wasm_new_accepts_preset_json() {
        let result = SqzWasm::new("default");
        assert!(result.is_ok());
        let result2 = SqzWasm::new("not a preset");
        assert!(result2.is_ok());
    }

    #[test]
    fn word_abbreviation_whole_word_only() {
        let result = abbreviate_words("the implementation is done");
        assert_eq!(result, "the impl is done");
        // "implementations" should NOT match "implementation"
        let result2 = abbreviate_words("multiple implementations");
        assert_eq!(result2, "multiple implementations");
    }
}
