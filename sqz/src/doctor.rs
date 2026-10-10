//! `sqz doctor` — deployment validator.
//!
//! `sqz init` writes configs; doctor answers the question that comes a
//! week later: "is sqz actually doing anything?" It checks four layers
//! and reports the gaps between them:
//!
//! 1. **Installation**: binary version, database, shell helpers.
//! 2. **Wiring**: per-client sqz configs (project and user level), and
//!    which tier each integration is in. A transparent client routes shell
//!    output through sqz on its own; an advisory one only saves tokens
//!    when the agent chooses to call sqz.
//! 3. **Overhead**: what sqz itself adds to every model request (MCP tool
//!    definitions, guidance files), including duplicate and stale copies.
//! 4. **Activity**: compressions logged recently, and log rows a past
//!    bug wrote wrongly.
//!
//! Client detection is best-effort by footprint (config dir or binary on
//! PATH); Cline has no reliable machine-level footprint.
//!
//! Exit codes: 0 = healthy, 1 = actionable problems found, 2 = sqz itself
//! is broken (database unreadable).

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqz_engine::{IntegrationTier, SteeringState};

use crate::shell_hook::ShellHook;

/// Whether the client application itself appears to exist on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Detected,
    NotDetected,
    /// No reliable machine-level footprint to check (Cline lives inside
    /// VS Code and only leaves project-level traces).
    Unknown,
}

/// Where a sqz config for the client was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    None,
    Project,
    Global,
    Both,
}

impl ConfigScope {
    fn from_flags(project: bool, global: bool) -> Self {
        match (project, global) {
            (true, true) => ConfigScope::Both,
            (true, false) => ConfigScope::Project,
            (false, true) => ConfigScope::Global,
            (false, false) => ConfigScope::None,
        }
    }

    pub fn configured(self) -> bool {
        self != ConfigScope::None
    }
}

/// One client's status line.
#[derive(Debug, Clone)]
pub struct ToolStatus {
    /// Display name as used by `generate_hook_configs`.
    pub name: &'static str,
    /// Value accepted by `sqz init --only`.
    pub only_flag: &'static str,
    pub client: Detection,
    pub config: ConfigScope,
    pub tier: IntegrationTier,
}

/// A diagnosed problem plus the command that fixes it, when one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
    pub fix: Option<String>,
}

/// Recent-activity numbers pulled from the session store.
#[derive(Debug, Clone, Copy, Default)]
pub struct Activity {
    pub total_compressions: u32,
    pub total_saved: u64,
    pub last7_compressions: u32,
    pub last7_saved: u64,
    /// Aggregate output exceeds aggregate input: the log has bad rows.
    pub net_negative: bool,
    /// MCP read-tool rows logged as larger than their input (pre-1.8
    /// estimator bug), and the tokens they hide from the saved total.
    pub inflated_rows: u32,
    pub inflated_tokens: u64,
}

/// State of the sqz-managed guidance and shell files doctor can repair.
#[derive(Debug, Clone)]
pub struct Managed {
    pub kiro_global: SteeringState,
    pub kiro_project: SteeringState,
    pub agents_md: SteeringState,
    pub zed_rules: SteeringState,
    pub claude_md: SteeringState,
    pub cursor_rules: SteeringState,
    pub windsurf_rules: SteeringState,
    pub cline_rules: SteeringState,
    pub shell_block_stale: bool,
    /// A Claude Code settings file has the sqz hook without the allow rules
    /// and PostToolUse hook current sqz writes next to it.
    pub claude_stale: bool,
    /// sqz's Claude Code hooks are in both the user-level and the project settings.
    pub claude_duplicate: bool,
    /// The OpenCode plugin file was written by an older sqz.
    pub opencode_plugin_stale: bool,
    /// Hook and plugin files that run a sqz binary that does not exist, as `file (binary)`.
    pub missing_binaries: Vec<String>,
    /// Hook files an older sqz wrote that current sqz no longer uses.
    pub legacy_hooks: Vec<String>,
    /// The project's `.gemini/settings.json` is not plain JSON, so `sqz init` won't edit it.
    pub gemini_unparseable: bool,
}

impl Managed {
    fn kiro_duplicate(&self) -> bool {
        is_managed(self.kiro_global) && is_managed(self.kiro_project)
    }
}

fn is_managed(state: SteeringState) -> bool {
    matches!(state, SteeringState::Current | SteeringState::Stale)
}

const FIX_CMD: &str = "sqz doctor --fix";

/// Pure diagnosis over collected state. Separated from collection and
/// rendering so the rules are unit-testable without a filesystem.
pub fn diagnose(tools: &[ToolStatus], activity: &Activity, managed: &Managed) -> Vec<Problem> {
    let mut problems = Vec::new();

    let (by_hand, unconfigured): (Vec<&ToolStatus>, Vec<&ToolStatus>) = tools
        .iter()
        .filter(|t| t.client == Detection::Detected && !t.config.configured())
        .partition(|t| t.name == "Gemini CLI" && managed.gemini_unparseable);
    if !unconfigured.is_empty() {
        let names: Vec<&str> = unconfigured.iter().map(|t| t.name).collect();
        let flags: Vec<&str> = unconfigured.iter().map(|t| t.only_flag).collect();
        problems.push(Problem {
            message: format!(
                "{} detected but not routed through sqz: {}",
                if names.len() == 1 { "1 client" } else { "clients" },
                names.join(", ")
            ),
            fix: Some(format!("sqz init --only {}", flags.join(","))),
        });
    }
    if !by_hand.is_empty() {
        problems.push(Problem {
            message: "Gemini CLI detected, but .gemini/settings.json is not plain JSON (comments?), so sqz will not edit it".to_string(),
            fix: Some("add sqz's BeforeTool hook to it by hand (sqz init --only gemini prints the entry)".to_string()),
        });
    }

    if managed.kiro_global == SteeringState::Stale || managed.kiro_project == SteeringState::Stale {
        problems.push(Problem {
            message: "Kiro steering was written by an older sqz, so the agent is reading outdated guidance".to_string(),
            fix: Some(FIX_CMD.to_string()),
        });
    }
    if managed.kiro_duplicate() {
        problems.push(Problem {
            message: "Kiro loads sqz steering twice (user-level and this project), on every request".to_string(),
            fix: Some(format!("{FIX_CMD}   (removes the project copy; the user-level one covers every workspace)")),
        });
    }
    for (file, state) in [
        ("AGENTS.md", managed.agents_md),
        (".rules", managed.zed_rules),
        ("CLAUDE.md", managed.claude_md),
    ] {
        if state == SteeringState::Stale {
            problems.push(Problem {
                message: format!(
                    "the sqz guidance in {file} was written by an older sqz, so agents read outdated or duplicated guidance on every request"
                ),
                fix: Some(format!(
                    "{FIX_CMD}   (replaces the sqz blocks with one current block; the rest of {file} stays as it is)"
                )),
            });
        }
    }
    let stale_rules: Vec<&str> = [
        ("Cursor", managed.cursor_rules),
        ("Windsurf", managed.windsurf_rules),
        ("Cline", managed.cline_rules),
    ]
    .into_iter()
    .filter(|(_, state)| *state == SteeringState::Stale)
    .map(|(name, _)| name)
    .collect();
    if !stale_rules.is_empty() {
        problems.push(Problem {
            message: format!(
                "the sqz rules for {} were written by an older sqz, so the agent is reading outdated guidance",
                stale_rules.join(", ")
            ),
            fix: Some(FIX_CMD.to_string()),
        });
    }
    if managed.claude_stale {
        problems.push(Problem {
            message: "the Claude Code hook comes from an older sqz install: without the two allow rules for its pipe it leaves commands uncompressed, and the PostToolUse hook is missing".to_string(),
            fix: Some(format!("{FIX_CMD}   (adds Bash(set -o pipefail) and Bash(sqz compress --cmd *) to the allow list)")),
        });
    }
    if managed.opencode_plugin_stale {
        problems.push(Problem {
            message: "the OpenCode plugin was written by an older sqz, so OpenCode 2 does not load it and runs every command uncompressed".to_string(),
            fix: Some(format!("{FIX_CMD}   (rewrites it for the sqz binary it already runs)")),
        });
    }
    if !managed.legacy_hooks.is_empty() {
        problems.push(Problem {
            message: format!(
                "an older sqz left hook files that current sqz no longer uses: {}",
                managed.legacy_hooks.join(", ")
            ),
            fix: Some(format!("{FIX_CMD}   (removes sqz's hooks from them; anything else in them stays)")),
        });
    }
    if !managed.missing_binaries.is_empty() {
        problems.push(Problem {
            message: format!(
                "these sqz hook files run a sqz binary that does not exist, so the hook fails each time: {}",
                managed.missing_binaries.join(", ")
            ),
            fix: Some(format!("{FIX_CMD}   (points them at this sqz binary)")),
        });
    }
    if managed.claude_duplicate {
        problems.push(Problem {
            message: "the sqz Claude Code hooks are installed twice (user-level and this project's .claude/settings.local.json), so a Bash call can go through sqz twice".to_string(),
            fix: Some(format!("{FIX_CMD}   (removes them from the project file and keeps its permissions; the user-level ones cover every project)")),
        });
    }
    if managed.shell_block_stale {
        problems.push(Problem {
            message: "the sqz block in your shell rc is outdated (older versions also defined a preexec function or DEBUG trap that can override your own)".to_string(),
            fix: Some(FIX_CMD.to_string()),
        });
    }

    if activity.inflated_rows > 0 {
        problems.push(Problem {
            message: format!(
                "{} log row(s) from sqz-mcp before 1.8.0 record verbatim output as larger than its input, hiding {} tokens{}",
                activity.inflated_rows,
                activity.inflated_tokens,
                if activity.net_negative { " and pushing the all-time total below zero" } else { "" },
            ),
            fix: Some(format!("{FIX_CMD}   (backs up the database, then corrects those rows)")),
        });
    }

    let configured: Vec<&ToolStatus> = tools.iter().filter(|t| t.config.configured()).collect();
    let any_transparent = configured.iter().any(|t| t.tier == IntegrationTier::Transparent);
    let advisory_names = configured
        .iter()
        .filter(|t| t.tier == IntegrationTier::Advisory)
        .map(|t| t.name)
        .collect::<Vec<_>>()
        .join(", ");

    if activity.total_compressions == 0 && !configured.is_empty() {
        problems.push(if any_transparent {
            Problem {
                message: "hooks are installed but nothing has been compressed yet — restart your AI tool so it picks up the hook".to_string(),
                fix: None,
            }
        } else {
            Problem {
                message: format!(
                    "nothing has been compressed yet. Only advisory integrations are configured ({advisory_names}), which save tokens only when the agent calls sqz"
                ),
                fix: Some("sqz discover --replay   (shows how much of your agent's tool output sqz could reach)".to_string()),
            }
        });
    } else if activity.total_compressions > 0 && activity.last7_compressions == 0 && any_transparent {
        problems.push(Problem {
            message: format!(
                "sqz has compressed {} outputs all-time but none in the last 7 days — a hook may have stopped firing (tool update, changed settings)",
                activity.total_compressions
            ),
            fix: Some("sqz init".to_string()),
        });
    }

    problems
}

/// Back the live database up to `<db>.bak-YYYYMMDD-HHMMSS` before a
/// repair. Resolves the path the same way the engine does (SQZ_DB_PATH,
/// else ~/.sqz/sessions.db). Returns `Ok(None)` when no file exists.
fn backup_database(store: &sqz_engine::SessionStore) -> Result<Option<PathBuf>, String> {
    let db = sqz_engine::SqzEngine::default_store_path();
    if !db.exists() {
        return Ok(None);
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let backup = PathBuf::from(format!("{}.bak-{stamp}", db.display()));
    store.backup_to(&backup).map_err(|e| e.to_string())?;
    Ok(Some(backup))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

fn binary_on_path(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file() || {
            #[cfg(windows)]
            {
                dir.join(format!("{name}.exe")).is_file()
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
    })
}

fn file_contains(path: &Path, needle: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.contains(needle))
        .unwrap_or(false)
}

/// Detect the client application's machine-level footprint.
fn detect_client(name: &str, home: Option<&Path>) -> Detection {
    let dir_exists = |rel: &str| home.map(|h| h.join(rel).exists()).unwrap_or(false);
    match name {
        "Claude Code" => yes_no(dir_exists(".claude") || dir_exists(".claude.json") || binary_on_path("claude")),
        "Cursor" => yes_no(dir_exists(".cursor") || binary_on_path("cursor-agent")),
        "Windsurf" => yes_no(dir_exists(".codeium")),
        "Cline" => Detection::Unknown,
        "Gemini CLI" => yes_no(dir_exists(".gemini") || binary_on_path("gemini")),
        "Kiro" => yes_no(dir_exists(".kiro") || binary_on_path("kiro")),
        "OpenCode" => yes_no(dir_exists(".config/opencode") || binary_on_path("opencode")),
        "Codex" => yes_no(dir_exists(".codex") || binary_on_path("codex")),
        "Zed" => yes_no(
            dir_exists(".config/zed")
                || binary_on_path("zed")
                || Path::new("/Applications/Zed.app").exists(),
        ),
        "Copilot CLI" => yes_no(dir_exists(".copilot") || binary_on_path("copilot")),
        _ => Detection::Unknown,
    }
}

fn yes_no(found: bool) -> Detection {
    if found {
        Detection::Detected
    } else {
        Detection::NotDetected
    }
}

/// Check whether sqz's config for a tool exists at project and user level.
///
/// Project paths come from `generate_hook_configs` so doctor stays in sync
/// with what `sqz init` actually writes; tools with merge-style installs
/// (Claude, Gemini, Kiro, OpenCode, Codex, Zed) additionally require the
/// file to mention sqz, since the file existing is not evidence we are in it.
fn detect_config(name: &str, project_config: &Path, project_dir: &Path, home: Option<&Path>) -> ConfigScope {
    let h = |rel: &str| home.map(|hm| hm.join(rel));
    let project = match name {
        // Merge targets: presence alone is not enough.
        "Claude Code" | "Gemini CLI" => file_contains(project_config, "sqz"),
        "OpenCode" => sqz_engine::find_opencode_config(project_dir)
            .map(|p| file_contains(&p, "sqz"))
            .unwrap_or(false),
        "Codex" | "Zed" => file_contains(project_config, "sqz"),
        "Cline" => {
            // .clinerules is either a sqz-authored file or a directory
            // holding sqz.md next to the user's own rules.
            let base = project_dir.join(".clinerules");
            if base.is_dir() {
                base.join("sqz.md").exists()
            } else {
                file_contains(&base, "sqz")
            }
        }
        // Files sqz owns outright: existence is the signal.
        _ => project_config.exists(),
    };

    let global = match name {
        "Claude Code" => sqz_engine::claude_user_settings_path()
            .map(|p| file_contains(&p, "sqz"))
            .unwrap_or(false),
        "Gemini CLI" => h(".gemini/settings.json").map(|p| file_contains(&p, "sqz")).unwrap_or(false),
        "Kiro" => {
            let steering = sqz_engine::kiro_user_steering_path().exists();
            let mcp = h(".kiro/settings/mcp.json").map(|p| file_contains(&p, "sqz")).unwrap_or(false);
            steering || mcp
        }
        "OpenCode" => {
            let cfg = h(".config/opencode/opencode.json").map(|p| file_contains(&p, "sqz")).unwrap_or(false)
                || h(".config/opencode/opencode.jsonc").map(|p| file_contains(&p, "sqz")).unwrap_or(false);
            let plugin = h(".config/opencode/plugin/sqz.ts").map(|p| p.exists()).unwrap_or(false)
                || h(".config/opencode/plugins/sqz.ts").map(|p| p.exists()).unwrap_or(false);
            cfg || plugin
        }
        "Codex" => h(".codex/config.toml").map(|p| file_contains(&p, "sqz")).unwrap_or(false),
        "Zed" => h(".config/zed/settings.json").map(|p| file_contains(&p, "sqz")).unwrap_or(false),
        "Copilot CLI" => sqz_engine::copilot_hooks_path().exists(),
        _ => false,
    };

    ConfigScope::from_flags(project, global)
}

/// `--only` flag value for each display name, kept in the order
/// `SUPPORTED_TOOL_NAMES` uses.
fn only_flag(name: &str) -> &'static str {
    match name {
        "Claude Code" => "claude",
        "Cursor" => "cursor",
        "Windsurf" => "windsurf",
        "Cline" => "cline",
        "Gemini CLI" => "gemini",
        "Kiro" => "kiro",
        "OpenCode" => "opencode",
        "Codex" => "codex",
        "Zed" => "zed",
        "Copilot CLI" => "copilot",
        _ => "",
    }
}

/// Collect per-tool status for every supported client.
pub fn collect_tool_statuses(project_dir: &Path) -> Vec<ToolStatus> {
    let home = home_dir();
    let configs = sqz_engine::generate_hook_configs("sqz");
    sqz_engine::SUPPORTED_TOOL_NAMES
        .iter()
        .map(|&name| {
            let project_config = configs
                .iter()
                .find(|c| c.tool_name == name)
                .map(|c| project_dir.join(&c.config_path))
                .unwrap_or_else(|| project_dir.join("(none)"));
            let client = detect_client(name, home.as_deref());
            let config = detect_config(name, &project_config, project_dir, home.as_deref());
            let tier = sqz_engine::integration_tier(name);
            ToolStatus { name, only_flag: only_flag(name), client, config, tier }
        })
        .collect()
}

/// Number and size of the tool definitions the installed `sqz-mcp`
/// advertises. Clients send these to the model with every request.
/// `None` when sqz-mcp isn't on PATH or doesn't answer within 5 seconds.
fn measure_mcp_tool_definitions() -> Option<(usize, usize)> {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    let mut child = Command::new("sqz-mcp")
        .args(["--transport", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let requests = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"sqz-doctor","version":"0"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ];
        for request in requests {
            if writeln!(stdin, "{request}").is_err() {
                break;
            }
        }
        // Dropping stdin sends EOF so the server exits after answering.
    }
    let stdout = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(stdout) = stdout {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                if msg.get("id") == Some(&serde_json::json!(2)) {
                    let _ = tx.send(msg);
                    return;
                }
            }
        });
    }
    let response = rx.recv_timeout(Duration::from_secs(5)).ok();
    let _ = child.kill();
    let _ = child.wait();
    let tools = response?.get("result")?.get("tools")?.as_array()?.clone();
    let bytes = tools.iter().map(|t| t.to_string().len()).sum();
    Some((tools.len(), bytes))
}

/// sqz guidance Kiro loads into every request for this project: user and
/// project steering, plus every sqz block in AGENTS.md (Kiro includes the
/// workspace AGENTS.md automatically). Returns (label, bytes) pairs.
fn kiro_guidance(project_dir: &Path) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for (label, path, global) in [
        ("~/.kiro/steering/sqz.md".to_string(), sqz_engine::kiro_user_steering_path(), true),
        (".kiro/steering/sqz.md".to_string(), sqz_engine::kiro_steering_path(project_dir), false),
    ] {
        if is_managed(sqz_engine::kiro_steering_state(&path, global)) {
            let bytes = std::fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0);
            found.push((label, bytes));
        }
    }
    found.extend(guidance_block_copies(&project_dir.join("AGENTS.md"), "AGENTS.md"));
    found
}

fn guidance_block_copies(path: &Path, label: &str) -> Vec<(String, usize)> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    sqz_engine::guidance_blocks(&content)
        .into_iter()
        .map(|block| {
            let kind = if block.legacy { " (old)" } else { "" };
            (format!("{label} block{kind}"), block.bytes)
        })
        .collect()
}

fn print_guidance(label: &str, copies: &[(String, usize)]) {
    let bytes: usize = copies.iter().map(|(_, b)| b).sum();
    let labels: Vec<&str> = copies.iter().map(|(l, _)| l.as_str()).collect();
    println!(
        "  {label:<16}{} cop{}, ~{} tokens{}{}",
        copies.len(),
        if copies.len() == 1 { "y" } else { "ies" },
        approx_tokens(bytes),
        if labels.is_empty() { "" } else { ": " },
        labels.join(", ")
    );
}

fn guidance_readers(tools: &[ToolStatus], project_dir: &Path) -> Vec<(Vec<&'static str>, PathBuf)> {
    let mut readers: Vec<(Vec<&'static str>, PathBuf)> = Vec::new();
    for t in tools.iter().filter(|t| matches!(t.name, "Codex" | "Zed") && t.config.configured()) {
        let path = if t.name == "Zed" {
            sqz_engine::zed_guidance_target(project_dir)
        } else {
            sqz_engine::agents_md_path(project_dir)
        };
        match readers.iter_mut().find(|(_, p)| *p == path) {
            Some((clients, _)) => clients.push(t.name),
            None => readers.push((vec![t.name], path)),
        }
    }
    readers
}

fn rules_state(project_dir: &Path, tool: &str) -> SteeringState {
    sqz_engine::sqz_rules_path(project_dir, tool)
        .map(|path| sqz_engine::sqz_rules_state(&path, tool))
        .unwrap_or(SteeringState::Missing)
}

fn approx_tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

/// Repair the sqz-managed state doctor knows how to repair. Every change
/// is to something sqz wrote; user-authored files are never touched.
fn apply_fixes(store: &sqz_engine::SessionStore, managed: &Managed, project_dir: &Path) -> Result<(), ()> {
    let (rows, _) = store.inflated_rows().unwrap_or((0, 0));
    if rows > 0 {
        match backup_database(store) {
            Ok(Some(path)) => println!("  ok  --fix: database backed up to {}", path.display()),
            Ok(None) => println!("  --  --fix: database file not found, skipping backup"),
            Err(e) => {
                println!("  ERR --fix: backup failed ({e}), not touching the database");
                return Err(());
            }
        }
        match store.clamp_inflated_rows() {
            Ok(n) => println!("  ok  --fix: corrected {n} log row(s) (input count raised to the real output size, saving 0)"),
            Err(e) => println!("  ERR --fix: update failed: {e}"),
        }
    }

    // Refreshes keep the binary path each file already uses; current_exe
    // is only the fallback when that binary no longer exists.
    let fallback = std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "sqz".to_string());
    if managed.kiro_global == SteeringState::Stale {
        match sqz_engine::refresh_kiro_steering(&sqz_engine::kiro_user_steering_path(), true, &fallback) {
            Ok(_) => println!("  ok  --fix: refreshed ~/.kiro/steering/sqz.md"),
            Err(e) => println!("  ERR --fix: could not refresh Kiro steering: {e}"),
        }
    }
    if managed.kiro_duplicate() {
        match sqz_engine::remove_kiro_steering(project_dir) {
            Ok(true) => println!("  ok  --fix: removed duplicate .kiro/steering/sqz.md (user-level copy covers this workspace)"),
            Ok(false) => {}
            Err(e) => println!("  ERR --fix: could not remove project steering: {e}"),
        }
    } else if managed.kiro_project == SteeringState::Stale {
        match sqz_engine::refresh_kiro_steering(&sqz_engine::kiro_steering_path(project_dir), false, &fallback) {
            Ok(_) => println!("  ok  --fix: refreshed .kiro/steering/sqz.md"),
            Err(e) => println!("  ERR --fix: could not refresh project steering: {e}"),
        }
    }

    for (file, state) in [("AGENTS.md", managed.agents_md), (".rules", managed.zed_rules)] {
        if state != SteeringState::Stale {
            continue;
        }
        match sqz_engine::refresh_agents_md_guidance(&project_dir.join(file), &fallback) {
            Ok(_) => println!("  ok  --fix: replaced the sqz guidance in {file} with one current block"),
            Err(e) => println!("  ERR --fix: could not refresh {file}: {e}"),
        }
    }
    if managed.claude_md == SteeringState::Stale {
        match sqz_engine::refresh_claude_md_guidance(project_dir, &fallback) {
            Ok(_) => println!("  ok  --fix: replaced the sqz guidance in CLAUDE.md with one current block"),
            Err(e) => println!("  ERR --fix: could not refresh CLAUDE.md: {e}"),
        }
    }
    for (tool, state) in [
        ("Cursor", managed.cursor_rules),
        ("Windsurf", managed.windsurf_rules),
        ("Cline", managed.cline_rules),
    ] {
        if state != SteeringState::Stale {
            continue;
        }
        let Some(path) = sqz_engine::sqz_rules_path(project_dir, tool) else {
            continue;
        };
        let shown = path.strip_prefix(project_dir).unwrap_or(&path).display().to_string();
        match sqz_engine::refresh_sqz_rules(&path, tool, &fallback) {
            Ok(_) => println!("  ok  --fix: refreshed {shown}"),
            Err(e) => println!("  ERR --fix: could not refresh {shown}: {e}"),
        }
    }

    if managed.shell_block_stale {
        match ShellHook::detect().refresh() {
            Ok(true) => println!("  ok  --fix: refreshed the sqz block in your shell rc (open a new shell to pick it up)"),
            Ok(false) => {}
            Err(e) => println!("  ERR --fix: {e}"),
        }
    }

    if managed.claude_duplicate {
        match sqz_engine::remove_claude_project_hooks_only(project_dir) {
            Ok(Some((path, true))) if path.exists() => println!(
                "  ok  --fix: removed the sqz hooks from {} (the user-level ones cover every project)",
                path.display()
            ),
            Ok(Some((path, true))) => println!(
                "  ok  --fix: removed {} (it held only sqz hooks the user-level ones cover)",
                path.display()
            ),
            Ok(_) => {}
            Err(e) => println!("  ERR --fix: could not remove the project's sqz hooks: {e}"),
        }
    }
    let missing = missing_hook_binaries(project_dir);
    for path in claude_settings_paths(project_dir) {
        let stale = sqz_engine::claude_settings_stale(&path);
        if !stale && !missing.iter().any(|(p, _)| *p == path) {
            continue;
        }
        match sqz_engine::refresh_claude_settings(&path, &fallback) {
            Ok(_) if stale => println!("  ok  --fix: added the PostToolUse hook and pipe allow rules to {}", path.display()),
            Ok(_) => println!("  ok  --fix: repointed the sqz hooks in {}", path.display()),
            Err(e) => println!("  ERR --fix: could not update {}: {e}", path.display()),
        }
    }
    let opencode = sqz_engine::opencode_plugin_path();
    if managed.opencode_plugin_stale && !missing.iter().any(|(p, _)| *p == opencode) {
        match sqz_engine::refresh_opencode_plugin(&opencode, &fallback) {
            Ok(_) => println!("  ok  --fix: refreshed {}", opencode.display()),
            Err(e) => println!("  ERR --fix: could not refresh {}: {e}", opencode.display()),
        }
    }
    let gemini = project_dir.join(".gemini").join("settings.json");
    for (path, _) in missing.iter().filter(|(p, _)| !claude_settings_paths(project_dir).contains(p)) {
        if sqz_engine::resolve_sqz_binary(&fallback).is_none() {
            println!("  ERR --fix: {fallback} does not exist, so {} was left as it is", path.display());
            continue;
        }
        let result = if *path == gemini {
            sqz_engine::merge_sqz_gemini_hook(path, &fallback)
        } else if *path == sqz_engine::copilot_hooks_path() {
            sqz_engine::install_copilot_hook(&fallback).map(|changed| changed.unwrap_or(false))
        } else {
            sqz_engine::install_opencode_plugin(&fallback)
        };
        match result {
            Ok(_) => println!("  ok  --fix: pointed {} at {fallback}", path.display()),
            Err(e) => println!("  ERR --fix: could not update {}: {e}", path.display()),
        }
    }
    if !managed.legacy_hooks.is_empty() {
        match sqz_engine::remove_legacy_sqz_hooks(project_dir) {
            Ok(changed) => {
                for (path, deleted) in changed {
                    if deleted {
                        println!("  ok  --fix: removed {} (an older sqz wrote it)", path.display());
                    } else {
                        println!("  ok  --fix: removed the old sqz hooks from {}", path.display());
                    }
                }
            }
            Err(e) => println!("  ERR --fix: could not remove the old sqz hooks: {e}"),
        }
    }
    Ok(())
}

/// The hook and plugin files `sqz init` writes that run a sqz binary.
fn hook_files(project_dir: &Path) -> Vec<PathBuf> {
    let mut files = claude_settings_paths(project_dir);
    files.push(project_dir.join(".gemini").join("settings.json"));
    files.push(sqz_engine::copilot_hooks_path());
    files.push(sqz_engine::opencode_plugin_path());
    files
}

/// Each hook file whose sqz binary does not resolve, with that binary.
fn missing_hook_binaries(project_dir: &Path) -> Vec<(PathBuf, String)> {
    hook_files(project_dir)
        .into_iter()
        .filter_map(|path| {
            let bin = sqz_engine::sqz_hook_binaries(&path)
                .into_iter()
                .find(|b| sqz_engine::resolve_sqz_binary(b).is_none())?;
            Some((path, bin))
        })
        .collect()
}

/// The Claude Code settings files `sqz init` writes, user and project.
fn claude_settings_paths(project_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = sqz_engine::claude_user_settings_path().into_iter().collect();
    paths.push(project_dir.join(".claude").join("settings.local.json"));
    paths
}

fn collect_managed(project_dir: &Path, hook: &ShellHook) -> Managed {
    Managed {
        kiro_global: sqz_engine::kiro_steering_state(&sqz_engine::kiro_user_steering_path(), true),
        kiro_project: sqz_engine::kiro_steering_state(&sqz_engine::kiro_steering_path(project_dir), false),
        agents_md: sqz_engine::agents_md_guidance_state(&sqz_engine::agents_md_path(project_dir)),
        zed_rules: sqz_engine::agents_md_guidance_state(&project_dir.join(".rules")),
        claude_md: sqz_engine::claude_md_guidance_state(project_dir),
        cursor_rules: rules_state(project_dir, "Cursor"),
        windsurf_rules: rules_state(project_dir, "Windsurf"),
        cline_rules: rules_state(project_dir, "Cline"),
        shell_block_stale: hook.needs_refresh(),
        claude_stale: claude_settings_paths(project_dir)
            .iter()
            .any(|p| sqz_engine::claude_settings_stale(p)),
        claude_duplicate: sqz_engine::claude_user_hooks_installed()
            && sqz_engine::claude_settings_have_sqz_hooks(
                &project_dir.join(".claude").join("settings.local.json"),
            ),
        opencode_plugin_stale: sqz_engine::opencode_plugin_stale(&sqz_engine::opencode_plugin_path()),
        missing_binaries: missing_hook_binaries(project_dir)
            .into_iter()
            .map(|(path, bin)| format!("{} ({bin})", path.display()))
            .collect(),
        legacy_hooks: sqz_engine::legacy_sqz_hook_files(project_dir)
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        gemini_unparseable: sqz_engine::gemini_settings_needs_update(
            &project_dir.join(".gemini").join("settings.json"),
            "sqz",
        )
        .is_err(),
    }
}

/// Run the full check and print the report. Returns the process exit code.
///
/// With `fix`, repairs what doctor can repair on its own: log rows a past
/// bug wrote wrongly (after backing up the database), stale or duplicate
/// sqz-managed Kiro steering, an outdated sqz shell block, and a Claude
/// Code hook missing the allow rules and PostToolUse entry.
pub fn run(fix: bool) -> i32 {
    let version = env!("CARGO_PKG_VERSION");
    println!("sqz doctor — {version}");
    println!();

    // ── Installation ────────────────────────────────────────────────
    println!("installation");
    println!("  ok  sqz {version}");
    let engine = match sqz_engine::SqzEngine::new() {
        Ok(e) => e,
        Err(e) => {
            println!("  ERR database: {e}");
            println!();
            println!("sqz cannot open its database. Check SQZ_DB_PATH and permissions on ~/.sqz.");
            return 2;
        }
    };
    let store = engine.session_store();
    println!("  ok  database open and readable");

    let hook = ShellHook::detect();
    let rc_path = hook.rc_path();
    let shell_installed = file_contains(&rc_path, hook.sentinel());
    if shell_installed {
        println!("  ok  shell helpers (sqz_run) in {}", rc_path.display());
    } else {
        println!("  --  no shell helpers in {} (optional: they only add sqz_run)", rc_path.display());
    }

    let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if fix {
        let managed = collect_managed(&project_dir, &hook);
        if apply_fixes(store, &managed, &project_dir).is_err() {
            return 2;
        }
    }
    let this_sqz = std::env::current_exe().and_then(std::fs::canonicalize).ok();
    for path in hook_files(&project_dir) {
        for bin in sqz_engine::sqz_hook_binaries(&path) {
            let Some(file) = sqz_engine::resolve_sqz_binary(&bin) else { continue };
            if std::fs::canonicalize(&file).ok() != this_sqz {
                println!("  --  {} runs {bin}, not this sqz binary", path.display());
            }
        }
    }
    let managed = collect_managed(&project_dir, &hook);

    // ── Clients ─────────────────────────────────────────────────────
    let tools = collect_tool_statuses(&project_dir);
    println!();
    println!("clients (project: {})", project_dir.display());
    for t in &tools {
        let mark = match (t.client, t.config.configured()) {
            (_, true) => "ok ",
            (Detection::Detected, false) => "!! ",
            (Detection::NotDetected, false) => "-- ",
            (Detection::Unknown, false) => "?  ",
        };
        let state = match t.config {
            ConfigScope::Both => "configured (project + global)",
            ConfigScope::Project => "configured (project)",
            ConfigScope::Global => "configured (global)",
            ConfigScope::None => match t.client {
                Detection::Detected => "detected, NOT configured",
                Detection::NotDetected => "not detected",
                Detection::Unknown => "no machine-level footprint to check",
            },
        };
        let tier = if t.config.configured() {
            match t.tier {
                IntegrationTier::Transparent => "transparent",
                IntegrationTier::Advisory => "advisory",
            }
        } else {
            ""
        };
        println!("  {mark} {:<12} {state:<31} {tier}", t.name);
    }
    let configured_tiers: Vec<IntegrationTier> =
        tools.iter().filter(|t| t.config.configured()).map(|t| t.tier).collect();
    if configured_tiers.contains(&IntegrationTier::Transparent) {
        println!("      transparent: a hook routes shell commands through sqz, no agent cooperation needed");
    }
    if configured_tiers.contains(&IntegrationTier::Advisory) {
        println!("      advisory: the client can't let sqz rewrite tool calls. sqz saves tokens only when");
        println!("      the agent pipes output through `sqz compress` or calls the sqz MCP tools; the");
        println!("      client's built-in shell and file tools bypass it");
    }

    // ── Overhead ────────────────────────────────────────────────────
    println!();
    println!("context overhead (sent with every model request)");
    match measure_mcp_tool_definitions() {
        Some((count, bytes)) => println!(
            "  sqz-mcp tools   {count} definitions, ~{} tokens",
            approx_tokens(bytes)
        ),
        None => println!("  sqz-mcp tools   not measured (sqz-mcp not on PATH or not answering)"),
    }
    let kiro_configured = tools.iter().any(|t| t.name == "Kiro" && t.config.configured());
    if kiro_configured {
        print_guidance("Kiro guidance", &kiro_guidance(&project_dir));
    }
    for (clients, path) in guidance_readers(&tools, &project_dir) {
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        print_guidance(&clients.join(", "), &guidance_block_copies(&path, &file));
    }

    // ── Activity ────────────────────────────────────────────────────
    let activity = {
        let stats = store.compression_stats().unwrap_or_default();
        let last7 = store.daily_gains(7).unwrap_or_default();
        let (inflated_rows, inflated_tokens) = store.inflated_rows().unwrap_or((0, 0));
        Activity {
            total_compressions: stats.total_compressions,
            total_saved: stats.tokens_saved(),
            last7_compressions: last7.iter().map(|g| g.compressions).sum(),
            last7_saved: last7.iter().map(|g| g.tokens_saved).sum(),
            net_negative: stats.is_net_negative(),
            inflated_rows,
            inflated_tokens,
        }
    };
    println!();
    println!("activity");
    if activity.net_negative {
        println!(
            "  all-time: {} compressions, net NEGATIVE (log has inflated rows, see problems)",
            activity.total_compressions
        );
    } else {
        println!(
            "  all-time: {} compressions, {} tokens saved",
            activity.total_compressions, activity.total_saved
        );
    }
    println!(
        "  last 7d:  {} compressions, {} tokens saved",
        activity.last7_compressions, activity.last7_saved
    );

    // ── Diagnosis ───────────────────────────────────────────────────
    let problems = diagnose(&tools, &activity, &managed);
    println!();
    if problems.is_empty() {
        println!("no problems found");
        0
    } else {
        println!("problems");
        for p in &problems {
            println!("  !! {}", p.message);
            if let Some(fix) = &p.fix {
                println!("      fix: {fix}");
            }
        }
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &'static str, flag: &'static str, client: Detection, config: ConfigScope) -> ToolStatus {
        ToolStatus { name, only_flag: flag, client, config, tier: sqz_engine::integration_tier(name) }
    }

    fn clean() -> Managed {
        Managed {
            kiro_global: SteeringState::Current,
            kiro_project: SteeringState::Missing,
            agents_md: SteeringState::Missing,
            zed_rules: SteeringState::Missing,
            claude_md: SteeringState::Missing,
            cursor_rules: SteeringState::Missing,
            windsurf_rules: SteeringState::Missing,
            cline_rules: SteeringState::Missing,
            shell_block_stale: false,
            claude_stale: false,
            claude_duplicate: false,
            opencode_plugin_stale: false,
            missing_binaries: Vec::new(),
            legacy_hooks: Vec::new(),
            gemini_unparseable: false,
        }
    }

    fn active() -> Activity {
        Activity { total_compressions: 100, last7_compressions: 5, ..Default::default() }
    }

    #[test]
    fn detected_unconfigured_clients_get_one_grouped_fix() {
        let tools = vec![
            tool("Claude Code", "claude", Detection::Detected, ConfigScope::None),
            tool("Cursor", "cursor", Detection::Detected, ConfigScope::None),
            tool("Kiro", "kiro", Detection::Detected, ConfigScope::Global),
            tool("Zed", "zed", Detection::NotDetected, ConfigScope::None),
        ];
        let problems = diagnose(&tools, &active(), &clean());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("Claude Code, Cursor"));
        assert!(!problems[0].message.contains("Kiro"), "configured client must not be flagged");
        assert!(!problems[0].message.contains("Zed"), "undetected client must not be flagged");
        assert_eq!(problems[0].fix.as_deref(), Some("sqz init --only claude,cursor"));
    }

    #[test]
    fn transparent_hook_idle_last_week_is_flagged() {
        let tools = vec![tool("Claude Code", "claude", Detection::Detected, ConfigScope::Both)];
        let activity = Activity { total_compressions: 412, last7_compressions: 0, ..Default::default() };
        let problems = diagnose(&tools, &activity, &clean());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("none in the last 7 days"));
    }

    #[test]
    fn advisory_only_idle_week_is_not_a_hook_failure() {
        // Kiro can't fire a hook, so "a hook stopped firing" would be wrong.
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both)];
        let activity = Activity { total_compressions: 412, last7_compressions: 0, ..Default::default() };
        assert!(diagnose(&tools, &activity, &clean()).is_empty());
    }

    #[test]
    fn transparent_hook_never_ran_suggests_restart() {
        let tools = vec![tool("Claude Code", "claude", Detection::Detected, ConfigScope::Project)];
        let problems = diagnose(&tools, &Activity::default(), &clean());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("restart your AI tool"));
    }

    #[test]
    fn advisory_only_never_ran_explains_why_and_points_at_replay() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Project)];
        let problems = diagnose(&tools, &Activity::default(), &clean());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("advisory"));
        assert!(problems[0].message.contains("Kiro"));
        assert!(problems[0].fix.as_deref().unwrap().contains("discover --replay"));
    }

    #[test]
    fn missing_shell_helpers_are_not_a_problem() {
        // The shell block only defines sqz_run; it compresses nothing on
        // its own, so its absence must not be reported as lost savings.
        let tools = vec![tool("Claude Code", "claude", Detection::Detected, ConfigScope::Both)];
        assert!(diagnose(&tools, &active(), &clean()).is_empty());
    }

    #[test]
    fn stale_and_duplicate_kiro_steering_are_flagged() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both)];
        let managed = Managed {
            kiro_global: SteeringState::Stale,
            kiro_project: SteeringState::Current,
            ..clean()
        };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].message.contains("older sqz"));
        assert!(problems[1].message.contains("twice"));
        assert!(problems.iter().all(|p| p.fix.as_deref().unwrap().starts_with(FIX_CMD)));
    }

    #[test]
    fn user_authored_steering_is_never_flagged() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both)];
        let managed = Managed {
            kiro_global: SteeringState::UserAuthored,
            kiro_project: SteeringState::UserAuthored,
            ..clean()
        };
        assert!(diagnose(&tools, &active(), &managed).is_empty());
    }

    #[test]
    fn stale_agents_md_and_rules_files_are_flagged() {
        let tools = vec![tool("Codex", "codex", Detection::Detected, ConfigScope::Both)];
        let managed = Managed {
            agents_md: SteeringState::Stale,
            cursor_rules: SteeringState::UserAuthored,
            windsurf_rules: SteeringState::Stale,
            cline_rules: SteeringState::Stale,
            ..clean()
        };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].message.contains("AGENTS.md") && problems[0].message.contains("older sqz"));
        assert!(problems[1].message.contains("Windsurf, Cline"), "{}", problems[1].message);
        assert!(!problems[1].message.contains("Cursor"));
        assert!(problems.iter().all(|p| p.fix.as_deref().unwrap().starts_with(FIX_CMD)));
    }

    #[test]
    fn current_and_user_guidance_files_are_not_flagged() {
        let tools = vec![tool("Zed", "zed", Detection::Detected, ConfigScope::Both)];
        let managed = Managed {
            agents_md: SteeringState::Current,
            zed_rules: SteeringState::UserAuthored,
            cursor_rules: SteeringState::Current,
            ..clean()
        };
        assert!(diagnose(&tools, &active(), &managed).is_empty());
    }

    #[test]
    fn the_shared_block_counts_once_and_old_blocks_are_labelled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        let block = sqz_engine::agents_md_guidance_block("/bin/sqz");
        std::fs::write(&path, format!("# Mine\n\n{block}")).unwrap();
        assert_eq!(
            guidance_block_copies(&path, "AGENTS.md"),
            vec![("AGENTS.md block".to_string(), block.trim_end_matches('\n').len())]
        );

        std::fs::write(
            &path,
            "<!-- BEGIN sqz-zed-guidance (auto-installed by sqz init; remove this block to disable) -->\nold\n<!-- END sqz-zed-guidance -->\n",
        )
        .unwrap();
        let copies = guidance_block_copies(&path, "AGENTS.md");
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].0, "AGENTS.md block (old)");
    }

    #[test]
    fn codex_and_zed_are_counted_for_the_file_each_one_reads() {
        let dir = tempfile::tempdir().unwrap();
        let tools = vec![
            tool("Codex", "codex", Detection::Detected, ConfigScope::Both),
            tool("Zed", "zed", Detection::Detected, ConfigScope::Both),
            tool("Cursor", "cursor", Detection::Detected, ConfigScope::Both),
        ];
        assert_eq!(
            guidance_readers(&tools, dir.path()),
            vec![(vec!["Codex", "Zed"], dir.path().join("AGENTS.md"))]
        );

        std::fs::write(dir.path().join(".rules"), "Tabs.\n").unwrap();
        assert_eq!(
            guidance_readers(&tools, dir.path()),
            vec![
                (vec!["Codex"], dir.path().join("AGENTS.md")),
                (vec!["Zed"], dir.path().join(".rules")),
            ]
        );
        assert!(guidance_readers(&tools[2..], dir.path()).is_empty());
    }

    #[test]
    fn gemini_settings_sqz_will_not_edit_get_a_manual_fix() {
        let tools = vec![
            tool("Gemini CLI", "gemini", Detection::Detected, ConfigScope::None),
            tool("Cursor", "cursor", Detection::Detected, ConfigScope::None),
        ];
        let managed = Managed { gemini_unparseable: true, ..clean() };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(problems[0].fix.as_deref(), Some("sqz init --only cursor"));
        assert!(problems[1].message.contains("not plain JSON"), "{}", problems[1].message);
        assert!(problems[1].fix.as_deref().unwrap().starts_with("add sqz's BeforeTool hook"));
    }

    #[test]
    fn hooks_running_a_missing_sqz_are_flagged() {
        let tools = vec![tool("Copilot CLI", "copilot", Detection::Detected, ConfigScope::Global)];
        let managed = Managed { missing_binaries: vec!["/h/.copilot/hooks/sqz.json (/usr/bin/sqz)".to_string()], ..clean() };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("sqz.json (/usr/bin/sqz)"), "{}", problems[0].message);
        assert!(problems[0].fix.as_deref().unwrap().starts_with(FIX_CMD));
    }

    #[test]
    fn claude_hooks_at_both_scopes_are_flagged() {
        let tools = vec![tool("Claude Code", "claude", Detection::Detected, ConfigScope::Both)];
        let managed = Managed { claude_duplicate: true, ..clean() };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("installed twice"));
        assert!(problems[0].fix.as_deref().unwrap().starts_with(FIX_CMD));
    }

    #[test]
    fn stale_shell_block_is_flagged() {
        let tools = vec![tool("Claude Code", "claude", Detection::Detected, ConfigScope::Both)];
        let managed = Managed { shell_block_stale: true, ..clean() };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("shell rc"));
    }

    #[test]
    fn stale_claude_hook_is_flagged_with_the_rules_it_adds() {
        let tools = vec![tool("Claude Code", "claude", Detection::Detected, ConfigScope::Global)];
        let managed = Managed { claude_stale: true, ..clean() };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("older sqz"));
        let fix = problems[0].fix.as_deref().unwrap();
        assert!(fix.starts_with(FIX_CMD) && fix.contains("Bash(sqz compress --cmd *)"), "{fix}");
    }

    #[test]
    fn stale_opencode_plugin_is_flagged() {
        let tools = vec![tool("OpenCode", "opencode", Detection::Detected, ConfigScope::Global)];
        let managed = Managed { opencode_plugin_stale: true, ..clean() };
        let problems = diagnose(&tools, &active(), &managed);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("OpenCode 2 does not load it"), "{}", problems[0].message);
        assert!(problems[0].fix.as_deref().unwrap().starts_with(FIX_CMD));
    }

    #[test]
    fn healthy_setup_reports_nothing() {
        let tools = vec![
            tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both),
            tool("Zed", "zed", Detection::NotDetected, ConfigScope::None),
            tool("Cline", "cline", Detection::Unknown, ConfigScope::None),
        ];
        assert!(diagnose(&tools, &active(), &clean()).is_empty());
    }

    #[test]
    fn unknown_footprint_without_config_is_not_a_problem() {
        // Cline can't be detected machine-wide; silence is correct.
        let tools = vec![tool("Cline", "cline", Detection::Unknown, ConfigScope::None)];
        assert!(diagnose(&tools, &active(), &clean()).is_empty());
    }

    #[test]
    fn inflated_rows_are_flagged_with_fix_command() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both)];
        let activity = Activity {
            total_compressions: 700,
            last7_compressions: 100,
            net_negative: true,
            inflated_rows: 2,
            inflated_tokens: 458_000,
            ..Default::default()
        };
        let problems = diagnose(&tools, &activity, &clean());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("2 log row(s)"));
        assert!(problems[0].message.contains("below zero"));
        assert!(problems[0].fix.as_deref().unwrap().starts_with(FIX_CMD));
    }

    #[test]
    fn inflated_rows_without_negative_total_omit_the_below_zero_clause() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both)];
        let activity = Activity {
            total_compressions: 700,
            last7_compressions: 100,
            inflated_rows: 1,
            inflated_tokens: 900,
            ..Default::default()
        };
        let problems = diagnose(&tools, &activity, &clean());
        assert_eq!(problems.len(), 1);
        assert!(!problems[0].message.contains("below zero"));
    }

    #[test]
    fn config_scope_flags() {
        assert_eq!(ConfigScope::from_flags(true, true), ConfigScope::Both);
        assert_eq!(ConfigScope::from_flags(true, false), ConfigScope::Project);
        assert_eq!(ConfigScope::from_flags(false, true), ConfigScope::Global);
        assert_eq!(ConfigScope::from_flags(false, false), ConfigScope::None);
        assert!(ConfigScope::Both.configured());
        assert!(!ConfigScope::None.configured());
    }

    #[test]
    fn every_supported_tool_has_an_only_flag() {
        for name in sqz_engine::SUPPORTED_TOOL_NAMES {
            assert!(!only_flag(name).is_empty(), "missing --only flag for {name}");
            // The flag must round-trip through the real parser.
            assert!(
                sqz_engine::parse_tool_list(only_flag(name)).is_ok(),
                "flag for {name} rejected by parse_tool_list"
            );
        }
    }

    #[test]
    fn tiers_match_what_init_installs() {
        use IntegrationTier::*;
        for (name, tier) in [
            ("Claude Code", Transparent),
            ("Copilot CLI", Transparent),
            ("Gemini CLI", Transparent),
            ("OpenCode", Transparent),
            ("Kiro", Advisory),
            ("Windsurf", Advisory),
            ("Cline", Advisory),
            ("Zed", Advisory),
        ] {
            assert_eq!(sqz_engine::integration_tier(name), tier, "{name}");
        }
    }
}
