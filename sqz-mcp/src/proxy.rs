//! MCP compression proxy: wrap any upstream MCP server and compress its
//! tool responses through the sqz engine.
//!
//! `sqz-mcp proxy -- <upstream command...>` spawns the upstream server and
//! relays newline-delimited JSON-RPC over stdio in both directions. On the
//! way back to the client it:
//!
//! - compresses `tools/call` text results (dedup `§ref` on repeats, the
//!   full pipeline on first pass, net-win gated, Safe-routed content left
//!   verbatim). Originals are persisted, so every compression is
//!   recoverable via the injected `sqz_expand` tool.
//! - compacts `tools/list` tool descriptions (whitespace + length cap,
//!   schemas untouched) unless `--no-desc`. With `--lazy-tools` the cap
//!   drops to one sentence and an injected `sqz_tool_help` tool serves
//!   the full original documentation on demand.
//! - appends a `sqz_expand` tool to the upstream's tool list and answers
//!   calls to it locally.
//! - appends a short protocol note to the `initialize` instructions so
//!   the model knows what `§ref:HASH§` means.
//!
//! Error results, non-text content, notifications, and every client→server
//! message (except `sqz_expand` calls) pass through verbatim.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use sqz_engine::{CacheResult, Result, SqzEngine, SqzError};

const EXPAND_TOOL_NAME: &str = "sqz_expand";
const HELP_TOOL_NAME: &str = "sqz_tool_help";
const NET_WIN_MIN_TOKENS: u32 = 16;
const MAX_DESCRIPTION_CHARS: usize = 400;
/// `--lazy-tools` cap: roughly one sentence per tool.
const LAZY_DESCRIPTION_CHARS: usize = 160;

const INSTRUCTIONS_NOTE: &str = "\n\nNote: tool results pass through the sqz \
compression proxy. A result of the form §ref:HASH§ means you have already \
seen this exact content earlier in the session. If you need any compressed \
or referenced result verbatim, call the sqz_expand tool with the ref token \
or hash prefix.";

const LAZY_TOOLS_NOTE: &str = " Tool descriptions are shortened to one \
sentence to save context; call the sqz_tool_help tool with a tool name to \
read the full original documentation.";

/// What kind of response we expect for an outstanding request id.
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    Initialize,
    ToolsList,
    ToolCall { name: String },
    Other,
}

struct ProxyState {
    engine: SqzEngine,
    pending: HashMap<String, Pending>,
    compress_descriptions: bool,
    no_cache: bool,
    /// `--lazy-tools`: shorten descriptions to one sentence and serve the
    /// originals on demand via the injected sqz_tool_help tool.
    lazy_tools: bool,
    /// Original tool descriptions captured from tools/list responses,
    /// keyed by tool name. Only populated in lazy mode.
    tool_docs: HashMap<String, String>,
}

impl ProxyState {
    fn id_key(id: &serde_json::Value) -> String {
        id.to_string()
    }

    /// Inspect a client→upstream message. Returns `Ok(Some(response))`
    /// when the message was handled locally (do not forward), `Ok(None)`
    /// to forward it verbatim.
    fn on_client_message(&mut self, line: &str) -> Result<Option<String>> {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            return Ok(None); // unparseable — relay untouched
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();

        if let Some(id) = id {
            let kind = match method {
                "initialize" => Pending::Initialize,
                "tools/list" => Pending::ToolsList,
                "tools/call" => {
                    let name = msg
                        .pointer("/params/name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    if name == EXPAND_TOOL_NAME {
                        return Ok(Some(self.answer_expand(&id, &msg)));
                    }
                    if name == HELP_TOOL_NAME {
                        return Ok(Some(self.answer_tool_help(&id, &msg)));
                    }
                    Pending::ToolCall { name }
                }
                _ => Pending::Other,
            };
            self.pending.insert(Self::id_key(&id), kind);
        }
        Ok(None)
    }

    /// Transform an upstream→client message. Returns the line to emit.
    fn on_upstream_message(&mut self, line: &str) -> String {
        let Ok(mut msg) = serde_json::from_str::<serde_json::Value>(line) else {
            return line.to_string();
        };
        let Some(id) = msg.get("id").cloned() else {
            return line.to_string(); // notification
        };
        let Some(kind) = self.pending.remove(&Self::id_key(&id)) else {
            return line.to_string();
        };
        if msg.get("error").is_some() {
            return line.to_string();
        }

        match kind {
            Pending::Initialize => self.rewrite_initialize(&mut msg),
            Pending::ToolsList => self.rewrite_tools_list(&mut msg),
            Pending::ToolCall { name } => self.rewrite_tool_result(&mut msg, &name),
            Pending::Other => {}
        }
        serde_json::to_string(&msg).unwrap_or_else(|_| line.to_string())
    }

    fn rewrite_initialize(&self, msg: &mut serde_json::Value) {
        let Some(result) = msg.get_mut("result") else { return };
        let existing = result
            .get("instructions")
            .and_then(|i| i.as_str())
            .unwrap_or("");
        let mut combined = format!("{existing}{INSTRUCTIONS_NOTE}");
        if self.lazy_tools {
            combined.push_str(LAZY_TOOLS_NOTE);
        }
        result["instructions"] = serde_json::Value::String(combined.trim_start().to_string());
    }

    fn rewrite_tools_list(&mut self, msg: &mut serde_json::Value) {
        let Some(tools) = msg
            .pointer_mut("/result/tools")
            .and_then(|t| t.as_array_mut())
        else {
            return;
        };
        if self.lazy_tools {
            // One sentence per tool; originals stashed for sqz_tool_help.
            // Schemas are untouched — the protocol needs them to call.
            for tool in tools.iter_mut() {
                let name = tool
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(desc) = tool.get("description").and_then(|d| d.as_str()) {
                    let brief = compact_description(desc, LAZY_DESCRIPTION_CHARS);
                    if brief.len() < desc.len() && !name.is_empty() {
                        self.tool_docs.insert(name, desc.to_string());
                        tool["description"] = serde_json::Value::String(brief);
                    }
                }
            }
        } else if self.compress_descriptions {
            for tool in tools.iter_mut() {
                if let Some(desc) = tool.get("description").and_then(|d| d.as_str()) {
                    let compact = compact_description(desc, MAX_DESCRIPTION_CHARS);
                    if compact.len() < desc.len() {
                        tool["description"] = serde_json::Value::String(compact);
                    }
                }
            }
        }
        if !tools.iter().any(|t| {
            t.get("name").and_then(|n| n.as_str()) == Some(EXPAND_TOOL_NAME)
        }) {
            tools.push(serde_json::json!({
                "name": EXPAND_TOOL_NAME,
                "description": "Expand a §ref:HASH§ token (or bare hex prefix) from a compressed tool result back to the original verbatim content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ref": { "type": "string", "description": "The §ref:HASH§ token or hex prefix to expand" }
                    },
                    "required": ["ref"]
                }
            }));
        }
        if self.lazy_tools
            && !tools.iter().any(|t| {
                t.get("name").and_then(|n| n.as_str()) == Some(HELP_TOOL_NAME)
            })
        {
            tools.push(serde_json::json!({
                "name": HELP_TOOL_NAME,
                "description": "Return the full original documentation for a tool whose description was shortened by the sqz proxy.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Tool name to look up" }
                    },
                    "required": ["name"]
                }
            }));
        }
    }

    fn rewrite_tool_result(&mut self, msg: &mut serde_json::Value, tool_name: &str) {
        let is_error = msg
            .pointer("/result/isError")
            .and_then(|e| e.as_bool())
            .unwrap_or(false);
        if is_error {
            return;
        }
        let Some(content) = msg
            .pointer_mut("/result/content")
            .and_then(|c| c.as_array_mut())
        else {
            return;
        };
        for item in content.iter_mut() {
            if item.get("type").and_then(|t| t.as_str()) != Some("text") {
                continue;
            }
            let Some(text) = item.get("text").and_then(|t| t.as_str()) else {
                continue;
            };
            if let Some(compressed) = self.compress_text(text, tool_name) {
                item["text"] = serde_json::Value::String(compressed);
            }
        }
    }

    /// Compress one text payload. Returns `None` to keep the original.
    fn compress_text(&mut self, text: &str, tool_name: &str) -> Option<String> {
        let tokens_original = estimate_tokens(text);
        if tokens_original < NET_WIN_MIN_TOKENS * 2 {
            return None;
        }
        // High-risk content (credentials, stack traces, migrations) stays
        // verbatim, matching the engine's safety routing everywhere else.
        if self.engine.route_compression_mode(text) == sqz_engine::CompressionMode::Safe {
            return None;
        }

        let result = if self.no_cache {
            self.engine.compress(text).ok().map(|c| CacheResult::Fresh { output: c })
        } else {
            self.engine.compress_with_cache(text).ok()
        }?;

        let (out, tokens_out) = match result {
            CacheResult::Dedup { inline_ref, token_cost } => (inline_ref, token_cost),
            CacheResult::Delta { delta_text, token_cost, .. } => (delta_text, token_cost),
            CacheResult::Fresh { output } => {
                // Spill ref: when entropy truncation dropped segments, the
                // untruncated original is in the cache (we're on the cached
                // path here) — point the agent at sqz_expand so truncation
                // is recoverable rather than lost.
                let truncated = output
                    .stages_applied
                    .iter()
                    .any(|s| s == "entropy_truncate");
                let mut data = output.data;
                if truncated && !self.no_cache {
                    let hash = sqz_engine::CacheManager::sha256_hex(text.as_bytes());
                    data.push_str(&format!(
                        "\n[full output: call sqz_expand with ref \"{}\"]",
                        &hash[..16]
                    ));
                }
                let t = estimate_tokens(&data);
                (data, t)
            }
        };
        if tokens_original.saturating_sub(tokens_out) < NET_WIN_MIN_TOKENS {
            return None;
        }
        let _ = self.engine.session_store().log_compression_with_project(
            tokens_original,
            tokens_out,
            &[],
            &format!("mcp-proxy:{tool_name}"),
            None,
        );
        Some(out)
    }

    fn answer_tool_help(&self, id: &serde_json::Value, msg: &serde_json::Value) -> String {
        let name = msg
            .pointer("/params/arguments/name")
            .and_then(|n| n.as_str())
            .unwrap_or("");
        let body = match self.tool_docs.get(name) {
            Some(doc) => serde_json::json!({
                "content": [{ "type": "text", "text": doc }],
                "isError": false
            }),
            None => serde_json::json!({
                "content": [{ "type": "text", "text": format!(
                    "no stored documentation for '{name}' — either its \
                     description was never shortened (already short), the \
                     name is unknown, or tools/list has not been called yet"
                ) }],
                "isError": true
            }),
        };
        serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": body }).to_string()
    }

    fn answer_expand(&self, id: &serde_json::Value, msg: &serde_json::Value) -> String {
        let raw = msg
            .pointer("/params/arguments/ref")
            .and_then(|r| r.as_str())
            .unwrap_or("");
        let prefix = raw
            .trim()
            .trim_start_matches("§ref:")
            .trim_end_matches('§')
            .trim();
        let body = match self.engine.cache_manager().expand_prefix(prefix) {
            Ok(Some(exp)) => {
                let text = match exp {
                    sqz_engine::ExpandResult::Original { bytes, .. } => {
                        String::from_utf8_lossy(&bytes).into_owned()
                    }
                    sqz_engine::ExpandResult::CompressedOnly { compressed, .. } => compressed,
                };
                serde_json::json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                })
            }
            Ok(None) => serde_json::json!({
                "content": [{ "type": "text", "text": format!("no cached content found for prefix '{prefix}'") }],
                "isError": true
            }),
            Err(e) => serde_json::json!({
                "content": [{ "type": "text", "text": format!("expand failed: {e}") }],
                "isError": true
            }),
        };
        serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": body }).to_string()
    }
}

/// Whitespace-collapse and cap a tool description at a sentence boundary.
fn compact_description(desc: &str, max_chars: usize) -> String {
    let collapsed: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= max_chars {
        return collapsed;
    }
    let head = sqz_engine::text_boundary::truncate_str(&collapsed, max_chars);
    match head.rfind(". ") {
        Some(idx) if idx > max_chars / 2 => format!("{}.", &head[..idx]),
        _ => format!("{head}…"),
    }
}

fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

/// Run the proxy: spawn `upstream_cmd` and relay stdio until EOF.
pub fn run_proxy(
    upstream_cmd: &[String],
    compress_descriptions: bool,
    no_cache: bool,
    lazy_tools: bool,
) -> Result<()> {
    if upstream_cmd.is_empty() {
        return Err(SqzError::Other(
            "proxy: no upstream command given (usage: sqz-mcp proxy -- <command...>)".into(),
        ));
    }
    let engine = SqzEngine::new()?;
    let mut child = Command::new(&upstream_cmd[0])
        .args(&upstream_cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| SqzError::Other(format!("proxy: failed to spawn upstream: {e}")))?;

    let child_stdin = child.stdin.take().ok_or_else(|| SqzError::Other("proxy: no child stdin".into()))?;
    let child_stdout = child.stdout.take().ok_or_else(|| SqzError::Other("proxy: no child stdout".into()))?;

    let state = Arc::new(Mutex::new(ProxyState {
        engine,
        pending: HashMap::new(),
        compress_descriptions,
        no_cache,
        lazy_tools,
        tool_docs: HashMap::new(),
    }));
    let child_stdin = Arc::new(Mutex::new(child_stdin));

    // Upstream → client (this thread's counterpart runs below on main).
    let state_up = Arc::clone(&state);
    let upstream_thread = std::thread::spawn(move || {
        let reader = BufReader::new(child_stdout);
        let stdout = std::io::stdout();
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let out = state_up.lock().unwrap().on_upstream_message(&line);
            let mut handle = stdout.lock();
            if writeln!(handle, "{out}").and_then(|_| handle.flush()).is_err() {
                break;
            }
        }
    });

    // Client → upstream (main thread).
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let local_response = state.lock().unwrap().on_client_message(&line)?;
        match local_response {
            Some(resp) => {
                let stdout = std::io::stdout();
                let mut handle = stdout.lock();
                if writeln!(handle, "{resp}").and_then(|_| handle.flush()).is_err() {
                    break;
                }
            }
            None => {
                let mut cs = child_stdin.lock().unwrap();
                if writeln!(cs, "{line}").and_then(|_| cs.flush()).is_err() {
                    break;
                }
            }
        }
    }

    drop(child_stdin);
    let _ = upstream_thread.join();
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> (ProxyState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("sessions.db");
        let engine =
            SqzEngine::with_preset_and_store(sqz_engine::Preset::default(), &store).unwrap();
        (
            ProxyState {
                engine,
                pending: HashMap::new(),
                compress_descriptions: true,
                no_cache: false,
                lazy_tools: false,
                tool_docs: HashMap::new(),
            },
            dir,
        )
    }

    /// Round-trip a tools/list through the proxy and return the rewritten
    /// tools array.
    fn list_tools(state: &mut ProxyState, tools: serde_json::Value) -> serde_json::Value {
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 77, "method": "tools/list" });
        assert!(state.on_client_message(&req.to_string()).unwrap().is_none());
        let resp = serde_json::json!({
            "jsonrpc": "2.0", "id": 77,
            "result": { "tools": tools }
        });
        let out: serde_json::Value =
            serde_json::from_str(&state.on_upstream_message(&resp.to_string())).unwrap();
        out.pointer("/result/tools").unwrap().clone()
    }

    fn call_and_respond(state: &mut ProxyState, id: u64, tool: &str, text: &str) -> serde_json::Value {
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": {} }
        });
        assert!(state.on_client_message(&req.to_string()).unwrap().is_none());
        let resp = serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": { "content": [{ "type": "text", "text": text }], "isError": false }
        });
        serde_json::from_str(&state.on_upstream_message(&resp.to_string())).unwrap()
    }

    #[test]
    fn lazy_tools_shortens_descriptions_and_serves_full_docs() {
        let (mut state, _dir) = test_state();
        state.lazy_tools = true;
        let long_desc = "Search issues across every project in the tracker. \
            Supports a rich query language with field filters, boolean \
            operators, ranges, and sorting. Results are paginated and can \
            be exported. Rate limits apply to unauthenticated callers, and \
            some fields require elevated permissions to read or write."
            .to_string();
        let tools = list_tools(
            &mut state,
            serde_json::json!([
                { "name": "issue_search", "description": long_desc,
                  "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } } }
            ]),
        );

        // Description shortened to roughly one sentence, schema untouched.
        let rewritten = tools[0]["description"].as_str().unwrap();
        assert!(rewritten.len() < long_desc.len());
        assert!(rewritten.len() <= LAZY_DESCRIPTION_CHARS + 4);
        assert!(tools[0].pointer("/inputSchema/properties/q").is_some());

        // Both helper tools injected.
        let names: Vec<&str> = tools
            .as_array().unwrap().iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&EXPAND_TOOL_NAME));
        assert!(names.contains(&HELP_TOOL_NAME));

        // sqz_tool_help returns the verbatim original.
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": HELP_TOOL_NAME, "arguments": { "name": "issue_search" } }
        });
        let out = state.on_client_message(&req.to_string()).unwrap().expect("answered locally");
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.pointer("/result/isError"), Some(&serde_json::json!(false)));
        assert_eq!(
            parsed.pointer("/result/content/0/text").unwrap().as_str().unwrap(),
            long_desc
        );

        // Unknown tool name: isError result, not a crash.
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": HELP_TOOL_NAME, "arguments": { "name": "nope" } }
        });
        let out = state.on_client_message(&req.to_string()).unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.pointer("/result/isError"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn lazy_tools_off_keeps_legacy_behavior() {
        let (mut state, _dir) = test_state();
        let short = "Compact enough already.".to_string();
        let tools = list_tools(
            &mut state,
            serde_json::json!([{ "name": "t1", "description": short, "inputSchema": {} }]),
        );
        // No help tool without lazy mode; short description untouched.
        let names: Vec<&str> = tools
            .as_array().unwrap().iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(!names.contains(&HELP_TOOL_NAME));
        assert_eq!(tools[0]["description"].as_str().unwrap(), short);
    }

    #[test]
    fn truncated_result_carries_expand_hint() {
        let (mut state, _dir) = test_state();
        // Entropy-truncation fixture: distinct low-entropy segments that
        // the truncator drops, plus high-entropy prose it keeps.
        let mut big = String::from("run marker for the proxy spill test\n\n");
        for i in 0..12u8 {
            let word: String = std::iter::repeat((b'a' + i) as char).take(10).collect();
            big.push_str(&format!("{} filler-{i}\n\n", format!("{word} ").repeat(30)));
        }
        big.push_str("The quick brown fox jumps over the lazy dog by silver rivers today.\n");

        let resp = call_and_respond(&mut state, 1, "logs", &big);
        let text = resp.pointer("/result/content/0/text").unwrap().as_str().unwrap();
        assert!(
            text.contains("[full output: call sqz_expand with ref \""),
            "expected sqz_expand hint on truncated result, got: {text}"
        );

        // The hinted prefix must resolve through the expand tool.
        let start = text.find("sqz_expand with ref \"").unwrap() + "sqz_expand with ref \"".len();
        let prefix: String = text[start..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        assert_eq!(prefix.len(), 16);
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "sqz_expand", "arguments": { "ref": prefix } }
        });
        let out = state.on_client_message(&req.to_string()).unwrap()
            .expect("sqz_expand is answered locally");
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let recovered = parsed.pointer("/result/content/0/text").unwrap().as_str().unwrap();
        assert_eq!(recovered, big, "expand must recover the exact original");
    }

    #[test]
    fn repeated_tool_result_becomes_ref() {
        let (mut state, _dir) = test_state();
        let big: String = (0..80)
            .map(|i| format!("issue row {i} status open priority medium assignee nobody\n"))
            .collect();
        let first = call_and_respond(&mut state, 1, "jira_search", &big);
        let first_text = first.pointer("/result/content/0/text").unwrap().as_str().unwrap();
        assert!(!first_text.starts_with("§ref:"), "first pass is not a ref");
        assert!(first_text.len() < big.len(), "first pass should compress");

        let second = call_and_respond(&mut state, 2, "jira_search", &big);
        let second_text = second.pointer("/result/content/0/text").unwrap().as_str().unwrap();
        assert!(second_text.starts_with("§ref:"), "repeat must dedup: {second_text}");
    }

    #[test]
    fn expand_recovers_original_bytes() {
        let (mut state, _dir) = test_state();
        let big: String = (0..80)
            .map(|i| format!("log line {i} with some detail that truncation might drop\n"))
            .collect();
        call_and_respond(&mut state, 1, "logs", &big);
        // Repeat to obtain the ref token.
        let second = call_and_respond(&mut state, 2, "logs", &big);
        let ref_token = second
            .pointer("/result/content/0/text")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();

        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "sqz_expand", "arguments": { "ref": ref_token } }
        });
        let resp = state.on_client_message(&req.to_string()).unwrap()
            .expect("sqz_expand must be answered locally");
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["id"], 3);
        assert_eq!(parsed["result"]["isError"], false);
        let text = parsed.pointer("/result/content/0/text").unwrap().as_str().unwrap();
        assert_eq!(text, big, "expand must return the original verbatim");
    }

    #[test]
    fn error_results_pass_verbatim() {
        let (mut state, _dir) = test_state();
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "db_query", "arguments": {} }
        });
        state.on_client_message(&req.to_string()).unwrap();
        let error_text: String = (0..60)
            .map(|i| format!("ERROR: constraint violation on row {i} in table users\n"))
            .collect();
        let resp = serde_json::json!({
            "jsonrpc": "2.0", "id": 5,
            "result": { "content": [{ "type": "text", "text": error_text }], "isError": true }
        });
        let out = state.on_upstream_message(&resp.to_string());
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            parsed.pointer("/result/content/0/text").unwrap().as_str().unwrap(),
            error_text
        );
    }

    #[test]
    fn tiny_results_pass_verbatim() {
        let (mut state, _dir) = test_state();
        let out = call_and_respond(&mut state, 7, "ping", "pong: ok");
        assert_eq!(
            out.pointer("/result/content/0/text").unwrap().as_str().unwrap(),
            "pong: ok"
        );
    }

    #[test]
    fn tools_list_gets_expand_tool_and_compact_descriptions() {
        let (mut state, _dir) = test_state();
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/list" });
        state.on_client_message(&req.to_string()).unwrap();

        let long_desc = "This    tool   searches issues. ".repeat(40);
        let resp = serde_json::json!({
            "jsonrpc": "2.0", "id": 9,
            "result": { "tools": [
                { "name": "search", "description": long_desc, "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } } }
            ]}
        });
        let out: serde_json::Value =
            serde_json::from_str(&state.on_upstream_message(&resp.to_string())).unwrap();
        let tools = out.pointer("/result/tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 2, "sqz_expand appended");
        assert_eq!(tools[1]["name"], EXPAND_TOOL_NAME);
        let desc = tools[0]["description"].as_str().unwrap();
        assert!(desc.len() <= MAX_DESCRIPTION_CHARS + 4, "description capped: {}", desc.len());
        assert!(!desc.contains("    "), "whitespace collapsed");
        // Schema untouched.
        assert!(tools[0].pointer("/inputSchema/properties/q").is_some());
    }

    #[test]
    fn initialize_instructions_note_appended() {
        let (mut state, _dir) = test_state();
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        state.on_client_message(&req.to_string()).unwrap();
        let resp = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "protocolVersion": "2025-06-18", "capabilities": {}, "serverInfo": { "name": "up" }, "instructions": "Upstream notes." }
        });
        let out: serde_json::Value =
            serde_json::from_str(&state.on_upstream_message(&resp.to_string())).unwrap();
        let instr = out.pointer("/result/instructions").unwrap().as_str().unwrap();
        assert!(instr.starts_with("Upstream notes."));
        assert!(instr.contains("sqz_expand"));
    }

    #[test]
    fn notifications_and_unknown_ids_pass_verbatim() {
        let (mut state, _dir) = test_state();
        let notif = r#"{"jsonrpc":"2.0","method":"notifications/progress","params":{"progress":5}}"#;
        assert_eq!(state.on_upstream_message(notif), notif);
        let unknown = r#"{"jsonrpc":"2.0","id":999,"result":{"content":[{"type":"text","text":"x"}]}}"#;
        assert_eq!(state.on_upstream_message(unknown), unknown);
    }

    #[test]
    fn safe_routed_content_stays_verbatim() {
        let (mut state, _dir) = test_state();
        let mut secret = String::from("deployment configuration follows for the api service\n");
        for i in 0..30 {
            secret.push_str(&format!("api_key = sk-live-{i:032}\npassword: hunter{i}\n"));
        }
        let out = call_and_respond(&mut state, 11, "read_config", &secret);
        assert_eq!(
            out.pointer("/result/content/0/text").unwrap().as_str().unwrap(),
            secret,
            "high-risk content must not be compressed"
        );
    }

    #[test]
    fn string_ids_are_correlated() {
        let (mut state, _dir) = test_state();
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": "abc-1", "method": "tools/call",
            "params": { "name": "t", "arguments": {} }
        });
        state.on_client_message(&req.to_string()).unwrap();
        assert!(state.pending.contains_key("\"abc-1\""));
    }
}
