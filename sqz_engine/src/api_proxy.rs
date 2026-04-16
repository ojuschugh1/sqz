/// API Proxy — compresses LLM API request payloads before forwarding.
///
/// Sits between the application and the LLM API (OpenAI, Anthropic, Google).
/// Intercepts the full request body, compresses the messages array
/// (system prompt, conversation history, tool results), and forwards
/// the compressed version. This attacks the 70-80% of tokens that
/// shell hooks and tool interception cannot reach.
///
/// Compression targets:
/// - System prompt: compress once, cache for the session
/// - Conversation history: summarize old turns, keep recent ones verbatim
/// - Tool results: apply the full sqz pipeline (strip nulls, TOON, condense, etc.)
/// - Repeated content: dedup across messages in the same request

use std::collections::HashMap;

use crate::error::{Result, SqzError};

/// Supported API formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    /// OpenAI: POST /v1/chat/completions
    OpenAi,
    /// Anthropic: POST /v1/messages
    Anthropic,
    /// Google: POST /v1/models/*/generateContent
    Google,
}

impl ApiFormat {
    /// Detect the API format from the request path.
    pub fn from_path(path: &str) -> Option<Self> {
        if path.contains("/chat/completions") {
            Some(ApiFormat::OpenAi)
        } else if path.contains("/messages") {
            Some(ApiFormat::Anthropic)
        } else if path.contains("/generateContent") {
            Some(ApiFormat::Google)
        } else {
            None
        }
    }

    /// The upstream API base URL for this format.
    pub fn default_upstream(&self) -> &'static str {
        match self {
            ApiFormat::OpenAi => "https://api.openai.com",
            ApiFormat::Anthropic => "https://api.anthropic.com",
            ApiFormat::Google => "https://generativelanguage.googleapis.com",
        }
    }
}

/// Configuration for the API proxy.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Port to listen on. Default: 8080.
    pub port: u16,
    /// Upstream API URLs (overrides defaults).
    pub upstreams: HashMap<String, String>,
    /// Maximum number of recent messages to keep verbatim.
    /// Older messages are summarized. Default: 10.
    pub keep_recent_messages: usize,
    /// Whether to compress system prompts. Default: true.
    pub compress_system: bool,
    /// Whether to compress tool results. Default: true.
    pub compress_tool_results: bool,
    /// Whether to summarize old conversation turns. Default: true.
    pub summarize_history: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            upstreams: HashMap::new(),
            keep_recent_messages: 10,
            compress_system: true,
            compress_tool_results: true,
            summarize_history: true,
        }
    }
}

/// Statistics from a single proxy compression pass.
#[derive(Debug, Clone, Default)]
pub struct ProxyStats {
    /// Original token count (estimated).
    pub tokens_original: u32,
    /// Compressed token count (estimated).
    pub tokens_compressed: u32,
    /// Number of messages compressed.
    pub messages_compressed: u32,
    /// Number of messages summarized (old history).
    pub messages_summarized: u32,
    /// Number of system prompt tokens saved.
    pub system_tokens_saved: u32,
    /// Number of tool result tokens saved.
    pub tool_result_tokens_saved: u32,
}

impl ProxyStats {
    pub fn tokens_saved(&self) -> u32 {
        self.tokens_original.saturating_sub(self.tokens_compressed)
    }

    pub fn reduction_pct(&self) -> f64 {
        if self.tokens_original == 0 {
            0.0
        } else {
            (1.0 - self.tokens_compressed as f64 / self.tokens_original as f64) * 100.0
        }
    }
}

/// Compress an API request body.
///
/// Parses the JSON body, compresses messages based on the config,
/// and returns the compressed JSON body + stats.
pub fn compress_request(
    body: &str,
    format: ApiFormat,
    config: &ProxyConfig,
    engine: &crate::engine::SqzEngine,
) -> Result<(String, ProxyStats)> {
    let mut parsed: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| SqzError::Other(format!("proxy: invalid JSON body: {e}")))?;

    let mut stats = ProxyStats::default();

    match format {
        ApiFormat::OpenAi => compress_openai(&mut parsed, config, engine, &mut stats)?,
        ApiFormat::Anthropic => compress_anthropic(&mut parsed, config, engine, &mut stats)?,
        ApiFormat::Google => compress_google(&mut parsed, config, engine, &mut stats)?,
    }

    let compressed_body = serde_json::to_string(&parsed)
        .map_err(|e| SqzError::Other(format!("proxy: JSON serialize error: {e}")))?;

    Ok((compressed_body, stats))
}

// ── OpenAI format ─────────────────────────────────────────────────────────

fn compress_openai(
    body: &mut serde_json::Value,
    config: &ProxyConfig,
    engine: &crate::engine::SqzEngine,
    stats: &mut ProxyStats,
) -> Result<()> {
    let messages = match body.get_mut("messages") {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return Ok(()), // no messages to compress
    };

    let total = messages.len();
    let keep_recent = config.keep_recent_messages.min(total);

    for (i, msg) in messages.iter_mut().enumerate() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let is_recent = i >= total.saturating_sub(keep_recent);

        match role.as_str() {
            "system" if config.compress_system => {
                compress_message_content(msg, engine, stats, "system")?;
            }
            "tool" if config.compress_tool_results => {
                compress_message_content(msg, engine, stats, "tool")?;
            }
            "assistant" | "user" if !is_recent && config.summarize_history => {
                summarize_message(msg, stats)?;
            }
            _ => {}
        }
    }

    Ok(())
}

// ── Anthropic format ──────────────────────────────────────────────────────

fn compress_anthropic(
    body: &mut serde_json::Value,
    config: &ProxyConfig,
    engine: &crate::engine::SqzEngine,
    stats: &mut ProxyStats,
) -> Result<()> {
    // Compress system prompt
    if config.compress_system {
        if let Some(system) = body.get_mut("system") {
            if let Some(text) = system.as_str() {
                let original_tokens = estimate_tokens(text);
                let compressed = engine.compress_or_passthrough(text);
                if compressed.tokens_compressed < original_tokens {
                    *system = serde_json::Value::String(compressed.data);
                    stats.system_tokens_saved += original_tokens - compressed.tokens_compressed;
                    stats.tokens_original += original_tokens;
                    stats.tokens_compressed += compressed.tokens_compressed;
                }
            }
        }
    }

    // Compress messages
    let messages = match body.get_mut("messages") {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return Ok(()),
    };

    let total = messages.len();
    let keep_recent = config.keep_recent_messages.min(total);

    for (i, msg) in messages.iter_mut().enumerate() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let is_recent = i >= total.saturating_sub(keep_recent);

        // Compress tool_result content blocks
        if role == "user" && config.compress_tool_results {
            if let Some(content) = msg.get_mut("content") {
                if let Some(arr) = content.as_array_mut() {
                    for block in arr.iter_mut() {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                            compress_content_block(block, engine, stats)?;
                        }
                    }
                }
            }
        }

        // Summarize old history
        if !is_recent && config.summarize_history && (role == "user" || role == "assistant") {
            summarize_message(msg, stats)?;
        }
    }

    Ok(())
}

// ── Google format ─────────────────────────────────────────────────────────

fn compress_google(
    body: &mut serde_json::Value,
    config: &ProxyConfig,
    engine: &crate::engine::SqzEngine,
    stats: &mut ProxyStats,
) -> Result<()> {
    // Compress system instruction
    if config.compress_system {
        if let Some(si) = body.get_mut("system_instruction") {
            if let Some(parts) = si.get_mut("parts") {
                if let Some(arr) = parts.as_array_mut() {
                    for part in arr.iter_mut() {
                        if let Some(text) = part.get_mut("text") {
                            if let Some(s) = text.as_str() {
                                let original_tokens = estimate_tokens(s);
                                let compressed = engine.compress_or_passthrough(s);
                                if compressed.tokens_compressed < original_tokens {
                                    *text = serde_json::Value::String(compressed.data);
                                    stats.system_tokens_saved +=
                                        original_tokens - compressed.tokens_compressed;
                                    stats.tokens_original += original_tokens;
                                    stats.tokens_compressed += compressed.tokens_compressed;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Compress contents
    let contents = match body.get_mut("contents") {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return Ok(()),
    };

    let total = contents.len();
    let keep_recent = config.keep_recent_messages.min(total);

    for (i, content) in contents.iter_mut().enumerate() {
        let is_recent = i >= total.saturating_sub(keep_recent);

        if let Some(parts) = content.get_mut("parts") {
            if let Some(arr) = parts.as_array_mut() {
                for part in arr.iter_mut() {
                    if let Some(text) = part.get_mut("text") {
                        if let Some(s) = text.as_str() {
                            if !is_recent && config.summarize_history && s.len() > 200 {
                                let summary = summarize_text(s);
                                let original_tokens = estimate_tokens(s);
                                let summary_tokens = estimate_tokens(&summary);
                                *text = serde_json::Value::String(summary);
                                stats.messages_summarized += 1;
                                stats.tokens_original += original_tokens;
                                stats.tokens_compressed += summary_tokens;
                            } else if config.compress_tool_results {
                                let original_tokens = estimate_tokens(s);
                                let compressed = engine.compress_or_passthrough(s);
                                if compressed.tokens_compressed < original_tokens {
                                    *text = serde_json::Value::String(compressed.data);
                                    stats.tokens_original += original_tokens;
                                    stats.tokens_compressed += compressed.tokens_compressed;
                                    stats.messages_compressed += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Compress the text content of a message.
fn compress_message_content(
    msg: &mut serde_json::Value,
    engine: &crate::engine::SqzEngine,
    stats: &mut ProxyStats,
    role: &str,
) -> Result<()> {
    let content = match msg.get_mut("content") {
        Some(v) => v,
        None => return Ok(()),
    };

    if let Some(text) = content.as_str() {
        let original_tokens = estimate_tokens(text);
        let compressed = engine.compress_or_passthrough(text);
        if compressed.tokens_compressed < original_tokens {
            *content = serde_json::Value::String(compressed.data);
            stats.messages_compressed += 1;
            let saved = original_tokens - compressed.tokens_compressed;
            match role {
                "system" => stats.system_tokens_saved += saved,
                "tool" => stats.tool_result_tokens_saved += saved,
                _ => {}
            }
            stats.tokens_original += original_tokens;
            stats.tokens_compressed += compressed.tokens_compressed;
        }
    }

    Ok(())
}

/// Compress a content block (Anthropic tool_result format).
fn compress_content_block(
    block: &mut serde_json::Value,
    engine: &crate::engine::SqzEngine,
    stats: &mut ProxyStats,
) -> Result<()> {
    if let Some(content) = block.get_mut("content") {
        if let Some(text) = content.as_str() {
            let original_tokens = estimate_tokens(text);
            let compressed = engine.compress_or_passthrough(text);
            if compressed.tokens_compressed < original_tokens {
                *content = serde_json::Value::String(compressed.data);
                stats.tool_result_tokens_saved +=
                    original_tokens - compressed.tokens_compressed;
                stats.tokens_original += original_tokens;
                stats.tokens_compressed += compressed.tokens_compressed;
                stats.messages_compressed += 1;
            }
        }
    }
    Ok(())
}

/// Summarize a message by keeping only the first and last sentences.
fn summarize_message(msg: &mut serde_json::Value, stats: &mut ProxyStats) -> Result<()> {
    let content = match msg.get_mut("content") {
        Some(v) => v,
        None => return Ok(()),
    };

    if let Some(text) = content.as_str() {
        if text.len() < 200 {
            return Ok(()); // too short to summarize
        }
        let original_tokens = estimate_tokens(text);
        let summary = summarize_text(text);
        let summary_tokens = estimate_tokens(&summary);

        if summary_tokens < original_tokens {
            *content = serde_json::Value::String(summary);
            stats.messages_summarized += 1;
            stats.tokens_original += original_tokens;
            stats.tokens_compressed += summary_tokens;
        }
    }

    Ok(())
}

/// Create a compact summary of text by keeping the first line and a
/// character count indicator.
fn summarize_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 3 {
        return text.to_string();
    }

    let first_line = lines[0];
    let last_line = lines[lines.len() - 1];
    let total_lines = lines.len();

    format!(
        "{first_line}\n[... {total_lines} lines, {} chars ...]\n{last_line}",
        text.len()
    )
}

/// Estimate token count (chars / 4, rounded up).
fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

/// Parse an HTTP request from raw bytes.
/// Returns (method, path, headers, body).
pub fn parse_http_request(raw: &[u8]) -> Result<(String, String, HashMap<String, String>, String)> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.lines();

    // Request line
    let request_line = lines.next().ok_or_else(|| SqzError::Other("empty request".into()))?;
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(SqzError::Other("malformed request line".into()));
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    // Headers
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(
                key.trim().to_lowercase(),
                value.trim().to_string(),
            );
        }
    }

    // Body: everything after the blank line
    let body_start = text.find("\r\n\r\n").map(|p| p + 4)
        .or_else(|| text.find("\n\n").map(|p| p + 2))
        .unwrap_or(text.len());
    let body = text[body_start..].to_string();

    Ok((method, path, headers, body))
}

/// Build an HTTP response from status, headers, and body.
pub fn build_http_response(status: u16, status_text: &str, headers: &[(&str, &str)], body: &str) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status} {status_text}\r\n");
    for (key, value) in headers {
        response.push_str(&format!("{key}: {value}\r\n"));
    }
    response.push_str(&format!("content-length: {}\r\n", body.len()));
    response.push_str("\r\n");
    response.push_str(body);
    response.into_bytes()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_format_from_path() {
        assert_eq!(ApiFormat::from_path("/v1/chat/completions"), Some(ApiFormat::OpenAi));
        assert_eq!(ApiFormat::from_path("/v1/messages"), Some(ApiFormat::Anthropic));
        assert_eq!(ApiFormat::from_path("/v1/models/gemini/generateContent"), Some(ApiFormat::Google));
        assert_eq!(ApiFormat::from_path("/unknown"), None);
    }

    #[test]
    fn test_compress_openai_request() {
        let engine = crate::engine::SqzEngine::new().unwrap();
        let config = ProxyConfig::default();

        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are a helpful assistant. You help with coding tasks. You follow best practices. You write clean code. You test everything."},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there! How can I help?"},
                {"role": "user", "content": "Write a function"}
            ]
        });

        let (compressed, stats) = compress_request(
            &serde_json::to_string(&body).unwrap(),
            ApiFormat::OpenAi,
            &config,
            &engine,
        ).unwrap();

        // Should parse as valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&compressed).unwrap();
        assert!(parsed.get("messages").is_some());
        assert!(parsed.get("model").is_some());
        // Model should be preserved
        assert_eq!(parsed["model"].as_str().unwrap(), "gpt-4");
    }

    #[test]
    fn test_compress_anthropic_request() {
        let engine = crate::engine::SqzEngine::new().unwrap();
        let config = ProxyConfig::default();

        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "system": "You are a helpful coding assistant with extensive knowledge of Rust, Python, and TypeScript.",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi! How can I help you today?"}
            ]
        });

        let (compressed, stats) = compress_request(
            &serde_json::to_string(&body).unwrap(),
            ApiFormat::Anthropic,
            &config,
            &engine,
        ).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&compressed).unwrap();
        assert!(parsed.get("system").is_some());
        assert!(parsed.get("messages").is_some());
        assert_eq!(parsed["model"].as_str().unwrap(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_compress_tool_results() {
        let engine = crate::engine::SqzEngine::new().unwrap();
        let config = ProxyConfig::default();

        // OpenAI format with tool result containing JSON with nulls
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "Get the data"},
                {"role": "tool", "content": "{\"id\":1,\"name\":\"Alice\",\"debug\":null,\"trace\":null,\"internal_id\":null,\"metadata\":{\"plan\":\"pro\",\"seats\":10,\"billing_cycle\":\"monthly\",\"internal_id\":null}}"}
            ]
        });

        let (compressed, stats) = compress_request(
            &serde_json::to_string(&body).unwrap(),
            ApiFormat::OpenAi,
            &config,
            &engine,
        ).unwrap();

        // Tool result should have been compressed
        let parsed: serde_json::Value = serde_json::from_str(&compressed).unwrap();
        let tool_content = parsed["messages"][1]["content"].as_str().unwrap();
        // Nulls should be stripped and/or TOON encoded
        assert!(
            !tool_content.contains("\"debug\":null") || tool_content.starts_with("TOON:"),
            "tool result should be compressed: {tool_content}"
        );
    }

    #[test]
    fn test_summarize_old_history() {
        let engine = crate::engine::SqzEngine::new().unwrap();
        let config = ProxyConfig {
            keep_recent_messages: 2,
            ..Default::default()
        };

        let long_content = "This is a very long message that contains a lot of detail about the implementation.\nIt spans multiple lines and discusses various aspects of the code.\nThe architecture is modular with clear separation of concerns.\nEach component handles a specific responsibility.\nThe database layer manages persistence and caching.\nThe API layer handles routing and validation.\nError handling is centralized for consistency.\nLogging is structured and searchable.\nThe deployment pipeline is fully automated.\nTests run on every commit to ensure quality.\nDocumentation is kept up to date with the code.\nPerformance is monitored in production.\nSecurity reviews happen before each release.\nThe team follows agile practices with two-week sprints.\nCode reviews are required for all changes.\nThe CI pipeline runs in under five minutes.\nStaging environments mirror production exactly.\nFeature flags control gradual rollouts.\nMetrics are collected for all user-facing operations.\nAlerts fire when error rates exceed thresholds.";
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": long_content},
                {"role": "assistant", "content": long_content},
                {"role": "user", "content": "Recent message 1"},
                {"role": "assistant", "content": "Recent response 1"}
            ]
        });

        let (compressed, stats) = compress_request(
            &serde_json::to_string(&body).unwrap(),
            ApiFormat::OpenAi,
            &config,
            &engine,
        ).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&compressed).unwrap();
        let messages = parsed["messages"].as_array().unwrap();

        // Recent messages should be preserved verbatim
        assert_eq!(messages[3]["content"].as_str().unwrap(), "Recent message 1");
        assert_eq!(messages[4]["content"].as_str().unwrap(), "Recent response 1");

        // Old messages should be summarized (shorter)
        let old_msg = messages[1]["content"].as_str().unwrap();
        assert!(old_msg.len() < long_content.len(),
            "old message should be summarized: {} vs {}", old_msg.len(), long_content.len());
    }

    #[test]
    fn test_summarize_text() {
        let text = "First line of content.\nSecond line.\nThird line.\nFourth line.\nLast line.";
        let summary = summarize_text(text);
        assert!(summary.contains("First line"));
        assert!(summary.contains("Last line"));
        assert!(summary.contains("5 lines"));
    }

    #[test]
    fn test_summarize_text_short() {
        let text = "Short text.\nOnly two lines.";
        let summary = summarize_text(text);
        assert_eq!(summary, text, "short text should not be summarized");
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn test_proxy_stats() {
        let stats = ProxyStats {
            tokens_original: 1000,
            tokens_compressed: 600,
            ..Default::default()
        };
        assert_eq!(stats.tokens_saved(), 400);
        assert!((stats.reduction_pct() - 40.0).abs() < 0.1);
    }

    #[test]
    fn test_proxy_stats_zero() {
        let stats = ProxyStats::default();
        assert_eq!(stats.tokens_saved(), 0);
        assert_eq!(stats.reduction_pct(), 0.0);
    }

    #[test]
    fn test_parse_http_request() {
        let raw = b"POST /v1/messages HTTP/1.1\r\nContent-Type: application/json\r\nAuthorization: Bearer sk-test\r\n\r\n{\"model\":\"claude\"}";
        let (method, path, headers, body) = parse_http_request(raw).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/v1/messages");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
        assert_eq!(headers.get("authorization").unwrap(), "Bearer sk-test");
        assert!(body.contains("claude"));
    }

    #[test]
    fn test_build_http_response() {
        let resp = build_http_response(200, "OK", &[("content-type", "application/json")], "{\"ok\":true}");
        let text = String::from_utf8(resp).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert!(text.contains("content-type: application/json"));
        assert!(text.ends_with("{\"ok\":true}"));
    }

    #[test]
    fn test_config_defaults() {
        let config = ProxyConfig::default();
        assert_eq!(config.port, 8080);
        assert_eq!(config.keep_recent_messages, 10);
        assert!(config.compress_system);
        assert!(config.compress_tool_results);
        assert!(config.summarize_history);
    }

    #[test]
    fn test_compress_preserves_model_field() {
        let engine = crate::engine::SqzEngine::new().unwrap();
        let config = ProxyConfig::default();

        for format in [ApiFormat::OpenAi, ApiFormat::Anthropic] {
            let body = match format {
                ApiFormat::OpenAi => serde_json::json!({
                    "model": "gpt-4-turbo",
                    "messages": [{"role": "user", "content": "hi"}]
                }),
                ApiFormat::Anthropic => serde_json::json!({
                    "model": "claude-sonnet-4-20250514",
                    "max_tokens": 1024,
                    "messages": [{"role": "user", "content": "hi"}]
                }),
                _ => continue,
            };

            let (compressed, _) = compress_request(
                &serde_json::to_string(&body).unwrap(),
                format,
                &config,
                &engine,
            ).unwrap();

            let parsed: serde_json::Value = serde_json::from_str(&compressed).unwrap();
            assert!(parsed.get("model").is_some(), "model field must be preserved for {:?}", format);
        }
    }

    use proptest::prelude::*;

    proptest! {
        /// Compression never produces invalid JSON.
        #[test]
        fn prop_compressed_output_is_valid_json(
            content in "[a-z ]{10,200}",
        ) {
            let engine = crate::engine::SqzEngine::new().unwrap();
            let config = ProxyConfig::default();
            let body = serde_json::json!({
                "model": "test",
                "messages": [{"role": "user", "content": content}]
            });

            let (compressed, _) = compress_request(
                &serde_json::to_string(&body).unwrap(),
                ApiFormat::OpenAi,
                &config,
                &engine,
            ).unwrap();

            // Must be valid JSON
            let parsed: std::result::Result<serde_json::Value, _> = serde_json::from_str(&compressed);
            prop_assert!(parsed.is_ok(), "compressed output must be valid JSON");
        }
    }
}
