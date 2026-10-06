//! # sqz-mcp
//!
//! MCP (Model Context Protocol) context-compression server and proxy for AI
//! coding agents. A thin adapter over [`sqz_engine`] that compresses tool
//! results before they enter the model's context window: deterministic, no
//! LLM calls, offline, and every compressed result is recoverable byte-exact.
//!
//! Works with any MCP client: Claude Code, Cursor, Windsurf, Cline, Gemini
//! CLI, Kiro, OpenCode, Codex CLI, Zed, Copilot CLI. `sqz init` (from the
//! `sqz-cli` crate) registers it for every client it finds. Published on the
//! official MCP Registry as `io.github.ojuschugh1/sqz`.
//!
//! ## Tools
//!
//! | Tool | Purpose |
//! |---|---|
//! | `sqz_read_file` | Read a file faithfully; a repeat read of unchanged content returns a ~13-token `§ref:HASH§`, a re-read after a small edit returns a line-level delta. Output is capped at 256 KB, cut on a line boundary, with `continue_with_offset` in the header |
//! | `sqz_grep` | Literal or regex search, `path:lineno:text` output, capped at `max_matches` hits and 100 KB; lines over 400 chars are clipped around the hit |
//! | `sqz_list_dir` | Directory listing that skips `.git`, `node_modules`, `target`, `dist`, `build`, `vendor`; capped at 1000 entries |
//! | `compress` | Run arbitrary text through the full sqz pipeline (per-command formatters, log folding, JSON compaction, session dedup) |
//! | `passthrough` | Return text unchanged (escape hatch for models that cannot parse `§ref§` tokens) |
//! | `expand` | Resolve a `§ref:HASH§` token or hex prefix to the original bytes |
//! | `sqz_recall` | Full-text search over everything sqz has compressed in past sessions |
//!
//! The read, grep and list tools are lossless: their savings come from the
//! dedup cache, not from dropping content. When a size cap cuts output the
//! header says so.
//!
//! ## Proxy: compress any other MCP server
//!
//! `sqz-mcp proxy -- <upstream command...>` wraps a stdio MCP server and, on
//! the way back to the client, compresses `tools/call` text results, compacts
//! `tools/list` descriptions, and injects an `sqz_expand` tool for recovery.
//! `--lazy-tools` shortens every description to one sentence and adds an
//! `sqz_tool_help` tool that serves the full documentation on demand;
//! `--no-desc` keeps descriptions verbatim; `--no-cache` disables dedup refs.
//! Error results, non-text content and every client-to-server message pass
//! through untouched. See [`proxy`].
//!
//! ## Running the server
//!
//! ```text
//! sqz-mcp                                   # stdio, the default for MCP clients
//! sqz-mcp --transport sse --port 3002       # SSE for network use
//! sqz-mcp --preset-dir ~/.sqz/presets       # hot-reloaded .toml presets
//! sqz-mcp proxy --lazy-tools -- npx -y @modelcontextprotocol/server-github
//! ```
//!
//! Set `SQZ_DB_PATH` to keep the dedup cache and stats per project instead of
//! in `~/.sqz/sessions.db`.
//!
//! ## MCP protocol
//!
//! - `initialize` — returns server capabilities
//! - `tools/list` — returns registered tools (optionally filtered by intent;
//!   with an `intent` parameter the built-in [`ToolSelector`] ranks tools by
//!   similarity and returns the top matches)
//! - `tools/call` — dispatches to the tools above
//!
//! ## Further reading
//!
//! - [MCP context compression guide](https://github.com/ojuschugh1/sqz/blob/main/docs/mcp-context-compression.md)
//! - [Quality benchmark: compression ratio vs information preservation](https://github.com/ojuschugh1/sqz/blob/main/docs/quality-benchmark.md)
//! - [How to stop AI coding agents from re-reading the same files](https://github.com/ojuschugh1/sqz/blob/main/docs/stop-rereading-files.md)
//! - [Repository](https://github.com/ojuschugh1/sqz)


pub mod proxy;
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
            "sqz_recall" => self.handle_sqz_recall(request),
            "sqz_read_file" => self.handle_sqz_read_file(request),
            "sqz_grep" => self.handle_sqz_grep(request),
            "sqz_list_dir" => self.handle_sqz_list_dir(request),
            // Default and explicit "compress": run the pipeline.
            _ => self.handle_compress(request),
        }
    }

    /// Log a compression event to the session store so `sqz gain` and
    /// `sqz stats` reflect MCP-path savings.
    fn log_compression(&self, tool_id: &str, tokens_original: u32, tokens_compressed: u32) {
        let project = std::env::current_dir().ok();
        let project_str = project.as_ref().map(|p| p.to_string_lossy().to_string());
        let _ = self.engine.session_store().log_compression_with_project(
            tokens_original,
            tokens_compressed,
            &[],
            tool_id,
            project_str.as_deref(),
        );
    }

    /// Compress pipeline — the historical default.
    ///
    /// Routes through the dedup cache so repeat calls with identical
    /// input return a 13-token `§ref:HASH§` instead of re-compressing.
    /// Same rationale as `handle_sqz_read_file`: the README's headline
    /// "92% saved on repeated reads" number lives in the cache, so
    /// every MCP tool that accepts arbitrary content should consult it.
    fn handle_compress(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        // The schema's `text` field; anything else is compressed serialized.
        let raw_input = match request.input.get("text").and_then(|v| v.as_str()) {
            Some(text) => text.to_string(),
            None => serde_json::to_string(&request.input)
                .map_err(|e| SqzError::Other(format!("input serialization error: {e}")))?,
        };

        let compressed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tokens_original = self.engine.count_tokens(&raw_input);
            self.compress_cached(&raw_input, tokens_original, false, true)
                .map(|(output, tokens_compressed)| (output, tokens_original, tokens_compressed))
        }));
        let (output, tokens_original, tokens_compressed) = match compressed {
            Ok(result) => result?,
            Err(_) => {
                self.engine.clear_poison();
                let tokens = estimate_tokens(&raw_input);
                (raw_input, tokens, tokens)
            }
        };
        self.log_compression(&request.tool_id, tokens_original, tokens_compressed);

        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output,
            tokens_original,
            tokens_compressed,
        })
    }

    /// Full-text search over everything that flowed through sqz this
    /// session (and earlier ones): cached command output and session
    /// summaries. BM25-ranked. Each `output` hit carries a ref the agent
    /// can pass to `expand` for the full original — so re-finding
    /// something already seen costs a snippet plus a ~13-token ref
    /// instead of re-running the command or re-reading the file.
    fn handle_sqz_recall(&mut self, request: ToolCallRequest) -> Result<ToolCallResponse> {
        let query = request
            .input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SqzError::Other("sqz_recall: input must be { \"query\": \"<terms>\" }".to_string())
            })?;
        let limit = request
            .input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, 25) as u32)
            .unwrap_or(5);

        let hits = self.engine.session_store().recall_search(query, limit)?;
        let mut output = if hits.is_empty() {
            format!("[sqz:recall no matches for \"{query}\"]")
        } else {
            let mut out = format!("[sqz:recall query=\"{query}\" hits={}]\n", hits.len());
            let mut any_output_kind = false;
            for (i, hit) in hits.iter().enumerate() {
                let snippet = hit.snippet.replace('\n', " ");
                if hit.kind == "output" {
                    any_output_kind = true;
                    out.push_str(&format!(
                        "{}. [{} {}] ref={} — {}\n",
                        i + 1,
                        hit.kind,
                        hit.created_at,
                        &hit.ref_hash[..hit.ref_hash.len().min(16)],
                        snippet,
                    ));
                } else {
                    out.push_str(&format!(
                        "{}. [{} {}] id={} — {}\n",
                        i + 1,
                        hit.kind,
                        hit.created_at,
                        hit.ref_hash,
                        snippet,
                    ));
                }
            }
            if any_output_kind {
                out.push_str("Pass a ref to the expand tool for the full original content.\n");
            }
            out
        };
        if output.ends_with('\n') {
            output.pop();
        }
        let tokens = estimate_tokens(&output);
        Ok(ToolCallResponse {
            tool_id: request.tool_id,
            output,
            tokens_original: tokens,
            tokens_compressed: tokens,
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
        // Accepts the bare prefix, the pasted `§ref:…§` token, and the
        // ranged `§ref:…:L40-80§` form (returns just those lines).
        let (prefix, range) = sqz_engine::parse_ref_token(raw);

        let result = self.engine.cache_manager().expand_ref(raw)?;
        let output = match result {
            Some(sqz_engine::ExpandResult::Original { bytes, hash }) => {
                // Best-effort UTF-8 conversion. Non-UTF-8 bytes become
                // the replacement character in the MCP response — this
                // is a necessary concession because JSON-RPC strings
                // must be UTF-8. Agents that truly need binary-safe
                // output should use the CLI's `sqz expand` which writes
                // raw bytes to stdout.
                let as_text = String::from_utf8_lossy(&bytes).into_owned();
                match range {
                    Some((a, b)) => format!("[sqz:expand hash={hash} lines={a}-{b}]\n{as_text}"),
                    None => format!("[sqz:expand hash={hash}]\n{as_text}"),
                }
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
    ///     compression for iterated edits. Only with `allow_delta`.
    ///   * `Fresh` → cache miss; emit the full compressed output.
    ///
    /// `text` of at most `REF_TOKENS` tokens skips the cache.
    ///
    /// Added in response to issue #12 follow-up: `handle_sqz_read_file`
    /// (and the grep / list_dir siblings) were calling
    /// `engine.compress()` directly, bypassing the cache entirely.
    /// Repeat reads got pipeline compression (~30%), not dedup refs
    /// (~92%). Routing through `engine.compress_with_cache()` restores
    /// the advertised behaviour.
    fn compress_cached(&self, text: &str, tokens: u32, lossless: bool, allow_delta: bool) -> Result<(String, u32)> {
        use sqz_engine::CacheResult;
        #[cfg(test)]
        tests::maybe_panic();
        if tokens <= REF_TOKENS {
            return self.compress_fresh(text, lossless);
        }
        // The file-read tools (sqz_read_file / sqz_grep / sqz_list_dir) read
        // faithfully (lossless); the general `compress` tool stays aggressive.
        let result = if lossless {
            self.engine.compress_with_cache_lossless(text)?
        } else {
            self.engine.compress_with_cache(text)?
        };
        Ok(match result {
            CacheResult::Dedup { inline_ref, token_cost } => (inline_ref, token_cost),
            CacheResult::Delta { delta_text, token_cost, .. } if allow_delta => (delta_text, token_cost),
            CacheResult::Delta { .. } => return self.compress_fresh(text, lossless),
            CacheResult::Fresh { output } => {
                let truncated = output.stages_applied.iter().any(|s| s == "entropy_truncate");
                if truncated && !self.engine.never_store(text) {
                    let hash = sqz_engine::CacheManager::sha256_hex(text.as_bytes());
                    let data = format!("{}\n[full output: call expand with prefix \"{}\"]", output.data, &hash[..16]);
                    let tokens = self.engine.count_tokens(&data);
                    (data, tokens)
                } else {
                    (output.data, output.tokens_compressed)
                }
            }
        })
    }

    /// What `compress_cached` returns on a cache miss, without the cache.
    fn compress_fresh(&self, text: &str, lossless: bool) -> Result<(String, u32)> {
        let output = if lossless {
            self.engine.compress_lossless(text)?
        } else {
            self.engine.compress(text)?
        };
        Ok((output.data, output.tokens_compressed))
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
    ///   * Absolute paths and paths through `..` are read as given;
    ///     there is no traversal guard. Path-traversal protection for
    ///     MCP is the host's job (its approval of the tool call), not
    ///     ours.
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

        // Cap on the bytes returned, applied after any line range so a
        // ranged read deep into a large file still works. The default is
        // roughly what hosts' own read tools allow; agents can raise it.
        let max_bytes = cap_from(&request.input, "max_bytes", READ_DEFAULT_MAX_BYTES);

        let path = std::path::PathBuf::from(path_str);
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > READ_HARD_LIMIT_BYTES {
                return Err(SqzError::Other(format!(
                    "sqz_read_file: '{}' is {} MB, too large to load; use head/sed/tail through the shell for files this size",
                    path.display(),
                    meta.len() / (1024 * 1024)
                )));
            }
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                return Err(SqzError::Other(format!(
                    "sqz_read_file: could not read '{}': {e}",
                    path.display()
                )))
            }
        };

        // UTF-8 with lossy fallback for non-text files — JSON-RPC
        // requires strings, so there's no byte-exact path here. Hosts
        // that care about binary safety should use their native tool.
        let full_text = String::from_utf8_lossy(&bytes).into_owned();

        // Optional line range, 1-based like the host's Read tool. The slice
        // is cut from the full text by byte offset so it is a verbatim
        // substring: if the whole file was read earlier in the session the
        // cache answers with a §ref:HASH:L<a>-<b>§ instead of the lines.
        let offset = request.input.get("offset").and_then(|v| v.as_u64()).map(|v| v.max(1) as usize);
        let limit = request.input.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
        let total_lines = full_text.lines().count();
        let (raw_text, range) = match (offset, limit) {
            (None, None) => (full_text, None),
            (offset, limit) => {
                let start = offset.unwrap_or(1);
                let end = match limit {
                    Some(n) => start.saturating_add(n).saturating_sub(1).min(total_lines),
                    None => total_lines,
                };
                if start > total_lines {
                    return Err(SqzError::Other(format!(
                        "sqz_read_file: offset {start} is past the end of '{}' ({total_lines} lines)",
                        path.display()
                    )));
                }
                (line_range(&full_text, start, end).to_string(), Some((start, end)))
            }
        };

        // Truncate on a line boundary so the returned text is whole lines
        // the next ranged read can continue from.
        let first_line = range.map(|(a, _)| a).unwrap_or(1);
        let mut shown = range;
        let mut cut = None;
        let raw_text = match truncate_to_lines(&raw_text, max_bytes) {
            Some(kept) => {
                let last = first_line + kept.lines().count().max(1) - 1;
                shown = Some((first_line, last));
                cut = Some((last, !kept.ends_with('\n')));
                kept.to_string()
            }
            None => raw_text,
        };

        let tokens_original = self.engine.count_tokens(&raw_text);
        let (compressed_data, tokens_compressed) =
            self.compress_cached(&raw_text, tokens_original, true, range.is_none())?;

        // Attach a small header so the agent knows where the content
        // came from and whether it was truncated. The header lives
        // outside the compressed body so it's not lost to any stage.
        let mut header = format!("[sqz_read_file path={} size={}", path.display(), bytes.len());
        if let Some((a, b)) = shown {
            header.push_str(&format!(" lines={a}-{b} of {total_lines}"));
        }
        if let Some((last, mid_line)) = cut {
            header.push_str(&format!(" truncated_to={max_bytes}"));
            if mid_line {
                header.push_str(&format!(" line_cut={last}"));
            }
            if last < total_lines {
                header.push_str(&format!(" continue_with_offset={}", last + 1));
            }
        }
        let output = format!("{header}]\n{compressed_data}");

        self.log_compression(&request.tool_id, tokens_original, tokens_compressed);

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

        // 1 = immediate children only, as the schema says. Values below 1
        // still list the immediate children.
        let max_depth = request
            .input
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1) as usize;

        let root = std::path::PathBuf::from(path_str);
        let mut listing = Listing {
            lines: Vec::new(),
            max_depth,
            max_entries: cap_from(&request.input, "max_entries", LIST_DEFAULT_MAX_ENTRIES),
            capped: false,
        };
        list_dir_recursive(&root, &root, 1, &mut listing)?;

        let raw = listing.lines.join("\n");
        let tokens_original = self.engine.count_tokens(&raw);
        let (compressed_data, tokens_compressed) = self.compress_cached(&raw, tokens_original, true, true)?;
        self.log_compression(&request.tool_id, tokens_original, tokens_compressed);

        let mut header = format!(
            "[sqz_list_dir path={} entries={}",
            root.display(),
            listing.lines.len()
        );
        if listing.capped {
            header.push_str(&format!(" stopped_at_max_entries={}", listing.max_entries));
        }
        let output = format!("{header}]\n{compressed_data}");

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

        let limits = GrepLimits {
            max_matches: request
                .input
                .get("max_matches")
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize,
            max_line_chars: cap_from(&request.input, "max_line_chars", GREP_DEFAULT_MAX_LINE_CHARS),
            max_bytes: cap_from(&request.input, "max_bytes", GREP_DEFAULT_MAX_BYTES),
        };

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

        let mut found = GrepResults::default();
        grep_walk(&root, pattern, regex.as_ref(), &limits, &mut found)?;

        let raw = found.lines.join("\n");
        let tokens_original = self.engine.count_tokens(&raw);
        let (compressed_data, tokens_compressed) = self.compress_cached(&raw, tokens_original, true, true)?;
        self.log_compression(&request.tool_id, tokens_original, tokens_compressed);

        let mut header = format!(
            "[sqz_grep pattern={:?} root={} matches={}",
            pattern,
            root.display(),
            found.lines.len(),
        );
        if found.lines.len() >= limits.max_matches {
            header.push_str(&format!(" max_matches_reached={}", limits.max_matches));
        }
        if found.byte_capped {
            header.push_str(&format!(" stopped_at_max_bytes={}", limits.max_bytes));
        }
        if found.clipped > 0 {
            header.push_str(&format!(
                " long_lines_clipped={} (max_line_chars={})",
                found.clipped, limits.max_line_chars
            ));
        }
        let output = format!("{header}]\n{compressed_data}");

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
        let mut input = stdin.lock();
        let mut buf = Vec::new();

        while read_line_bytes(&mut input, &mut buf)
            .map_err(|e| SqzError::Other(format!("stdin read error: {e}")))?
        {
            let response = match std::str::from_utf8(&buf) {
                Ok(line) if line.trim().is_empty() => continue,
                // Notifications (no id) return None and must produce no output.
                Ok(line) => match self.handle_jsonrpc_line(line) {
                    Some(response) => response,
                    None => continue,
                },
                Err(e) => JsonRpcResponse::err(None, -32700, format!("parse error: {e}")),
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

        let id = req.id.clone();
        let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match req.method.as_str() {
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

            "ping" => JsonRpcResponse::ok(req.id, serde_json::json!({})),

            _ => JsonRpcResponse::err(req.id, -32601, format!("method not found: {}", req.method)),
        }));
        Some(response.unwrap_or_else(|_| {
            self.engine.clear_poison();
            JsonRpcResponse::err(id, -32603, format!("internal error: {} panicked", req.method))
        }))
    }
}

/// Read one line into `buf` without its `\n` or `\r\n`, as `BufRead::lines`
/// does, but as bytes: a line that is not UTF-8 is left to the caller
/// instead of ending the stream. `false` at end of input.
fn read_line_bytes(reader: &mut impl BufRead, buf: &mut Vec<u8>) -> std::io::Result<bool> {
    buf.clear();
    if reader.read_until(b'\n', buf)? == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    Ok(true)
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
            description: "Compress text or JSON you already have, such as a \
                long tool result, through the sqz pipeline. It reads no files \
                and runs no commands."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text or JSON to compress."
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
                "entropy_truncate: low-information segments are dropped and marked; the output then ends with the expand prefix of the full original".to_string(),
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
            description: "Return the text unchanged, for when you need it \
                raw rather than compressed."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to return as is."
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
            description: "Return the original content behind a ref. \
                `§ref:HASH§` stands for content sent earlier in the session; \
                `§ref:HASH:L40-80§` for lines 40-80 of content sent in full, \
                and expands to just those lines."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prefix": {
                        "type": "string",
                        "description": "The ref token pasted whole, or its hex prefix."
                    }
                },
                "required": ["prefix"]
            }),
            compression_transforms: vec![
                "none: returns cached original bytes".to_string(),
            ],
            ..Default::default()
        },
        // Session-corpus search. Everything that flows through sqz is
        // FTS5-indexed (command output, session summaries), so an agent
        // can re-find content it has already seen instead of re-running
        // the command or re-reading the file. Hits come back as short
        // snippets plus a ref that `expand` resolves to the full bytes.
        ToolDefinition {
            id: "sqz_recall".to_string(),
            name: "Recall (Search Session Memory)".to_string(),
            description: "Search everything sqz has seen, this session and \
                earlier, before re-running a command or re-reading a file. \
                Pass a hit's ref to `expand` for the full text."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search terms; all must match."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of hits, 1-25.",
                        "default": 5
                    }
                },
                "required": ["query"]
            }),
            compression_transforms: vec![
                "bm25_rank: hits ordered by FTS5 relevance".to_string(),
                "snippet: ~24 tokens of context per hit with >>match<< markers".to_string(),
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
            description: "Read a file faithfully, stripping only ANSI \
                escapes. Prefer it over the built-in read for files over \
                2KB or ones you may read again: a repeat or ranged re-read \
                returns a short ref (see `expand`) and a re-read after an \
                edit returns only the changed lines."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path, absolute or relative to the working directory."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "First line to return, 1-based."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Number of lines to return."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Byte cap, cut at a line boundary; 0 for no cap. When cut, the header gives continue_with_offset.",
                        "default": 262144
                    }
                },
                "required": ["path"]
            }),
            compression_transforms: vec![
                "sha256_cache: repeat reads of unchanged content return a ~13-token §ref:HASH§ token".to_string(),
                "slice_ref: a line range of content already read in full returns §ref:HASH:L<a>-<b>§".to_string(),
                "delta: a re-read after a small edit returns only the changed lines".to_string(),
                "ansi_strip: removes color/formatting codes".to_string(),
                "max_bytes: output past the cap is cut at a line boundary and the header says where to continue".to_string(),
                "lossless: no lines are folded or summarized".to_string(),
            ],
            ..Default::default()
        },
        ToolDefinition {
            id: "sqz_list_dir".to_string(),
            name: "List Directory (Compressed)".to_string(),
            description: "List a directory, skipping hidden entries and \
                node_modules, target, dist, build, vendor and __pycache__. \
                Prefer it over `ls -la` for a project layout."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to list.",
                        "default": "."
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Levels to recurse; 1 lists direct children only.",
                        "default": 1
                    },
                    "max_entries": {
                        "type": "integer",
                        "description": "Entry cap; 0 for no cap.",
                        "default": 1000
                    }
                }
            }),
            compression_transforms: vec![
                "skip_bulk_dirs: .git, node_modules, target, dist, build, vendor, __pycache__ omitted".to_string(),
                "max_entries: listing stops at the cap (default 1000); the header says so".to_string(),
                "sha256_cache: repeat listings dedupe via §ref§".to_string(),
                "lossless: every listed entry survives verbatim".to_string(),
            ],
            ..Default::default()
        },
        ToolDefinition {
            id: "sqz_grep".to_string(),
            name: "Grep Files (Compressed)".to_string(),
            description: "Search files for a literal string or regex, \
                returning `path:lineno:text` lines. Prefer it over the \
                built-in grep when a search may match more than a few \
                lines: output is capped and a repeat search returns a \
                short ref."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Text to find, a literal substring unless `regex` is true."
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search.",
                        "default": "."
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "Treat `pattern` as a regex.",
                        "default": false
                    },
                    "max_matches": {
                        "type": "integer",
                        "description": "Stop after this many matches.",
                        "default": 200
                    },
                    "max_line_chars": {
                        "type": "integer",
                        "description": "Clip longer match lines around the hit; 0 for no cap.",
                        "default": 400
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Stop at this many bytes of output; 0 for no cap.",
                        "default": 100000
                    }
                },
                "required": ["pattern"]
            }),
            compression_transforms: vec![
                "max_matches: search stops after the cap (default 200); the header reports the match count".to_string(),
                "max_line_chars: a match line over 400 chars is clipped around the hit and marked".to_string(),
                "max_bytes: search stops at 100000 bytes of output; the header says so".to_string(),
                "sha256_cache: repeat searches dedupe via §ref§".to_string(),
                "lossless: match lines under the caps survive verbatim".to_string(),
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

/// Default cap on the text `sqz_read_file` returns. In line with hosts'
/// own read tools; a whole 1 MB file is ~250K tokens, more than most
/// context windows.
const READ_DEFAULT_MAX_BYTES: usize = 256 * 1024;
/// Files above this are refused rather than loaded into memory.
const READ_HARD_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
/// Default cap on the total text `sqz_grep` returns.
const GREP_DEFAULT_MAX_BYTES: usize = 100_000;
/// Default cap on one `sqz_grep` match line. Minified bundles and data
/// files put whole megabytes on one line.
const GREP_DEFAULT_MAX_LINE_CHARS: usize = 400;
/// Default cap on `sqz_list_dir` entries.
const LIST_DEFAULT_MAX_ENTRIES: usize = 1000;
/// Token cost of a `§ref:HASH§`.
const REF_TOKENS: u32 = 13;

/// An optional size cap from the tool input: `default` when absent, no
/// cap when 0.
fn cap_from(input: &serde_json::Value, key: &str, default: usize) -> usize {
    match input.get(key).and_then(|v| v.as_u64()) {
        Some(0) => usize::MAX,
        Some(v) => usize::try_from(v).unwrap_or(usize::MAX),
        None => default,
    }
}

/// The longest prefix of `text` that ends on a line boundary and fits in
/// `max_bytes`, or `None` when `text` already fits. A first line longer
/// than the cap is cut at a char boundary instead, so something is
/// always returned.
fn truncate_to_lines(text: &str, max_bytes: usize) -> Option<&str> {
    if text.len() <= max_bytes {
        return None;
    }
    let mut cut = max_bytes;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    match text[..cut].rfind('\n') {
        Some(nl) => Some(&text[..=nl]),
        None => Some(&text[..cut]),
    }
}

/// Clip a long match line to a window of `max_chars` around the first hit
/// so one result can't flood the context. `hit` is the byte range of the
/// match. `None` when the line already fits.
fn clip_line(line: &str, hit: (usize, usize), max_chars: usize) -> Option<String> {
    if line.len() <= max_chars {
        return None;
    }
    let total = line.chars().count();
    if total <= max_chars {
        return None;
    }
    let (hit_start, hit_end) = hit;
    let hit_start_char = line[..hit_start].chars().count();
    let hit_chars = line[hit_start..hit_end].chars().count();
    let lead = max_chars.saturating_sub(hit_chars) / 2;
    let mut from = hit_start_char.saturating_sub(lead);
    let to = (from + max_chars).min(total);
    from = to.saturating_sub(max_chars);
    let window: String = line.chars().skip(from).take(to - from).collect();
    Some(format!(
        "{}{}{} [line clipped: {total} chars]",
        if from > 0 { "…" } else { "" },
        window,
        if to < total { "…" } else { "" },
    ))
}

/// Lines `start..=end` (1-based) of `text` as a verbatim substring,
/// trailing newline included when the file has one there.
fn line_range(text: &str, start: usize, end: usize) -> &str {
    let mut line = 1;
    let mut begin = None;
    for (i, ch) in text.char_indices() {
        if line == start && begin.is_none() {
            begin = Some(i);
        }
        if ch == '\n' {
            if line == end {
                return &text[begin.unwrap_or(i)..=i];
            }
            line += 1;
        }
    }
    begin.map(|b| &text[b..]).unwrap_or("")
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
    out: &mut Listing,
) -> Result<()> {
    if depth > out.max_depth || out.capped {
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
        if out.capped {
            break;
        }
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

        if out.lines.len() >= out.max_entries {
            out.capped = true;
            break;
        }
        out.lines.push(format!("{type_char} {}", rel.display()));

        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) && depth < out.max_depth {
            let _ = list_dir_recursive(root, &path, depth + 1, out);
        }
    }

    Ok(())
}

/// Entries collected by `list_dir_recursive`. `capped` is set only when
/// an entry was actually left out.
struct Listing {
    lines: Vec<String>,
    max_depth: usize,
    max_entries: usize,
    capped: bool,
}

/// Walk `root` looking for files containing `pattern`. Appends one line
/// per match to `out` in `path:lineno:text` format (ripgrep-style).
///
/// Stops after `max_matches` hits or `max_bytes` of output. There's no
/// point flooding the agent with thousands of hits, and if they really need
/// that many they should use the native ripgrep via Bash (which gets hooked
/// and compressed anyway).
fn grep_walk(
    root: &std::path::Path,
    needle: &str,
    regex: Option<&regex::Regex>,
    limits: &GrepLimits,
    out: &mut GrepResults,
) -> Result<()> {
    if out.full(limits) {
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
        grep_one_file(root, needle, regex, limits, out)?;
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
        if out.full(limits) {
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
                grep_walk(&path, needle, regex, limits, out)?;
            }
            Ok(_) => {
                grep_one_file(&path, needle, regex, limits, out)?;
            }
            Err(_) => continue,
        }
    }

    Ok(())
}

/// Caps for one `sqz_grep` call.
struct GrepLimits {
    max_matches: usize,
    max_line_chars: usize,
    max_bytes: usize,
}

/// Matches collected so far, with what the caps cut.
#[derive(Default)]
struct GrepResults {
    lines: Vec<String>,
    bytes: usize,
    clipped: usize,
    byte_capped: bool,
}

impl GrepResults {
    fn full(&self, limits: &GrepLimits) -> bool {
        self.byte_capped || self.lines.len() >= limits.max_matches
    }

    /// Add one match line unless it would push the output past the byte
    /// cap. The first match is always kept so a capped search says what
    /// it found.
    fn push(&mut self, line: String, clipped: bool, limits: &GrepLimits) {
        let cost = line.len() + 1;
        if !self.lines.is_empty() && self.bytes.saturating_add(cost) > limits.max_bytes {
            self.byte_capped = true;
            return;
        }
        self.bytes += cost;
        self.lines.push(line);
        if clipped {
            self.clipped += 1;
        }
    }
}

/// Search a single file line-by-line, appending matches to `out`.
/// Skips binary files (content not UTF-8) silently — the `Read` tool
/// would also struggle with those and the agent doesn't care.
fn grep_one_file(
    file: &std::path::Path,
    needle: &str,
    regex: Option<&regex::Regex>,
    limits: &GrepLimits,
    out: &mut GrepResults,
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
        if out.full(limits) {
            break;
        }
        let hit = match regex {
            Some(r) => r.find(line).map(|m| (m.start(), m.end())),
            None => line.find(needle).map(|i| (i, i + needle.len())),
        };
        if let Some(hit) = hit {
            let clipped = clip_line(line, hit, limits.max_line_chars);
            let was_clipped = clipped.is_some();
            let shown = clipped.unwrap_or_else(|| line.to_string());
            out.push(
                format!("{}:{}:{}", file.display(), lineno + 1, shown),
                was_clipped,
                limits,
            );
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

    thread_local! {
        static PANIC_NEXT_ENGINE_CALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Make the next engine call on this thread panic, standing in for an engine bug.
    pub(crate) fn panic_next_engine_call() {
        PANIC_NEXT_ENGINE_CALL.with(|p| p.set(true));
    }

    pub(crate) fn maybe_panic() {
        if PANIC_NEXT_ENGINE_CALL.with(|p| p.replace(false)) {
            panic!("injected engine panic");
        }
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

    #[test]
    fn test_compress_tool_compresses_the_text_field() {
        let (mut server, _dir) = make_server();
        let mut text: String = (0..300)
            .map(|i| format!("2026-10-01T12:00:{:02}Z INFO worker {i} finished job {i} in {} ms\n", i % 60, i % 97))
            .collect();
        text.push_str("ERROR worker 299 failed: disk full\n");
        let resp = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "compress".to_string(),
                input: serde_json::json!({ "text": text }),
                intent: None,
            })
            .unwrap();
        let head: String = resp.output.chars().take(200).collect();
        assert!(!resp.output.starts_with("TOON:{text:"), "{head}");
        assert!(resp.output.contains("disk full"), "{head}");
    }

    #[test]
    fn test_compress_tool_names_the_expand_prefix_when_it_drops_segments() {
        let (mut server, _dir) = make_server();
        let mut text = String::from("run marker for the compress spill test\n\n");
        for i in 0..12u8 {
            let word: String = std::iter::repeat((b'a' + i) as char).take(10).collect();
            text.push_str(&format!("{} filler-{i}\n\n", format!("{word} ").repeat(30)));
        }
        text.push_str("The quick brown fox jumps over the lazy dog by silver rivers today.\n");
        let call = |server: &mut McpServer, tool: &str, input: serde_json::Value| {
            server
                .handle_tool_call(ToolCallRequest { tool_id: tool.to_string(), input, intent: None })
                .unwrap()
        };

        let resp = call(&mut server, "compress", serde_json::json!({ "text": text }));
        let hash = sqz_engine::CacheManager::sha256_hex(text.as_bytes());
        assert!(resp.output.contains("low-information segments omitted"), "{}", resp.output);
        assert!(
            resp.output.ends_with(&format!("\n[full output: call expand with prefix \"{}\"]", &hash[..16])),
            "{}",
            resp.output
        );
        assert_eq!(resp.tokens_compressed, server.engine.count_tokens(&resp.output));

        let expanded = call(&mut server, "expand", serde_json::json!({ "prefix": &hash[..16] }));
        assert_eq!(expanded.output, format!("[sqz:expand hash={hash}]\n{text}"));

        let compress = default_tool_definitions().into_iter().find(|t| t.id == "compress").unwrap();
        assert!(compress.compression_transforms.iter().any(|t| t.starts_with("entropy_truncate:")));
    }

    fn high_risk_texts() -> Vec<(&'static str, String)> {
        let hex = |s: &str| sqz_engine::CacheManager::sha256_hex(s.as_bytes());
        let frames: String = (0..40)
            .map(|i| format!("  {i:>2}: myapp::scheduler::step_{i}\n             at ./src/scheduler/mod_{}.rs:{}:5\n", i % 7, 40 + 7 * i))
            .collect();
        let tables: String = ["customers", "orders", "refunds", "payments", "shipments", "invoices"]
            .iter()
            .map(|t| format!("CREATE TABLE {t} (\n    id BIGSERIAL PRIMARY KEY,\n    status TEXT NOT NULL DEFAULT 'pending',\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE INDEX idx_{t}_created_at ON {t} (created_at);\n"))
            .collect();
        let pem: String = (0..20).map(|i| hex(&format!("pem {i}")) + "\n").collect();
        let creds: String = ["stripe", "sendgrid", "github", "datadog", "sentry"]
            .iter()
            .map(|s| format!("{s}_api_key: sk_test_{}\n{s}_endpoint: https://{s}.example.com/v1\n", &hex(s)[..32]))
            .collect();
        vec![
            ("panic", format!("thread 'main' panicked at src/scheduler/queue.rs:217:31:\ncalled `Option::unwrap()` on a `None` value\nstack backtrace:\n{frames}")),
            ("migration", format!("BEGIN;\n{tables}ALTER TABLE refunds ADD COLUMN reason TEXT;\nCOMMIT;\n")),
            ("keys", format!("-----BEGIN RSA PRIVATE KEY-----\n{pem}-----END RSA PRIVATE KEY-----\n{creds}")),
        ]
    }

    #[test]
    fn test_compress_tool_returns_high_risk_text_unchanged() {
        let (mut server, _dir) = make_server();
        for (name, text) in high_risk_texts() {
            assert!(text.len() > 500, "{name}");
            for run in ["first", "second"] {
                let resp = server
                    .handle_tool_call(ToolCallRequest {
                        tool_id: "compress".to_string(),
                        input: serde_json::json!({ "text": text }),
                        intent: None,
                    })
                    .unwrap();
                assert_eq!(resp.output, text, "{name}, {run} call");
            }
            let stored = server.engine.cache_manager().check_dedup_with_meta(text.as_bytes()).unwrap();
            assert!(stored.is_none(), "{name} was stored");
        }
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
        let store = TempDir::new().expect("tempdir");
        let server = McpServer::new_with_store(dir.path(), &store.path().join("sessions.db"))
            .expect("McpServer::new_with_store");

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
        let store = TempDir::new().expect("tempdir");
        let server = McpServer::new_with_store(dir.path(), &store.path().join("sessions.db"))
            .expect("McpServer::new_with_store");
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

    /// Test a panic while compressing answers compress with the text as given.
    #[test]
    fn test_jsonrpc_compress_panic_returns_the_text() {
        let (mut server, _dir) = make_server();
        let text = "2026-10-06T12:00:00Z INFO worker finished job in 3 ms\n".repeat(40);
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "compress", "arguments": { "text": text } }
        })
        .to_string();
        panic_next_engine_call();
        let resp = server.handle_jsonrpc_line_unwrap(&call);
        assert!(resp.error.is_none(), "{:?}", resp.error);
        assert_eq!(resp.result.unwrap()["content"][0]["text"], text.as_str());

        let resp = server.handle_jsonrpc_line_unwrap(&call);
        let out = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
        assert!(out.len() < text.len(), "compression works again: {out}");
    }

    /// Test a panic in any other tool returns -32603 and the server keeps serving.
    #[test]
    fn test_jsonrpc_tool_panic_returns_an_error() {
        let (mut server, dir) = make_server();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "first line of notes\nsecond line of notes\n").unwrap();
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "sqz_read_file", "arguments": { "path": file.to_string_lossy() } }
        })
        .to_string();
        panic_next_engine_call();
        let resp = server.handle_jsonrpc_line_unwrap(&call);
        assert_eq!(resp.id, Some(serde_json::json!(2)));
        assert_eq!(resp.error.expect("an error").code, -32603);

        let resp = server.handle_jsonrpc_line_unwrap(&call);
        assert!(resp.error.is_none(), "{:?}", resp.error);
        let text = resp.result.unwrap()["content"][0]["text"].as_str().unwrap().to_string();
        assert!(text.contains("second line of notes"), "{text}");
    }

    /// Test JSON-RPC ping returns an empty result.
    #[test]
    fn test_jsonrpc_ping() {
        let (mut server, _dir) = make_server();
        let resp = server.handle_jsonrpc_line_unwrap(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);
        assert!(resp.error.is_none(), "ping should not error: {:?}", resp.error);
        assert_eq!(resp.result, Some(serde_json::json!({})));
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

    /// Test stdin lines split the way BufRead::lines splits them, as bytes.
    #[test]
    fn test_read_line_bytes_splits_like_lines() {
        let text = "a\nb\r\nc\r\r\n\r\n\n\rd\ne";
        let mut reader = text.as_bytes();
        let mut buf = Vec::new();
        let mut got = Vec::new();
        while read_line_bytes(&mut reader, &mut buf).unwrap() {
            got.push(String::from_utf8(buf.clone()).unwrap());
        }
        let want: Vec<String> = text.as_bytes().lines().map(|l| l.unwrap()).collect();
        assert_eq!(got, want);

        let mut reader: &[u8] = b"{\"text\":\"caf\xe9\"}\nnext\n";
        assert!(read_line_bytes(&mut reader, &mut buf).unwrap());
        assert_eq!(buf, b"{\"text\":\"caf\xe9\"}");
        assert!(read_line_bytes(&mut reader, &mut buf).unwrap());
        assert_eq!(buf, b"next");
        assert!(!read_line_bytes(&mut reader, &mut buf).unwrap());
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
    fn test_tool_descriptions_stay_short_and_explain_refs_once() {
        let mut total = 0;
        for tool in default_tool_definitions() {
            let params: Vec<String> = tool.input_schema["properties"]
                .as_object()
                .map(|props| {
                    props
                        .values()
                        .filter_map(|p| p["description"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            for text in params.iter().chain(std::iter::once(&tool.description)) {
                assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "{}: {text}", tool.id);
                total += text.len();
            }
            assert_eq!(
                tool.description.contains("§ref:HASH:L40-80§"),
                tool.id == "expand",
                "{}",
                tool.id
            );
        }
        assert!(total < 2000, "tool and parameter descriptions are {total} bytes");
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
    fn recall_tool_finds_previously_compressed_content() {
        let (mut server, _dir) = make_server();
        // Compress something so it lands in the cache and recall index.
        let text = format!(
            "unique-recall-marker deployment rollout failed with status 503\n{}",
            "surrounding output line with ordinary words\n".repeat(20)
        );
        let req = ToolCallRequest {
            tool_id: "compress".to_string(),
            input: serde_json::json!({ "text": text }),
            intent: None,
        };
        server.handle_tool_call(req).unwrap();

        let req = ToolCallRequest {
            tool_id: "sqz_recall".to_string(),
            input: serde_json::json!({ "query": "rollout 503" }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).unwrap();
        assert!(
            resp.output.contains("hits=") && resp.output.contains("ref="),
            "expected a ranked hit with a ref: {}",
            resp.output
        );
        assert!(resp.output.contains(">>rollout<<"), "snippet markers missing: {}", resp.output);

        // The advertised ref must expand back to the original content.
        let ref_prefix = resp
            .output
            .split("ref=")
            .nth(1)
            .unwrap()
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect::<String>();
        let req = ToolCallRequest {
            tool_id: "expand".to_string(),
            input: serde_json::json!({ "prefix": ref_prefix }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).unwrap();
        assert!(
            resp.output.contains("unique-recall-marker"),
            "expand must recover the indexed original: {}",
            resp.output
        );
    }

    #[test]
    fn recall_tool_reports_no_matches_cleanly() {
        let (mut server, _dir) = make_server();
        let req = ToolCallRequest {
            tool_id: "sqz_recall".to_string(),
            input: serde_json::json!({ "query": "nothing-indexed-yet-zzz" }),
            intent: None,
        };
        let resp = server.handle_tool_call(req).unwrap();
        assert!(resp.output.contains("no matches"), "{}", resp.output);
    }

    #[test]
    fn test_default_tools_advertise_recall() {
        let (mut server, _dir) = make_server();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let resp = server.handle_jsonrpc_line_unwrap(line);
        let tools = resp.result.unwrap().get("tools").cloned().unwrap();
        let ids: Vec<String> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();
        assert!(ids.iter().any(|n| n == "sqz_recall"), "tools: {ids:?}");
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

    fn numbered_lines(n: usize) -> String {
        (1..=n).map(|i| format!("line {i:06} of the big file\n")).collect()
    }

    fn read(server: &mut McpServer, input: serde_json::Value) -> String {
        server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input,
                intent: None,
            })
            .expect("read should succeed")
            .output
    }

    #[test]
    fn test_sqz_read_file_respects_max_bytes() {
        // Truncation lands on a line boundary and the header says where
        // to continue, so the agent can page through the rest.
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("big.txt");
        let content = numbered_lines(100);
        std::fs::write(&file_path, &content).expect("write big file");
        let line_len = "line 000001 of the big file\n".len();

        let out = read(&mut server, serde_json::json!({ "path": file_path.to_string_lossy(), "max_bytes": line_len * 3 + 5 }));
        let (header, body) = out.split_once('\n').unwrap();
        assert!(
            header.contains(&format!(" lines=1-3 of 100 truncated_to={} continue_with_offset=4]", line_len * 3 + 5)),
            "{header}"
        );
        assert_eq!(body, line_range(&content, 1, 3));

        let next = read(&mut server, serde_json::json!({ "path": file_path.to_string_lossy(), "offset": 4, "limit": 2 }));
        assert!(next.ends_with("line 000004 of the big file\nline 000005 of the big file\n"), "{next}");
    }

    #[test]
    fn test_sqz_read_file_default_cap_and_ranged_read_past_it() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("huge.log");
        let content = numbered_lines(20_000);
        assert!(content.len() > READ_DEFAULT_MAX_BYTES);
        std::fs::write(&file_path, &content).unwrap();

        let out = read(&mut server, serde_json::json!({ "path": file_path.to_string_lossy() }));
        let (header, body) = out.split_once('\n').unwrap();
        assert!(header.contains(&format!("truncated_to={READ_DEFAULT_MAX_BYTES}")), "{header}");
        assert!(body.len() <= READ_DEFAULT_MAX_BYTES && body.ends_with('\n'));
        let shown = body.lines().count();
        assert!(header.contains(&format!(" lines=1-{shown} of 20000 ")), "{header}");
        assert!(header.contains(&format!("continue_with_offset={}", shown + 1)), "{header}");

        // The cap applies after the range, so a read deep into the file works.
        let deep = read(&mut server, serde_json::json!({ "path": file_path.to_string_lossy(), "offset": 19_999 }));
        assert!(deep.contains(" lines=19999-20000 of 20000]"), "{deep}");
        assert!(!deep.contains("truncated_to"), "{deep}");
        assert!(deep.ends_with("line 019999 of the big file\nline 020000 of the big file\n"));

        // 0 lifts the cap.
        let all = read(&mut server, serde_json::json!({ "path": file_path.to_string_lossy(), "max_bytes": 0 }));
        assert!(!all.contains("truncated_to"));
    }

    #[test]
    fn test_sqz_read_file_reports_a_cut_line() {
        // A single line longer than the cap is cut mid-line; the header
        // says which line so the agent doesn't loop on the same offset.
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("min.js");
        std::fs::write(&file_path, format!("{}\nsecond line\n", "é".repeat(1000))).unwrap();

        let out = read(&mut server, serde_json::json!({ "path": file_path.to_string_lossy(), "max_bytes": 101 }));
        let (header, body) = out.split_once('\n').unwrap();
        assert!(header.contains(" lines=1-1 of 2 truncated_to=101 line_cut=1 continue_with_offset=2]"), "{header}");
        assert_eq!(body, "é".repeat(50), "cut on a char boundary");
    }

    #[test]
    fn test_sqz_read_file_refuses_files_over_the_hard_limit() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("sparse.bin");
        let f = std::fs::File::create(&file_path).unwrap();
        f.set_len(READ_HARD_LIMIT_BYTES + 1).unwrap();
        let err = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input: serde_json::json!({ "path": file_path.to_string_lossy() }),
                intent: None,
            })
            .unwrap_err()
            .to_string();
        assert!(err.contains("too large") && err.contains("sparse.bin"), "{err}");
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
    fn test_sqz_read_file_never_stores_credential_mentions() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("app.log");
        let mut content: String = (0..40)
            .map(|i| format!("2026-10-05T12:00:{i:02}Z INFO  GET /healthz 200 1ms\n"))
            .collect();
        content.push_str("2026-10-05T12:30:00Z DEBUG db config host=db.internal user=app password=hunter2\n");
        std::fs::write(&file_path, &content).expect("write test file");
        for run in ["first", "second"] {
            let resp = server
                .handle_tool_call(ToolCallRequest {
                    tool_id: "sqz_read_file".to_string(),
                    input: serde_json::json!({ "path": file_path.to_string_lossy() }),
                    intent: None,
                })
                .unwrap();
            assert!(resp.output.contains(&content), "{run} read: {}", resp.output);
        }
        let stored = server.engine.cache_manager().check_dedup_with_meta(content.as_bytes()).unwrap();
        assert!(stored.is_none());
    }

    fn python_module(fns: usize) -> String {
        (1..=fns)
            .map(|i| format!("def handler_{i:03}(request):\n    token = request.headers.get('Authorization')\n    return verify(token, scope='handler_{i:03}')\n\n"))
            .collect()
    }

    #[test]
    fn test_sqz_read_file_ranged_read_returns_lines_with_header() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("auth.py");
        let content = python_module(50);
        std::fs::write(&file_path, &content).unwrap();

        let resp = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input: serde_json::json!({ "path": file_path.to_string_lossy(), "offset": 5, "limit": 4 }),
                intent: None,
            })
            .expect("ranged read");
        let (header, body) = resp.output.split_once('\n').unwrap();
        assert!(header.ends_with(" lines=5-8 of 200]"), "{header}");
        assert_eq!(body, line_range(&content, 5, 8));
        assert_eq!(body.lines().count(), 4);
        assert!(body.starts_with("def handler_002"), "{body}");
        assert!(body.ends_with("\n\n"), "line 8 is the blank line after handler_002");

        // offset alone reads to the end; limit alone reads from the top.
        let tail = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input: serde_json::json!({ "path": file_path.to_string_lossy(), "offset": 197 }),
                intent: None,
            })
            .unwrap();
        assert!(tail.output.contains(" lines=197-200 of 200]"), "{}", tail.output);
        let head = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input: serde_json::json!({ "path": file_path.to_string_lossy(), "limit": 3 }),
                intent: None,
            })
            .unwrap();
        assert!(head.output.contains(" lines=1-3 of 200]"), "{}", head.output);
        assert!(head.output.ends_with("    return verify(token, scope='handler_001')\n"), "{}", head.output);

        let past_end = server.handle_tool_call(ToolCallRequest {
            tool_id: "sqz_read_file".to_string(),
            input: serde_json::json!({ "path": file_path.to_string_lossy(), "offset": 500 }),
            intent: None,
        });
        assert!(past_end.is_err());
    }

    #[test]
    fn test_sqz_read_file_slice_after_full_read_returns_line_range_ref() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("auth.py");
        let content = python_module(50);
        std::fs::write(&file_path, &content).unwrap();
        let hash = sqz_engine::CacheManager::sha256_hex(content.as_bytes());

        let full = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input: serde_json::json!({ "path": file_path.to_string_lossy() }),
                intent: None,
            })
            .unwrap();
        assert!(!full.output.contains("§ref:"));

        // The commenter's case: 8.5% of reads were a line range of a file
        // already read in full. Exact-hash dedup and near-duplicate
        // matching both miss this; containment catches it.
        let slice = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input: serde_json::json!({ "path": file_path.to_string_lossy(), "offset": 41, "limit": 40 }),
                intent: None,
            })
            .unwrap();
        let expected = format!("§ref:{}:L41-80§", &hash[..16]);
        assert!(slice.output.ends_with(&expected), "{}", slice.output);
        assert!(slice.output.contains(" lines=41-80 of 200]"));
        assert!(slice.tokens_compressed < 25 && slice.tokens_original > 200, "{} / {}", slice.tokens_compressed, slice.tokens_original);

        // The ranged ref expands to exactly those lines.
        let expanded = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "expand".to_string(),
                input: serde_json::json!({ "prefix": expected }),
                intent: None,
            })
            .unwrap();
        let (header, body) = expanded.output.split_once('\n').unwrap();
        assert!(header.contains("lines=41-80"), "{header}");
        assert_eq!(body, line_range(&content, 41, 80));

        // Two lines of a file are too small to be worth a ref: content comes back.
        let tiny = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_read_file".to_string(),
                input: serde_json::json!({ "path": file_path.to_string_lossy(), "offset": 1, "limit": 2 }),
                intent: None,
            })
            .unwrap();
        assert!(!tiny.output.contains("§ref:"), "{}", tiny.output);
        assert!(tiny.output.contains("def handler_001"));
    }

    #[test]
    fn test_sqz_read_file_delta_after_an_edit_numbers_lines_from_one() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("rows.txt");
        let rows: Vec<String> = (1..=60).map(|i| format!("row {i:02}\n")).collect();
        std::fs::write(&file_path, rows.concat()).unwrap();
        let path = file_path.to_string_lossy().to_string();
        read(&mut server, serde_json::json!({ "path": path }));
        let hash = sqz_engine::CacheManager::sha256_hex(rows.concat().as_bytes());

        std::fs::write(&file_path, [&rows[..2], &rows[3..]].concat().concat()).unwrap();
        let out = read(&mut server, serde_json::json!({ "path": path }));
        let expected = format!(
            "§delta:{}§\n @@ skip 1 unchanged lines @@\n row 02\n-[1 lines removed at L3]\n row 04\n @@ skip 56 unchanged lines @@",
            &hash[..16]
        );
        assert!(out.ends_with(&expected), "{out}");
    }

    #[test]
    fn test_sqz_read_file_ranged_reread_is_never_a_delta() {
        let (mut server, dir) = make_server();
        let file_path = dir.path().join("lines.txt");
        let content: String = (1..=500)
            .map(|i| format!("line {i:04} the quick brown fox needle={}\n", i % 7 == 0))
            .collect();
        std::fs::write(&file_path, &content).unwrap();
        let path = file_path.to_string_lossy().to_string();
        read(&mut server, serde_json::json!({ "path": path, "offset": 1, "limit": 50 }));

        let next = read(&mut server, serde_json::json!({ "path": path, "offset": 11, "limit": 50 }));
        let (header, body) = next.split_once('\n').unwrap();
        assert!(header.ends_with(" lines=11-60 of 500]"), "{header}");
        assert_eq!(body, line_range(&content, 11, 60));
    }

    #[test]
    fn test_content_smaller_than_a_ref_is_never_a_ref_or_delta() {
        let (mut server, dir) = make_server();
        let root = project_dir(&dir);
        std::fs::write(root.join("empty.txt"), "").unwrap();
        std::fs::write(root.join("ok.txt"), "\x1b[32mok\x1b[0m\n").unwrap();
        let compress = |server: &mut McpServer, text: &str| {
            server
                .handle_tool_call(ToolCallRequest {
                    tool_id: "compress".to_string(),
                    input: serde_json::json!({ "text": text }),
                    intent: None,
                })
                .unwrap()
                .output
        };
        for run in ["first", "second"] {
            assert_eq!(compress(&mut server, ""), "", "{run}");
            let empty = read(&mut server, serde_json::json!({ "path": root.join("empty.txt").to_string_lossy() }));
            assert!(empty.ends_with("size=0]\n"), "{run}: {empty}");
            let ok = read(&mut server, serde_json::json!({ "path": root.join("ok.txt").to_string_lossy() }));
            assert!(ok.ends_with("]\nok\n"), "{run}: {ok}");
            let none = grep(&mut server, serde_json::json!({ "pattern": "no such text", "path": root.to_string_lossy() }));
            assert!(none.ends_with("matches=0]\n"), "{run}: {none}");
        }
    }

    #[test]
    fn test_read_tools_count_tokens_with_one_tokenizer() {
        // Regression: tokens_original used chars/4 while tokens_compressed
        // came from the BPE tokenizer, so a verbatim grep result logged
        // as a negative saving.
        let (mut server, dir) = make_server();
        for i in 0..40 {
            std::fs::write(dir.path().join(format!("f{i}.rs")), "fn needle_fn() -> Result<(), Error> { Ok(()) }\n").unwrap();
        }
        let resp = server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_grep".to_string(),
                input: serde_json::json!({ "pattern": "needle_fn", "path": dir.path().to_string_lossy() }),
                intent: None,
            })
            .unwrap();
        assert!(
            resp.tokens_compressed <= resp.tokens_original,
            "verbatim result must never log as negative savings: {} -> {}",
            resp.tokens_original,
            resp.tokens_compressed
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

    fn list(server: &mut McpServer, input: serde_json::Value) -> String {
        server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_list_dir".to_string(),
                input,
                intent: None,
            })
            .expect("list should succeed")
            .output
    }

    /// A project dir next to the test store, so listings and searches
    /// don't see `sessions.db`.
    fn project_dir(dir: &TempDir) -> std::path::PathBuf {
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn test_sqz_list_dir_depth_one_is_immediate_children() {
        let (mut server, dir) = make_server();
        let root = project_dir(&dir);
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("src/nested/deep.rs"), "").unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();
        let path = root.to_string_lossy().to_string();

        let one = list(&mut server, serde_json::json!({ "path": path }));
        assert!(one.contains("entries=2]"), "{one}");
        assert!(one.contains("d src") && one.contains("f README.md"), "{one}");
        assert!(!one.contains("lib.rs"), "default depth lists immediate children only: {one}");

        let two = list(&mut server, serde_json::json!({ "path": path, "max_depth": 2 }));
        assert!(two.contains("entries=4]"), "{two}");
        let nested = format!("d {}", Path::new("src").join("nested").display());
        assert!(two.contains("lib.rs") && two.contains(&nested), "{two}");
        assert!(!two.contains("deep.rs"), "{two}");

        let zero = list(&mut server, serde_json::json!({ "path": path, "max_depth": 0 }));
        assert!(zero.contains("entries=2]"), "{zero}");
    }

    #[test]
    fn test_sqz_list_dir_max_entries() {
        let (mut server, dir) = make_server();
        let root = project_dir(&dir);
        for i in 0..12 {
            std::fs::write(root.join(format!("f{i:02}.txt")), "").unwrap();
        }
        let path = root.to_string_lossy().to_string();

        let capped = list(&mut server, serde_json::json!({ "path": path, "max_entries": 5 }));
        assert!(capped.contains("entries=5 stopped_at_max_entries=5]"), "{capped}");
        assert!(capped.contains("f04.txt") && !capped.contains("f05.txt"), "{capped}");

        // Exactly at the cap is not a cut.
        let exact = list(&mut server, serde_json::json!({ "path": path, "max_entries": 12 }));
        assert!(exact.contains("entries=12]"), "{exact}");

        // Skipped entries after the last listed one don't trip the cap.
        std::fs::create_dir(root.join("vendor")).unwrap();
        let skipped = list(&mut server, serde_json::json!({ "path": path, "max_entries": 12 }));
        assert!(skipped.contains("entries=12]"), "{skipped}");
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
        assert!(resp.output.contains("matches=10 max_matches_reached=10]"),
            "should stop at max_matches; got: {}", resp.output);
    }

    fn grep(server: &mut McpServer, input: serde_json::Value) -> String {
        server
            .handle_tool_call(ToolCallRequest {
                tool_id: "sqz_grep".to_string(),
                input,
                intent: None,
            })
            .expect("grep should succeed")
            .output
    }

    #[test]
    fn test_sqz_grep_clips_long_lines_around_the_hit() {
        // One minified line can be megabytes; the match must still show.
        let (mut server, dir) = make_server();
        let root = project_dir(&dir);
        let line = format!("{}needle_here{}", "é".repeat(3000), "ü".repeat(3000));
        std::fs::write(root.join("bundle.min.js"), format!("{line}\nshort needle_here line\n")).unwrap();
        let input = serde_json::json!({ "pattern": "needle_here", "path": root.to_string_lossy() });

        let out = grep(&mut server, input.clone());
        let (header, body) = out.split_once('\n').unwrap();
        assert!(header.contains("matches=2 long_lines_clipped=1 (max_line_chars=400)]"), "{header}");
        let first = body.lines().next().unwrap();
        let shown = first.split_once("bundle.min.js:1:").map(|(_, s)| s).unwrap();
        assert_eq!(
            shown,
            format!("…{}needle_here{}… [line clipped: 6011 chars]", "é".repeat(194), "ü".repeat(195))
        );
        assert!(body.contains("bundle.min.js:2:short needle_here line"), "{body}");

        // 0 lifts both caps. A fresh store so the reply isn't a delta
        // against the clipped result.
        let (mut fresh, _store) = make_server();
        let mut uncapped = input;
        uncapped["max_line_chars"] = 0.into();
        uncapped["max_bytes"] = 0.into();
        let whole = grep(&mut fresh, uncapped);
        assert!(!whole.contains("clipped"), "{}", whole.chars().take(200).collect::<String>());
        assert!(whole.contains(&line));
    }

    #[test]
    fn test_clip_line_windows() {
        let line: String = ('a'..='z').cycle().take(1000).collect();
        // Hit at the start: no leading ellipsis.
        let start = clip_line(&line, (0, 3), 100).unwrap();
        assert!(start.starts_with("abc") && start.contains("… [line clipped: 1000 chars]"), "{start}");
        // Hit at the end: window ends at the end of the line.
        let end = clip_line(&line, (997, 1000), 100).unwrap();
        assert!(end.starts_with('…') && end.ends_with(&format!("{} [line clipped: 1000 chars]", &line[900..])), "{end}");
        // Hit longer than the window: the window starts at the hit.
        let long = clip_line(&line, (500, 800), 100).unwrap();
        assert!(long.starts_with(&format!("…{}", &line[500..600])), "{long}");
        // Short lines are left alone.
        assert!(clip_line("short", (0, 1), 100).is_none());
    }

    #[test]
    fn test_sqz_grep_caps_total_bytes() {
        let (mut server, dir) = make_server();
        let root = project_dir(&dir);
        let content: String = (0..2000).map(|i| format!("{i:05} match {}\n", "x".repeat(200))).collect();
        std::fs::write(root.join("wide.txt"), content).unwrap();

        let out = grep(&mut server, serde_json::json!({
            "pattern": "match",
            "path": root.to_string_lossy(),
            "max_matches": 10_000
        }));
        let (header, body) = out.split_once('\n').unwrap();
        assert!(header.contains(&format!("stopped_at_max_bytes={GREP_DEFAULT_MAX_BYTES}")), "{header}");
        assert!(!header.contains("max_matches_reached"), "{header}");
        assert!(body.len() <= GREP_DEFAULT_MAX_BYTES, "{}", body.len());
        assert!(body.lines().count() > 100, "{}", body.lines().count());
    }
}
