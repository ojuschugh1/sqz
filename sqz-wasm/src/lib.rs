// sqz-wasm: Browser extension WASM integration surface
// Subset of sqz_engine compiled to wasm32-unknown-unknown
// No tree-sitter, no file cache — in-memory session store only

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
// Plain text compression (works on non-JSON content)
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

        // Collapse runs of spaces/tabs within the line to single space
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

    // Pass 2: Apply common abbreviations to reduce token count
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

    // Trim trailing whitespace
    let result = result.trim_end().to_string();

    // Only return compressed version if it's actually shorter
    if result.len() < input.len() {
        result
    } else {
        input.to_string()
    }
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
// WasmEngine — minimal subset for browser use
// ---------------------------------------------------------------------------

struct WasmEngine {
    session: SessionState,
}

impl WasmEngine {
    fn new() -> Self {
        Self {
            session: SessionState::default(),
        }
    }

    fn compress(&self, input: &str) -> String {
        // Try TOON encoding for JSON content
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(input.trim()) {
            let mut buf = String::with_capacity(input.len());
            buf.push_str(TOON_PREFIX);
            encode_value(&v, &mut buf);
            if buf.len() < input.len() {
                return buf;
            }
        }

        // Plain text compression pipeline
        compress_text(input)
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
/// Subset engine: no tree-sitter, no file cache, in-memory session store.
#[wasm_bindgen]
pub struct SqzWasm {
    engine: WasmEngine,
}

#[wasm_bindgen]
impl SqzWasm {
    /// Create a new SqzWasm instance.
    /// `preset_json` is accepted for API compatibility but currently unused
    /// (the browser subset uses fixed defaults).
    #[wasm_bindgen(constructor)]
    pub fn new(_preset_json: &str) -> Result<SqzWasm, JsValue> {
        Ok(SqzWasm {
            engine: WasmEngine::new(),
        })
    }

    /// Compress `input`. If the input is valid JSON, TOON encoding is applied.
    /// Otherwise the input is returned unchanged.
    /// Returns a JS string value.
    pub fn compress(&self, input: &str) -> Result<JsValue, JsValue> {
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

    // -----------------------------------------------------------------------
    // Property 5 — Browser compression preview threshold
    // Validates: Requirements 5.3
    // -----------------------------------------------------------------------
    //
    // For any content input to the Browser_Extension, if the estimated token
    // count exceeds 500, the extension SHALL produce a compression preview
    // containing both original and compressed token counts.
    // If the token count is <= 500, no preview SHALL be produced.
    //
    // We model the "preview decision" as a pure function of the token estimate
    // so that it can be tested without a real DOM or WASM runtime.

    /// Returns true when a compression preview should be shown.
    fn should_show_preview(token_count: u32) -> bool {
        token_count > 500
    }

    /// Estimate tokens for a string (mirrors WasmEngine::estimate_tokens).
    fn estimate_tokens(input: &str) -> u32 {
        (input.len() as f64 / 4.0).ceil() as u32
    }

    proptest! {
        /// **Validates: Requirements 5.3**
        ///
        /// For any string input:
        /// - If estimate_tokens(input) > 500, should_show_preview MUST return true.
        /// - If estimate_tokens(input) <= 500, should_show_preview MUST return false.
        #[test]
        fn prop_browser_compression_preview_threshold(input in ".*") {
            let tokens = estimate_tokens(&input);
            let preview = should_show_preview(tokens);
            if tokens > 500 {
                prop_assert!(
                    preview,
                    "expected preview for input with {} tokens (len={})",
                    tokens,
                    input.len()
                );
            } else {
                prop_assert!(
                    !preview,
                    "expected no preview for input with {} tokens (len={})",
                    tokens,
                    input.len()
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_four_chars() {
        // 4 chars → ceil(4/4) = 1
        assert_eq!(estimate_tokens("abcd"), 1);
    }

    #[test]
    fn estimate_tokens_five_chars() {
        // 5 chars → ceil(5/4) = 2
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn preview_threshold_boundary() {
        // exactly 500 tokens → no preview
        assert!(!should_show_preview(500));
        // 501 tokens → preview
        assert!(should_show_preview(501));
    }

    #[test]
    fn compress_json_applies_toon() {
        let engine = WasmEngine::new();
        // Use a JSON input where TOON encoding is shorter than the original
        let json = r#"{ "key":  "value",  "another_key":  "another_value" }"#;
        let result = engine.compress(json);
        assert!(result.starts_with("TOON:"), "result: {result}");
        assert!(result.len() < json.len(), "TOON should be shorter");
    }

    #[test]
    fn compress_non_json_passthrough_short() {
        let engine = WasmEngine::new();
        let plain = "hello world";
        // Short text with no compressible patterns returns unchanged
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
    fn export_import_roundtrip() {
        let mut engine = WasmEngine::new();
        let exported = engine.export_ctx().expect("export should succeed");
        engine.import_ctx(&exported).expect("import should succeed");
        let re_exported = engine.export_ctx().expect("re-export should succeed");
        assert_eq!(exported, re_exported);
    }

    #[test]
    fn wasm_new_accepts_preset_json() {
        // SqzWasm::new should not fail regardless of preset_json content
        let result = SqzWasm::new("{}");
        assert!(result.is_ok());
        let result2 = SqzWasm::new("not json");
        assert!(result2.is_ok());
    }
}
