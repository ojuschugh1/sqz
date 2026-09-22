//! `sqz discover --replay` — counterfactual replay of real agent
//! transcripts through the current sqz engine.
//!
//! The chicken-and-egg problem with "install sqz and see what it saves"
//! is that sqz cannot observe agent sessions until the hook is installed.
//! But the agent clients already keep transcripts on disk — Claude Code
//! under `~/.claude/projects/**/*.jsonl`, Kiro under
//! `~/.kiro/sessions/cli/*.jsonl` — and those transcripts contain every
//! tool output the model was actually sent. Replaying them through the
//! engine answers, on the user's own data: "what would sqz have done?"
//!
//! Honesty rules, enforced by the wording in [`render`]:
//! - These are **counterfactual estimates**, not measurements. The
//!   outputs were not compressed at the time.
//! - Each session file replays against a **fresh throwaway store**, so
//!   dedup references never pretend to span sessions.
//! - Replay compresses at replay-time timestamps, so ref expiry inside a
//!   very long real session is not simulated; repeats are assumed to
//!   stay within the freshness window.
//! - Nothing leaves the machine. The throwaway stores are deleted.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::cli_proxy::{CliProxy, InterceptOptions};

/// Transcript dialects sqz can replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    /// Claude Code: `{"type":"assistant"|"user","message":{"content":[...]}}`
    Claude,
    /// Kiro CLI: `{"version":"v1","kind":"AssistantMessage"|"ToolResults","data":{...}}`
    Kiro,
}

/// One tool output extracted from a transcript, in session order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// Command line for shell tools, tool name otherwise. Used as the
    /// compression label so per-command formatters fire exactly as they
    /// would have live.
    pub label: String,
    pub text: String,
}

/// Sniff the dialect from the first parseable line.
pub fn detect_format(first_line: &str) -> Option<TranscriptFormat> {
    let v: serde_json::Value = serde_json::from_str(first_line.trim()).ok()?;
    if v.get("kind").is_some() && v.get("version").is_some() {
        return Some(TranscriptFormat::Kiro);
    }
    if v.get("type").is_some() {
        return Some(TranscriptFormat::Claude);
    }
    None
}

/// Derive the compression label from a tool_use. Shell tools contribute
/// their command line (so `git status` hits the git formatter); everything
/// else contributes the tool name.
fn command_label(name: &str, input: &serde_json::Value) -> String {
    let lowered = name.to_ascii_lowercase();
    let shellish = matches!(lowered.as_str(), "bash" | "powershell" | "shell" | "execute_bash" | "executebash" | "run_terminal_cmd");
    if shellish {
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

/// Feed one Claude Code transcript line through the extraction state
/// machine. `pending` maps tool_use id → label until the result arrives.
fn extract_claude_line(
    v: &serde_json::Value,
    pending: &mut HashMap<String, String>,
    out: &mut Vec<ToolOutput>,
) {
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
                    pending.insert(id.to_string(), command_label(name, &input));
                }
            }
        }
        Some("user") => {
            for item in content {
                if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    let label = item
                        .get("tool_use_id")
                        .and_then(|i| i.as_str())
                        .and_then(|id| pending.remove(id))
                        .unwrap_or_else(|| "tool".to_string());
                    let text = item
                        .get("content")
                        .map(claude_result_text)
                        .unwrap_or_default();
                    if !text.trim().is_empty() {
                        out.push(ToolOutput { label, text });
                    }
                }
            }
        }
        _ => {}
    }
}

/// Feed one Kiro CLI transcript line through the extraction state machine.
fn extract_kiro_line(
    v: &serde_json::Value,
    pending: &mut HashMap<String, String>,
    out: &mut Vec<ToolOutput>,
) {
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
                    pending.insert(id.to_string(), command_label(name, &input));
                }
            }
        }
        Some("ToolResults") => {
            for item in content {
                if item.get("kind").and_then(|k| k.as_str()) == Some("toolResult") {
                    let Some(data) = item.get("data") else { continue };
                    let label = data
                        .get("toolUseId")
                        .and_then(|i| i.as_str())
                        .and_then(|id| pending.remove(id))
                        .unwrap_or_else(|| "tool".to_string());
                    let text = data
                        .get("content")
                        .and_then(|c| c.as_array())
                        .map(|items| {
                            items
                                .iter()
                                .filter(|i| i.get("kind").and_then(|k| k.as_str()) == Some("text"))
                                .filter_map(|i| i.get("data").and_then(|d| d.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    if !text.trim().is_empty() {
                        out.push(ToolOutput { label, text });
                    }
                }
            }
        }
        _ => {}
    }
}

/// Extract every tool output from a transcript, in order.
pub fn extract_tool_outputs<R: BufRead>(reader: R, format: TranscriptFormat) -> Vec<ToolOutput> {
    let mut pending: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match format {
            TranscriptFormat::Claude => extract_claude_line(&v, &mut pending, &mut out),
            TranscriptFormat::Kiro => extract_kiro_line(&v, &mut pending, &mut out),
        }
    }
    out
}

/// Aggregate result of replaying one or more session files.
#[derive(Debug, Default, Clone)]
pub struct ReplayReport {
    pub sessions: usize,
    pub outputs: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// group key → (tokens_in, tokens_saved), sorted by saved desc in render.
    pub groups: HashMap<String, (u64, u64)>,
    pub skipped_files: usize,
}

impl ReplayReport {
    pub fn tokens_saved(&self) -> u64 {
        self.tokens_in.saturating_sub(self.tokens_out)
    }
    pub fn reduction_pct(&self) -> f64 {
        if self.tokens_in == 0 {
            0.0
        } else {
            self.tokens_saved() as f64 * 100.0 / self.tokens_in as f64
        }
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
                let home = PathBuf::from(home);
                roots.push(home.join(".claude").join("projects"));
                roots.push(home.join(".kiro").join("sessions").join("cli"));
            }
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

fn collect_jsonl(dir: &Path, cutoff: Option<std::time::SystemTime>, out: &mut Vec<PathBuf>, depth: u8) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, cutoff, out, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let fresh = match (cutoff, entry.metadata().and_then(|m| m.modified())) {
                (Some(c), Ok(m)) => m >= c,
                _ => true,
            };
            if fresh {
                out.push(path);
            }
        }
    }
}

/// Replay one session file against a fresh throwaway store; fold the
/// per-command stats into `report`. Returns false when the file has no
/// recognisable format or no tool outputs.
fn replay_file(path: &Path, report: &mut ReplayReport) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    let mut reader = std::io::BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).unwrap_or(0) == 0 {
        return false;
    }
    let Some(format) = detect_format(&first_line) else { return false };

    // Re-open so the first line goes through extraction too.
    let Ok(file) = std::fs::File::open(path) else { return false };
    let outputs = extract_tool_outputs(std::io::BufReader::new(file), format);
    if outputs.is_empty() {
        return false;
    }

    // Fresh store per session: dedup references must not span sessions.
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
    for output in &outputs {
        let _ = proxy.intercept_output_with_options(&output.label, &output.text, InterceptOptions::default());
    }

    // Read the throwaway store back for consistent token accounting
    // (same counters the live hook path logs).
    let folded = (|| -> Option<()> {
        let store = sqz_engine::SessionStore::open_or_create(&db).ok()?;
        let stats = store.compression_stats().ok()?;
        report.outputs += stats.total_compressions;
        report.tokens_in += stats.total_tokens_in;
        report.tokens_out += stats.total_tokens_out;
        for cmd in store.command_breakdown(10_000).ok()? {
            let entry = report.groups.entry(group_key(&cmd.command)).or_insert((0, 0));
            entry.0 += cmd.tokens_in;
            entry.1 += cmd.tokens_saved;
        }
        Some(())
    })()
    .is_some();

    // Best-effort cleanup of the throwaway store.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{}", db.display(), suffix)));
    }

    if folded {
        report.sessions += 1;
    }
    folded
}

/// Render the report. Kept separate from collection so tests can assert
/// on the wording that carries the honesty requirements.
pub fn render(report: &ReplayReport, days: u32) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "sqz discover — counterfactual replay (last {days} days)\n"
    ));
    out.push_str(&"─".repeat(56));
    out.push('\n');
    out.push_str(&format!(
        "\n  Sessions replayed:   {}\n  Tool outputs:        {}\n  Tokens (original):   {}\n  Tokens (compressed): {}\n\n  Estimated avoidable: {} tokens ({:.1}%)\n",
        report.sessions,
        report.outputs,
        report.tokens_in,
        report.tokens_out,
        report.tokens_saved(),
        report.reduction_pct(),
    ));

    let mut groups: Vec<(&String, &(u64, u64))> = report.groups.iter().collect();
    groups.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    let top: Vec<_> = groups.into_iter().filter(|(_, (_, saved))| *saved > 0).take(8).collect();
    if !top.is_empty() {
        out.push_str("\n  Largest opportunities:\n\n");
        out.push_str(&format!("    {:<18} {:>12} {:>12}\n", "source", "tokens in", "avoidable"));
        for (group, (tin, saved)) in top {
            // Dedup and slice hits are logged under internal labels;
            // translate them for the report.
            let display = match group.as_str() {
                "dedup" => "(repeated output)",
                "slice" => "(ranged re-read)",
                other => other,
            };
            let name = if display.chars().count() > 16 {
                let cut: String = display.chars().take(15).collect();
                format!("{cut}…")
            } else {
                display.to_string()
            };
            out.push_str(&format!("    {:<18} {:>12} {:>12}\n", name, tin, saved));
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
         To start capturing these savings:\n    sqz init --global\n",
    );
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
             Looked in ~/.claude/projects and ~/.kiro/sessions/cli.\n\
             Point at a file or directory with: sqz discover --replay --transcripts PATH"
        );
        return 1;
    }
    eprintln!("[sqz] replaying {} transcript file(s)…", files.len());
    let mut report = ReplayReport::default();
    for file in &files {
        if !replay_file(file, &mut report) {
            report.skipped_files += 1;
        }
    }
    if report.sessions == 0 {
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
        assert_eq!(detect_format("not json"), None);
        assert_eq!(detect_format("{\"foo\":1}"), None);
    }

    #[test]
    fn extracts_claude_tool_outputs_with_labels() {
        let outputs = extract_tool_outputs(CLAUDE_LINES.as_bytes(), TranscriptFormat::Claude);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].label, "git status");
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
    fn render_carries_the_counterfactual_wording() {
        let mut report = ReplayReport::default();
        report.sessions = 2;
        report.outputs = 10;
        report.tokens_in = 1000;
        report.tokens_out = 700;
        report.groups.insert("git".to_string(), (600, 200));
        let text = render(&report, 7);
        assert!(text.contains("counterfactual replay"));
        assert!(text.contains("estimates, not"));
        assert!(text.contains("nothing was uploaded"));
        assert!(text.contains("300 tokens (30.0%)"));
        assert!(text.contains("git"));
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
        assert_eq!(report.sessions, 1);
        assert_eq!(report.outputs, 2);
        assert!(report.tokens_in > 0);
        assert!(
            report.tokens_out < report.tokens_in,
            "repeat must dedup: in={} out={}",
            report.tokens_in,
            report.tokens_out
        );
        assert!(report.groups.contains_key("cat"), "{:?}", report.groups);
    }

    #[test]
    fn find_transcripts_honours_override_and_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jsonl"), "{}").unwrap();
        std::fs::write(dir.path().join("b.json"), "{}").unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/c.jsonl"), "{}").unwrap();
        let found = find_transcripts(7, Some(dir.path()));
        assert_eq!(found.len(), 2, "{found:?}");
        // A direct file path bypasses the extension filter and mtime window.
        let single = find_transcripts(0, Some(&dir.path().join("b.json")));
        assert_eq!(single.len(), 1);
    }
}
