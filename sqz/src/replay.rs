//! `sqz discover --replay` — counterfactual replay of real agent
//! transcripts through the current sqz engine.
//!
//! The chicken-and-egg problem with "install sqz and see what it saves"
//! is that sqz cannot observe agent sessions until it is wired in. But the
//! agent clients already keep transcripts on disk (Claude Code under
//! `~/.claude/projects`, the Kiro CLI under `~/.kiro/sessions/cli`, the
//! Kiro IDE under `~/.kiro/sessions/<workspace>/<session>/`), and those
//! contain every tool output the model was sent. Replaying them answers,
//! on the user's own data: "what would sqz have done?"
//!
//! Each output is replayed the way sqz could actually have reached it on
//! that client, not all through one pipeline:
//! - shell output goes through the same path as the shell hook, split on
//!   transparent clients into commands the hook rewrites and ones it skips;
//! - built-in file reads, searches and listings: the sqz MCP tools that
//!   replace them are lossless, so only dedup counts;
//! - other MCP servers' results get exactly what `sqz-mcp proxy` does;
//! - web fetches, edits, sub-agent results and the client's own tools are
//!   out of reach on every client, so they are counted, not replayed.
//!
//! Honesty rules, enforced by the wording in [`render`]:
//! - These are **counterfactual estimates**, not measurements. The
//!   outputs were not compressed at the time.
//! - Each transcript file replays against a **fresh throwaway store**, so
//!   dedup references never pretend to span sessions.
//! - Replay compresses at replay-time timestamps, so ref expiry inside a
//!   very long real session is not simulated; repeats are assumed to
//!   stay within the freshness window.
//! - Nothing leaves the machine. The throwaway stores are deleted.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

use sqz_engine::{CacheResult, IntegrationTier};

use crate::cli_proxy::{CliProxy, InterceptOptions};

/// Transcript dialects sqz can replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    /// Claude Code: `{"type":"assistant"|"user","message":{"content":[...]}}`
    Claude,
    /// Kiro CLI: `{"version":"v1","kind":"AssistantMessage"|"ToolResults","data":{...}}`
    Kiro,
    /// Kiro IDE: `{"id":…,"timestamp":…,"payload":{"type":"tool_call"|"tool_result",…}}`
    KiroIde,
}

impl TranscriptFormat {
    pub fn client(self) -> Client {
        match self {
            TranscriptFormat::Claude => Client::ClaudeCode,
            TranscriptFormat::Kiro => Client::KiroCli,
            TranscriptFormat::KiroIde => Client::KiroIde,
        }
    }
}

/// The agent client a transcript came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Client {
    ClaudeCode,
    KiroIde,
    KiroCli,
}

impl Client {
    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::KiroIde => "Kiro IDE",
            Client::KiroCli => "Kiro CLI",
        }
    }

    pub fn tier(self) -> IntegrationTier {
        sqz_engine::integration_tier(match self {
            Client::ClaudeCode => "Claude Code",
            Client::KiroIde | Client::KiroCli => "Kiro",
        })
    }
}

/// How sqz could have reached one tool output on the client that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reach {
    /// Shell command the client's hook rewrites: compressed with no help
    /// from the agent.
    ShellHooked,
    /// Shell command sqz only sees if the agent pipes it through
    /// `sqz compress`.
    Shell,
    /// Built-in file read, search or listing. The sqz MCP tools that
    /// replace them are lossless, so only dedup counts.
    FileRead,
    /// Another MCP server's tool, reachable by running the server behind
    /// `sqz-mcp proxy`.
    Mcp,
    /// Output that already went through sqz.
    AlreadySqz,
    /// Web fetch and search, edits, sub-agent results, the client's own
    /// tools: nothing sqz can intercept.
    Unreachable,
}

impl Reach {
    /// Whether sqz could have saved anything on this output.
    pub fn reachable(self) -> bool {
        matches!(self, Reach::ShellHooked | Reach::Shell | Reach::FileRead | Reach::Mcp)
    }
}

/// One tool output extracted from a transcript, in session order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// Tool name as the client logged it (the Kiro IDE's generic `mcp`
    /// tool becomes `mcp_<server>_<tool>`).
    pub tool: String,
    /// Command line for shell tools, tool name otherwise. Used as the
    /// compression label so per-command formatters fire exactly as they
    /// would have live.
    pub label: String,
    /// What sqz would have been handed: the command's output without the
    /// client's wrapper, the MCP server's text, the read result.
    pub text: String,
}

/// Sniff the dialect from the first parseable line.
pub fn detect_format(first_line: &str) -> Option<TranscriptFormat> {
    let v: serde_json::Value = serde_json::from_str(first_line.trim()).ok()?;
    if v.get("kind").is_some() && v.get("version").is_some() {
        return Some(TranscriptFormat::Kiro);
    }
    if v.get("payload").and_then(|p| p.get("type")).is_some() {
        return Some(TranscriptFormat::KiroIde);
    }
    if v.get("type").is_some() {
        return Some(TranscriptFormat::Claude);
    }
    None
}

/// Lowercase alphanumerics only, so `read_file`, `readFile` and
/// `ReadFile` compare equal.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

const SHELL_TOOLS: &[&str] = &[
    "bash", "shell", "executebash", "runterminalcmd", "runterminalcommand",
    "runshellcommand", "powershell", "pwsh", "terminal",
];

/// Built-in reads, searches and listings that `sqz_read_file`, `sqz_grep`
/// and `sqz_list_dir` stand in for. Filename globs have no sqz
/// counterpart, so they count as out of reach.
const FILE_TOOLS: &[&str] = &[
    "read", "readfile", "readfiles", "readcode", "readmultiplefiles", "fsread",
    "notebookread", "grep", "grepsearch", "ls", "listdirectory", "listdir", "search",
];

/// Kiro CLI built-ins. The CLI logs MCP tools under their bare names, so
/// anything not listed here counts as MCP there.
const KIRO_CLI_BUILTINS: &[&str] = &[
    "shell", "executebash", "read", "fsread", "write", "fswrite", "grep", "glob",
    "webfetch", "websearch", "todolist", "introspect", "summary", "skillstool",
    "useaws", "knowledge", "thinking", "delegate", "usesubagent", "subagent",
    "reportissue", "dummy", "code",
];

fn is_shell_tool(name: &str) -> bool {
    SHELL_TOOLS.contains(&normalize(name).as_str())
}

/// sqz's own MCP tools under any client's naming (`mcp__sqz__…`,
/// `mcp_sqz_…`, bare `sqz_read_file`).
fn is_sqz_tool(name: &str) -> bool {
    let n = normalize(name);
    n.starts_with("mcpsqz") || n.starts_with("sqz")
}

/// A command that already pipes through sqz, or runs sqz itself. The
/// hook skips these too.
fn already_through_sqz(command: &str) -> bool {
    command.contains("sqz compress")
        || command.contains("sqz.exe compress")
        || command
            .split_whitespace()
            .next()
            .map_or(false, |first| first == "sqz" || first.ends_with("/sqz"))
}

/// Whether Claude Code's PreToolUse hook would rewrite this command. Runs
/// the real hook so the answer tracks whatever it rewrites today.
fn hook_rewrites(command: &str) -> bool {
    let input = serde_json::json!({ "tool_name": "Bash", "tool_input": { "command": command } });
    sqz_engine::process_hook(&input.to_string())
        .ok()
        .and_then(|out| serde_json::from_str::<serde_json::Value>(&out).ok())
        .map_or(false, |v| v.pointer("/hookSpecificOutput/updatedInput").is_some())
}

/// Which way sqz could have reached `output` on `client`.
pub fn classify(client: Client, output: &ToolOutput) -> Reach {
    if is_shell_tool(&output.tool) {
        if already_through_sqz(&output.label) {
            return Reach::AlreadySqz;
        }
        if client.tier() == IntegrationTier::Transparent && hook_rewrites(&output.label) {
            return Reach::ShellHooked;
        }
        return Reach::Shell;
    }
    if is_sqz_tool(&output.tool) {
        return Reach::AlreadySqz;
    }
    let n = normalize(&output.tool);
    if FILE_TOOLS.contains(&n.as_str()) {
        return Reach::FileRead;
    }
    let mcp = match client {
        Client::ClaudeCode => output.tool.starts_with("mcp__"),
        Client::KiroIde => output.tool.starts_with("mcp_"),
        Client::KiroCli => !KIRO_CLI_BUILTINS.contains(&n.as_str()),
    };
    if mcp {
        Reach::Mcp
    } else {
        Reach::Unreachable
    }
}

/// Derive the compression label from a tool call. Shell tools contribute
/// their command line (so `git status` hits the git formatter); everything
/// else contributes the tool name.
fn command_label(name: &str, input: &serde_json::Value) -> String {
    if is_shell_tool(name) {
        if let Some(cmd) = input.get("command").and_then(|c| c.as_str()) {
            if !cmd.trim().is_empty() {
                return cmd.trim().to_string();
            }
        }
    }
    name.to_string()
}

/// First whitespace token of a label — the report's grouping key
/// (`git status` and `git diff` both group under `git`).
pub fn group_key(label: &str) -> String {
    label.split_whitespace().next().unwrap_or("(unknown)").to_string()
}

/// Concatenate the text blocks of a Claude `tool_result` content value,
/// which is either a plain string or an array of `{type:"text",text}`.
fn claude_result_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Tool call id → (tool, label) until the result arrives.
type Pending = HashMap<String, (String, String)>;

fn unmatched() -> (String, String) {
    ("tool".to_string(), "tool".to_string())
}

/// Feed one Claude Code transcript line through the extraction state
/// machine.
fn extract_claude_line(v: &serde_json::Value, pending: &mut Pending, out: &mut Vec<ToolOutput>) {
    let Some(content) = v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) else {
        return;
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("assistant") => {
            for item in content {
                if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let (Some(id), Some(name)) = (
                        item.get("id").and_then(|i| i.as_str()),
                        item.get("name").and_then(|n| n.as_str()),
                    ) else {
                        continue;
                    };
                    let input = item.get("input").cloned().unwrap_or(serde_json::Value::Null);
                    pending.insert(id.to_string(), (name.to_string(), command_label(name, &input)));
                }
            }
        }
        Some("user") => {
            for item in content {
                if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    let (tool, label) = item
                        .get("tool_use_id")
                        .and_then(|i| i.as_str())
                        .and_then(|id| pending.remove(id))
                        .unwrap_or_else(unmatched);
                    let text = item.get("content").map(claude_result_text).unwrap_or_default();
                    if !text.trim().is_empty() {
                        out.push(ToolOutput { tool, label, text });
                    }
                }
            }
        }
        _ => {}
    }
}

/// Text of one Kiro CLI tool result item. Shell results are JSON
/// (`{exit_status, stdout, stderr}`), MCP results usually JSON too
/// (`{content:[{type:"text",text}]}`).
fn kiro_cli_item_text(item: &serde_json::Value) -> Option<String> {
    match item.get("kind").and_then(|k| k.as_str()) {
        Some("text") => item.get("data").and_then(|d| d.as_str()).map(str::to_string),
        Some("json") => {
            let data = item.get("data")?;
            if data.get("stdout").is_some() || data.get("stderr").is_some() {
                let mut text = data.get("stdout").and_then(|s| s.as_str()).unwrap_or("").to_string();
                let stderr = data.get("stderr").and_then(|s| s.as_str()).unwrap_or("");
                if !stderr.is_empty() {
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str(stderr);
                }
                return Some(text);
            }
            if let Some(items) = data.get("content").and_then(|c| c.as_array()) {
                let texts: Vec<&str> = items
                    .iter()
                    .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                    .collect();
                if !texts.is_empty() {
                    return Some(texts.join("\n"));
                }
            }
            Some(data.to_string())
        }
        _ => None,
    }
}

/// Feed one Kiro CLI transcript line through the extraction state machine.
fn extract_kiro_line(v: &serde_json::Value, pending: &mut Pending, out: &mut Vec<ToolOutput>) {
    let Some(content) = v.get("data").and_then(|d| d.get("content")).and_then(|c| c.as_array()) else {
        return;
    };
    match v.get("kind").and_then(|k| k.as_str()) {
        Some("AssistantMessage") => {
            for item in content {
                if item.get("kind").and_then(|k| k.as_str()) == Some("toolUse") {
                    let Some(data) = item.get("data") else { continue };
                    let (Some(id), Some(name)) = (
                        data.get("toolUseId").and_then(|i| i.as_str()),
                        data.get("name").and_then(|n| n.as_str()),
                    ) else {
                        continue;
                    };
                    let input = data.get("input").cloned().unwrap_or(serde_json::Value::Null);
                    pending.insert(id.to_string(), (name.to_string(), command_label(name, &input)));
                }
            }
        }
        Some("ToolResults") => {
            for item in content {
                if item.get("kind").and_then(|k| k.as_str()) == Some("toolResult") {
                    let Some(data) = item.get("data") else { continue };
                    let (tool, label) = data
                        .get("toolUseId")
                        .and_then(|i| i.as_str())
                        .and_then(|id| pending.remove(id))
                        .unwrap_or_else(unmatched);
                    let text = data
                        .get("content")
                        .and_then(|c| c.as_array())
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(kiro_cli_item_text)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    if !text.trim().is_empty() {
                        out.push(ToolOutput { tool, label, text });
                    }
                }
            }
        }
        _ => {}
    }
}

/// The Kiro IDE wraps shell output as `Output:\n…\n\nExit Code: N` (or,
/// in older versions, `{"output":"…","exitCode":N}`). sqz would only have
/// seen the output, without the terminal's `\r\n`.
fn kiro_shell_payload(content: &str) -> String {
    if content.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(output) = v.get("output").and_then(|o| o.as_str()) {
                return output.replace("\r\n", "\n");
            }
        }
    }
    let body = content.strip_prefix("Output:\n").unwrap_or(content);
    let body = match body.rfind("\n\nExit Code: ") {
        Some(i) if body[i + 13..].trim().chars().all(|c| c.is_ascii_digit() || c == '-') => &body[..i],
        _ => body,
    };
    body.replace("\r\n", "\n").trim_end_matches('\r').to_string()
}

/// The Kiro IDE returns a read as one `<file name=… language=…>\n<content>\n…
/// \n</content>\n</file>` block per file, plus a `Note:` when the read was
/// cut. `sqz_read_file` would have returned each file's text on its own,
/// so replay sees those. Anything else (images, errors) is kept whole.
fn kiro_read_payloads(content: &str) -> Vec<String> {
    let mut texts = Vec::new();
    for block in content.split("\n<file name=\"") {
        let Some(start) = block.find(">\n<content>\n") else { continue };
        let body = &block[start + ">\n<content>\n".len()..];
        // The last close tag in the block, so a file that itself contains
        // `</content>` isn't cut short.
        if let Some(end) = body.rfind("\n</content>") {
            texts.push(body[..end].to_string());
        }
    }
    if texts.is_empty() {
        vec![content.to_string()]
    } else {
        texts
    }
}

/// The Kiro IDE stores an MCP result as `{"response":"<text>",
/// "imageBase64Urls":[…]}`; the proxy would have seen the inner text.
fn kiro_mcp_payload(content: &str) -> String {
    if content.starts_with("{\"response\"") {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(content) {
            if map.keys().all(|k| k == "response" || k == "imageBase64Urls") {
                if let Some(text) = map.get("response").and_then(|r| r.as_str()) {
                    return text.to_string();
                }
            }
        }
    }
    content.to_string()
}

/// Feed one Kiro IDE transcript line through the extraction state machine.
fn extract_kiro_ide_line(v: &serde_json::Value, pending: &mut Pending, out: &mut Vec<ToolOutput>) {
    let Some(p) = v.get("payload") else { return };
    match p.get("type").and_then(|t| t.as_str()) {
        Some("tool_call") => {
            let (Some(id), Some(name)) = (
                p.get("toolCallId").and_then(|i| i.as_str()),
                p.get("toolName").and_then(|n| n.as_str()),
            ) else {
                return;
            };
            let args = p.get("args").cloned().unwrap_or(serde_json::Value::Null);
            // The generic `mcp` tool names its server and tool in its args.
            let tool = match (
                name,
                args.get("serverName").and_then(|s| s.as_str()),
                args.get("toolName").and_then(|t| t.as_str()),
            ) {
                ("mcp", Some(server), Some(tool)) => format!("mcp_{}_{tool}", server.replace('-', "_")),
                _ => name.to_string(),
            };
            let label = command_label(&tool, &args);
            pending.insert(id.to_string(), (tool, label));
        }
        Some("tool_result") => {
            let (tool, label) = p
                .get("toolCallId")
                .and_then(|i| i.as_str())
                .and_then(|id| pending.remove(id))
                .unwrap_or_else(unmatched);
            let content = p.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let texts = if is_shell_tool(&tool) {
                vec![kiro_shell_payload(content)]
            } else if tool.starts_with("mcp_") {
                vec![kiro_mcp_payload(content)]
            } else if matches!(normalize(&tool).as_str(), "readfile" | "readfiles" | "readcode") {
                kiro_read_payloads(content)
            } else {
                vec![content.to_string()]
            };
            for text in texts {
                if !text.trim().is_empty() {
                    out.push(ToolOutput { tool: tool.clone(), label: label.clone(), text });
                }
            }
        }
        _ => {}
    }
}

/// Extract every tool output from a transcript, in order.
pub fn extract_tool_outputs<R: BufRead>(reader: R, format: TranscriptFormat) -> Vec<ToolOutput> {
    let mut pending = Pending::new();
    let mut out = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match format {
            TranscriptFormat::Claude => extract_claude_line(&v, &mut pending, &mut out),
            TranscriptFormat::Kiro => extract_kiro_line(&v, &mut pending, &mut out),
            TranscriptFormat::KiroIde => extract_kiro_ide_line(&v, &mut pending, &mut out),
        }
    }
    out
}

/// Outputs and tokens for one reach class.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub outputs: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Part of the saving that came from lossy compression (the MCP
    /// proxy's first pass) rather than dedup or formatting.
    pub lossy_saved: u64,
}

impl Bucket {
    fn add(&mut self, replayed: Replayed) {
        self.outputs += 1;
        self.tokens_in += replayed.tokens_in;
        self.tokens_out += replayed.tokens_out;
        if replayed.lossy {
            self.lossy_saved += replayed.tokens_in.saturating_sub(replayed.tokens_out);
        }
    }

    fn merge(&mut self, other: &Bucket) {
        self.outputs += other.outputs;
        self.tokens_in += other.tokens_in;
        self.tokens_out += other.tokens_out;
        self.lossy_saved += other.lossy_saved;
    }

    pub fn saved(&self) -> u64 {
        self.tokens_in.saturating_sub(self.tokens_out)
    }

    pub fn pct(&self) -> f64 {
        if self.tokens_in == 0 {
            0.0
        } else {
            self.saved() as f64 * 100.0 / self.tokens_in as f64
        }
    }
}

/// Replay results for one client.
#[derive(Debug, Default, Clone)]
pub struct ClientReport {
    /// Distinct sessions (a Kiro IDE session's sub-agent runs count once).
    pub sessions: HashSet<PathBuf>,
    pub sub_runs: usize,
    pub reach: BTreeMap<Reach, Bucket>,
}

/// Aggregate result of replaying one or more transcript files.
#[derive(Debug, Default, Clone)]
pub struct ReplayReport {
    pub clients: BTreeMap<Client, ClientReport>,
    /// Source group → (tokens in, tokens avoidable), reachable outputs only.
    pub groups: HashMap<String, (u64, u64)>,
    pub skipped_files: usize,
}

impl ReplayReport {
    pub fn sessions(&self) -> usize {
        self.clients.values().map(|c| c.sessions.len()).sum()
    }

    fn sum(&self, keep: impl Fn(Reach) -> bool) -> Bucket {
        let mut total = Bucket::default();
        for client in self.clients.values() {
            for (reach, bucket) in &client.reach {
                if keep(*reach) {
                    total.merge(bucket);
                }
            }
        }
        total
    }

    /// Every tool output, reachable or not.
    pub fn all(&self) -> Bucket {
        self.sum(|_| true)
    }

    /// What sqz could save if the agent used every route.
    pub fn reachable(&self) -> Bucket {
        self.sum(Reach::reachable)
    }

    /// What the installed hooks save with no help from the agent.
    pub fn automatic(&self) -> Bucket {
        self.sum(|r| r == Reach::ShellHooked)
    }
}

/// Find replayable transcripts modified within the last `days`.
///
/// Scans the Claude Code and Kiro locations; `override_path` (file or
/// directory) replaces the default scan entirely.
pub fn find_transcripts(days: u32, override_path: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    match override_path {
        Some(p) if p.is_file() => return vec![p.to_path_buf()],
        Some(p) => roots.push(p.to_path_buf()),
        None => {
            if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
                roots.push(PathBuf::from(home).join(".claude").join("projects"));
            }
            // Kiro CLI sessions live in `cli/`, Kiro IDE sessions in
            // `<workspace>/<session>/` (plus `sub-executions/` for
            // sub-agent runs).
            roots.push(sqz_engine::kiro_user_root().join("sessions"));
        }
    }
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(u64::from(days) * 86_400));
    let mut files = Vec::new();
    for root in roots {
        collect_jsonl(&root, cutoff, &mut files, 0);
    }
    files.sort();
    files
}

/// Directories that hold copies of workspace files or spilled tool output,
/// never transcripts.
const NOT_TRANSCRIPT_DIRS: &[&str] = &["snapshots", "tool-outputs"];

fn collect_jsonl(dir: &Path, cutoff: Option<std::time::SystemTime>, out: &mut Vec<PathBuf>, depth: u8) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| NOT_TRANSCRIPT_DIRS.contains(&n));
            if !skip {
                collect_jsonl(&path, cutoff, out, depth + 1);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() == 0 {
                continue;
            }
            let fresh = match (cutoff, meta.modified()) {
                (Some(c), Ok(m)) => m >= c,
                _ => true,
            };
            if fresh {
                out.push(path);
            }
        }
    }
}

/// The session a transcript belongs to. A Kiro IDE session is its
/// directory, and a sub-agent run under `sub-executions/` belongs to it.
fn session_key(path: &Path, format: TranscriptFormat) -> (PathBuf, bool) {
    let parent = path.parent();
    if format == TranscriptFormat::KiroIde {
        let in_sub_runs = parent.and_then(|p| p.file_name()).and_then(|n| n.to_str()) == Some("sub-executions");
        let session = if in_sub_runs { parent.and_then(|p| p.parent()) } else { parent };
        if let Some(session) = session {
            return (session.to_path_buf(), in_sub_runs);
        }
    }
    (path.to_path_buf(), false)
}

/// One replayed output: tokens the model saw, tokens it would have seen
/// (both from the engine's tokenizer), and whether the saving was lossy.
#[derive(Debug, Clone, Copy)]
struct Replayed {
    tokens_in: u64,
    tokens_out: u64,
    lossy: bool,
}

/// Replay one output the way sqz could have reached it.
fn replay_one(proxy: &CliProxy, reach: Reach, output: &ToolOutput) -> Replayed {
    let engine = proxy.engine();
    let tokens_in = u64::from(engine.count_tokens(&output.text));
    let mut lossy = false;
    let shown = match reach {
        Reach::ShellHooked | Reach::Shell => Some(proxy.intercept_output_with_options(
            &output.label,
            &output.text,
            InterceptOptions::default(),
        )),
        Reach::FileRead => engine.compress_with_cache_lossless(&output.text).ok().map(|r| match r {
            CacheResult::Dedup { inline_ref, .. } => inline_ref,
            CacheResult::Delta { delta_text, .. } => delta_text,
            CacheResult::Fresh { output } => output.data,
        }),
        Reach::Mcp => engine.compress_tool_result(&output.text, true).map(|c| {
            lossy = !c.repeat;
            c.text
        }),
        Reach::AlreadySqz | Reach::Unreachable => None,
    };
    let tokens_out = shown.map_or(tokens_in, |s| u64::from(engine.count_tokens(&s)));
    Replayed { tokens_in, tokens_out, lossy }
}

fn sniff_format(path: &Path) -> Option<TranscriptFormat> {
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    reader
        .lines()
        .take(20)
        .map_while(|l| l.ok())
        .find(|l| !l.trim().is_empty())
        .and_then(|l| detect_format(&l))
}

/// Replay one transcript file against a fresh throwaway store and fold the
/// result into `report`. Returns false when the file has no recognisable
/// format or no tool outputs.
fn replay_file(path: &Path, report: &mut ReplayReport) -> bool {
    let Some(format) = sniff_format(path) else { return false };
    let Ok(file) = std::fs::File::open(path) else { return false };
    let outputs = extract_tool_outputs(std::io::BufReader::new(file), format);
    if outputs.is_empty() {
        return false;
    }

    // Fresh store per transcript: dedup references must not span sessions.
    let db = std::env::temp_dir().join(format!(
        "sqz_replay_{}_{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let engine = match sqz_engine::SqzEngine::with_preset_and_store(sqz_engine::Preset::default(), &db) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let proxy = CliProxy::with_engine(engine);

    let client = format.client();
    let entry = report.clients.entry(client).or_default();
    for output in &outputs {
        let reach = classify(client, output);
        let replayed = replay_one(&proxy, reach, output);
        entry.reach.entry(reach).or_default().add(replayed);
        if reach.reachable() {
            let group = report.groups.entry(group_key(&output.label)).or_insert((0, 0));
            group.0 += replayed.tokens_in;
            group.1 += replayed.tokens_in.saturating_sub(replayed.tokens_out);
        }
    }
    let (session, sub_run) = session_key(path, format);
    entry.sessions.insert(session);
    if sub_run {
        entry.sub_runs += 1;
    }

    drop(proxy);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{}", db.display(), suffix)));
    }
    true
}

/// `1234567` → `1,234,567`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn reach_name(reach: Reach, tier: IntegrationTier) -> &'static str {
    match (reach, tier) {
        (Reach::ShellHooked, _) => "shell, hook rewrites",
        (Reach::Shell, IntegrationTier::Transparent) => "shell, hook skips",
        (Reach::Shell, IntegrationTier::Advisory) => "shell",
        (Reach::FileRead, _) => "file reads",
        (Reach::Mcp, _) => "other MCP servers",
        (Reach::AlreadySqz, _) => "already through sqz",
        (Reach::Unreachable, _) => "out of reach",
    }
}

/// `A`, `A and B`, `A, B and C`.
fn join_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Word-wrap `text` to `width` columns, each line indented two spaces.
fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && 2 + line.len() + 1 + word.len() > width {
            out.push_str(&format!("  {line}\n"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&format!("  {line}\n"));
    }
    out
}

/// How the reachable rows could actually be captured on clients of `tier`.
fn tier_note(tier: IntegrationTier, clients: &[&str]) -> String {
    let names = join_names(clients);
    let text = match tier {
        IntegrationTier::Transparent => format!(
            "On {names} the hook rewrites simple shell commands on its own. Pipes, chains \
             and redirects are skipped and need the agent to pipe them, file reads need \
             the sqz MCP tools (lossless, so dedup only) and MCP servers need \
             `sqz-mcp proxy` (lossy on first pass, sqz_expand recovers the rest)."
        ),
        IntegrationTier::Advisory => format!(
            "{names} can't let sqz rewrite tool calls, so each saving needs the agent: \
             shell output piped through `sqz compress`, file reads done with the sqz MCP \
             tools (lossless, so dedup only), MCP servers run behind `sqz-mcp proxy` \
             (lossy on first pass, sqz_expand recovers the rest)."
        ),
    };
    wrap(&text, 76)
}

/// Shorten a source name for the table, dropping the MCP prefix first.
fn display_source(group: &str) -> String {
    let name = group
        .strip_prefix("mcp__")
        .or_else(|| group.strip_prefix("mcp_"))
        .unwrap_or(group);
    if name.chars().count() > 30 {
        let cut: String = name.chars().take(29).collect();
        format!("{cut}…")
    } else {
        name.to_string()
    }
}

/// Render the report. Kept separate from collection so tests can assert
/// on the wording that carries the honesty requirements.
pub fn render(report: &ReplayReport, days: u32) -> String {
    let mut out = String::new();
    out.push_str(&format!("sqz discover — counterfactual replay (last {days} days)\n"));
    out.push_str(&"─".repeat(56));
    out.push('\n');

    for (client, c) in &report.clients {
        let runs = if c.sub_runs > 0 {
            format!(", {} sub-agent run{}", thousands(c.sub_runs as u64), plural(c.sub_runs))
        } else {
            String::new()
        };
        let tier = client.tier();
        out.push_str(&format!(
            "\n  {}: {} session{}{runs}, {}\n\n",
            client.name(),
            thousands(c.sessions.len() as u64),
            plural(c.sessions.len()),
            match tier {
                IntegrationTier::Transparent => "transparent",
                IntegrationTier::Advisory => "advisory",
            }
        ));
        out.push_str(&format!("    {:<22} {:>8} {:>12} {:>12}\n", "", "outputs", "tokens in", "avoidable"));
        for (reach, b) in &c.reach {
            let avoidable = if reach.reachable() {
                format!("{:>12} {:>5.1}%", thousands(b.saved()), b.pct())
            } else {
                format!("{:>12}", "-")
            };
            out.push_str(&format!(
                "    {:<22} {:>8} {:>12} {}\n",
                reach_name(*reach, tier),
                thousands(u64::from(b.outputs)),
                thousands(b.tokens_in),
                avoidable
            ));
        }
    }
    for tier in [IntegrationTier::Transparent, IntegrationTier::Advisory] {
        let names: Vec<&str> = report
            .clients
            .keys()
            .filter(|c| c.tier() == tier)
            .map(|c| c.name())
            .collect();
        if !names.is_empty() {
            out.push('\n');
            out.push_str(&tier_note(tier, &names));
        }
    }

    let all = report.all();
    let reachable = report.reachable();
    let automatic = report.automatic();
    out.push_str(&format!(
        "\n  Avoidable with no help from the agent:   {} tokens\n  \
         Avoidable if the agent uses every route: {} tokens, {:.1}% of all tool output\n",
        thousands(automatic.saved()),
        thousands(reachable.saved()),
        if all.tokens_in == 0 { 0.0 } else { reachable.saved() as f64 * 100.0 / all.tokens_in as f64 },
    ));
    if reachable.lossy_saved > 0 {
        out.push_str(&format!(
            "  Of that, {} is the MCP proxy's lossy first pass.\n",
            thousands(reachable.lossy_saved)
        ));
    }

    let mut groups: Vec<(&String, &(u64, u64))> = report.groups.iter().collect();
    groups.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then_with(|| a.0.cmp(b.0)));
    let top: Vec<_> = groups.into_iter().filter(|(_, (_, saved))| *saved > 0).take(8).collect();
    if !top.is_empty() {
        out.push_str("\n  Largest opportunities:\n\n");
        out.push_str(&format!("    {:<30} {:>12} {:>12}\n", "source", "tokens in", "avoidable"));
        for (group, (tin, saved)) in top {
            out.push_str(&format!(
                "    {:<30} {:>12} {:>12}\n",
                display_source(group),
                thousands(*tin),
                thousands(*saved)
            ));
        }
    }

    if report.skipped_files > 0 {
        out.push_str(&format!(
            "\n  {} file(s) skipped (unrecognised format or no tool output).\n",
            report.skipped_files
        ));
    }

    out.push_str(
        "\n  This is a counterfactual replay estimate: your transcripts were\n  \
         replayed through the current sqz engine. These outputs were not\n  \
         compressed at the time, so the numbers are estimates, not\n  \
         measurements. Repeats are assumed to stay within the reference\n  \
         freshness window. Analysis ran locally; nothing was uploaded.\n\n  \
         To start capturing what's reachable:\n    \
         sqz init --global                    hooks, guidance and the sqz MCP tools\n",
    );
    if report.sum(|r| r == Reach::Mcp).saved() > 0 {
        out.push_str("    sqz-mcp proxy -- <server command>    put in front of a stdio MCP server\n");
    }
    out
}

/// Entry point for `sqz discover --replay`. Returns the process exit code.
pub fn run(days: u32, transcripts: Option<PathBuf>) -> i32 {
    // Replaying thousands of outputs would flood stderr with the
    // per-compression progress lines meant for the hook path.
    std::env::set_var("SQZ_QUIET", "1");
    let files = find_transcripts(days, transcripts.as_deref());
    if files.is_empty() {
        eprintln!(
            "[sqz] no transcripts found in the last {days} days.\n\
             Looked in ~/.claude/projects and ~/.kiro/sessions.\n\
             Point at a file or directory with: sqz discover --replay --transcripts PATH"
        );
        return 1;
    }
    use std::io::IsTerminal;
    let progress = std::io::stderr().is_terminal();
    if !progress {
        eprintln!("[sqz] replaying {} transcript file(s)…", files.len());
    }
    let mut report = ReplayReport::default();
    for (i, file) in files.iter().enumerate() {
        if progress {
            eprint!("\r[sqz] replaying transcript {}/{}…", i + 1, files.len());
        }
        if !replay_file(file, &mut report) {
            report.skipped_files += 1;
        }
    }
    if progress {
        eprintln!();
    }
    if report.sessions() == 0 {
        eprintln!(
            "[sqz] found {} file(s) but none contained replayable tool output.",
            files.len()
        );
        return 1;
    }
    print!("{}", render(&report, days));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_LINES: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"On branch main\nnothing to commit"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/tmp/a.rs"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"fn main() {}"}]}]}}
"#;

    const KIRO_LINES: &str = r#"{"version":"v1","kind":"AssistantMessage","data":{"content":[{"kind":"text","data":""},{"kind":"toolUse","data":{"toolUseId":"k1","name":"execute_bash","input":{"command":"cargo test"}}}]}}
{"version":"v1","kind":"ToolResults","data":{"content":[{"kind":"toolResult","data":{"toolUseId":"k1","content":[{"kind":"text","data":"test result: ok. 3 passed"}]}}]}}
"#;

    const KIRO_IDE_LINES: &str = r#"{"id":"m1","timestamp":1,"payload":{"type":"user","content":"hi"}}
{"id":"m2","timestamp":2,"payload":{"type":"tool_call","toolCallId":"c1","toolName":"execute_bash","kind":"execute","args":{"command":"git status"}}}
{"id":"m3","timestamp":3,"payload":{"type":"tool_result","toolCallId":"c1","content":"Output:\nOn branch main\r\nnothing to commit\r\n\nExit Code: 0","success":true}}
{"id":"m4","timestamp":4,"payload":{"type":"tool_call","toolCallId":"c2","toolName":"mcp_builder_mcp_internalsearch","args":{"query":"x"}}}
{"id":"m5","timestamp":5,"payload":{"type":"tool_result","toolCallId":"c2","content":"{\"response\":\"result text\",\"imageBase64Urls\":[]}","success":true}}
{"id":"m6","timestamp":6,"payload":{"type":"tool_call","toolCallId":"c3","toolName":"mcp","args":{"serverName":"mcm-mcp-server-mcp","toolName":"get_cm","toolArgs":"{}"}}}
{"id":"m7","timestamp":7,"payload":{"type":"tool_result","toolCallId":"c3","content":"{\"status\":\"success\"}","success":true}}
{"id":"m8","timestamp":8,"payload":{"type":"tool_call","toolCallId":"c4","toolName":"web_fetch","args":{"url":"https://example.com"}}}
{"id":"m9","timestamp":9,"payload":{"type":"tool_result","toolCallId":"c4","content":"Example Domain","success":true}}
"#;

    #[test]
    fn detects_formats() {
        assert_eq!(
            detect_format(CLAUDE_LINES.lines().next().unwrap()),
            Some(TranscriptFormat::Claude)
        );
        assert_eq!(
            detect_format(KIRO_LINES.lines().next().unwrap()),
            Some(TranscriptFormat::Kiro)
        );
        assert_eq!(
            detect_format(KIRO_IDE_LINES.lines().next().unwrap()),
            Some(TranscriptFormat::KiroIde)
        );
        assert_eq!(detect_format("not json"), None);
        assert_eq!(detect_format("{\"foo\":1}"), None);
    }

    #[test]
    fn extracts_claude_tool_outputs_with_labels() {
        let outputs = extract_tool_outputs(CLAUDE_LINES.as_bytes(), TranscriptFormat::Claude);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].label, "git status");
        assert_eq!(outputs[0].tool, "Bash");
        assert!(outputs[0].text.contains("On branch main"));
        assert_eq!(outputs[1].label, "Read");
        assert_eq!(outputs[1].text, "fn main() {}");
    }

    #[test]
    fn extracts_kiro_tool_outputs_with_labels() {
        let outputs = extract_tool_outputs(KIRO_LINES.as_bytes(), TranscriptFormat::Kiro);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].label, "cargo test");
        assert!(outputs[0].text.contains("3 passed"));
    }

    #[test]
    fn extracts_kiro_cli_json_results() {
        // Shell and MCP results come as JSON items.
        let lines = r#"{"version":"v1","kind":"AssistantMessage","data":{"content":[{"kind":"toolUse","data":{"toolUseId":"a","name":"shell","input":{"command":"ls"}}},{"kind":"toolUse","data":{"toolUseId":"b","name":"ReadInternalWebsites","input":{}}}]}}
{"version":"v1","kind":"ToolResults","data":{"content":[{"kind":"toolResult","data":{"toolUseId":"a","content":[{"kind":"json","data":{"exit_status":"exit status: 0","stdout":"a.txt\nb.txt","stderr":"warn"}}]}},{"kind":"toolResult","data":{"toolUseId":"b","content":[{"kind":"json","data":{"content":[{"type":"text","text":"page body"}]}}]}}]}}
"#;
        let outputs = extract_tool_outputs(lines.as_bytes(), TranscriptFormat::Kiro);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].text, "a.txt\nb.txt\nwarn");
        assert_eq!(outputs[1].text, "page body");
        assert_eq!(classify(Client::KiroCli, &outputs[0]), Reach::Shell);
        assert_eq!(classify(Client::KiroCli, &outputs[1]), Reach::Mcp);
    }

    #[test]
    fn extracts_kiro_ide_outputs_without_client_wrappers() {
        let outputs = extract_tool_outputs(KIRO_IDE_LINES.as_bytes(), TranscriptFormat::KiroIde);
        assert_eq!(outputs.len(), 4);
        assert_eq!(outputs[0].label, "git status");
        assert_eq!(outputs[0].text, "On branch main\nnothing to commit");
        assert_eq!(outputs[1].text, "result text");
        assert_eq!(outputs[2].tool, "mcp_mcm_mcp_server_mcp_get_cm");
        assert_eq!(outputs[2].text, "{\"status\":\"success\"}");
        let reaches: Vec<Reach> = outputs.iter().map(|o| classify(Client::KiroIde, o)).collect();
        assert_eq!(reaches, [Reach::Shell, Reach::Mcp, Reach::Mcp, Reach::Unreachable]);
    }

    #[test]
    fn kiro_read_payloads_unwrap_each_file() {
        let one = "<file name=\"/p/a.rs\" language=\"rust\" >\n<content>\nfn a() {}\nfn b() {}\n</content>\n</file>\nNote: This read was limited to 2 lines.";
        assert_eq!(kiro_read_payloads(one), ["fn a() {}\nfn b() {}"]);
        let two = "<file name=\"/p/a.rs\" language=\"rust\" >\n<content>\nA\n</content>\n</file>\n<file name=\"/p/b.md\" language=\"markdown\" >\n<content>\nuses </content> inline\nB\n</content>\n</file>";
        assert_eq!(kiro_read_payloads(two), ["A", "uses </content> inline\nB"]);
        assert_eq!(kiro_read_payloads("Image: /tmp/x.png"), ["Image: /tmp/x.png"]);

        let call = r#"{"id":"1","timestamp":1,"payload":{"type":"tool_call","toolCallId":"r","toolName":"read_files","args":{}}}"#;
        let result = serde_json::json!({"id": "2", "timestamp": 2, "payload": {"type": "tool_result", "toolCallId": "r", "content": two, "success": true}});
        let outputs = extract_tool_outputs(format!("{call}\n{result}\n").as_bytes(), TranscriptFormat::KiroIde);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[1].text, "uses </content> inline\nB");
        assert_eq!(classify(Client::KiroIde, &outputs[0]), Reach::FileRead);
    }

    #[test]
    fn kiro_shell_payload_handles_both_wrappers() {
        assert_eq!(kiro_shell_payload("Output:\nline\r\n\nExit Code: 1"), "line");
        assert_eq!(kiro_shell_payload("{\"output\":\"a\\r\\nb\",\"exitCode\":0}"), "a\nb");
        assert_eq!(kiro_shell_payload("Command timed out"), "Command timed out");
        // Output that itself mentions an exit code is left alone.
        assert_eq!(kiro_shell_payload("Output:\nsee\n\nExit Code: docs"), "see\n\nExit Code: docs");
    }

    #[test]
    fn classifies_by_client() {
        let out = |tool: &str, label: &str| ToolOutput { tool: tool.into(), label: label.into(), text: "x".into() };
        // Claude Code's hook rewrites commands and pipelines, not ones that
        // change shell state or follow a log.
        assert_eq!(classify(Client::ClaudeCode, &out("Bash", "git status")), Reach::ShellHooked);
        assert_eq!(classify(Client::ClaudeCode, &out("Bash", "git log | head")), Reach::ShellHooked);
        assert_eq!(classify(Client::ClaudeCode, &out("Bash", "export X=1 && make")), Reach::Shell);
        assert_eq!(classify(Client::ClaudeCode, &out("Bash", "tail -f app.log")), Reach::Shell);
        // Kiro has no rewrite hook at all.
        assert_eq!(classify(Client::KiroIde, &out("execute_bash", "git status")), Reach::Shell);
        assert_eq!(classify(Client::KiroIde, &out("execute_bash", "git diff 2>&1 | sqz compress")), Reach::AlreadySqz);
        assert_eq!(classify(Client::KiroIde, &out("readFile", "readFile")), Reach::FileRead);
        assert_eq!(classify(Client::KiroIde, &out("grep_search", "grep_search")), Reach::FileRead);
        assert_eq!(classify(Client::KiroIde, &out("mcp_sqz_sqz_read_file", "x")), Reach::AlreadySqz);
        assert_eq!(classify(Client::KiroIde, &out("str_replace", "x")), Reach::Unreachable);
        assert_eq!(classify(Client::ClaudeCode, &out("mcp__github__list_issues", "x")), Reach::Mcp);
        assert_eq!(classify(Client::ClaudeCode, &out("mcp__sqz__sqz_grep", "x")), Reach::AlreadySqz);
        assert_eq!(classify(Client::ClaudeCode, &out("WebFetch", "x")), Reach::Unreachable);
        assert_eq!(classify(Client::KiroCli, &out("web_fetch", "x")), Reach::Unreachable);
        assert_eq!(classify(Client::KiroCli, &out("read", "x")), Reach::FileRead);
        // Filename globs have no sqz counterpart.
        assert_eq!(classify(Client::KiroCli, &out("glob", "x")), Reach::Unreachable);
        assert_eq!(classify(Client::ClaudeCode, &out("Glob", "x")), Reach::Unreachable);
        assert_eq!(classify(Client::KiroIde, &out("file_search", "x")), Reach::Unreachable);
        assert_eq!(classify(Client::KiroIde, &out("list_directory", "x")), Reach::FileRead);
    }

    #[test]
    fn unmatched_result_falls_back_to_generic_label() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"orphan","content":"some output"}]}}"#;
        let outputs = extract_tool_outputs(line.as_bytes(), TranscriptFormat::Claude);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].label, "tool");
    }

    #[test]
    fn group_key_takes_first_token() {
        assert_eq!(group_key("git status --short"), "git");
        assert_eq!(group_key("Read"), "Read");
        assert_eq!(group_key(""), "(unknown)");
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn render_carries_the_counterfactual_wording() {
        let mut report = ReplayReport::default();
        let kiro = report.clients.entry(Client::KiroIde).or_default();
        kiro.sessions.insert(PathBuf::from("a"));
        kiro.sub_runs = 2;
        kiro.reach.insert(Reach::Shell, Bucket { outputs: 10, tokens_in: 1000, tokens_out: 700, ..Default::default() });
        kiro.reach.insert(Reach::Unreachable, Bucket { outputs: 5, tokens_in: 500, tokens_out: 500, ..Default::default() });
        report.groups.insert("git".to_string(), (600, 200));
        let text = render(&report, 7);
        assert!(text.contains("counterfactual replay"));
        assert!(text.contains("estimates, not"));
        assert!(text.contains("nothing was uploaded"));
        assert!(text.contains("Kiro IDE: 1 session, 2 sub-agent runs, advisory"), "{text}");
        assert!(text.contains("Kiro IDE can't let sqz rewrite tool calls"), "{text}");
        assert!(text.contains("Avoidable with no help from the agent:   0 tokens"), "{text}");
        assert!(text.contains("every route: 300 tokens, 20.0% of all tool output"), "{text}");
        assert!(text.contains("out of reach"), "{text}");
        assert!(text.contains("git"));
        assert!(!text.contains("sqz-mcp proxy -- "), "no MCP savings, no proxy line: {text}");
        assert!(!text.contains("lossy first pass"), "{text}");

        let claude = report.clients.entry(Client::ClaudeCode).or_default();
        claude.sessions.insert(PathBuf::from("b"));
        claude.reach.insert(Reach::ShellHooked, Bucket { outputs: 4, tokens_in: 400, tokens_out: 100, ..Default::default() });
        claude.reach.insert(Reach::Mcp, Bucket { outputs: 2, tokens_in: 1000, tokens_out: 200, lossy_saved: 700 });
        report.clients.entry(Client::KiroCli).or_default().sessions.insert(PathBuf::from("c"));
        let text = render(&report, 7);
        assert!(text.contains("Claude Code: 1 session, transparent"), "{text}");
        assert!(text.contains("shell, hook rewrites"), "{text}");
        assert!(text.contains("\n  On Claude Code the hook rewrites simple shell commands"), "{text}");
        assert!(text.contains("\n  Kiro IDE and Kiro CLI can't let sqz rewrite tool calls"), "{text}");
        assert_eq!(text.matches("can't let sqz rewrite").count(), 1, "one note per tier: {text}");
        assert!(text.contains("Avoidable with no help from the agent:   300 tokens"), "{text}");
        assert!(text.contains("Of that, 700 is the MCP proxy's lossy first pass."), "{text}");
        assert!(text.contains("sqz-mcp proxy -- <server command>"), "{text}");
        assert!(text.lines().all(|l| l.chars().count() <= 100), "{text}");
    }

    #[test]
    fn wrap_and_join_names() {
        assert_eq!(join_names(&["A"]), "A");
        assert_eq!(join_names(&["A", "B"]), "A and B");
        assert_eq!(join_names(&["A", "B", "C"]), "A, B and C");
        let wrapped = wrap("one two three four five six", 14);
        assert_eq!(wrapped, "  one two\n  three four\n  five six\n");
        assert!(wrapped.lines().all(|l| l.len() <= 14));
    }

    #[test]
    fn replay_file_end_to_end_with_repeat_dedup() {
        // Two identical large outputs: the second must dedup to a ref,
        // so tokens_out < tokens_in and the session is counted.
        let big: String = (0..80)
            .map(|i| format!("line {i}: the quick brown fox jumps over the lazy dog\n"))
            .collect();
        let mut lines = String::new();
        for (id, _) in [("a", 0), ("b", 1)] {
            lines.push_str(&format!(
                "{}\n",
                serde_json::json!({
                    "type": "assistant",
                    "message": {"content": [{"type": "tool_use", "id": id, "name": "Bash",
                        "input": {"command": "cat notes.txt"}}]}
                })
            ));
            lines.push_str(&format!(
                "{}\n",
                serde_json::json!({
                    "type": "user",
                    "message": {"content": [{"type": "tool_result", "tool_use_id": id,
                        "content": big}]}
                })
            ));
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, lines).unwrap();

        let mut report = ReplayReport::default();
        assert!(replay_file(&path, &mut report));
        assert_eq!(report.sessions(), 1);
        let claude = &report.clients[&Client::ClaudeCode];
        let hooked = claude.reach[&Reach::ShellHooked];
        assert_eq!(hooked.outputs, 2);
        assert!(hooked.tokens_in > 0);
        assert!(
            hooked.tokens_out < hooked.tokens_in,
            "repeat must dedup: in={} out={}",
            hooked.tokens_in,
            hooked.tokens_out
        );
        assert!(report.groups.contains_key("cat"), "{:?}", report.groups);
    }

    #[test]
    fn file_reads_replay_lossless_and_unreachable_is_not_replayed() {
        // A first read of source code must not "save" anything: the sqz
        // read tools are lossless. Only the repeat is avoidable.
        let code: String = (0..60)
            .map(|i| format!("pub fn handler_{i}(req: &Request) -> Response {{\n    respond(req, {i})\n}}\n\n"))
            .collect();
        let call = |id: &str, tool: &str| {
            serde_json::json!({"id": id, "timestamp": 1, "payload": {"type": "tool_call", "toolCallId": id, "toolName": tool, "args": {}}})
        };
        let result = |id: &str, content: &str| {
            serde_json::json!({"id": id, "timestamp": 1, "payload": {"type": "tool_result", "toolCallId": id, "content": content, "success": true}})
        };
        let lines = [
            call("r1", "read_file"), result("r1", &code),
            call("r2", "read_file"), result("r2", &code),
            call("w1", "web_fetch"), result("w1", &code.replace("handler", "page")),
        ]
        .map(|v| v.to_string())
        .join("\n");
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("ws").join("sess");
        std::fs::create_dir_all(session.join("sub-executions")).unwrap();
        std::fs::write(session.join("messages.jsonl"), &lines).unwrap();
        std::fs::write(session.join("sub-executions").join("run1.jsonl"), &lines).unwrap();

        let mut report = ReplayReport::default();
        for file in find_transcripts(7, Some(dir.path())) {
            assert!(replay_file(&file, &mut report));
        }
        let kiro = &report.clients[&Client::KiroIde];
        assert_eq!(kiro.sessions.len(), 1, "a sub-agent run belongs to its session");
        assert_eq!(kiro.sub_runs, 1);
        let reads = kiro.reach[&Reach::FileRead];
        assert_eq!(reads.outputs, 4);
        let one_read = reads.tokens_in / 4;
        assert!(reads.saved() > one_read, "each file's repeat read dedups: {reads:?}");
        assert!(reads.saved() < one_read * 2 + 50, "first reads stay verbatim: {reads:?}");
        let web = kiro.reach[&Reach::Unreachable];
        assert_eq!(web.tokens_in, web.tokens_out);
    }

    #[test]
    fn find_transcripts_honours_override_and_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jsonl"), "{}").unwrap();
        std::fs::write(dir.path().join("b.json"), "{}").unwrap();
        std::fs::write(dir.path().join("empty.jsonl"), "").unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/c.jsonl"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join("sess/snapshots/x")).unwrap();
        std::fs::write(dir.path().join("sess/snapshots/x/copy.jsonl"), "{}").unwrap();
        let found = find_transcripts(7, Some(dir.path()));
        assert_eq!(found.len(), 2, "{found:?}");
        // A direct file path bypasses the extension filter and mtime window.
        let single = find_transcripts(0, Some(&dir.path().join("b.json")));
        assert_eq!(single.len(), 1);
    }
}
