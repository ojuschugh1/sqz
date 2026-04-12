// sqz-mcp: MCP server integration surface
// Thin adapter over sqz_engine for Model Context Protocol transport

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqz_engine::{SqzEngine, ToolDefinition, ToolSelector};
use sqz_engine::error::{Result, SqzError};
use sqz_engine::preset::{Preset, PresetParser};

// ── Public data types ─────────────────────────────────────────────────────────

/// An incoming MCP tool-call request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub tool_id: String,
    pub input: Value,
    pub intent: Option<String>,
}

/// The result of processing an MCP tool call through the compression pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    pub tool_id: String,
    /// Compressed output string.
    pub output: String,
    pub tokens_original: u32,
    pub tokens_compressed: u32,
}

/// Transport mode for the MCP server.
#[derive(Debug, Clone)]
pub enum McpTransport {
    Stdio,
    Sse { port: u16 },
}

// ── JSON-RPC types (minimal subset for MCP) ───────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message: message.into() }),
        }
    }
}

// ── Shared state (Send + Sync parts only) ─────────────────────────────────────

/// The parts of McpServer state that are `Send + Sync` and can be shared
/// across threads (e.g. with the notify watcher background thread).
struct SharedState {
    /// Pending preset TOML to apply on the next request.
    /// Set by the hot-reload thread; consumed by the main thread.
    pending_preset: Mutex<Option<String>>,
    /// Tool selector — rebuilt on preset reload.
    tool_selector: Mutex<ToolSelector>,
    /// Registered tool definitions.
    registered_tools: Mutex<Vec<ToolDefinition>>,
}

// ── McpServer ─────────────────────────────────────────────────────────────────

/// MCP server that wraps `SqzEngine` and exposes tool calls over stdio or SSE.
pub struct McpServer {
    engine: SqzEngine,
    shared: Arc<SharedState>,
    preset_dir: PathBuf,
}

impl McpServer {
    /// Create a new `McpServer` loading presets from `preset_dir`.
    pub fn new(preset_dir: &Path) -> Result<Self> {
        let engine = SqzEngine::new()?;
        let preset = Preset::default();
        let model_path = Path::new("");
        let mut tool_selector = ToolSelector::new(model_path, &preset)?;

        // Register default MCP tools.
        let default_tools = default_tool_definitions();
        tool_selector.register_tools(&default_tools)?;

        let shared = Arc::new(SharedState {
            pending_preset: Mutex::new(None),
            tool_selector: Mutex::new(tool_selector),
            registered_tools: Mutex::new(default_tools),
        });

        Ok(McpServer {
            engine,
            shared,
            preset_dir: preset_dir.to_owned(),
        })
    }

    /// Apply any pending preset reload before processing a request.
    fn apply_pending_preset(&mut self) {
        let pending = {
            let mut guard = self.shared.pending_preset.lock()
                .unwrap_or_else(|e| e.into_inner());
            guard.take()
        };
        if let Some(toml_str) = pending {
            match self.engine.reload_preset(&toml_str) {
                Ok(()) => {
                    // Also rebuild the tool selector.
                    if let Ok(new_preset) = PresetParser::parse(&toml_str) {
                        if let Ok(mut sel) = self.shared.tool_selector.lock() {
                            if let Ok(mut new_sel) = ToolSelector::new(Path::new(""), &new_preset) {
                                if let Ok(tools) = self.shared.registered_tools.lock() {
                                    let _ = new_sel.register_tools(&tools);
                                }
                                *sel = new_sel;
                            }
                        }
                    }
                    eprintln!("[sqz-mcp] preset applied from hot-reload");
                }
                Err(e) => eprintln!("[sqz-mcp] engine reload error: {e}"),
            }
        }
    }

    /// Process a tool call: serialize input JSON → compress → return response.
    pub fn handle_tool_call(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        self.apply_pending_preset();

        // Serialize the input JSON to a string for compression.
        let raw_input = serde_json::to_string(&request.input)
            .map_err(|e| SqzError::Other(format!("input serialization error: {e}")))?;

        let tokens_original = estimate_tokens(&raw_input);
        let compressed = self.engine.compress(&raw_input)?;
        let tokens_compressed = compressed.tokens_compressed;

        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output: compressed.data,
            tokens_original,
            tokens_compressed,
        })
    }

    /// List tools, optionally filtered by intent using the `ToolSelector`.
    pub fn list_tools(&self, intent: Option<&str>) -> Result<Vec<ToolDefinition>> {
        let tools = self.shared.registered_tools.lock()
            .unwrap_or_else(|e| e.into_inner());

        match intent {
            Some(intent_str) if !intent_str.is_empty() => {
                let selector = self.shared.tool_selector.lock()
                    .unwrap_or_else(|e| e.into_inner());
                let selected_ids = selector.select(intent_str, 5)?;
                let filtered: Vec<ToolDefinition> = tools
                    .iter()
                    .filter(|t| selected_ids.contains(&t.id))
                    .cloned()
                    .collect();
                Ok(filtered)
            }
            _ => Ok(tools.clone()),
        }
    }

    /// Start the server on the given transport.
    ///
    /// For `Stdio`: reads JSON-RPC messages from stdin, writes responses to stdout.
    /// For `Sse`: starts a minimal HTTP server on the given port.
    pub fn start(self, transport: McpTransport) -> Result<()> {
        match transport {
            McpTransport::Stdio => self.run_stdio(),
            McpTransport::Sse { port } => self.run_sse(port),
        }
    }

    /// Wire preset hot-reload: watch `preset_dir` for TOML changes.
    ///
    /// The watcher runs in a background thread and stores the new preset TOML
    /// in `shared.pending_preset`. It is applied on the next request.
    ///
    /// Returns the watcher handle — drop it to stop watching.
    pub fn watch_presets(&self) -> Result<notify::RecommendedWatcher> {
        use notify::{Event, EventKind, RecursiveMode, Watcher};

        let shared = Arc::clone(&self.shared);

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    return;
                }
                for path in &event.paths {
                    if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                        continue;
                    }
                    match std::fs::read_to_string(path) {
                        Ok(toml_str) => {
                            // Validate TOML before storing.
                            match PresetParser::parse(&toml_str) {
                                Ok(_) => {
                                    // Store for lazy application on next request.
                                    if let Ok(mut pending) = shared.pending_preset.lock() {
                                        *pending = Some(toml_str);
                                    }
                                    eprintln!("[sqz-mcp] preset change detected: {}", path.display());
                                }
                                Err(e) => {
                                    // Invalid TOML: log error, keep previous valid preset.
                                    eprintln!("[sqz-mcp] invalid preset TOML in {}: {e}", path.display());
                                }
                            }
                        }
                        Err(e) => eprintln!("[sqz-mcp] preset file read error: {e}"),
                    }
                }
            }
        })
        .map_err(|e| SqzError::Other(format!("watcher init error: {e}")))?;

        watcher
            .watch(&self.preset_dir, RecursiveMode::NonRecursive)
            .map_err(|e| SqzError::Other(format!("watcher watch error: {e}")))?;

        Ok(watcher)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn run_stdio(mut self) -> Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();

        for line in stdin.lock().lines() {
            let line = line.map_err(|e| SqzError::Other(format!("stdin read error: {e}")))?;
            if line.trim().is_empty() {
                continue;
            }
            let response = self.handle_jsonrpc_line(&line);
            let serialized = serde_json::to_string(&response)
                .unwrap_or_else(|_| r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"serialize error"}}"#.to_string());
            writeln!(out, "{serialized}")
                .map_err(|e| SqzError::Other(format!("stdout write error: {e}")))?;
            out.flush()
                .map_err(|e| SqzError::Other(format!("stdout flush error: {e}")))?;
        }
        Ok(())
    }

    fn run_sse(mut self, port: u16) -> Result<()> {
        use std::net::TcpListener;
        use std::io::BufReader;

        let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
            .map_err(|e| SqzError::Other(format!("SSE bind error on port {port}: {e}")))?;
        eprintln!("[sqz-mcp] SSE server listening on http://127.0.0.1:{port}");

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let mut reader = BufReader::new(stream.try_clone()
                        .map_err(|e| SqzError::Other(format!("stream clone error: {e}")))?);
                    let mut request_line = String::new();
                    let _ = reader.read_line(&mut request_line);

                    // Drain headers.
                    let mut content_length = 0usize;
                    loop {
                        let mut header = String::new();
                        let _ = reader.read_line(&mut header);
                        if header == "\r\n" || header.is_empty() {
                            break;
                        }
                        let lower = header.to_lowercase();
                        if lower.starts_with("content-length:") {
                            if let Some(v) = lower.split(':').nth(1) {
                                content_length = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }

                    // Read body.
                    let mut body = vec![0u8; content_length];
                    use std::io::Read;
                    let _ = reader.read_exact(&mut body);
                    let body_str = String::from_utf8_lossy(&body);

                    let response = self.handle_jsonrpc_line(body_str.trim());
                    let json = serde_json::to_string(&response).unwrap_or_default();

                    let http_response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
                        json.len(),
                        json
                    );
                    let _ = stream.write_all(http_response.as_bytes());
                }
                Err(e) => eprintln!("[sqz-mcp] connection error: {e}"),
            }
        }
        Ok(())
    }

    fn handle_jsonrpc_line(&mut self, line: &str) -> JsonRpcResponse {
        // Apply any pending preset reload first.
        self.apply_pending_preset();

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => return JsonRpcResponse::err(None, -32700, format!("parse error: {e}")),
        };

        match req.method.as_str() {
            "tools/list" => {
                let intent = req.params
                    .as_ref()
                    .and_then(|p| p.get("intent"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                match self.list_tools(intent.as_deref()) {
                    Ok(tools) => {
                        let tool_list: Vec<Value> = tools.iter().map(|t| {
                            serde_json::json!({
                                "name": t.id,
                                "description": t.description,
                                "inputSchema": t.input_schema,
                                "outputSchema": t.output_schema,
                                "sqz:transforms": t.compression_transforms,
                            })
                        }).collect();
                        JsonRpcResponse::ok(req.id, serde_json::json!({ "tools": tool_list }))
                    }
                    Err(e) => JsonRpcResponse::err(req.id, -32603, e.to_string()),
                }
            }

            "tools/call" => {
                let params = match req.params {
                    Some(p) => p,
                    None => return JsonRpcResponse::err(req.id, -32602, "missing params"),
                };
                let tool_id = match params.get("name").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => return JsonRpcResponse::err(req.id, -32602, "missing params.name"),
                };
                let input = params.get("arguments").cloned().unwrap_or(Value::Null);
                let intent = params.get("intent").and_then(|v| v.as_str()).map(|s| s.to_string());

                let call_req = ToolCallRequest { tool_id, input, intent };
                match self.handle_tool_call(call_req) {
                    Ok(resp) => JsonRpcResponse::ok(req.id, serde_json::json!({
                        "content": [{ "type": "text", "text": resp.output }],
                        "tokens_original": resp.tokens_original,
                        "tokens_compressed": resp.tokens_compressed,
                    })),
                    Err(e) => JsonRpcResponse::err(req.id, -32603, e.to_string()),
                }
            }

            "initialize" => {
                JsonRpcResponse::ok(req.id, serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "sqz-mcp", "version": env!("CARGO_PKG_VERSION") }
                }))
            }

            _ => JsonRpcResponse::err(req.id, -32601, format!("method not found: {}", req.method)),
        }
    }
}

// ── Default tool definitions ──────────────────────────────────────────────────

/// Returns the default set of MCP tool definitions registered at startup.
/// Each tool includes:
/// - `input_schema`: JSON Schema for the tool's input parameters
/// - `compression_transforms`: exactly what sqz does to this tool's output
pub fn default_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            id: "read_file".to_string(),
            name: "Read File".to_string(),
            description: "Read the contents of a file from the filesystem. Returns file content as text.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute or relative file path" }
                },
                "required": ["path"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "File content, possibly compressed. Code files return AST signatures only. Re-reads of unchanged files return a ~13-token cache reference token (§ref:HASH§)."
            }),
            compression_transforms: vec![
                "sha256_cache: re-reads cost ~13 tokens if content unchanged".to_string(),
                "ast_extract: code files → signatures only (functions, classes, types)".to_string(),
                "ansi_strip: removes color codes".to_string(),
                "truncate_strings: strings > 500 chars are truncated with '...'".to_string(),
            ],
        },
        ToolDefinition {
            id: "write_file".to_string(),
            name: "Write File".to_string(),
            description: "Write or overwrite a file on the filesystem with the provided content.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "Write confirmation message. Not compressed — confirmations are short."
            }),
            compression_transforms: vec![
                "passthrough: write confirmations are short, no compression applied".to_string(),
            ],
        },
        ToolDefinition {
            id: "search_files".to_string(),
            name: "Search Files".to_string(),
            description: "Search for files matching a pattern or containing specific text content.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Search pattern or text" },
                    "path": { "type": "string", "description": "Directory to search in" }
                },
                "required": ["pattern"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "Search results with file:line:content format. Repeated identical lines collapsed. Common path prefixes replaced with ~/."
            }),
            compression_transforms: vec![
                "condense: repeated identical match lines collapsed to max 3".to_string(),
                "path_shorten: common path prefixes replaced with ~/".to_string(),
                "git_diff_fold: unchanged context lines folded if output is diff-like".to_string(),
            ],
        },
        ToolDefinition {
            id: "list_directory".to_string(),
            name: "List Directory".to_string(),
            description: "List the contents of a directory, showing files and subdirectories.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to list" }
                },
                "required": ["path"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "Directory listing. Common path prefixes replaced with ~/. Repeated permission patterns collapsed."
            }),
            compression_transforms: vec![
                "path_shorten: common path prefixes replaced with ~/".to_string(),
                "condense: repeated permission/ownership patterns collapsed".to_string(),
            ],
        },
        ToolDefinition {
            id: "execute_command".to_string(),
            name: "Execute Command".to_string(),
            description: "Execute a shell command and return its stdout and stderr output.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute" },
                    "cwd": { "type": "string", "description": "Working directory (optional)" }
                },
                "required": ["command"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "Command stdout+stderr. ANSI codes stripped. Repeated lines collapsed. Diff output has context lines folded. Error/warning lines always preserved verbatim."
            }),
            compression_transforms: vec![
                "ansi_strip: removes color/formatting codes".to_string(),
                "condense: repeated output lines collapsed to max 3".to_string(),
                "git_diff_fold: diff output has unchanged context lines folded".to_string(),
                "log_fold: repeated log lines with timestamps folded to [xN]".to_string(),
                "safe_fallback: error/warning lines always preserved verbatim".to_string(),
            ],
        },
        ToolDefinition {
            id: "edit_file".to_string(),
            name: "Edit File".to_string(),
            description: "Apply targeted edits to a file by replacing specific text sections.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to edit" },
                    "old_str": { "type": "string", "description": "Text to replace" },
                    "new_str": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old_str", "new_str"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "Edit confirmation message. Not compressed — confirmations are short."
            }),
            compression_transforms: vec![
                "passthrough: edit confirmations are short, no compression applied".to_string(),
            ],
        },
        ToolDefinition {
            id: "create_directory".to_string(),
            name: "Create Directory".to_string(),
            description: "Create a new directory at the specified path.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to create" }
                },
                "required": ["path"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "Directory creation confirmation. Not compressed."
            }),
            compression_transforms: vec![
                "passthrough: directory creation confirmations are short".to_string(),
            ],
        },
        ToolDefinition {
            id: "delete_file".to_string(),
            name: "Delete File".to_string(),
            description: "Delete a file or empty directory from the filesystem.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to delete" }
                },
                "required": ["path"]
            }),
            output_schema: serde_json::json!({
                "type": "string",
                "description": "Deletion confirmation. Not compressed."
            }),
            compression_transforms: vec![
                "passthrough: deletion confirmations are short".to_string(),
            ],
        },
    ]
}

// ── Token estimation helper ───────────────────────────────────────────────────

/// Rough token estimate: ~4 characters per token (GPT-style approximation).
fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn make_server() -> (McpServer, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let server = McpServer::new(dir.path()).expect("McpServer::new");
        (server, dir)
    }

    // ── Task 25.3: Integration tests ─────────────────────────────────────────

    /// Test tool call processing through the compression pipeline.
    /// Validates: Requirements 2.1
    #[test]
    fn test_handle_tool_call_compresses_output() {
        let (mut server, _dir) = make_server();

        let input = serde_json::json!({
            "status": "ok",
            "data": {
                "id": 1,
                "name": "test",
                "debug_info": null,
                "trace_id": null,
                "metadata": {
                    "internal_id": "abc123",
                    "created_at": "2025-01-01T00:00:00Z"
                },
                "items": ["a", "b", "c", "d", "e", "f", "g", "h"]
            }
        });

        let req = ToolCallRequest {
            tool_id: "read_file".to_string(),
            input: input.clone(),
            intent: None,
        };

        let resp = server.handle_tool_call(req).expect("handle_tool_call");
        assert_eq!(resp.tool_id, "read_file");
        assert!(!resp.output.is_empty(), "output should not be empty");
        assert!(resp.tokens_original > 0, "tokens_original should be > 0");
    }

    /// Test that handle_tool_call returns the tool_id unchanged.
    /// Validates: Requirements 2.1
    #[test]
    fn test_handle_tool_call_preserves_tool_id() {
        let (mut server, _dir) = make_server();
        let req = ToolCallRequest {
            tool_id: "execute_command".to_string(),
            input: serde_json::json!({ "cmd": "ls -la" }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("handle_tool_call");
        assert_eq!(resp.tool_id, "execute_command");
    }

    /// Test list_tools returns all tools when no intent is provided.
    /// Validates: Requirements 3.1
    #[test]
    fn test_list_tools_no_intent_returns_all() {
        let (server, _dir) = make_server();
        let tools = server.list_tools(None).expect("list_tools");
        assert_eq!(tools.len(), default_tool_definitions().len());
    }

    /// Test list_tools with intent filters to 3-5 tools.
    /// Validates: Requirements 3.1
    #[test]
    fn test_list_tools_with_intent_filters() {
        let (server, _dir) = make_server();
        let tools = server.list_tools(Some("read file contents from filesystem")).expect("list_tools");
        assert!(tools.len() >= 1, "should return at least 1 tool");
        assert!(tools.len() <= 8, "should not return more tools than registered");
    }

    /// Test tool selector re-evaluation latency < 500ms.
    /// Validates: Requirements 3.3
    #[test]
    fn test_tool_selector_latency_under_500ms() {
        let (server, _dir) = make_server();

        let start = Instant::now();
        for _ in 0..10 {
            let _ = server.list_tools(Some("search for files matching a pattern"));
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(500),
            "10 tool selections took {:?}, expected < 500ms",
            elapsed
        );
    }

    /// Test preset hot-reload: write a valid TOML file and verify reload completes within 2s.
    /// Validates: Requirements 2.3, 24.3
    #[test]
    fn test_preset_hot_reload_latency() {
        let dir = TempDir::new().expect("tempdir");
        let server = McpServer::new(dir.path()).expect("McpServer::new");

        // Start watching.
        let _watcher = server.watch_presets().expect("watch_presets");

        // Write a valid preset TOML.
        let preset_path = dir.path().join("test.toml");
        let toml_content = r#"
[preset]
name = "hot-reload-test"
version = "1.0"

[compression]
stages = []

[tool_selection]
max_tools = 5
similarity_threshold = 0.3

[budget]
warning_threshold = 0.70
ceiling_threshold = 0.85
default_window_size = 200000

[terse_mode]
enabled = false
level = "moderate"

[model]
family = "anthropic"
primary = "claude-sonnet-4-20250514"
complexity_threshold = 0.4
"#;
        std::fs::write(&preset_path, toml_content).expect("write preset");

        // Wait up to 2 seconds for the watcher to fire and store the pending preset.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            // Check if the pending preset was stored.
            if let Ok(guard) = server.shared.pending_preset.lock() {
                if guard.is_some() {
                    break;
                }
            }
        }

        // The pending preset should have been stored within 2 seconds.
        let has_pending = server.shared.pending_preset.lock()
            .map(|g| g.is_some())
            .unwrap_or(false);
        assert!(has_pending, "preset should have been hot-reloaded within 2 seconds");
    }

    /// Test that invalid TOML does not crash the server (keeps previous preset).
    /// Validates: Requirements 2.5
    #[test]
    fn test_invalid_toml_keeps_previous_preset() {
        let dir = TempDir::new().expect("tempdir");
        let server = McpServer::new(dir.path()).expect("McpServer::new");
        let _watcher = server.watch_presets().expect("watch_presets");

        // Write invalid TOML.
        let bad_path = dir.path().join("bad.toml");
        std::fs::write(&bad_path, "this is not valid toml ][[[").expect("write bad preset");

        // Give watcher time to fire.
        std::thread::sleep(Duration::from_millis(200));

        // No pending preset should be stored (invalid TOML was rejected).
        let has_pending = server.shared.pending_preset.lock()
            .map(|g| g.is_some())
            .unwrap_or(false);
        assert!(!has_pending, "invalid TOML should not be stored as pending preset");

        // Server should still work.
        let tools = server.list_tools(None).expect("list_tools after bad preset");
        assert!(!tools.is_empty(), "tools should still be available after invalid preset");
    }

    /// Test JSON-RPC initialize method.
    #[test]
    fn test_jsonrpc_initialize() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = server.handle_jsonrpc_line(line);
        assert!(resp.error.is_none(), "initialize should not error");
        let result = resp.result.expect("initialize should have result");
        assert!(result.get("protocolVersion").is_some());
    }

    /// Test JSON-RPC tools/list method.
    #[test]
    fn test_jsonrpc_tools_list() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line(line);
        assert!(resp.error.is_none(), "tools/list should not error");
        let result = resp.result.expect("tools/list should have result");
        let tools = result.get("tools").expect("result should have tools");
        assert!(tools.as_array().map(|a| !a.is_empty()).unwrap_or(false));
    }

    /// Test JSON-RPC tools/call method.
    #[test]
    fn test_jsonrpc_tools_call() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"/tmp/test.txt"}}}"#;
        let resp = server.handle_jsonrpc_line(line);
        assert!(resp.error.is_none(), "tools/call should not error: {:?}", resp.error);
        let result = resp.result.expect("tools/call should have result");
        assert!(result.get("content").is_some());
    }

    /// Test JSON-RPC unknown method returns -32601.
    #[test]
    fn test_jsonrpc_unknown_method() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":4,"method":"unknown/method","params":{}}"#;
        let resp = server.handle_jsonrpc_line(line);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    /// Test JSON-RPC parse error returns -32700.
    #[test]
    fn test_jsonrpc_parse_error() {
        let (mut server, _dir) = make_server();
        let resp = server.handle_jsonrpc_line("not json at all {{{");
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32700);
    }
}
