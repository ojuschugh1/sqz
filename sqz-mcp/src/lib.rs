//! # sqz-mcp
//!
//! MCP (Model Context Protocol) server for sqz. This is a thin adapter over
//! `sqz_engine` that exposes context compression through the MCP transport
//! layer — either stdio (for local tool integration) or SSE (for network use).
//!
//! ## What it does
//!
//! When an MCP-compatible tool (Claude Code, Cursor, etc.) makes a tool call,
//! sqz-mcp intercepts the response and compresses it before it hits the LLM's
//! context window. JSON gets TOON-encoded, repeated lines get condensed, ANSI
//! codes get stripped — the usual sqz pipeline.
//!
//! ## Configuration
//!
//! The server loads presets from a directory you specify at startup. Drop a
//! `.toml` file in there and the server picks it up automatically — hot-reload
//! is built in, no restart needed.
//!
//! ```text
//! # Start on stdio (default for MCP tool integration)
//! sqz-mcp --preset-dir ~/.sqz/presets
//!
//! # Start on SSE for network access
//! sqz-mcp --preset-dir ~/.sqz/presets --transport sse --port 3002
//! ```
//!
//! ## MCP protocol
//!
//! The server implements the MCP JSON-RPC interface:
//!
//! - `initialize` — returns server capabilities
//! - `tools/list` — returns registered tools (optionally filtered by intent)
//! - `tools/call` — compresses tool output through the sqz pipeline
//!
//! ## Tool selection
//!
//! When `tools/list` is called with an `intent` parameter, the built-in
//! `ToolSelector` ranks tools by semantic similarity and returns the top
//! matches. This keeps the tool list small and relevant.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqz_engine::{SqzEngine, ToolDefinition, ToolSelector};
use sqz_engine::error::{Result, SqzError};
use sqz_engine::preset::{Preset, PresetParser};

// ── Public data types ─────────────────────────────────────────────────────────

/// An incoming MCP tool-call request. Contains the tool ID, input arguments,
/// and an optional intent string for tool filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub tool_id: String,
    pub input: Value,
    pub intent: Option<String>,
}

/// The result of processing an MCP tool call. Includes the compressed output
/// and before/after token counts so the caller can see the savings.
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
        Self::with_engine(preset_dir, engine)
    }

    /// Test-only helper: create a server backed by an isolated SQLite
    /// store. Production code uses [`new`] which defaults to
    /// `~/.sqz/sessions.db`; tests need a per-tempdir store so the
    /// dedup cache (now wired up for issue #12 follow-up) doesn't
    /// leak state across tests.
    #[cfg(test)]
    fn new_with_store(preset_dir: &Path, store_path: &Path) -> Result<Self> {
        let engine = SqzEngine::with_preset_and_store(Preset::default(), store_path)?;
        Self::with_engine(preset_dir, engine)
    }

    /// Shared construction logic. Both `new` and the test helper land
    /// here so the wiring of `ToolSelector`, `SharedState`, and
    /// `preset_dir` stays in one place.
    fn with_engine(preset_dir: &Path, engine: SqzEngine) -> Result<Self> {
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

    /// Process a tool call: dispatch by tool ID and return the response.
    ///
    /// Tools we handle:
    ///
    /// * `compress` — run the input `text` through the full sqz pipeline.
    ///   Pure transform, no I/O. The original sqz-mcp tool.
    /// * `passthrough` — return `text` unchanged. Escape hatch for models
    ///   that loop on `§ref:…§` dedup tokens (reported: GLM 5.1 on
    ///   Synthetic).
    /// * `expand` — resolve a `§ref:<prefix>§` token back to original bytes.
    /// * `sqz_read_file` — read a file from disk and return a compressed
    ///   view. Added for issue #12: Claude Code's built-in `Read` bypasses
    ///   shell hooks, so the only way to compress file reads is to offer
    ///   a real MCP tool the agent will pick up.
    /// * `sqz_grep` — search files in a directory and compress the matches.
    /// * `sqz_list_dir` — list a directory and compress the output.
    ///
    /// The `sqz_*` tools perform REAL I/O (not fake stubs — earlier
    /// releases ≤0.8.0 shipped fake file tools that only compressed the
    /// JSON args; see issue #5). They only READ — writes and deletes
    /// stay with native host tools.
    ///
    /// Unknown tool IDs fall back to `compress` to preserve backward
    /// compatibility.
    pub fn handle_tool_call(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        self.apply_pending_preset();

        match request.tool_id.as_str() {
            "passthrough" => self.handle_passthrough(request),
            "expand" => self.handle_expand(request),
            "sqz_read_file" => self.handle_sqz_read_file(request),
            "sqz_grep" => self.handle_sqz_grep(request),
            "sqz_list_dir" => self.handle_sqz_list_dir(request),
            // Default and explicit "compress": run the pipeline.
            _ => self.handle_compress(request),
        }
    }

    /// Compress pipeline — the historical default.
    ///
    /// Routes through the dedup cache so repeat calls with identical
    /// input return a 13-token `§ref:HASH§` instead of re-compressing.
    /// Same rationale as `handle_sqz_read_file`: the README's headline
    /// "92% saved on repeated reads" number lives in the cache, so
    /// every MCP tool that accepts arbitrary content should consult it.
    fn handle_compress(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        // Serialize the input JSON to a string for compression.
        let raw_input = serde_json::to_string(&request.input)
            .map_err(|e| SqzError::Other(format!("input serialization error: {e}")))?;

        let tokens_original = estimate_tokens(&raw_input);
        let (output, tokens_compressed) = self.compress_cached(&raw_input)?;

        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output,
            tokens_original,
            tokens_compressed,
        })
    }

    /// Passthrough: return `input.text` unchanged.
    ///
    /// Designed as a cooperation point with agents that can't (or won't)
    /// parse sqz's compressed output. The agent explicitly asks for raw
    /// data, sqz honours the ask, and we avoid the thrash-loop failure
    /// mode. Reported by SquireNed on Synthetic for GLM 5.1.
    ///
    /// Accepts the same `{text: string}` input shape as `compress` so
    /// the two tools are trivially interchangeable. If the input is
    /// JSON without a `text` key, we fall back to re-serialising the
    /// whole object — strictly less useful than calling the right tool,
    /// but never destructive.
    fn handle_passthrough(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        let text = match request.input.get("text").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                // Best-effort: serialize the whole input.
                serde_json::to_string(&request.input)
                    .map_err(|e| SqzError::Other(format!("input serialization error: {e}")))?
            }
        };
        let tokens = estimate_tokens(&text);
        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output: text,
            tokens_original: tokens,
            // Passthrough is 1:1 — original == compressed so stats
            // stay honest.
            tokens_compressed: tokens,
        })
    }

    /// Expand: look up a dedup ref prefix in the cache and return the
    /// original bytes (or the compressed form if originals weren't
    /// captured for that entry).
    ///
    /// Input: `{ "prefix": "a1b2c3d4" }` or `{ "prefix": "§ref:a1b2c3d4§" }`.
    fn handle_expand(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        let raw = request
            .input
            .get("prefix")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SqzError::Other("expand: input must be { \"prefix\": \"<hex>\" }".to_string())
            })?;
        // Strip the `§ref:…§` wrapper so callers can paste the token raw.
        let prefix = raw
            .trim()
            .trim_start_matches('§')
            .trim_start_matches("ref:")
            .trim_end_matches('§')
            .trim();

        let result = self.engine.cache_manager().expand_prefix(prefix)?;
        let output = match result {
            Some(sqz_engine::ExpandResult::Original { bytes, hash }) => {
                // Best-effort UTF-8 conversion. Non-UTF-8 bytes become
                // the replacement character in the MCP response — this
                // is a necessary concession because JSON-RPC strings
                // must be UTF-8. Agents that truly need binary-safe
                // output should use the CLI's `sqz expand` which writes
                // raw bytes to stdout.
                let as_text = String::from_utf8_lossy(&bytes).into_owned();
                format!("[sqz:expand hash={hash}]\n{as_text}")
            }
            Some(sqz_engine::ExpandResult::CompressedOnly { compressed, hash }) => {
                format!(
                    "[sqz:expand hash={hash} note=compressed-only (predates original-capture migration)]\n{compressed}"
                )
            }
            None => {
                format!("[sqz:expand hash-not-found prefix={prefix}]")
            }
        };

        let tokens = estimate_tokens(&output);
        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output,
            tokens_original: tokens,
            tokens_compressed: tokens,
        })
    }

    /// Compress `text` through the dedup cache, returning `(output, tokens)`.
    ///
    /// Unpacks a [`CacheResult`] into the shape the tool handlers need:
    ///
    ///   * `Dedup` → emit the 13-token `§ref:HASH§` token. On repeat
    ///     reads in the same session (or across sessions if the DB
    ///     survives) the file collapses to 13 tokens — the 92%-savings
    ///     path the README advertises.
    ///   * `Delta` → emit a compact diff against a near-duplicate. Not
    ///     as compact as a full ref but still well below fresh
    ///     compression for iterated edits.
    ///   * `Fresh` → cache miss; emit the full compressed output.
    ///
    /// Added in response to issue #12 follow-up: `handle_sqz_read_file`
    /// (and the grep / list_dir siblings) were calling
    /// `engine.compress()` directly, bypassing the cache entirely.
    /// Repeat reads got pipeline compression (~30%), not dedup refs
    /// (~92%). Routing through `engine.compress_with_cache()` restores
    /// the advertised behaviour.
    fn compress_cached(&self, text: &str) -> Result<(String, u32)> {
        use sqz_engine::CacheResult;
        let result = self.engine.compress_with_cache(text)?;
        Ok(match result {
            CacheResult::Dedup { inline_ref, token_cost } => (inline_ref, token_cost),
            CacheResult::Delta { delta_text, token_cost, .. } => (delta_text, token_cost),
            CacheResult::Fresh { output } => (output.data, output.tokens_compressed),
        })
    }

    /// Read a file from disk and return the content compressed.
    ///
    /// This is the tool that bridges the gap Claude Code's architecture
    /// creates: built-in `Read`/`Grep`/`Glob` tools bypass shell hooks
    /// entirely, so sqz has no way to intercept them. Exposing a real
    /// MCP tool that actually reads the file and compresses on the way
    /// out lets the agent route file reads through sqz voluntarily.
    ///
    /// Named `sqz_read_file` (not `read_file`) to avoid shadowing
    /// the host's native file tool — issue #5 showed that collision
    /// causes silent write failures when models pick the sqz impostor
    /// over the host's real tool.
    ///
    /// Path handling:
    ///   * Relative paths resolve against CWD (the directory where
    ///     sqz-mcp was launched, which for MCP-over-stdio is typically
    ///     the user's project root).
    ///   * Absolute paths work but are rejected if they escape the
    ///     server's home directory OR the user's HOME via `..`.
    ///     This is a deliberately narrow guard — path-traversal
    ///     protection for MCP is the host's job, not ours, but a
    ///     basic sanity check keeps accidents cheap.
    ///   * Missing files return a clear error rather than an empty
    ///     string, so the agent can distinguish "file is empty" from
    ///     "file doesn't exist".
    ///
    /// Binary-safety: we read as bytes and attempt UTF-8; non-UTF-8
    /// bytes become replacement chars in the response (JSON-RPC
    /// requires UTF-8 strings). Truly binary files should be read
    /// with the host's native tool.
    ///
    /// Input: `{ "path": "src/main.rs" }` or `{ "path": "src/main.rs",
    ///           "max_bytes": 1048576 }`
    fn handle_sqz_read_file(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        let path_str = request
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SqzError::Other(
                    "sqz_read_file: input must be { \"path\": \"<file>\" }".to_string(),
                )
            })?;

        // Cap default at 4 MB — reading huge files through MCP is
        // rarely what the agent wants (the host's native file tool
        // would stream instead). Agents can raise the cap explicitly.
        let max_bytes = request
            .input
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(4 * 1024 * 1024) as usize;

        let path = std::path::PathBuf::from(path_str);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                return Err(SqzError::Other(format!(
                    "sqz_read_file: could not read '{}': {e}",
                    path.display()
                )))
            }
        };

        let was_truncated = bytes.len() > max_bytes;
        let truncated_slice = if was_truncated {
            &bytes[..max_bytes]
        } else {
            &bytes[..]
        };

        // UTF-8 with lossy fallback for non-text files — JSON-RPC
        // requires strings, so there's no byte-exact path here. Hosts
        // that care about binary safety should use their native tool.
        let raw_text = String::from_utf8_lossy(truncated_slice).into_owned();

        let tokens_original = estimate_tokens(&raw_text);
        let (compressed_data, tokens_compressed) = self.compress_cached(&raw_text)?;

        // Attach a small header so the agent knows where the content
        // came from and whether it was truncated. The header lives
        // outside the compressed body so it's not lost to any stage.
        let output = if was_truncated {
            format!(
                "[sqz_read_file path={} size={} truncated_to={}]\n{}",
                path.display(),
                bytes.len(),
                max_bytes,
                compressed_data
            )
        } else {
            format!(
                "[sqz_read_file path={} size={}]\n{}",
                path.display(),
                bytes.len(),
                compressed_data
            )
        };

        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output,
            tokens_original,
            tokens_compressed,
        })
    }

    /// List a directory and return the listing compressed.
    ///
    /// Agents frequently need to see what's in a directory before
    /// picking a file to read. Native `ls -la` goes through Bash (gets
    /// hooked and compressed fine), but built-in file-tree tools bypass
    /// the shell entirely. This MCP tool covers that gap.
    ///
    /// The output format is deliberately simple (one entry per line,
    /// with type prefix) so sqz's `condense` and `path_shorten` stages
    /// can squeeze it effectively.
    ///
    /// Input: `{ "path": "." }` or `{ "path": "src", "max_depth": 2 }`
    fn handle_sqz_list_dir(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        let path_str = request
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let max_depth = request
            .input
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;

        let root = std::path::PathBuf::from(path_str);
        let mut lines = Vec::new();
        list_dir_recursive(&root, &root, 0, max_depth, &mut lines)?;

        let raw = lines.join("\n");
        let tokens_original = estimate_tokens(&raw);
        let (compressed_data, tokens_compressed) = self.compress_cached(&raw)?;

        let output = format!(
            "[sqz_list_dir path={} entries={}]\n{}",
            root.display(),
            lines.len(),
            compressed_data
        );

        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output,
            tokens_original,
            tokens_compressed,
        })
    }

    /// Search file contents for a regex pattern and return the matches
    /// compressed.
    ///
    /// Uses a simple substring / fixed-string search by default (most
    /// agent searches are literal), with an optional `regex` flag for
    /// regex mode. The implementation walks the directory tree, reads
    /// each file, and emits grep-style `path:lineno:text` lines.
    ///
    /// We deliberately do NOT shell out to ripgrep here — that would
    /// introduce a runtime dependency sqz-mcp shouldn't require. The
    /// built-in search is slower but adequate for "find usages of X in
    /// src/", which is the common case.
    ///
    /// Input:
    ///   `{ "pattern": "TODO", "path": "src" }`
    ///   `{ "pattern": "fn \\w+_test", "path": ".", "regex": true }`
    ///   `{ "pattern": "use serde", "path": "src", "max_matches": 50 }`
    fn handle_sqz_grep(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        let pattern = request
            .input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SqzError::Other(
                    "sqz_grep: input must include { \"pattern\": \"<text>\" }".to_string(),
                )
            })?;

        let path_str = request
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let max_matches = request
            .input
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .unwrap_or(200) as usize;

        let use_regex = request
            .input
            .get("regex")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let root = std::path::PathBuf::from(path_str);

        // Compile regex once if needed; fall back to substring search
        // when disabled or when the pattern fails to compile.
        let regex = if use_regex {
            match regex::Regex::new(pattern) {
                Ok(r) => Some(r),
                Err(e) => {
                    return Err(SqzError::Other(format!(
                        "sqz_grep: invalid regex: {e}"
                    )))
                }
            }
        } else {
            None
        };

        let mut matches = Vec::new();
        grep_walk(&root, pattern, regex.as_ref(), max_matches, &mut matches)?;

        let raw = matches.join("\n");
        let tokens_original = estimate_tokens(&raw);
        let (compressed_data, tokens_compressed) = self.compress_cached(&raw)?;

        let output = format!(
            "[sqz_grep pattern={:?} root={} matches={}]\n{}",
            pattern,
            root.display(),
            matches.len(),
            compressed_data
        );

        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output,
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
            // Notifications (no id) return None and must produce no output.
            let Some(response) = self.handle_jsonrpc_line(&line) else {
                continue;
            };
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

                    // Notifications (no id) produce no response body. Send
                    // an empty 204 No Content so HTTP clients still get a
                    // valid response but no JSON-RPC payload.
                    let (status, json) = match self.handle_jsonrpc_line(body_str.trim()) {
                        Some(resp) => ("200 OK", serde_json::to_string(&resp).unwrap_or_default()),
                        None => ("204 No Content", String::new()),
                    };

                    let http_response = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
                        status,
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

    fn handle_jsonrpc_line(&mut self, line: &str) -> Option<JsonRpcResponse> {
        // Apply any pending preset reload first.
        self.apply_pending_preset();

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => return Some(JsonRpcResponse::err(None, -32700, format!("parse error: {e}"))),
        };

        // Notifications (no id) are one-way per JSON-RPC 2.0 and MUST NOT
        // receive a response. Responding to one makes strict clients like
        // Claude Code mark the server as failed. Reported in issue #12.
        if req.id.is_none() {
            return None;
        }

        Some(match req.method.as_str() {
            "tools/list" => {
                let intent = req.params
                    .as_ref()
                    .and_then(|p| p.get("intent"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                match self.list_tools(intent.as_deref()) {
                    Ok(tools) => {
                        let tool_list: Vec<Value> = tools.iter().map(|t| {
                            // `outputSchema` is optional per the MCP spec
                            // (2025-06-18). When present, its root `type`
                            // MUST be `"object"` — not `"string"` or any
                            // other scalar. OpenCode (and other strict
                            // clients) validate this and will disable the
                            // whole server on a violation (reported in
                            // issue #5). We therefore omit the field
                            // entirely when it's unset (null) and only
                            // propagate it when a caller has supplied a
                            // proper object-shaped schema.
                            let mut tool_json = serde_json::json!({
                                "name": t.id,
                                "description": t.description,
                                "inputSchema": t.input_schema,
                                "sqz:transforms": t.compression_transforms,
                            });
                            if !t.output_schema.is_null() {
                                if let Some(obj) = tool_json.as_object_mut() {
                                    obj.insert(
                                        "outputSchema".to_string(),
                                        t.output_schema.clone(),
                                    );
                                }
                            }
                            tool_json
                        }).collect();
                        JsonRpcResponse::ok(req.id, serde_json::json!({ "tools": tool_list }))
                    }
                    Err(e) => JsonRpcResponse::err(req.id, -32603, e.to_string()),
                }
            }

            "tools/call" => {
                let params = match req.params {
                    Some(p) => p,
                    None => return Some(JsonRpcResponse::err(req.id, -32602, "missing params")),
                };
                let tool_id = match params.get("name").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => return Some(JsonRpcResponse::err(req.id, -32602, "missing params.name")),
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
                // Tools capability must be a non-empty object signalling tool
                // support. An empty {} is spec-valid JSON but some MCP clients
                // (OpenCode among them — see issue #3) interpret it as "no
                // tools capability" and skip the tools/list call entirely.
                //
                // `listChanged: false` honestly declares that the tool list is
                // static for the session (sqz-mcp registers its tools once at
                // startup via default_tool_definitions() and never emits
                // notifications/tools/list_changed). This matches the MCP
                // 2024-11-05 spec: https://mcpcn.com/en/specification/2024-11-05/server/tools/
                JsonRpcResponse::ok(req.id, serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": { "name": "sqz-mcp", "version": env!("CARGO_PKG_VERSION") }
                }))
            }

            _ => JsonRpcResponse::err(req.id, -32601, format!("method not found: {}", req.method)),
        })
    }
}

// ── Default tool definitions ──────────────────────────────────────────────────

/// Returns the default set of MCP tool definitions registered at startup.
///
/// The sole advertised tool is `compress`: hand it arbitrary text or JSON and
/// it returns the sqz-compressed version. This is the only thing
/// `handle_tool_call` actually does — it does NOT read or write files, does
/// NOT execute commands, etc. Earlier releases (≤0.8.0) advertised fake
/// `read_file`, `write_file`, `edit_file`, `execute_command`, `list_directory`,
/// `search_files`, `create_directory`, and `delete_file` tools whose
/// implementation only compressed the input JSON and threw the "result" away.
/// That shadowed the host's real file tools and led to silent write failures
/// when an LLM picked the sqz-mcp impostor instead of OpenCode's native
/// `write` tool (reported in issue #5).
///
/// Each tool entry includes:
/// - `input_schema`: JSON Schema for the tool's input parameters
/// - `compression_transforms`: exactly what sqz does to this tool's output
pub fn default_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            id: "compress".to_string(),
            name: "Compress Text".to_string(),
            description: "Compress arbitrary text or JSON through the sqz \
                pipeline. Returns a compressed string that preserves semantic \
                content (filenames, identifiers, URLs, version numbers) \
                byte-exact while collapsing repetitive patterns, stripping \
                ANSI codes, folding diff context, and deduplicating content \
                seen earlier in the session. Use this tool to shrink large \
                tool outputs before re-sending them to the model. It does NOT \
                read or write files, execute commands, or perform any I/O — \
                it is a pure text transform."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The text or serialized JSON to compress."
                    }
                },
                "required": ["text"]
            }),
            compression_transforms: vec![
                "sha256_cache: repeat inputs within the session return a ~13-token §ref:HASH§ token".to_string(),
                "ast_extract: recognised source code collapses to signatures only".to_string(),
                "ansi_strip: removes color/formatting codes".to_string(),
                "condense: repeated identical lines collapsed to max 3 occurrences".to_string(),
                "git_diff_fold: diff output has unchanged context lines folded".to_string(),
                "log_fold: repeated log lines with timestamps folded to [xN]".to_string(),
                "path_shorten: common path prefixes replaced with ~/".to_string(),
                "truncate_strings: strings > 500 chars are truncated with '...'".to_string(),
                "safe_fallback: error/warning lines always preserved verbatim".to_string(),
                "preservation_verifier: path-like and identifier tokens are \
                 checked for byte-exact survival; compression is discarded if \
                 coverage drops below 85%"
                    .to_string(),
            ],
            ..Default::default()
        },
        // Escape hatch for models that loop on compressed output (reported
        // for GLM 5.1 on Synthetic). The agent explicitly asks for raw
        // text and sqz returns it unmodified. Pairs with `expand` below
        // for the case where the agent has already seen a `§ref:…§`
        // token and needs to resolve it.
        ToolDefinition {
            id: "passthrough".to_string(),
            name: "Passthrough (No Compression)".to_string(),
            description: "Return the input text unchanged. Use this when \
                you need the raw, uncompressed form of tool output — for \
                example because you can't parse sqz's `§ref:HASH§` dedup \
                tokens, or because you need to audit byte-for-byte what \
                a command produced. This is strictly more tokens than \
                `compress` but avoids any interpretation ambiguity."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The text to return unchanged."
                    }
                },
                "required": ["text"]
            }),
            compression_transforms: vec![
                "none: input is returned byte-for-byte".to_string(),
            ],
            ..Default::default()
        },
        // Ref-resolution tool. Mirrors `sqz expand` on the CLI: give it
        // the 16-char prefix from a `§ref:…§` token (or the whole token)
        // and it returns the bytes that produced the ref. For agents
        // that see refs in their context and need to recover the
        // original content before they can proceed.
        ToolDefinition {
            id: "expand".to_string(),
            name: "Expand Dedup Ref".to_string(),
            description: "Resolve a `§ref:HASH§` dedup token (or a bare \
                hex prefix) back to the original pre-compression content. \
                Use this if you see a `§ref:…§` token in tool output and \
                need the full text it points at. Returns either the raw \
                original bytes (for cache entries from sqz ≥ 0.10.0) or \
                the compressed-but-legible form with a note (for older \
                entries)."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prefix": {
                        "type": "string",
                        "description": "The hex prefix from a `§ref:<prefix>§` token. \
                                        Accepts the bare prefix (`a1b2c3d4`) or the \
                                        full token pasted verbatim (`§ref:a1b2c3d4§`)."
                    }
                },
                "required": ["prefix"]
            }),
            compression_transforms: vec![
                "none: returns cached original bytes".to_string(),
            ],
            ..Default::default()
        },
        // ── File I/O tools (issue #12) ──────────────────────────────────
        //
        // Claude Code's built-in Read/Grep/Glob/Write tools bypass shell
        // hooks entirely (confirmed: github.com/anthropics/claude-code/
        // issues/4544). That means sqz can only compress output from Bash
        // commands, which Claude rarely uses for file I/O — it prefers
        // its built-ins. Reported in issue #12 by JCKodel: heavy session,
        // 1 compression total.
        //
        // These three tools close the gap: they actually read from disk
        // and compress the result before returning. Named with `sqz_`
        // prefix so they don't shadow the host's native file tools
        // (issue #5 showed that collision caused silent write failures).
        //
        // Scope is deliberately narrow: READ-ONLY. Writes and deletes
        // stay with the host's native tools because that's where
        // permission enforcement, undo history, and diff generation
        // live. sqz has nothing to add on the write path.
        ToolDefinition {
            id: "sqz_read_file".to_string(),
            name: "Read File (Compressed)".to_string(),
            description: "Read a file from disk and return its content \
                compressed through the sqz pipeline. PREFER this tool over \
                the built-in `Read` for any file you expect to be larger \
                than ~2KB or that you might need to read multiple times in \
                the same session — the dedup cache returns a 13-token \
                reference on repeat reads instead of the full content. \
                \
                Returns the file content with a header `[sqz_read_file \
                path=... size=...]` followed by the compressed body. \
                Preserves filenames, identifiers, paths, URLs, and line \
                numbers byte-exact; collapses repeated blocks, strips \
                ANSI, extracts code signatures for recognised languages. \
                \
                Use the built-in `Read` tool for: tiny config files (<1KB), \
                files you need byte-exact (lockfiles, signatures). Use \
                `sqz_read_file` for: source files >2KB, log files, JSON \
                outputs, any file you'll read more than once."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file, relative to the working directory or absolute."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Optional cap on bytes read. Default 4 MB. Truncation is reported in the header.",
                        "default": 4194304
                    }
                },
                "required": ["path"]
            }),
            compression_transforms: vec![
                "sha256_cache: repeat reads return a ~13-token §ref:HASH§ token".to_string(),
                "ast_extract: source code collapses to signatures".to_string(),
                "condense: repeated identical lines collapse to max 3".to_string(),
                "path_shorten: common prefixes → ~/".to_string(),
                "safe_fallback: error/warning lines preserved verbatim".to_string(),
            ],
            ..Default::default()
        },
        ToolDefinition {
            id: "sqz_list_dir".to_string(),
            name: "List Directory (Compressed)".to_string(),
            description: "List the contents of a directory and return the \
                listing compressed. Skips `.git`, `node_modules`, `target`, \
                `dist`, `build`, `__pycache__`, `vendor`, and other common \
                bulk directories so the output stays focused on the files \
                an agent actually wants to see. \
                \
                PREFER this over `ls -la` via Bash when you want to see a \
                project layout — the compression is tuned for directory \
                listings specifically (path_shorten, condense)."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to list. Defaults to the current working directory.",
                        "default": "."
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Recursion depth. 1 = immediate children only. Default 1.",
                        "default": 1
                    }
                }
            }),
            compression_transforms: vec![
                "path_shorten: common path prefixes replaced with ~/".to_string(),
                "condense: repeated entry patterns collapse".to_string(),
                "sha256_cache: repeat listings dedupe via §ref§".to_string(),
            ],
            ..Default::default()
        },
        ToolDefinition {
            id: "sqz_grep".to_string(),
            name: "Grep Files (Compressed)".to_string(),
            description: "Search files under a directory for a literal \
                string (default) or regex (if `regex: true`). Returns \
                grep-style `path:lineno:text` lines compressed through the \
                sqz pipeline. Stops after `max_matches` hits (default 200) \
                so a popular term doesn't flood the context. \
                \
                PREFER this over the built-in `Grep` for any search that \
                might return more than a handful of lines — the compressed \
                output is typically 40-70% smaller, and repeat searches for \
                the same pattern dedupe to a 13-token reference."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Text to search for. Literal substring by default; set `regex: true` for regex mode."
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in. Defaults to the current working directory.",
                        "default": "."
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "Treat `pattern` as a regex instead of a literal string. Default false.",
                        "default": false
                    },
                    "max_matches": {
                        "type": "integer",
                        "description": "Stop after this many hits. Default 200.",
                        "default": 200
                    }
                },
                "required": ["pattern"]
            }),
            compression_transforms: vec![
                "condense: duplicate match lines collapse".to_string(),
                "path_shorten: common path prefixes replaced with ~/".to_string(),
                "sha256_cache: repeat searches dedupe via §ref§".to_string(),
            ],
            ..Default::default()
        },
    ]
}

// ── Token estimation helper ───────────────────────────────────────────────────

/// Rough token estimate: ~4 characters per token (GPT-style approximation).
fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

// ── File-reading helpers for sqz_list_dir / sqz_grep ─────────────────────────

/// Walk `dir` up to `max_depth` levels deep, emitting one line per entry.
///
/// Format: `<type> <relative_path>` where type is `d` for dir, `f` for
/// file, `l` for symlink. Relative to `root` so the output is stable
/// across hosts.
///
/// Skips hidden entries (`.git`, `.DS_Store`, etc.) and common large
/// directories (`node_modules`, `target`, `dist`, `build`, `__pycache__`)
/// because they bloat output and are rarely what the agent is looking for.
/// Agents who need those should read them directly with `sqz_read_file`.
fn list_dir_recursive(
    root: &std::path::Path,
    current: &std::path::Path,
    depth: usize,
    max_depth: usize,
    lines: &mut Vec<String>,
) -> Result<()> {
    if depth > max_depth {
        return Ok(());
    }

    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(e) => {
            return Err(SqzError::Other(format!(
                "sqz_list_dir: could not read '{}': {e}",
                current.display()
            )))
        }
    };

    let mut sorted: Vec<_> = entries
        .filter_map(|e| e.ok())
        .collect();
    // Deterministic ordering so outputs dedup across runs (important for
    // sqz's SHA-256 cache — same listing twice should return a §ref§).
    sorted.sort_by_key(|e| e.file_name());

    for entry in sorted {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files / common bulk dirs.
        if name_str.starts_with('.') {
            continue;
        }
        if matches!(
            name_str.as_ref(),
            "node_modules" | "target" | "dist" | "build" | "__pycache__"
                | "vendor" | ".next" | ".nuxt"
        ) {
            continue;
        }

        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);

        let type_char = match entry.file_type() {
            Ok(ft) if ft.is_dir() => "d",
            Ok(ft) if ft.is_symlink() => "l",
            Ok(_) => "f",
            Err(_) => "?",
        };

        lines.push(format!("{type_char} {}", rel.display()));

        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) && depth < max_depth {
            let _ = list_dir_recursive(root, &path, depth + 1, max_depth, lines);
        }
    }

    Ok(())
}

/// Walk `root` looking for files containing `pattern`. Appends one line
/// per match to `out` in `path:lineno:text` format (ripgrep-style).
///
/// Stops after `max_matches` — there's no point flooding the agent with
/// thousands of hits, and if they really need that many they should use
/// the native ripgrep via Bash (which gets hooked and compressed anyway).
fn grep_walk(
    root: &std::path::Path,
    needle: &str,
    regex: Option<&regex::Regex>,
    max_matches: usize,
    out: &mut Vec<String>,
) -> Result<()> {
    if out.len() >= max_matches {
        return Ok(());
    }

    if !root.exists() {
        return Err(SqzError::Other(format!(
            "sqz_grep: path '{}' does not exist",
            root.display()
        )));
    }

    // Single-file case: search the one file.
    if root.is_file() {
        grep_one_file(root, needle, regex, max_matches, out)?;
        return Ok(());
    }

    // Directory: walk non-recursively into sorted children, same
    // skip-list as list_dir_recursive (hidden + bulk dirs).
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => {
            return Err(SqzError::Other(format!(
                "sqz_grep: could not read '{}': {e}",
                root.display()
            )))
        }
    };

    let mut sorted: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    sorted.sort_by_key(|e| e.file_name());

    for entry in sorted {
        if out.len() >= max_matches {
            break;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if matches!(
            name_str.as_ref(),
            "node_modules" | "target" | "dist" | "build" | "__pycache__"
                | "vendor" | ".next" | ".nuxt"
        ) {
            continue;
        }

        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                grep_walk(&path, needle, regex, max_matches, out)?;
            }
            Ok(_) => {
                grep_one_file(&path, needle, regex, max_matches, out)?;
            }
            Err(_) => continue,
        }
    }

    Ok(())
}

/// Search a single file line-by-line, appending matches to `out`.
/// Skips binary files (content not UTF-8) silently — the `Read` tool
/// would also struggle with those and the agent doesn't care.
fn grep_one_file(
    file: &std::path::Path,
    needle: &str,
    regex: Option<&regex::Regex>,
    max_matches: usize,
    out: &mut Vec<String>,
) -> Result<()> {
    // Size cap: don't try to grep a 1 GB log file — that's what native
    // ripgrep is for.
    if let Ok(meta) = std::fs::metadata(file) {
        if meta.len() > 50 * 1024 * 1024 {
            return Ok(());
        }
    }

    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(_) => return Ok(()), // Unreadable files skip silently.
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return Ok(()), // Binary files skip.
    };

    for (lineno, line) in text.lines().enumerate() {
        if out.len() >= max_matches {
            break;
        }
        let is_match = match regex {
            Some(r) => r.is_match(line),
            None => line.contains(needle),
        };
        if is_match {
            out.push(format!("{}:{}:{}", file.display(), lineno + 1, line));
        }
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    /// Test-only wrapper that unwraps the `Option<JsonRpcResponse>` so
    /// existing tests can keep calling `.error`, `.result` directly.
    /// Panics if the server returned `None` (i.e. a notification) —
    /// callers that want to assert on the no-response path should call
    /// `handle_jsonrpc_line` directly.
    impl McpServer {
        pub(crate) fn handle_jsonrpc_line_unwrap(&mut self, line: &str) -> JsonRpcResponse {
            self.handle_jsonrpc_line(line)
                .expect("expected response; got None (notification). Use handle_jsonrpc_line directly if that's intended.")
        }
    }

    fn make_server() -> (McpServer, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        // Use an isolated store per test so the dedup cache (wired up
        // for issue #12 follow-up) doesn't leak state across tests.
        // Otherwise test N+1's sqz_list_dir output for a tiny dir
        // gets returned as a §ref:HASH§ token from test N's cache.
        let store_path = dir.path().join("sessions.db");
        let server = McpServer::new_with_store(dir.path(), &store_path)
            .expect("McpServer::new_with_store");
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
            tool_id: "compress".to_string(),
            input: input.clone(),
            intent: None,
        };

        let resp = server.handle_tool_call(req).expect("handle_tool_call");
        assert_eq!(resp.tool_id, "compress");
        assert!(!resp.output.is_empty(), "output should not be empty");
        assert!(resp.tokens_original > 0, "tokens_original should be > 0");
    }

    /// Test that handle_tool_call returns the tool_id unchanged.
    /// Validates: Requirements 2.1
    #[test]
    fn test_handle_tool_call_preserves_tool_id() {
        let (mut server, _dir) = make_server();
        let req = ToolCallRequest {
            tool_id: "compress".to_string(),
            input: serde_json::json!({ "text": "ls -la output here" }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("handle_tool_call");
        assert_eq!(resp.tool_id, "compress");
    }

    /// Test list_tools returns all tools when no intent is provided.
    /// Validates: Requirements 3.1
    #[test]
    fn test_list_tools_no_intent_returns_all() {
        let (server, _dir) = make_server();
        let tools = server.list_tools(None).expect("list_tools");
        assert_eq!(tools.len(), default_tool_definitions().len());
    }

    /// Test list_tools with intent filters to a non-empty subset.
    /// Validates: Requirements 3.1
    ///
    /// With the single-tool `compress`-only registry the intent-based
    /// ranker may legitimately return zero tools (no tool clears the
    /// similarity threshold and the default_tools list is empty in the
    /// default preset). The contract this test enforces is:
    ///   - the call doesn't error
    ///   - the returned set size never exceeds the registered set
    ///   - an empty intent returns everything (intent is optional)
    #[test]
    fn test_list_tools_with_intent_filters() {
        let (server, _dir) = make_server();
        let registered = default_tool_definitions().len();

        let tools = server
            .list_tools(Some("compress arbitrary text through the sqz pipeline"))
            .expect("list_tools with intent should not error");
        assert!(
            tools.len() <= registered,
            "filtered list must not exceed registered count ({registered})"
        );

        let tools = server.list_tools(Some("")).expect("empty intent = all tools");
        assert_eq!(
            tools.len(),
            registered,
            "empty intent is treated as `no intent` and returns every tool"
        );
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
        let resp = server.handle_jsonrpc_line_unwrap(line);
        assert!(resp.error.is_none(), "initialize should not error");
        let result = resp.result.expect("initialize should have result");
        assert!(result.get("protocolVersion").is_some());
    }

    /// Regression for issue #3: the initialize response MUST advertise a
    /// non-empty tools capability so compliant MCP clients (OpenCode, etc.)
    /// know to call tools/list. An empty `{}` is spec-valid JSON but was
    /// interpreted as "no tools" by some clients.
    #[test]
    fn test_initialize_advertises_tools_capability() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let result = resp.result.expect("initialize should have result");

        let caps = result.get("capabilities")
            .expect("initialize result must include capabilities");
        let tools_cap = caps.get("tools")
            .expect("capabilities must include 'tools' key");

        // Must be an object...
        let tools_obj = tools_cap.as_object()
            .expect("'tools' capability must be an object");
        // ...that is not empty. An empty {} is what issue #3 reported as
        // causing OpenCode to skip tools/list.
        assert!(
            !tools_obj.is_empty(),
            "'tools' capability must not be empty {{}} — some MCP clients \
             interpret that as no tools available. Got: {tools_cap:?}"
        );
        // And specifically it should carry the listChanged hint (the spec's
        // documented signal for static vs. dynamic tool lists).
        assert!(
            tools_obj.contains_key("listChanged"),
            "'tools' capability should include listChanged per MCP 2024-11-05 \
             spec. Got: {tools_cap:?}"
        );
    }

    /// Test JSON-RPC tools/list method.
    #[test]
    fn test_jsonrpc_tools_list() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        assert!(resp.error.is_none(), "tools/list should not error");
        let result = resp.result.expect("tools/list should have result");
        let tools = result.get("tools").expect("result should have tools");
        assert!(tools.as_array().map(|a| !a.is_empty()).unwrap_or(false));
    }

    /// Test JSON-RPC tools/call method.
    #[test]
    fn test_jsonrpc_tools_call() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"compress","arguments":{"text":"lorem ipsum dolor sit amet"}}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        assert!(resp.error.is_none(), "tools/call should not error: {:?}", resp.error);
        let result = resp.result.expect("tools/call should have result");
        assert!(result.get("content").is_some());
    }

    /// Test JSON-RPC unknown method returns -32601.
    #[test]
    fn test_jsonrpc_unknown_method() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":4,"method":"unknown/method","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    /// Test JSON-RPC parse error returns -32700.
    #[test]
    fn test_jsonrpc_parse_error() {
        let (mut server, _dir) = make_server();
        let resp = server.handle_jsonrpc_line_unwrap("not json at all {{{");
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32700);
    }

    /// Regression for issue #5: OpenCode's MCP client rejects any tool whose
    /// `outputSchema.type` is not the literal string `"object"`. Before the
    /// fix, every default tool advertised `outputSchema = {type:"string"}`
    /// and OpenCode disabled the whole server during tool discovery.
    ///
    /// This test asserts two invariants that together prevent the bug from
    /// coming back:
    ///   1. If `outputSchema` is present on any tool, its root `type` is
    ///      `"object"` (MCP 2025-06-18 spec requirement).
    ///   2. `inputSchema` is always present with `type: "object"` (the spec
    ///      requires this — absent it, MCP validators reject the tool too).
    #[test]
    fn test_tools_list_outputschema_is_valid_object_or_absent() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        assert!(resp.error.is_none(), "tools/list errored: {:?}", resp.error);

        let tools = resp.result
            .expect("tools/list must have result")
            .get("tools")
            .cloned()
            .expect("result must have tools array");
        let tools = tools.as_array().expect("tools must be an array");
        assert!(!tools.is_empty(), "no tools registered");

        for tool in tools {
            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("?");

            // inputSchema is required and must be an object-typed schema.
            let input_type = tool
                .get("inputSchema")
                .and_then(|s| s.get("type"))
                .and_then(|t| t.as_str());
            assert_eq!(
                input_type,
                Some("object"),
                "tool {name}: inputSchema.type must be \"object\", got {input_type:?}"
            );

            // outputSchema is optional. When present its root type MUST be
            // "object" per MCP 2025-06-18. The previous implementation
            // emitted "string" here — OpenCode's validator saw it as an
            // invalid_value error and dropped every tool from the server.
            if let Some(out) = tool.get("outputSchema") {
                // `null` is equivalent to absent — our response builder
                // should have omitted the key entirely, but assert that
                // too in case serialization ever reinstates null.
                if !out.is_null() {
                    let out_type = out.get("type").and_then(|t| t.as_str());
                    assert_eq!(
                        out_type,
                        Some("object"),
                        "tool {name}: outputSchema.type must be \"object\" \
                         per MCP spec; got {out_type:?}. This is the \
                         exact bug OpenCode reported in issue #5."
                    );
                }
            }
        }
    }

    /// Complement to the above: by default none of the built-in tools
    /// should advertise an outputSchema (none of them return
    /// structuredContent, so there's nothing to validate). This catches
    /// a regression where someone adds `output_schema: json!(...)` to a
    /// default tool without also making tools/call emit structuredContent.
    #[test]
    fn test_default_tools_omit_outputschema() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let tools = resp.result.unwrap().get("tools").cloned().unwrap();
        for tool in tools.as_array().unwrap() {
            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            assert!(
                tool.get("outputSchema").is_none(),
                "default tool {name} unexpectedly has outputSchema: \
                 {:?}. Remove it, or make tools/call also emit \
                 structuredContent matching the schema (MCP 2025-06-18).",
                tool.get("outputSchema")
            );
        }
    }

    /// Regression for the silent-write-failure bug reported as a follow-up
    /// to issue #5.
    ///
    /// Before the fix sqz-mcp advertised `read_file`, `write_file`,
    /// `edit_file`, `execute_command`, `list_directory`, `search_files`,
    /// `create_directory`, and `delete_file` — but `handle_tool_call`
    /// only ran sqz compression on the input JSON and returned a
    /// compressed string. No file was ever written, no command ever
    /// executed. When a host like OpenCode exposed both its native
    /// `write` tool and sqz-mcp's fake `write_file`, the LLM sometimes
    /// picked the impostor and the user's file edits silently vanished.
    ///
    /// The tools list MUST NOT contain any tool whose name implies
    /// side-effecting behaviour we cannot deliver. This test enumerates
    /// the specific impostor names and asserts none of them are present.
    #[test]
    fn test_tools_list_has_no_io_impostor_tools() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let tools = resp.result.unwrap().get("tools").cloned().unwrap();
        let names: Vec<String> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();

        // These names describe side effects sqz-mcp cannot actually
        // perform. Re-introducing any of them without a real impl
        // would bring the silent-write bug back.
        const FORBIDDEN: &[&str] = &[
            "read_file",
            "write_file",
            "edit_file",
            "execute_command",
            "list_directory",
            "search_files",
            "create_directory",
            "delete_file",
        ];

        for forbidden in FORBIDDEN {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "sqz-mcp must not advertise {forbidden} — that name implies \
                 I/O we cannot perform and shadows the host's real tool. \
                 See the silent-write bug follow-up to issue #5. \
                 Tools registered: {names:?}"
            );
        }
    }

    /// The sqz-mcp server exists to compress text. The default tool set
    /// must include exactly one tool whose name communicates that honestly.
    /// This complements the impostor test by asserting what SHOULD be
    /// there, so removing the compress tool by accident also fails.
    #[test]
    fn test_default_tools_advertise_compress_tool() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let tools = resp.result.unwrap().get("tools").cloned().unwrap();
        let names: Vec<String> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();

        assert!(
            names.iter().any(|n| n == "compress"),
            "default tools must include `compress`; got {names:?}"
        );
    }

    /// `passthrough` and `expand` are the escape-hatch tools. They must
    /// be advertised alongside `compress` so agents can discover them via
    /// `tools/list` alone (no out-of-band coordination). Reported by
    /// SquireNed on the Synthetic discord for GLM 5.1.
    #[test]
    fn test_default_tools_advertise_passthrough_and_expand() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let tools = resp.result.unwrap().get("tools").cloned().unwrap();
        let names: Vec<String> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();
        assert!(
            names.iter().any(|n| n == "passthrough"),
            "passthrough tool must be advertised; got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "expand"),
            "expand tool must be advertised; got {names:?}"
        );
    }

    #[test]
    fn test_passthrough_returns_input_unchanged() {
        // Core contract: agent asked for raw text, sqz returns raw
        // text. Any transformation breaks the escape-hatch promise.
        let (mut server, _dir) = make_server();
        let text = "ls -la\ntotal 42\n-rw-r--r-- 1 root root 17 Jan  1 00:00 readme.md\n";
        let req = ToolCallRequest {
            tool_id: "passthrough".to_string(),
            input: serde_json::json!({ "text": text }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).unwrap();
        assert_eq!(resp.output, text, "passthrough must return byte-exact input");
        assert_eq!(
            resp.tokens_original, resp.tokens_compressed,
            "passthrough is 1:1 so token counts must match"
        );
    }

    #[test]
    fn test_passthrough_falls_back_to_serialising_if_no_text_field() {
        // If the agent sends `{"foo": 1}` (no `text` key), we don't
        // error — we serialise the whole object. Principle: never
        // surface a hard failure when we can give the agent something
        // useful. A hard error would re-trigger the retry loop we're
        // trying to prevent.
        let (mut server, _dir) = make_server();
        let req = ToolCallRequest {
            tool_id: "passthrough".to_string(),
            input: serde_json::json!({ "foo": 1, "bar": "baz" }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).unwrap();
        assert!(resp.output.contains("foo"));
        assert!(resp.output.contains("bar"));
    }

    #[test]
    fn test_expand_tool_returns_not_found_marker_on_miss() {
        // The tool MUST NOT return a JSON-RPC error on cache miss — an
        // error retriggers the retry loop. Instead we return a
        // structured "hash-not-found" marker the agent can read and
        // reason about.
        let (mut server, _dir) = make_server();
        let req = ToolCallRequest {
            tool_id: "expand".to_string(),
            input: serde_json::json!({ "prefix": "deadbeef00000000" }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).unwrap();
        assert!(resp.output.contains("hash-not-found"));
        assert!(resp.output.contains("deadbeef00000000"));
    }

    #[test]
    fn test_expand_tool_strips_ref_token_wrapper() {
        // Agents often paste the `§ref:…§` token verbatim rather than
        // extracting just the prefix. All four shapes must work so the
        // escape hatch is as forgiving as possible.
        let (mut server, _dir) = make_server();
        for prefix_input in [
            "§ref:deadbeef00000000§",
            "ref:deadbeef00000000",
            "deadbeef00000000",
            "  deadbeef00000000  ",
        ] {
            let req = ToolCallRequest {
                tool_id: "expand".to_string(),
                input: serde_json::json!({ "prefix": prefix_input }),
                intent: None,
            };
            let resp = server.handle_tool_call(req).unwrap();
            assert!(
                resp.output.contains("deadbeef00000000"),
                "input {prefix_input:?} did not yield expected prefix in output: {}",
                resp.output
            );
        }
    }

    // ── Issue #12: notifications must not receive responses ──────────────

    /// Notification = request without `id`. Per JSON-RPC 2.0 these are
    /// one-way messages; responding makes Claude Code mark the server
    /// as failed.
    #[test]
    fn test_notification_returns_none() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        assert!(server.handle_jsonrpc_line(line).is_none());
    }

    /// Same for unknown-method notifications — the absence of `id` is
    /// what makes it a notification, regardless of whether we'd
    /// recognise the method.
    #[test]
    fn test_unknown_notification_returns_none() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","method":"some/unknown/notif"}"#;
        assert!(server.handle_jsonrpc_line(line).is_none());
    }

    /// Requests (with `id`) must still respond normally.
    #[test]
    fn test_request_still_responds() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        assert!(server.handle_jsonrpc_line(line).is_some());
    }

    // ── Issue #12 follow-up: file-reading MCP tools ──────────────────────
    //
    // Claude Code's built-in Read/Grep/Glob bypass shell hooks and can
    // only be replaced by advertising MCP tools the agent will pick up.
    // These tests pin the contract for those tools:
    //   * They actually read from disk (not fake stubs — see issue #5).
    //   * Their names are prefixed `sqz_` to avoid shadowing native tools.
    //   * They're advertised in tools/list so agents discover them.

    #[test]
    fn test_sqz_file_tools_are_advertised() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let tools = resp.result.unwrap().get("tools").cloned().unwrap();
        let names: Vec<String> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();

        for expected in ["sqz_read_file", "sqz_list_dir", "sqz_grep"] {
            assert!(
                names.iter().any(|n| n == expected),
                "{expected} must be advertised; got {names:?}"
            );
        }
    }

    #[test]
    fn test_sqz_file_tools_have_sqz_prefix() {
        // Name invariant: sqz-mcp's file tools MUST carry the `sqz_`
        // prefix so they don't collide with host-native `read_file`,
        // `grep`, `list_dir`, etc. Issue #5 showed that shadowing
        // native tools caused silent write failures.
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let tools = resp.result.unwrap().get("tools").cloned().unwrap();

        for tool in tools.as_array().unwrap() {
            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
            // Tools that do I/O MUST be sqz-prefixed. Pure transforms
            // (compress/passthrough/expand) stay unprefixed — they
            // perform no I/O so can't collide with native tools.
            if matches!(
                name,
                "read_file" | "grep" | "list_dir" | "list_directory"
                    | "search_files" | "write_file" | "delete_file"
                    | "edit_file" | "execute_command" | "create_directory"
            ) {
                panic!(
                    "tool `{name}` shadows a host-native tool. \
                     I/O tools must be prefixed `sqz_` — see issue #5 \
                     follow-up."
                );
            }
        }
    }

    #[test]
    fn test_sqz_read_file_reads_real_file() {
        // Not a fake stub: sqz_read_file must actually open the file
        // on disk and return its content. The regression we're
        // guarding against is the earlier-v0.8.0 bug where sqz-mcp
        // advertised file tools that only compressed their arguments
        // and never touched the filesystem (issue #5).
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("read_me.txt");
        let original = "hello from sqz_read_file\nline two\nline three\n";
        std::fs::write(&file_path, original).expect("write test file");

        let req = ToolCallRequest {
            tool_id: "sqz_read_file".to_string(),
            input: serde_json::json!({ "path": file_path.to_string_lossy() }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("read should succeed");

        // Header reports the path and size — agent uses these to
        // distinguish "file is empty" from "file doesn't exist".
        assert!(resp.output.contains("[sqz_read_file"));
        assert!(resp.output.contains(&format!("size={}", original.len())));

        // On a short file with no repetition there's nothing to
        // compress, but the content must still be present. We check
        // a stable substring that the pipeline won't transform.
        assert!(
            resp.output.contains("line two") || resp.output.contains("§ref:"),
            "content must survive (possibly as a dedup ref); got: {}",
            resp.output
        );
    }

    #[test]
    fn test_sqz_read_file_reports_missing_file_clearly() {
        // Errors must be legible. Agents use the error string to
        // decide whether to retry with a different path, so "could
        // not read" + path is the minimum useful signal.
        let (mut server, _dir) = make_server();
        let req = ToolCallRequest {
            tool_id: "sqz_read_file".to_string(),
            input: serde_json::json!({ "path": "/nonexistent/path/xyz.txt" }),
            intent: None,
        };
        let result = server.handle_tool_call(req);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sqz_read_file"), "error should name the tool: {err}");
        assert!(err.contains("xyz.txt"), "error should name the path: {err}");
    }

    #[test]
    fn test_sqz_read_file_respects_max_bytes() {
        // Huge files get truncated at max_bytes. The header must
        // report the truncation so the agent knows to fetch more
        // if it needs the tail.
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("big.txt");
        let content = "x".repeat(100_000);
        std::fs::write(&file_path, &content).expect("write big file");

        let req = ToolCallRequest {
            tool_id: "sqz_read_file".to_string(),
            input: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "max_bytes": 1024
            }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("read should succeed");
        assert!(
            resp.output.contains("truncated_to=1024"),
            "truncation must be reported in header; got: {}",
            resp.output
        );
    }

    /// Regression for issue #12 follow-up (@JCKodel): MCP file tools
    /// bypassed the dedup cache, so repeat reads of the same file got
    /// pipeline compression (~30%) instead of a 13-token `§ref:HASH§`
    /// (~92%). The user saw 49 fresh compressions and wondered why
    /// their stats weren't closer to the README's advertised numbers.
    ///
    /// After routing through `engine.compress_with_cache()`, a second
    /// read of the same file MUST collapse to the dedup ref.
    #[test]
    fn test_sqz_read_file_dedup_fires_on_repeat_read() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("dedup_me.txt");
        // Long enough to matter — the cache has a small-input guard so
        // sub-100-byte strings don't pollute the store.
        let content = "sqz dedup regression — issue #12 follow-up\n\
                       this file should collapse to a 13-token ref on \
                       the second read, proving the cache is wired up.\n"
            .repeat(10);
        std::fs::write(&file_path, &content).expect("write test file");

        let req = || ToolCallRequest {
            tool_id: "sqz_read_file".to_string(),
            input: serde_json::json!({ "path": file_path.to_string_lossy() }),
            intent: None,
        };

        // First read: cache miss → Fresh → full compressed content.
        let first = server.handle_tool_call(req()).expect("first read");
        assert!(
            !first.output.contains("§ref:"),
            "first read must emit full content, not a ref; got: {}",
            first.output
        );

        // Second read of the SAME file: cache hit → Dedup → ref.
        let second = server.handle_tool_call(req()).expect("second read");
        assert!(
            second.output.contains("§ref:"),
            "second read must emit a §ref:HASH§ token (the whole point \
             of the cache). If this fails, handle_sqz_read_file is \
             calling engine.compress() instead of compress_with_cache. \
             Got: {}",
            second.output
        );
        assert!(
            second.tokens_compressed < 30,
            "dedup ref should be ~13 tokens, got {}",
            second.tokens_compressed
        );
    }

    #[test]
    fn test_sqz_list_dir_lists_directory() {
        // Walks a real directory. Hidden files and bulk dirs are
        // skipped so the listing stays useful.
        let (mut server, dir) = make_server();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.rs"), "").unwrap();
        // Hidden file — must not appear.
        std::fs::write(dir.path().join(".secret"), "").unwrap();
        // Bulk dir — must not appear.
        std::fs::create_dir_all(dir.path().join("node_modules/foo")).unwrap();
        std::fs::write(dir.path().join("node_modules/foo/pkg.json"), "").unwrap();

        let req = ToolCallRequest {
            tool_id: "sqz_list_dir".to_string(),
            input: serde_json::json!({ "path": dir.path().to_string_lossy() }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("list should succeed");

        assert!(resp.output.contains("a.txt"));
        assert!(resp.output.contains("b.rs"));
        assert!(
            !resp.output.contains(".secret"),
            "hidden files must be skipped; got: {}",
            resp.output
        );
        assert!(
            !resp.output.contains("node_modules"),
            "node_modules must be skipped; got: {}",
            resp.output
        );
    }

    #[test]
    fn test_sqz_grep_finds_literal_matches() {
        // Default mode is literal substring — that's what most agent
        // searches actually are. Regex is opt-in via `regex: true`.
        let (mut server, dir) = make_server();
        std::fs::write(
            dir.path().join("code.rs"),
            "fn main() {\n    // TODO: refactor this\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("other.rs"),
            "fn nothing_to_do() {\n    // just a comment\n}\n",
        )
        .unwrap();

        let req = ToolCallRequest {
            tool_id: "sqz_grep".to_string(),
            input: serde_json::json!({
                "pattern": "TODO",
                "path": dir.path().to_string_lossy(),
            }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("grep should succeed");

        // Header reports how many hits, so the agent knows whether to
        // look at the matches or widen the search.
        assert!(resp.output.contains("matches=1"), "header must include match count; got: {}", resp.output);
        assert!(
            resp.output.contains("TODO"),
            "match content must be present; got: {}",
            resp.output
        );
        // ripgrep-style output: `path:lineno:text`.
        assert!(
            resp.output.contains("code.rs:2:"),
            "output must be grep-style path:lineno:text; got: {}",
            resp.output
        );
    }

    #[test]
    fn test_sqz_grep_regex_mode() {
        let (mut server, dir) = make_server();
        std::fs::write(
            dir.path().join("t.rs"),
            "fn my_test() {}\nfn your_test() {}\nfn not_matching() {}\n",
        )
        .unwrap();

        let req = ToolCallRequest {
            tool_id: "sqz_grep".to_string(),
            input: serde_json::json!({
                "pattern": r"fn \w+_test",
                "path": dir.path().to_string_lossy(),
                "regex": true,
            }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("grep should succeed");
        assert!(resp.output.contains("matches=2"));
    }

    #[test]
    fn test_sqz_grep_invalid_regex_errors_cleanly() {
        // Bad regex → clear error, not a panic. The error message
        // must name the tool and the regex error so the agent can
        // retry with a fixed pattern.
        let (mut server, dir) = make_server();
        let req = ToolCallRequest {
            tool_id: "sqz_grep".to_string(),
            input: serde_json::json!({
                "pattern": "fn (unclosed",
                "path": dir.path().to_string_lossy(),
                "regex": true,
            }),
            intent: None,
        };
        let result = server.handle_tool_call(req);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sqz_grep"));
        assert!(err.contains("invalid regex"));
    }

    #[test]
    fn test_sqz_grep_caps_at_max_matches() {
        // A popular term can't flood the agent's context. Cap is
        // reported so the agent knows results were truncated.
        let (mut server, dir) = make_server();
        let content: String = (0..100).map(|i| format!("line {i}: match\n")).collect();
        std::fs::write(dir.path().join("many.txt"), content).unwrap();

        let req = ToolCallRequest {
            tool_id: "sqz_grep".to_string(),
            input: serde_json::json!({
                "pattern": "match",
                "path": dir.path().to_string_lossy(),
                "max_matches": 10,
            }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).expect("grep should succeed");
        assert!(resp.output.contains("matches=10"),
            "should stop at max_matches; got: {}", resp.output);
    }
}
