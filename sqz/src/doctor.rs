//! `sqz doctor` — deployment validator.
//!
//! `sqz init` writes configs; doctor answers the question that comes a
//! week later: "is sqz actually doing anything?" It checks three layers
//! and reports the gaps between them:
//!
//! 1. **Installation** — binary version, database path, writability.
//! 2. **Wiring** — shell hook in the rc file, per-client sqz configs
//!    (project and, where sqz manages one, user-level).
//! 3. **Activity** — compressions logged recently. Installed-but-idle is
//!    the failure mode users actually hit (hook not firing, AI tool not
//!    restarted), and a binary-exists check can't see it.
//!
//! Client detection is best-effort by footprint (config dir or binary on
//! PATH) and says so; Cline has no reliable machine-level footprint and
//! reports "unknown" unless a project config exists.
//!
//! Exit codes: 0 = healthy, 1 = actionable problems found, 2 = sqz itself
//! is broken (database unreadable).

use std::path::{Path, PathBuf};

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
}

/// Pure diagnosis over collected state. Separated from collection and
/// rendering so the rules are unit-testable without a filesystem.
pub fn diagnose(tools: &[ToolStatus], shell_hooked: bool, activity: &Activity) -> Vec<Problem> {
    let mut problems = Vec::new();

    if !shell_hooked {
        problems.push(Problem {
            message: "shell hook not installed — commands run outside AI tools are not compressed".to_string(),
            fix: Some("sqz init".to_string()),
        });
    }

    let unconfigured: Vec<&ToolStatus> = tools
        .iter()
        .filter(|t| t.client == Detection::Detected && !t.config.configured())
        .collect();
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

    let anything_configured = shell_hooked || tools.iter().any(|t| t.config.configured());
    if activity.total_compressions == 0 {
        if anything_configured {
            problems.push(Problem {
                message: "hooks are installed but nothing has been compressed yet — restart your AI tool so it picks up the hook".to_string(),
                fix: None,
            });
        }
        // Nothing configured AND nothing logged is fully covered by the
        // unconfigured-clients / shell-hook problems above.
    } else if activity.last7_compressions == 0 {
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
            let steering = h(".kiro/steering/sqz.md").map(|p| p.exists()).unwrap_or(false);
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
            ToolStatus { name, only_flag: only_flag(name), client, config }
        })
        .collect()
}

/// Run the full check and print the report. Returns the process exit code.
pub fn run() -> i32 {
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
    let activity = {
        let stats = store.compression_stats().unwrap_or_default();
        let last7 = store.daily_gains(7).unwrap_or_default();
        Activity {
            total_compressions: stats.total_compressions,
            total_saved: stats.tokens_saved(),
            last7_compressions: last7.iter().map(|g| g.compressions).sum(),
            last7_saved: last7.iter().map(|g| g.tokens_saved).sum(),
        }
    };
    println!("  ok  database open and readable");

    // ── Shell hook ──────────────────────────────────────────────────
    let hook = ShellHook::detect();
    let rc_path = hook.rc_path();
    let shell_hooked = rc_path.exists()
        && std::fs::read_to_string(&rc_path)
            .map(|s| s.contains(hook.sentinel()))
            .unwrap_or(false);
    println!();
    println!("shell");
    if shell_hooked {
        println!("  ok  hook installed in {}", rc_path.display());
    } else {
        println!("  --  no hook in {}", rc_path.display());
    }

    // ── Clients ─────────────────────────────────────────────────────
    let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
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
        println!("  {mark} {:<12} {state}", t.name);
    }

    // ── Activity ────────────────────────────────────────────────────
    println!();
    println!("activity");
    println!(
        "  all-time: {} compressions, {} tokens saved",
        activity.total_compressions, activity.total_saved
    );
    println!(
        "  last 7d:  {} compressions, {} tokens saved",
        activity.last7_compressions, activity.last7_saved
    );

    // ── Diagnosis ───────────────────────────────────────────────────
    let problems = diagnose(&tools, shell_hooked, &activity);
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
        ToolStatus { name, only_flag: flag, client, config }
    }

    #[test]
    fn detected_unconfigured_clients_get_one_grouped_fix() {
        let tools = vec![
            tool("Claude Code", "claude", Detection::Detected, ConfigScope::None),
            tool("Cursor", "cursor", Detection::Detected, ConfigScope::None),
            tool("Kiro", "kiro", Detection::Detected, ConfigScope::Global),
            tool("Zed", "zed", Detection::NotDetected, ConfigScope::None),
        ];
        let activity = Activity { total_compressions: 100, last7_compressions: 5, ..Default::default() };
        let problems = diagnose(&tools, true, &activity);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("Claude Code, Cursor"));
        assert!(!problems[0].message.contains("Kiro"), "configured client must not be flagged");
        assert!(!problems[0].message.contains("Zed"), "undetected client must not be flagged");
        assert_eq!(problems[0].fix.as_deref(), Some("sqz init --only claude,cursor"));
    }

    #[test]
    fn installed_but_idle_last_week_is_flagged() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both)];
        let activity = Activity { total_compressions: 412, last7_compressions: 0, ..Default::default() };
        let problems = diagnose(&tools, true, &activity);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("none in the last 7 days"));
    }

    #[test]
    fn configured_but_never_ran_suggests_restart() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Project)];
        let activity = Activity::default();
        let problems = diagnose(&tools, true, &activity);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("restart your AI tool"));
    }

    #[test]
    fn missing_shell_hook_is_flagged_even_when_active() {
        let tools = vec![tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both)];
        let activity = Activity { total_compressions: 10, last7_compressions: 10, ..Default::default() };
        let problems = diagnose(&tools, false, &activity);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("shell hook"));
        assert_eq!(problems[0].fix.as_deref(), Some("sqz init"));
    }

    #[test]
    fn healthy_setup_reports_nothing() {
        let tools = vec![
            tool("Kiro", "kiro", Detection::Detected, ConfigScope::Both),
            tool("Zed", "zed", Detection::NotDetected, ConfigScope::None),
            tool("Cline", "cline", Detection::Unknown, ConfigScope::None),
        ];
        let activity = Activity { total_compressions: 10, last7_compressions: 3, ..Default::default() };
        assert!(diagnose(&tools, true, &activity).is_empty());
    }

    #[test]
    fn unknown_footprint_without_config_is_not_a_problem() {
        // Cline can't be detected machine-wide; silence is correct.
        let tools = vec![tool("Cline", "cline", Detection::Unknown, ConfigScope::None)];
        let activity = Activity { total_compressions: 1, last7_compressions: 1, ..Default::default() };
        assert!(diagnose(&tools, true, &activity).is_empty());
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
}
