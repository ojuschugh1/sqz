mod cli_proxy;
mod shell_hook;
mod tests;
mod vizit;

use clap::{Parser, Subcommand};
use sqz_engine::SqzEngine;
use sqz_engine::{EntropyAnalyzer, InfoLevel};
use sqz_engine::{TeeManager, TeeMode};
use sqz_engine::{DashboardConfig, DashboardMetrics, DashboardServer, CommandBreakdown, ToolBreakdown, SessionHistoryEntry};

use cli_proxy::CliProxy;
use shell_hook::ShellHook;

// ── CLI argument model ────────────────────────────────────────────────────

const SQZ_BANNER: &str = r#"
  ███████╗ ██████╗ ███████╗
  ██╔════╝██╔═══██╗╚══███╔╝
  ███████╗██║   ██║  ███╔╝
  ╚════██║██║▄▄ ██║ ███╔╝
  ███████║╚██████╔╝███████╗
  ╚══════╝ ╚══▀▀═╝ ╚══════╝
  The Context Intelligence Layer
"#;

#[derive(Parser)]
#[command(
    name = "sqz",
    version,
    about = "sqz — universal context intelligence layer",
    before_help = SQZ_BANNER,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Install shell hooks and create default presets.
    ///
    /// By default, installs hooks at project scope (`.claude/settings.local.json`
    /// in the current directory). Pass `--global` to install at user scope
    /// (`~/.claude/settings.json`) so the hook fires in every Claude Code
    /// session on this machine — the common case on first install.
    Init {
        /// Skip confirmation prompt and install everything.
        #[arg(long, short)]
        yes: bool,

        /// Install hooks at user scope (~/.claude/settings.json) so they
        /// apply to every project on this machine, not just cwd.
        ///
        /// Without this flag, `sqz init` writes to the current project's
        /// `.claude/settings.local.json`. That file is gitignored and
        /// only active when Claude Code is running inside that project —
        /// which is a common foot-gun if you ran `sqz init` once in one
        /// repo and expected it to work everywhere.
        #[arg(long, short, alias = "g")]
        global: bool,

        /// Comma-separated list of agents to install. Only the named
        /// tools get configured; all others are skipped.
        ///
        /// Accepts: claude, cursor, windsurf, cline, gemini, opencode,
        /// codex, zed. Aliases like `claude-code`, `gemini-cli`, `roo`
        /// (= cline), `zed-editor` are also recognised.
        ///
        /// Example: `sqz init --only opencode`
        ///          `sqz init --only opencode,codex`
        ///
        /// Requested in issue #11 (@shochdoerfer): single-agent users
        /// wanted a way to opt in to just their agent rather than
        /// getting hook configs sprayed across every tool sqz knows
        /// about.
        #[arg(long, value_name = "TOOLS", conflicts_with = "skip")]
        only: Option<String>,

        /// Comma-separated list of agents to skip. Every other
        /// supported tool is installed normally.
        ///
        /// Same name vocabulary as `--only`. Cannot be combined with
        /// `--only`.
        ///
        /// Example: `sqz init --skip cursor,windsurf`
        #[arg(long, value_name = "TOOLS")]
        skip: Option<String>,

        /// Also write a repo-level CI hook at `.github/hooks/sqz.json`.
        ///
        /// GitHub Copilot coding agent (cloud) and repo-scoped Copilot
        /// CLI sessions load hooks from that path. The file bootstraps
        /// sqz via install.sh on session start and pipes bash output
        /// through it, so CI agent runs get the same token savings as
        /// local ones. Commit the file for it to take effect.
        #[arg(long)]
        ci: bool,
    },

    /// Compress text from stdin or a positional argument.
    Compress {
        /// Text to compress. If omitted, reads from stdin.
        text: Option<String>,
        /// Compression mode: safe (preserve everything), default (balanced), aggressive (max reduction).
        #[arg(long, default_value = "auto")]
        mode: String,
        /// Show verifier confidence score alongside token reduction.
        #[arg(long)]
        verify: bool,
        /// Bypass the dedup cache entirely — never emit a `§ref:HASH§`
        /// token, even for content that's been seen before.
        ///
        /// Added after SquireNed reported on-list that GLM 5.1 on
        /// Synthetic loops when it receives dedup refs it can't parse.
        /// Also honoured via `SQZ_NO_DEDUP=1` in the environment, which
        /// is what the shell hook uses so the agent can flip the switch
        /// without re-invoking sqz.
        #[arg(long)]
        no_cache: bool,
        /// Label this compression with the name of the command that
        /// produced the input (e.g. "git", "cargo", "kubectl"). Shown
        /// in `sqz stats` and `sqz gain --history` so you can see which
        /// commands are saving the most tokens.
        ///
        /// Previously passed only via the `SQZ_CMD` environment
        /// variable baked into the rewritten command. That approach
        /// was sh-specific — `SQZ_CMD=cmd foo` is syntactically
        /// invalid in PowerShell (parsed as a command name) and in
        /// cmd.exe (also parsed as a command name), which broke every
        /// Windows hook. `--cmd` is shell-neutral. Reported in issue
        /// #10. The `SQZ_CMD` env var is still honoured for backward
        /// compatibility with the POSIX shell hook scripts; `--cmd`
        /// wins if both are present.
        #[arg(long, value_name = "NAME")]
        cmd: Option<String>,
        /// Disable n-gram phrase abbreviation (the `«A1»` legend rewrite).
        ///
        /// Abbreviation is ON by default. Turn it off when the output
        /// contains identifiers an agent will copy-paste verbatim — SHAs,
        /// file paths, URLs — because the abbreviator keeps only the first
        /// occurrence of a repeated phrase and rewrites the rest to `«A1»`,
        /// silently corrupting any SHA/path that lives inside it. Also
        /// honoured via `SQZ_NO_ABBREV=1` in the environment (parallel to
        /// `--no-cache` / `SQZ_NO_DEDUP`), which is what the shell hook
        /// uses so the agent can flip the switch without re-invoking sqz.
        #[arg(long)]
        no_abbrev: bool,
    },

    /// Expand a `§ref:PREFIX§` dedup token back to the content it points at.
    ///
    /// Agents that can't parse sqz's dedup refs see `§ref:a1b2c3d4§`
    /// mid-output and don't know what to do with it. Running
    /// `sqz expand a1b2c3d4` (or any longer prefix) returns the original
    /// pre-compression bytes from the cache. If the cache entry predates
    /// the original-capture migration (v0.10.0), the compressed-but-
    /// legible form is returned with a note explaining why.
    ///
    /// Accepts either the prefix alone (`sqz expand a1b2c3d4`) or the
    /// full `§ref:…§` token pasted verbatim (`sqz expand §ref:a1b2c3d4§`).
    Expand {
        /// Hash prefix or full `§ref:PREFIX§` token to look up.
        #[arg(required = true)]
        prefix: String,
    },

    /// Export a session to CTX format.
    Export {
        /// Session ID to export.
        session_id: String,
    },

    /// Import a CTX file into the session store.
    Import {
        /// Path to the .ctx file.
        file: String,
    },

    /// Show current token budget and usage.
    Status {
        /// Output as JSON (for programmatic consumption by VS Code extension, etc.)
        #[arg(long)]
        json: bool,
    },

    /// Show cost summary for a session.
    Cost {
        /// Session ID.
        session_id: String,
    },

    /// Analyze per-block Shannon entropy of a file or stdin.
    Analyze {
        /// File path to analyze. If omitted, reads from stdin.
        file: Option<String>,

        /// High-percentile threshold for HighInfo classification (default 60).
        #[arg(long, default_value_t = 60.0)]
        high: f64,

        /// Low-percentile threshold for LowInfo classification (default 25).
        #[arg(long, default_value_t = 25.0)]
        low: f64,
    },

    /// List and retrieve saved uncompressed outputs (tee mode).
    Tee {
        #[command(subcommand)]
        action: Option<TeeAction>,
    },

    /// Launch local web dashboard with real-time metrics.
    Dashboard {
        /// Port to serve the dashboard on.
        #[arg(long, default_value_t = 3001)]
        port: u16,
    },

    /// [Coming soon] Transparent HTTP proxy that compresses requests to OpenAI/Anthropic/Google AI.
    /// Sits between your app and the API — no code changes required.
    Proxy {
        /// Port to listen on.
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },

    /// Remove sqz shell hooks and AI tool configs.
    Uninstall {
        /// Skip confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },

    /// Show a full compression stats report for a session.
    Stats {
        /// Session ID. If omitted, shows aggregate stats for the default agent.
        session_id: Option<String>,

        /// Filter stats to a specific project directory.
        /// Use "." for the current directory, or pass an absolute path.
        /// Use "--project list" to see all tracked projects.
        #[arg(long, short)]
        project: Option<String>,

        /// Show per-command token usage breakdown (top consumers).
        #[arg(long, short)]
        breakdown: bool,

        /// Output as JSON (for programmatic consumption by VS Code extension, etc.)
        #[arg(long)]
        json: bool,

        /// Show an estimated billed-cost saving under a prompt-cached
        /// serving model. Token reduction is not the same as cost
        /// reduction (arXiv:2607.12161): with provider caching, a saved
        /// tool-output token avoids one cache write (default ×1.25 the
        /// input price) plus N cache reads (default ×0.10 per re-read
        /// turn) — not one list-price token. Assumptions are printed
        /// with the estimate; adjust them with the flags below.
        #[arg(long)]
        cost: bool,

        /// Input price in USD per 1M tokens (default: 3.00).
        #[arg(long, value_name = "USD", requires = "cost")]
        price_in: Option<f64>,

        /// Cache-write premium multiplier (default: 1.25).
        #[arg(long, value_name = "MULT", requires = "cost")]
        cache_write_mult: Option<f64>,

        /// Cache-read discount multiplier (default: 0.10).
        #[arg(long, value_name = "MULT", requires = "cost")]
        cache_read_mult: Option<f64>,

        /// Expected turns that re-read saved tokens (default: 10).
        #[arg(long, value_name = "N", requires = "cost")]
        reread_turns: Option<f64>,
    },

    /// Show accumulated token savings over time.
    Gain {
        /// Number of days to show (default: 7).
        #[arg(long, default_value_t = 7)]
        days: u32,

        /// Filter gains to a specific project directory.
        /// Use "." for the current directory, or pass an absolute path.
        #[arg(long, short)]
        project: Option<String>,
    },

    /// Find missed savings opportunities by analyzing recent command history.
    Discover {
        /// Number of days to analyze (default: 7).
        #[arg(long, default_value_t = 7)]
        days: u32,
    },

    /// Resume a previous session — inject a session guide into the context.
    Resume {
        /// Session ID to resume. If omitted, uses the most recent session.
        session_id: Option<String>,
    },

    /// Process a PreToolUse hook invocation from an AI coding tool.
    /// Reads the tool call JSON from stdin, rewrites bash commands to pipe
    /// through sqz, and outputs the modified JSON.
    Hook {
        /// The AI tool sending the hook: claude, cursor, windsurf, cline.
        tool: String,
    },

    /// Proactively evict stale context to free tokens before compaction hits.
    /// Summarizes old items and outputs an eviction report.
    Compact,

    /// Clear the dedup cache, compression stats, or both.
    ///
    /// Useful when switching models, starting fresh on a project, or if
    /// stale `§ref:HASH§` tokens are confusing the agent.
    Reset {
        /// Only clear the dedup cache (keep stats and session history).
        #[arg(long)]
        cache_only: bool,

        /// Only clear compression stats (keep the cache).
        #[arg(long)]
        stats_only: bool,

        /// Scope the reset to a specific project directory.
        /// Use "." for the current directory, or pass an absolute path.
        /// Without this, resets globally.
        #[arg(long, short)]
        project: Option<String>,

        /// Skip confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },

    /// Print the OpenCode plugin TypeScript to stdout for manual install.
    ///
    /// Lets users install sqz into OpenCode by hand — useful when an
    /// existing `opencode.jsonc` has comments they want to preserve
    /// (`sqz init` strips them on write because serde_json can't
    /// round-trip JSONC). Reported on issue #6 by @Icaruk.
    ///
    /// Usage:
    ///   sqz print-opencode-plugin > ~/.config/opencode/plugins/sqz.ts
    #[command(name = "print-opencode-plugin")]
    PrintOpencodePlugin,

    /// Launch a live terminal dashboard for AI agent sessions.
    ///
    /// Displays a real-time, auto-refreshing table of all active agent
    /// sessions tracked by sqz — like htop but for AI agents.
    /// Press q or Ctrl-C to exit.
    Vizit {
        /// Refresh interval in seconds (default: 2).
        #[arg(long, default_value_t = 2)]
        refresh: u64,

        /// Path to the sessions database.
        /// Defaults to ~/.sqz/sessions.db.
        #[arg(long)]
        db: Option<std::path::PathBuf>,

        /// Disable ANSI color output regardless of terminal capabilities.
        #[arg(long)]
        no_color: bool,
    },

    /// Manage pinned cache entries.
    ///
    /// Pinned entries bypass the 30-minute TTL freshness check, so a
    /// `§ref:HASH§` token to pinned content stays dedup-able forever.
    /// Useful for stable knowledge-base content (a wiki, project
    /// documentation, system prompts) that is persistently re-injected
    /// into every agent and therefore never compacted out of the LLM's
    /// context.
    Pin {
        #[command(subcommand)]
        action: PinAction,
    },
}

#[derive(Subcommand)]
enum TeeAction {
    /// List all saved tee entries.
    List,
    /// Retrieve a saved output by its id.
    Get {
        /// The tee entry id.
        id: String,
    },
}

#[derive(Subcommand)]
enum PinAction {
    /// Compress and pin a file or string. Pinned entries bypass the TTL
    /// freshness check so dedup refs stay valid across long sessions.
    Add {
        /// File path, or `-` to read from stdin. A bare string is also
        /// accepted and treated as literal content.
        input: String,
    },
    /// Pin an already-cached hash by its full hash or short prefix.
    /// Useful when you want to pin content that sqz has previously
    /// compressed without re-compressing it.
    Mark {
        /// Cache entry hash (full or short prefix).
        hash: String,
    },
    /// Remove the pin from a hash, returning it to normal TTL behaviour.
    Remove {
        /// Cache entry hash (full or short prefix).
        hash: String,
    },
    /// List every pinned cache entry.
    List,
    /// Unpin every entry.
    Clear {
        /// Skip confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    match cli.command {
        None => {
            // When invoked with no subcommand (e.g. piped from shell hook),
            // run the proxy event loop.
            let proxy = match CliProxy::new() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[sqz] failed to initialise engine: {e}");
                    std::process::exit(1);
                }
            };
            if let Err(e) = proxy.run_proxy() {
                eprintln!("[sqz] proxy error: {e}");
                std::process::exit(1);
            }
        }

        Some(Command::Init { yes, global, only, skip, ci }) => cmd_init(yes, global, only, skip, ci),
        Some(Command::Compress { text, mode, verify, no_cache, cmd, no_abbrev }) => {
            cmd_compress(text, &mode, verify, no_cache, cmd, no_abbrev)
        }
        Some(Command::Expand { prefix }) => cmd_expand(&prefix),
        Some(Command::Export { session_id }) => cmd_export(&session_id),
        Some(Command::Import { file }) => cmd_import(&file),
        Some(Command::Status { json }) => cmd_status(json),
        Some(Command::Cost { session_id }) => cmd_cost(&session_id),
        Some(Command::Analyze { file, high, low }) => cmd_analyze(file, high, low),
        Some(Command::Tee { action }) => cmd_tee(action),
        Some(Command::Dashboard { port }) => cmd_dashboard(port),
        Some(Command::Proxy { port }) => cmd_proxy(port),
        Some(Command::Uninstall { yes }) => cmd_uninstall(yes),
        Some(Command::Stats { session_id, project, breakdown, json, cost, price_in, cache_write_mult, cache_read_mult, reread_turns }) => {
            let cost_opts = if cost {
                let mut a = sqz_engine::SavingsAssumptions::default();
                if let Some(v) = price_in { a.price_in_per_mtok = v; }
                if let Some(v) = cache_write_mult { a.cache_write_mult = v; }
                if let Some(v) = cache_read_mult { a.cache_read_mult = v; }
                if let Some(v) = reread_turns { a.reread_turns = v; }
                Some(a)
            } else {
                None
            };
            cmd_stats(session_id, project, breakdown, json, cost_opts)
        }
        Some(Command::Gain { days, project }) => cmd_gain(days, project),
        Some(Command::Discover { days }) => cmd_discover(days),
        Some(Command::Resume { session_id }) => cmd_resume(session_id),
        Some(Command::Hook { tool }) => cmd_hook(&tool),
        Some(Command::Compact) => cmd_compact(),
        Some(Command::Reset { cache_only, stats_only, project, yes }) => {
            cmd_reset(cache_only, stats_only, project, yes)
        }
        Some(Command::PrintOpencodePlugin) => cmd_print_opencode_plugin(),
        Some(Command::Vizit { refresh, db, no_color }) => cmd_vizit(refresh, db, no_color),
        Some(Command::Pin { action }) => cmd_pin(action),
    }
}

// ── Command implementations ───────────────────────────────────────────────

/// `sqz init` — detect shell, install hook, create default preset.
fn cmd_init(skip_confirm: bool, global: bool, only: Option<String>, skip: Option<String>, ci: bool) {
    use std::io::Write;

    // Parse the agent filter first so typos fail fast, before we
    // touch any files. Clap's `conflicts_with` already guarantees
    // --only and --skip aren't both set, so the match below is
    // exhaustive.
    let filter = match (only.as_deref(), skip.as_deref()) {
        (None, None) => sqz_engine::ToolFilter::All,
        (Some(raw), None) => match sqz_engine::parse_tool_list(raw) {
            Ok(names) if names.is_empty() => {
                // --only "" or --only "   " — pointless but harmless;
                // treat as "install nothing AI-tool-related, just the
                // shell hook and preset."
                sqz_engine::ToolFilter::Only(names)
            }
            Ok(names) => sqz_engine::ToolFilter::Only(names),
            Err(e) => {
                eprintln!("[sqz] init: {e}");
                std::process::exit(1);
            }
        },
        (None, Some(raw)) => match sqz_engine::parse_tool_list(raw) {
            Ok(names) => sqz_engine::ToolFilter::Skip(names),
            Err(e) => {
                eprintln!("[sqz] init: {e}");
                std::process::exit(1);
            }
        },
        (Some(_), Some(_)) => {
            // Unreachable: clap's conflicts_with catches this before
            // we get here. Defensive exit in case clap config drifts.
            eprintln!("[sqz] init: --only and --skip are mutually exclusive");
            std::process::exit(1);
        }
    };

    let hook = ShellHook::detect();
    let rc_path = hook.rc_path();
    let preset_dir = default_preset_dir();
    let preset_path = preset_dir.join("default.toml");
    let sqz_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "sqz".to_string())
        // Normalize backslashes to forward slashes on Windows. Claude Code
        // runs hook commands through Git Bash where `\` is an escape char,
        // so `C:\Users\...\sqz.exe` → `C:Users...sqz.exe` → "not found".
        // Forward slashes work in Git Bash, cmd.exe, and PowerShell.
        .replace('\\', "/");
    let project_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let scope = if global {
        sqz_engine::InstallScope::Global
    } else {
        sqz_engine::InstallScope::Project
    };

    // ── Phase 1: Build the plan ──────────────────────────────────────

    let mut plan: Vec<(String, String, bool)> = Vec::new(); // (path, action, is_new)

    // Shell hook
    let rc_exists = rc_path.exists();
    let rc_has_hook = rc_exists && std::fs::read_to_string(&rc_path)
        .map(|s| s.contains(hook.sentinel()))
        .unwrap_or(false);
    if !rc_has_hook {
        plan.push((
            rc_path.display().to_string(),
            if rc_exists { "append shell hook".to_string() } else { "create with shell hook".to_string() },
            !rc_exists,
        ));
    }

    // Default preset
    if !preset_path.exists() {
        plan.push((
            preset_path.display().to_string(),
            "create default preset".to_string(),
            true,
        ));
    }

    // AI tool hooks
    let tool_configs = sqz_engine::generate_hook_configs(&sqz_path);
    // Cache whether the existing OpenCode .jsonc would lose comments
    // during the merge — surface it in the plan below and again after
    // install as a nudge.
    let opencode_existing = sqz_engine::find_opencode_config(&project_dir);
    let opencode_jsonc_has_comments =
        sqz_engine::opencode_config_has_comments(&project_dir);
    for config in &tool_configs {
        // Respect the --only/--skip filter from the CLI. Skipped tools
        // generate no plan line AND no file write — the user shouldn't
        // see "[sqz] will create .cursor/rules/sqz.mdc" when they asked
        // for opencode only.
        if !filter.includes(&config.tool_name) {
            continue;
        }

        // OpenCode is special: the installer merges into whichever of
        // opencode.json / opencode.jsonc already exists, rather than
        // blindly creating a parallel opencode.json. Use a dry-run to
        // only list it in the plan when there's actually something to
        // change — otherwise a second `sqz init` announces a merge that
        // won't happen, which looks broken (@Icaruk on issue #6).
        if config.tool_name == "OpenCode" {
            match sqz_engine::plan_opencode_config_change(&project_dir) {
                Ok(planned) if planned.will_change => {
                    let is_existing = opencode_existing.is_some();
                    let note = if is_existing && opencode_jsonc_has_comments {
                        format!(
                            "{} hook config (merge sqz entries — comments in .jsonc \
                             will be dropped)",
                            config.tool_name
                        )
                    } else if is_existing {
                        format!("{} hook config (merge sqz entries)", config.tool_name)
                    } else {
                        format!("{} hook config", config.tool_name)
                    };
                    plan.push((
                        planned.target_path.display().to_string(),
                        note,
                        !is_existing,
                    ));
                }
                Ok(_) => {
                    // Already fully configured — nothing to announce.
                }
                Err(_) => {
                    // Fall back to the old behaviour: announce a merge.
                    // The actual install step will surface the error.
                    if let Some(path) = &opencode_existing {
                        plan.push((
                            path.display().to_string(),
                            format!("{} hook config (merge sqz entries)", config.tool_name),
                            false,
                        ));
                    } else {
                        let full_path = project_dir.join(&config.config_path);
                        plan.push((
                            full_path.display().to_string(),
                            format!("{} hook config", config.tool_name),
                            true,
                        ));
                    }
                }
            }
            continue;
        }

        // Codex has two install surfaces and neither is a simple
        // "write file if missing": AGENTS.md is append-in-place, and
        // ~/.codex/config.toml is a user-level TOML we merge with
        // toml_edit. Show both in the plan so the user knows what
        // sqz init will touch.
        if config.tool_name == "Codex" {
            let agents_path = sqz_engine::agents_md_path(&project_dir);
            let agents_exists = agents_path.exists();
            plan.push((
                agents_path.display().to_string(),
                if agents_exists {
                    "Codex guidance (append to existing AGENTS.md)".to_string()
                } else {
                    "Codex guidance (create AGENTS.md)".to_string()
                },
                !agents_exists,
            ));

            let codex_toml = sqz_engine::codex_config_path();
            let codex_toml_exists = codex_toml.exists();
            plan.push((
                codex_toml.display().to_string(),
                if codex_toml_exists {
                    "Codex MCP registration (merge [mcp_servers.sqz] into existing config)".to_string()
                } else {
                    "Codex MCP registration (create user-level config.toml)".to_string()
                },
                !codex_toml_exists,
            ));
            continue;
        }

        // Windsurf: announce the new rules path and legacy migration.
        if config.tool_name == "Windsurf" {
            let target = project_dir.join(&config.config_path);
            let legacy = project_dir.join(".windsurfrules");
            let legacy_is_sqz = legacy.exists()
                && std::fs::read_to_string(&legacy)
                    .map(|s| s.starts_with("# sqz — Token-Optimized CLI Output"))
                    .unwrap_or(false);
            if !target.exists() {
                plan.push((
                    target.display().to_string(),
                    "Windsurf rules file".to_string(),
                    true,
                ));
            }
            if legacy_is_sqz {
                plan.push((
                    legacy.display().to_string(),
                    "remove legacy .windsurfrules (moved to .windsurf/rules/)".to_string(),
                    false,
                ));
            }
            continue;
        }

        // Cline: .clinerules may be a directory.
        if config.tool_name == "Cline" {
            let base = project_dir.join(".clinerules");
            let target = if base.is_dir() { base.join("sqz.md") } else { base };
            if !target.exists() {
                plan.push((
                    target.display().to_string(),
                    "Cline rules file".to_string(),
                    true,
                ));
            }
            continue;
        }

        // Copilot CLI: user-level hook file, only when Copilot is installed.
        if config.tool_name == "Copilot CLI" {
            let path = sqz_engine::copilot_hooks_path();
            let copilot_present = path
                .parent()
                .and_then(|p| p.parent())
                .map(|home| home.exists())
                .unwrap_or(false);
            if copilot_present && !path.exists() {
                plan.push((
                    path.display().to_string(),
                    "Copilot CLI hook".to_string(),
                    true,
                ));
            }
            continue;
        }

        // Kiro: steering file + project MCP config + legacy hook cleanup.
        if config.tool_name == "Kiro" {
            let steering = sqz_engine::kiro_steering_path(&project_dir);
            let steering_current = steering.exists()
                && std::fs::read_to_string(&steering)
                    .map(|s| s == sqz_engine::kiro_integration::kiro_steering_content(&sqz_path))
                    .unwrap_or(false);
            if !steering_current {
                let user_owned = steering.exists()
                    && std::fs::read_to_string(&steering)
                        .map(|s| !s.contains("sqz-kiro-steering"))
                        .unwrap_or(false);
                if !user_owned {
                    plan.push((
                        steering.display().to_string(),
                        "Kiro steering file".to_string(),
                        !steering.exists(),
                    ));
                }
            }
            let mcp = sqz_engine::kiro_mcp_path(&project_dir);
            let mcp_has_sqz = mcp.exists()
                && std::fs::read_to_string(&mcp)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .and_then(|v| v.get("mcpServers")?.get("sqz").map(|_| ()))
                    .is_some();
            if !mcp_has_sqz {
                plan.push((
                    mcp.display().to_string(),
                    if mcp.exists() {
                        "Kiro MCP registration (merge mcpServers.sqz)".to_string()
                    } else {
                        "Kiro MCP registration".to_string()
                    },
                    !mcp.exists(),
                ));
            }
            let legacy = sqz_engine::kiro_legacy_hook_path(&project_dir);
            if legacy.exists() {
                plan.push((
                    legacy.display().to_string(),
                    "remove legacy Kiro hook (never worked; Kiro can't rewrite tool input)"
                        .to_string(),
                    false,
                ));
            }
            continue;
        }

        // Zed (issue #38): guidance file + user-level settings.json
        // MCP registration. Announce what will actually happen — in
        // particular, a JSONC settings file will NOT be edited; the
        // install step prints a copy-paste snippet instead.
        if config.tool_name == "Zed" {
            let guidance_target = sqz_engine::zed_guidance_target(&project_dir);
            let guidance_current = guidance_target.exists()
                && std::fs::read_to_string(&guidance_target)
                    .map(|s| s.contains("BEGIN sqz-zed-guidance"))
                    .unwrap_or(false);
            if !guidance_current {
                plan.push((
                    guidance_target.display().to_string(),
                    if guidance_target.exists() {
                        "Zed Agent guidance (append sqz block)".to_string()
                    } else {
                        "Zed Agent guidance".to_string()
                    },
                    !guidance_target.exists(),
                ));
            }

            let zed_settings = sqz_engine::zed_settings_path();
            if !zed_settings.exists() {
                plan.push((
                    zed_settings.display().to_string(),
                    "Zed MCP registration (create settings.json with context_servers.sqz)"
                        .to_string(),
                    true,
                ));
            } else {
                match std::fs::read_to_string(&zed_settings) {
                    Ok(text) if serde_json::from_str::<serde_json::Value>(&text).is_ok() => {
                        let has_sqz = serde_json::from_str::<serde_json::Value>(&text)
                            .ok()
                            .and_then(|v| {
                                v.get("context_servers")?.get("sqz").map(|_| ())
                            })
                            .is_some();
                        if !has_sqz {
                            plan.push((
                                zed_settings.display().to_string(),
                                "Zed MCP registration (merge context_servers.sqz)".to_string(),
                                false,
                            ));
                        }
                    }
                    _ => {
                        // JSONC or unreadable — we won't touch it; the
                        // install step prints manual instructions. No
                        // plan line since no file will change.
                    }
                }
            }
            continue;
        }

        // Claude Code at global scope: we merge into ~/.claude/settings.json
        // instead of creating a project-level .claude/settings.local.json.
        // Show the real target in the plan so the user can see we're
        // touching their user-level config — and abort if they don't
        // want that.
        if config.tool_name == "Claude Code" && scope == sqz_engine::InstallScope::Global {
            if let Some(target) = sqz_engine::claude_user_settings_path() {
                let target_exists = target.exists();
                plan.push((
                    target.display().to_string(),
                    if target_exists {
                        "Claude Code hooks (merge sqz entries into user settings)".to_string()
                    } else {
                        "Claude Code hooks (create user settings.json)".to_string()
                    },
                    !target_exists,
                ));
            }
            continue;
        }

        let full_path = project_dir.join(&config.config_path);

        // Claude Code at project scope merges into an existing
        // settings.local.json rather than skipping it, so the plan must
        // probe whether the merge would change anything — otherwise a
        // stale install (e.g. the pre-PowerShell "Bash"-only matcher
        // from before the issue #26 fix) reports "everything is already
        // set up" and never heals.
        if config.tool_name == "Claude Code" && full_path.exists() {
            match sqz_engine::claude_project_settings_needs_update(&full_path, &sqz_path) {
                Ok(true) => {
                    plan.push((
                        full_path.display().to_string(),
                        format!(
                            "{} hook config (update stale sqz entries)",
                            config.tool_name
                        ),
                        false,
                    ));
                }
                Ok(false) => { /* current — nothing to announce */ }
                Err(_) => { /* unparseable — installer will leave it alone */ }
            }
            continue;
        }

        if !full_path.exists() {
            plan.push((
                full_path.display().to_string(),
                format!("{} hook config", config.tool_name),
                true,
            ));
        }
    }

    // Repo-level CI hook (opt-in via --ci)
    if ci {
        let ci_path = sqz_engine::ci_hooks_path(&project_dir);
        match std::fs::read_to_string(&ci_path) {
            Err(_) => plan.push((
                ci_path.display().to_string(),
                "CI hook for Copilot coding agent (commit this file)".to_string(),
                true,
            )),
            Ok(s) if s == sqz_engine::ci_hook_content() => {}
            Ok(s) if s.contains("hook copilot") => plan.push((
                ci_path.display().to_string(),
                "CI hook (refresh stale sqz content)".to_string(),
                false,
            )),
            Ok(_) => eprintln!(
                "[sqz] note: {} exists but wasn't written by sqz; leaving it alone.",
                ci_path.display()
            ),
        }
    }

    // ── Phase 2: Show the plan ───────────────────────────────────────

    if plan.is_empty() {
        println!("[sqz] everything is already set up. Nothing to do.");
        return;
    }

    println!("[sqz] detected shell: {:?}", hook);
    println!();
    println!("The following files will be modified:");
    println!();
    for (path, action, is_new) in &plan {
        let tag = if *is_new { "create" } else { "modify" };
        println!("  [{tag}] {path}");
        println!("         {action}");
    }
    println!();

    // ── Phase 3: Ask for confirmation ────────────────────────────────

    if !skip_confirm {
        print!("Do you want to continue? [Y/n] ");
        let _ = std::io::stdout().flush();

        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            eprintln!("[sqz] could not read input, aborting.");
            std::process::exit(1);
        }
        let answer = answer.trim().to_lowercase();
        if !answer.is_empty() && answer != "y" && answer != "yes" {
            println!("[sqz] aborted.");
            return;
        }
    }

    // ── Phase 4: Execute the plan ────────────────────────────────────

    // Shell hook
    if !rc_has_hook {
        match hook.install() {
            Ok(true) => println!("[sqz] ✓ hook installed to {}", rc_path.display()),
            Ok(false) => println!("[sqz] ✓ hook already present in {}", rc_path.display()),
            Err(e) => {
                eprintln!("[sqz] ✗ warning: {e}");
                eprintln!("  shell hook installation failed; output will pass uncompressed.");
            }
        }
    } else {
        println!("[sqz] ✓ shell hook already present in {}", rc_path.display());
    }

    // Shell completions (silent, non-critical)
    install_completions(&hook);

    // Default preset
    if let Err(e) = std::fs::create_dir_all(&preset_dir) {
        eprintln!("[sqz] ✗ warning: could not create preset dir {}: {e}", preset_dir.display());
    } else if !preset_path.exists() {
        match std::fs::write(&preset_path, DEFAULT_PRESET_TOML) {
            Ok(()) => println!("[sqz] ✓ default preset written to {}", preset_path.display()),
            Err(e) => eprintln!("[sqz] ✗ warning: could not write preset: {e}"),
        }
    } else {
        println!("[sqz] ✓ default preset already exists at {}", preset_path.display());
    }

    // AI tool hooks — merge/install runs after the user confirms.
    // The plan above already flagged any `.jsonc` comments-will-be-lost
    // concern so the user could Ctrl-C before we got here.
    let installed_tools =
        sqz_engine::install_tool_hooks_scoped_filtered(&project_dir, &sqz_path, scope, &filter);
    for tool in &installed_tools {
        println!("[sqz] ✓ {} hook installed", tool);
    }

    // Repo-level CI hook (opt-in via --ci)
    if ci {
        match sqz_engine::install_ci_hook(&project_dir) {
            Ok(true) => {
                let p = sqz_engine::ci_hooks_path(&project_dir);
                println!("[sqz] ✓ CI hook written to {}", p.display());
                println!("      Commit it so Copilot coding agent runs pick up sqz.");
            }
            Ok(false) => {}
            Err(e) => eprintln!("[sqz] ✗ warning: CI hook install failed: {e}"),
        }
    }

    // Zed's settings.json is JSONC more often than not (Zed's template
    // ships comments), and sqz refuses to rewrite JSONC (issue #6
    // lesson). When that happened above, hand the user the snippet.
    if filter.includes("Zed") {
        let zed_settings = sqz_engine::zed_settings_path();
        if let Ok(text) = std::fs::read_to_string(&zed_settings) {
            let is_strict_json = serde_json::from_str::<serde_json::Value>(&text).is_ok();
            if !is_strict_json && !text.contains("\"sqz\"") {
                println!();
                println!(
                    "[sqz] note: {} contains comments (JSONC), so sqz did not edit it.",
                    zed_settings.display()
                );
                println!("      To finish Zed setup, add this to the file yourself:");
                println!();
                println!("{}", sqz_engine::zed_mcp_snippet());
                println!();
                println!("      (In Zed: run the 'zed: open settings file' action, paste,");
                println!("       then 'zed: reload context servers'.)");
            }
        }
    }

    println!();
    println!("[sqz] init complete. Restart your shell or source the RC file.");
    if !global && installed_tools.iter().any(|t| t == "Claude Code") {
        // Tell the user the hook they just installed only fires inside
        // *this* project. This is the bit 76vangel missed: they expected
        // sqz to work across "multiple projects" after one install.
        println!();
        println!("[sqz] note: Claude Code hook installed at project scope.");
        println!("      To enable sqz in every project on this machine, re-run:");
        println!("         sqz init --global");
        println!("      (writes ~/.claude/settings.json, merges with existing user settings)");
    }
}

/// `sqz compress [text] [--mode safe|default|aggressive|auto] [--verify] [--no-cache] [--cmd NAME]`
///
/// The `no_cache` flag (also via `SQZ_NO_DEDUP=1`) turns off the dedup
/// cache so no `§ref:…§` token is ever returned. Added after SquireNed
/// reported on the Synthetic discord that GLM 5.1 loops when served
/// refs it can't parse.
///
/// The `cmd` parameter labels the compression in stats. Prefer passing
/// it as `--cmd NAME` (shell-neutral — works in POSIX shells, PowerShell,
/// and cmd.exe). Legacy `SQZ_CMD=NAME` env var is still honoured for
/// backward compatibility with the POSIX shell hook scripts. `--cmd`
/// wins if both are set.
fn cmd_compress(text: Option<String>, mode: &str, show_verify: bool, no_cache: bool, cmd: Option<String>, no_abbrev: bool) {
    let is_stdin = text.is_none();
    let input = match text {
        Some(t) => t,
        None => {
            use std::io::Read;
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("[sqz] stdin read error: {e}");
                std::process::exit(1);
            }
            buf
        }
    };

    // Hook rewrites append the command's exit status as a trailer;
    // strip it and re-raise it as our own exit code on every path out.
    let (input, marker_exit) = if is_stdin {
        sqz_engine::strip_exit_marker(&input)
    } else {
        (input, None)
    };
    let finish = |code: Option<i32>| {
        if let Some(c) = code {
            use std::io::Write;
            let _ = std::io::stdout().flush();
            std::process::exit(c);
        }
    };

    // Merge CLI flags with env vars — for each switch, either source turns
    // it on. This lets shell-hook callers set SQZ_NO_DEDUP=1 / SQZ_NO_ABBREV=1
    // in their shell config without editing any commands, while an explicit
    // CLI flag still works on its own. `abbreviate` defaults ON, so we only
    // flip it off when --no-abbrev OR SQZ_NO_ABBREV was given.
    let env_opts = cli_proxy::InterceptOptions::from_env();
    let opts = cli_proxy::InterceptOptions {
        no_cache: no_cache || env_opts.no_cache,
        abbreviate: !no_abbrev && env_opts.abbreviate,
    };

    // When reading from stdin in auto mode (the shell hook path), route
    // through CliProxy to get dedup cache, per-command formatters, context
    // refs, and predictive pre-caching.
    //
    // Label resolution order:
    //   1. `--cmd` CLI flag (issue #10, preferred — shell-neutral)
    //   2. `SQZ_CMD` env var (legacy POSIX shell hook)
    //   3. "stdin" (unknown source)
    if mode == "auto" && is_stdin {
        let label = cmd
            .or_else(|| std::env::var("SQZ_CMD").ok())
            .unwrap_or_else(|| "stdin".to_string());
        let proxy = match CliProxy::new() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[sqz] proxy init error: {e}");
                print!("{input}");
                finish(marker_exit);
                return;
            }
        };
        let compressed = proxy.intercept_output_with_options(&label, &input, opts);
        print!("{}", compressed);
        finish(marker_exit);
        return;
    }

    // Explicit mode override or positional text arg — use engine directly
    let engine = require_engine();

    // Apply mode override if specified
    let result = match mode {
        "safe" => {
            eprintln!("[sqz] mode: safe (preserving all content)");
            engine.compress_with_mode(&input, sqz_engine::CompressionMode::Safe)
        }
        "aggressive" => {
            eprintln!("[sqz] mode: aggressive (maximum reduction)");
            engine.compress_with_mode(&input, sqz_engine::CompressionMode::Aggressive)
        }
        "default" => {
            engine.compress_with_mode(&input, sqz_engine::CompressionMode::Default)
        }
        _ => engine.compress(&input), // auto: confidence router decides
    };    match result {
        Ok(c) => {
            print!("{}", c.data);
            let reduction = (1.0 - c.compression_ratio) * 100.0;

            // Log to session DB for cumulative stats
            let project = std::env::current_dir().ok();
            let project_str = project.as_ref().map(|p| p.to_string_lossy().to_string());
            let _ = engine.session_store().log_compression_with_project(
                c.tokens_original,
                c.tokens_compressed,
                &c.stages_applied,
                mode,
                project_str.as_deref(),
            );

            if show_verify {
                let confidence = c.verify.as_ref().map(|v| v.confidence).unwrap_or(1.0);
                let passed = c.verify.as_ref().map(|v| v.passed).unwrap_or(true);
                let status = if passed { "✓" } else { "⚠" };
                eprintln!(
                    "[sqz] {}/{} tokens ({:.0}% reduction) | confidence {:.0}% {}",
                    c.tokens_compressed,
                    c.tokens_original,
                    reduction,
                    confidence * 100.0,
                    status,
                );
            } else {
                eprintln!(
                    "[sqz] {}/{} tokens ({:.0}% reduction)",
                    c.tokens_compressed,
                    c.tokens_original,
                    reduction,
                );
            }
        }
        Err(e) => {
            eprintln!("[sqz] fallback: compression error: {e}");
            print!("{input}");
        }
    }
    finish(marker_exit);
}

/// `sqz export <session-id>` — export session to CTX.
fn cmd_export(session_id: &str) {
    let engine = require_engine();
    match engine.export_ctx(session_id) {
        Ok(ctx) => println!("{ctx}"),
        Err(e) => {
            eprintln!("[sqz] export error: {e}");
            std::process::exit(1);
        }
    }
}

/// `sqz expand <ref-or-prefix>` — resolve a dedup ref back to full content.
///
/// Designed for agents that can't parse `§ref:…§` tokens and end up
/// thrashing around them (reported for GLM 5.1 on Synthetic by
/// SquireNed: "500 13-token requests" instead of one legible one). The
/// agent sees the ref mid-output, runs `sqz expand a1b2c3d4` (or pastes
/// the whole `§ref:a1b2c3d4§` token verbatim), and gets the original
/// pre-compression bytes back on stdout.
///
/// Output separation:
///   * stdout — the resolved content itself (nothing else), so pipelines
///     like `sqz expand … | grep foo` just work.
///   * stderr — human-readable metadata (full hash, note about
///     compressed-only fallback). The agent can ignore stderr and still
///     get exactly what it asked for.
///
/// Exit codes: `0` on hit, `1` on no-match, `2` on ambiguous-prefix, `3`
/// on engine init / DB error. Lets shell pipelines detect each case.
fn cmd_expand(raw: &str) {
    // Strip the `§ref:…§` wrapper so callers can paste the token as-is.
    // Also accept a leading `ref:` without the §'s, which is what some
    // terminals render §ref:…§ as when unicode is stripped.
    let trimmed = raw.trim();
    let prefix = trimmed
        .strip_prefix('§')
        .unwrap_or(trimmed)
        .strip_prefix("ref:")
        .unwrap_or(trimmed.strip_prefix('§').unwrap_or(trimmed))
        .trim_end_matches('§')
        .trim()
        .to_string();

    let engine = match sqz_engine::SqzEngine::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[sqz] expand: engine init error: {e}");
            std::process::exit(3);
        }
    };

    match engine.cache_manager().expand_prefix(&prefix) {
        Ok(Some(sqz_engine::ExpandResult::Original { hash, bytes })) => {
            use std::io::Write;
            // Write raw bytes — the original may legitimately not be
            // UTF-8 (command output often has non-text sequences), and
            // println! would garble or reject them.
            let mut out = std::io::stdout().lock();
            if let Err(e) = out.write_all(&bytes) {
                eprintln!("[sqz] expand: stdout write error: {e}");
                std::process::exit(3);
            }
            eprintln!("[sqz] expanded ref prefix '{prefix}' → {} bytes (full hash {hash})", bytes.len());
        }
        Ok(Some(sqz_engine::ExpandResult::CompressedOnly { hash, compressed })) => {
            // Pre-migration cache entry — we only have the compressed
            // form. Still useful: agent gets legible text instead of an
            // opaque ref. Note tells the user how to capture the true
            // original if they need it.
            println!("{compressed}");
            eprintln!(
                "[sqz] expanded ref prefix '{prefix}' (compressed form only; full hash {hash})"
            );
            eprintln!(
                "[sqz] note: this cache entry predates the original-capture migration."
            );
            eprintln!(
                "[sqz] to capture the true original bytes, re-run the command that produced"
            );
            eprintln!(
                "[sqz] this ref with 'SQZ_NO_DEDUP=1' or '--no-cache'."
            );
        }
        Ok(None) => {
            eprintln!(
                "[sqz] expand: no cache entry matches prefix '{prefix}'."
            );
            eprintln!("[sqz] hint: the ref may be from a different machine or a wiped ~/.sqz/sessions.db.");
            std::process::exit(1);
        }
        Err(e) => {
            // Ambiguous prefix is the most common error here; surface
            // the raw engine message since it includes the "use a longer
            // prefix" recommendation.
            eprintln!("[sqz] expand: {e}");
            std::process::exit(2);
        }
    }
}

/// `sqz import <file>` — import CTX file.
fn cmd_import(file: &str) {
    let ctx = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[sqz] could not read file '{file}': {e}");
            std::process::exit(1);
        }
    };
    let engine = require_engine();
    match engine.import_ctx(&ctx) {
        Ok(id) => println!("[sqz] imported session: {id}"),
        Err(e) => {
            eprintln!("[sqz] import error: {e}");
            std::process::exit(1);
        }
    }
}

/// `sqz status` — show current budget/usage.
fn cmd_status(json: bool) {
    let engine = require_engine();
    let report = engine.usage_report("default");

    // Also pull cumulative stats from the session store so the JSON
    // output includes historical savings — this is what the VS Code
    // extension status bar needs.
    let cs = engine.session_store().compression_stats().unwrap_or_default();

    if json {
        let obj = serde_json::json!({
            "consumed": report.consumed,
            "windowSize": report.allocated,
            "percentUsed": report.consumed_pct * 100.0,
            "available": report.available,
            "totalCompressions": cs.total_compressions,
            "tokensIn": cs.total_tokens_in,
            "tokensOut": cs.total_tokens_out,
            "tokensSaved": cs.tokens_saved(),
            "avgReduction": cs.reduction_pct(),
        });
        println!("{}", serde_json::to_string(&obj).unwrap());
        return;
    }

    println!("agent:     {}", report.agent_id);
    println!("consumed:  {} tokens ({:.1}%)", report.consumed, report.consumed_pct * 100.0);
    println!("pinned:    {} tokens", report.pinned);
    println!("available: {} tokens", report.available);
    println!("allocated: {} tokens", report.allocated);
}

/// `sqz cost <session-id>` — show cost summary.
fn cmd_cost(session_id: &str) {
    let engine = require_engine();
    match engine.cost_summary(session_id) {
        Ok(s) => {
            println!("session:              {session_id}");
            println!("total tokens:         {}", s.total_tokens);
            println!("total cost:           ${:.6}", s.total_usd);
            println!("cache savings:        ${:.6}", s.cache_savings_usd);
            println!("compression savings:  ${:.6}", s.compression_savings_usd);
        }
        Err(e) => {
            eprintln!("[sqz] cost error: {e}");
            std::process::exit(1);
        }
    }
}

/// `sqz analyze [file]` — show per-block entropy scores.
fn cmd_analyze(file: Option<String>, high_pct: f64, low_pct: f64) {
    let source = match file {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[sqz] could not read file '{path}': {e}");
                std::process::exit(1);
            }
        },
        None => {
            use std::io::Read;
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("[sqz] stdin read error: {e}");
                std::process::exit(1);
            }
            buf
        }
    };

    let analyzer = EntropyAnalyzer::with_thresholds(high_pct, low_pct);
    let blocks = analyzer.analyze(&source);

    if blocks.is_empty() {
        println!("[sqz] no blocks found in input");
        return;
    }

    for (i, block) in blocks.iter().enumerate() {
        let level_tag = match block.info_level {
            InfoLevel::HighInfo => "HighInfo",
            InfoLevel::MediumInfo => "MediumInfo",
            InfoLevel::LowInfo => "LowInfo",
        };
        println!(
            "block {}: lines {}-{} | entropy {:.4} | {}",
            i + 1,
            block.line_range.start + 1,
            block.line_range.end,
            block.entropy,
            level_tag,
        );
    }

    let high_count = blocks.iter().filter(|b| b.info_level == InfoLevel::HighInfo).count();
    let med_count = blocks.iter().filter(|b| b.info_level == InfoLevel::MediumInfo).count();
    let low_count = blocks.iter().filter(|b| b.info_level == InfoLevel::LowInfo).count();
    println!(
        "\n[sqz] {} blocks total: {} HighInfo, {} MediumInfo, {} LowInfo",
        blocks.len(),
        high_count,
        med_count,
        low_count,
    );
}

/// `sqz tee [list|get <id>]` — list or retrieve saved uncompressed outputs.
fn cmd_tee(action: Option<TeeAction>) {
    // TeeManager uses default dir (~/.sqz/tee/); mode doesn't matter for list/get.
    let mgr = TeeManager::with_default_dir(TeeMode::Never);

    match action {
        None | Some(TeeAction::List) => {
            match mgr.list() {
                Ok(entries) if entries.is_empty() => {
                    println!("[sqz] no saved tee entries");
                }
                Ok(entries) => {
                    for e in &entries {
                        println!(
                            "{} | {} | exit {} | {} bytes",
                            e.id, e.command, e.exit_code, e.size_bytes
                        );
                    }
                    println!("\n[sqz] {} entries", entries.len());
                }
                Err(e) => {
                    eprintln!("[sqz] tee list error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some(TeeAction::Get { id }) => {
            match mgr.get(&id) {
                Ok(content) => print!("{content}"),
                Err(e) => {
                    eprintln!("[sqz] tee get error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

/// `sqz dashboard [--port N]` — launch local web dashboard.
fn cmd_dashboard(port: u16) {
    use sqz_engine::SessionStore;

    let config = DashboardConfig { port };
    let metrics = std::sync::Arc::new(std::sync::Mutex::new(DashboardMetrics::default()));
    let server = DashboardServer::new(config, metrics.clone());
    let metrics_handle = server.metrics_handle();

    // Background thread: poll session store every 5s, update dashboard metrics.
    std::thread::spawn(move || {
        let store_path = dirs_next::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".sqz")
            .join("sessions.db");
        let store = SessionStore::open_or_create(&store_path).ok();

        loop {
            if let Some(ref store) = store {
                // Top-level metrics.
                if let Ok(stats) = store.compression_stats() {
                    if let Ok(mut m) = metrics_handle.lock() {
                        m.tokens_total = stats.total_tokens_in;
                        m.tokens_saved = stats.tokens_saved();
                        if stats.total_tokens_in > 0 {
                            m.compression_ratio =
                                1.0 - stats.total_tokens_out as f64 / stats.total_tokens_in as f64;
                        }
                    }
                }
                // Cache hit/miss counters.
                if let Ok((hits, misses)) = store.cache_hit_miss_counts() {
                    if let Ok(mut m) = metrics_handle.lock() {
                        m.cache_hits = hits;
                        m.cache_misses = misses;
                    }
                }
                // Per-tool breakdown (from stages_applied).
                if let Ok(tools) = store.per_tool_breakdown() {
                    if let Ok(mut m) = metrics_handle.lock() {
                        m.per_tool = tools
                            .iter()
                            .map(|(name, tin, tout, cnt)| ToolBreakdown {
                                tool_name: name.clone(),
                                tokens_input: *tin,
                                tokens_output: *tout,
                                cost_usd: 0.0,
                                call_count: *cnt,
                            })
                            .collect();
                    }
                }
                // Per-command breakdown (from mode column).
                if let Ok(commands) = store.per_command_breakdown() {
                    if let Ok(mut m) = metrics_handle.lock() {
                        m.per_command = commands
                            .iter()
                            .map(|(name, tin, tout, cnt)| CommandBreakdown {
                                command: name.clone(),
                                tokens_original: *tin,
                                tokens_compressed: *tout,
                                invocations: *cnt,
                            })
                            .collect();
                    }
                }
                // Session history.
                if let Ok(sessions) = store.list_sessions(50) {
                    if let Ok(mut m) = metrics_handle.lock() {
                        m.sessions = sessions.iter().map(|s| SessionHistoryEntry::from(s)).collect();
                        // Auto-create synthetic dashboard session when metrics > 0.
                        if m.tokens_saved > 0
                            && !m.sessions.iter().any(|s| s.id == "dashboard")
                        {
                            let _ = store.save_dashboard_session(
                                "dashboard",
                                "sqz dashboard session (auto-created)",
                            );
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });

    println!("[sqz] starting dashboard on http://127.0.0.1:{port}");
    if let Err(e) = server.run() {
        eprintln!("[sqz] dashboard error: {e}");
        std::process::exit(1);
    }
}

/// `sqz proxy [--port N]` — transparent HTTP proxy that compresses API requests.
fn cmd_proxy(port: u16) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let engine = require_engine();
    let config = sqz_engine::ProxyConfig {
        port,
        ..Default::default()
    };

    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[sqz] proxy: failed to bind to {addr}: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("[sqz] proxy listening on http://{addr}");
    eprintln!("[sqz] configure your API client to use http://{addr} as the base URL");
    eprintln!("[sqz] example: OPENAI_BASE_URL=http://{addr}/v1");
    eprintln!("[sqz] example: ANTHROPIC_BASE_URL=http://{addr}");
    eprintln!();

    for stream in listener.incoming() {
        match stream {
            Ok(mut client) => {
                // Read the full request
                let mut buf = vec![0u8; 1024 * 1024]; // 1MB max
                let n = match client.read(&mut buf) {
                    Ok(n) if n > 0 => n,
                    _ => continue,
                };
                buf.truncate(n);

                // Parse the request
                let (method, path, _headers, body) = match sqz_engine::parse_http_request(&buf) {
                    Ok(r) => r,
                    Err(e) => {
                        let resp = sqz_engine::build_http_response(
                            400, "Bad Request",
                            &[("content-type", "text/plain")],
                            &format!("sqz proxy: {e}"),
                        );
                        let _ = client.write_all(&resp);
                        continue;
                    }
                };

                // Health check endpoint
                if path == "/health" || path == "/" {
                    let resp = sqz_engine::build_http_response(
                        200, "OK",
                        &[("content-type", "application/json")],
                        r#"{"status":"ok","service":"sqz-proxy"}"#,
                    );
                    let _ = client.write_all(&resp);
                    continue;
                }

                // Only handle POST requests to API endpoints
                if method != "POST" {
                    let resp = sqz_engine::build_http_response(
                        405, "Method Not Allowed",
                        &[("content-type", "text/plain")],
                        "sqz proxy: only POST is supported",
                    );
                    let _ = client.write_all(&resp);
                    continue;
                }

                // Detect API format from path
                let format = match sqz_engine::ApiFormat::from_path(&path) {
                    Some(f) => f,
                    None => {
                        let resp = sqz_engine::build_http_response(
                            404, "Not Found",
                            &[("content-type", "text/plain")],
                            &format!("sqz proxy: unknown API path: {path}"),
                        );
                        let _ = client.write_all(&resp);
                        continue;
                    }
                };

                // Compress the request body
                let (compressed_body, stats) = match sqz_engine::compress_request(
                    &body, format, &config, &engine,
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("[sqz] proxy: compression error: {e}, forwarding uncompressed");
                        (body.clone(), sqz_engine::ProxyStats::default())
                    }
                };

                if stats.tokens_saved() > 0 {
                    eprintln!(
                        "[sqz] proxy: {}/{} tokens ({:.0}% reduction) | {} msgs compressed, {} summarized",
                        stats.tokens_compressed, stats.tokens_original,
                        stats.reduction_pct(),
                        stats.messages_compressed, stats.messages_summarized,
                    );
                }

                // Log to session store
                let project = std::env::current_dir().ok();
                let project_str = project.as_ref().map(|p| p.to_string_lossy().to_string());
                let _ = engine.session_store().log_compression_with_project(
                    stats.tokens_original,
                    stats.tokens_compressed,
                    &["proxy".to_string()],
                    &format!("proxy:{:?}", format),
                    project_str.as_deref(),
                );

                // Build the response with the compressed body.
                // In a full implementation, this would forward to the upstream API
                // and stream the response back. For now, return the compressed
                // request body so the caller can inspect what sqz would send.
                let response_json = serde_json::json!({
                    "sqz_proxy": true,
                    "original_tokens": stats.tokens_original,
                    "compressed_tokens": stats.tokens_compressed,
                    "reduction_pct": format!("{:.1}%", stats.reduction_pct()),
                    "messages_compressed": stats.messages_compressed,
                    "messages_summarized": stats.messages_summarized,
                    "compressed_body": serde_json::from_str::<serde_json::Value>(&compressed_body)
                        .unwrap_or(serde_json::Value::String(compressed_body)),
                });

                let resp_body = serde_json::to_string_pretty(&response_json).unwrap_or_default();
                let resp = sqz_engine::build_http_response(
                    200, "OK",
                    &[("content-type", "application/json"), ("x-sqz-tokens-saved", &stats.tokens_saved().to_string())],
                    &resp_body,
                );
                let _ = client.write_all(&resp);
            }
            Err(e) => {
                eprintln!("[sqz] proxy: connection error: {e}");
            }
        }
    }
}

/// `sqz uninstall` — remove sqz shell hooks and AI tool configs.
fn cmd_uninstall(skip_confirm: bool) {
    use std::io::Write;

    let hook = ShellHook::detect();
    println!("[sqz] detected shell: {:?}", hook);

    // Build list of files to remove
    let mut files_to_remove: Vec<(String, bool)> = Vec::new(); // (path, exists)

    // Shell RC hook
    let rc_path = hook.rc_path();
    let rc_has_hook = rc_path.exists() && std::fs::read_to_string(&rc_path)
        .map(|s| s.contains(hook.sentinel()))
        .unwrap_or(false);
    if rc_has_hook {
        files_to_remove.push((rc_path.display().to_string(), true));
    }

    // AI tool configs — use the same source of truth as init
    // to avoid install/uninstall path drift.
    let project_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let sqz_path_str = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "sqz".to_string())
        .replace('\\', "/"); //  normalize for Git Bash on Windows
    let tool_configs = sqz_engine::generate_hook_configs(&sqz_path_str);
    for config in &tool_configs {
        // OpenCode's config is merged in place by `install_tool_hooks`
        // (via `update_opencode_config_detailed`) rather than written
        // as a standalone sqz file. Wiping the whole file would
        // destroy unrelated user config that was merged into
        // `opencode.json`/`opencode.jsonc`. Discover + handle it
        // separately below.
        if config.tool_name == "OpenCode" {
            continue;
        }
        // Codex is also surgical (AGENTS.md is append-only, not a sqz
        // file; ~/.codex/config.toml may hold other MCP servers).
        // Handled in dedicated cleanup blocks further down.
        if config.tool_name == "Codex" {
            continue;
        }
        // Zed is surgical too: its placeholder config_path is AGENTS.md
        // (shared with Codex and the user's own content) and its MCP
        // entry lives inside the user's Zed settings.json. Deleting
        // either whole file would destroy unrelated content. Handled
        // in dedicated cleanup blocks further down.
        if config.tool_name == "Zed" {
            continue;
        }
        // Kiro: steering file is only removed when sqz-managed, and the
        // MCP entry is removed surgically. Handled further down.
        if config.tool_name == "Kiro" {
            continue;
        }
        // Copilot CLI: user-level file, handled in its dedicated block.
        if config.tool_name == "Copilot CLI" {
            let path = sqz_engine::copilot_hooks_path();
            if path.exists() {
                files_to_remove.push((path.display().to_string(), true));
            }
            continue;
        }
        // Windsurf: new rules file plus a legacy sqz-authored .windsurfrules.
        if config.tool_name == "Windsurf" {
            let target = project_dir.join(&config.config_path);
            if target.exists() {
                files_to_remove.push((target.display().to_string(), true));
            }
            let legacy = project_dir.join(".windsurfrules");
            let legacy_is_sqz = legacy.exists()
                && std::fs::read_to_string(&legacy)
                    .map(|s| s.starts_with("# sqz — Token-Optimized CLI Output"))
                    .unwrap_or(false);
            if legacy_is_sqz {
                files_to_remove.push((legacy.display().to_string(), true));
            }
            continue;
        }
        // Cline: only list what will actually be removed (sqz-authored).
        if config.tool_name == "Cline" {
            let base = project_dir.join(".clinerules");
            let target = if base.is_dir() { base.join("sqz.md") } else { base };
            let is_sqz = target.is_file()
                && std::fs::read_to_string(&target)
                    .map(|s| s.starts_with("# sqz — Token-Optimized CLI Output"))
                    .unwrap_or(false);
            if is_sqz {
                files_to_remove.push((target.display().to_string(), true));
            }
            continue;
        }
        let full = project_dir.join(&config.config_path);
        if full.exists() {
            files_to_remove.push((full.display().to_string(), true));
        }
    }

    // Repo-level CI hook, only when sqz-authored.
    let ci_hook_path = sqz_engine::ci_hooks_path(&project_dir);
    let ci_hook_is_sqz = ci_hook_path.exists()
        && std::fs::read_to_string(&ci_hook_path)
            .map(|s| s.contains("hook copilot"))
            .unwrap_or(false);
    if ci_hook_is_sqz {
        files_to_remove.push((ci_hook_path.display().to_string(), true));
    }

    // OpenCode project config (opencode.json OR opencode.jsonc): sqz
    // will surgically remove its own `mcp.sqz` entry and the `"sqz"`
    // entry from `plugin[]`, leaving any other config intact. If the
    // resulting file is empty or only contains the default $schema
    // line that sqz itself wrote on first install, the file is
    // removed entirely. This replaces the pre-issue-#6 behaviour of
    // wiping the whole file — which would have destroyed user config
    // merged in at `sqz init` time.
    let opencode_config = sqz_engine::find_opencode_config(&project_dir);
    let opencode_config_display = opencode_config
        .as_ref()
        .map(|p| p.display().to_string());
    if let Some(path) = &opencode_config_display {
        files_to_remove.push((format!("{path} (sqz entries only)"), true));
    }

    // Codex project-level AGENTS.md: surgically strip the sqz block
    // while preserving any other rules the user added. See
    // codex_integration::remove_agents_md_guidance.
    let agents_md = sqz_engine::agents_md_path(&project_dir);
    let agents_md_exists = agents_md.exists();
    if agents_md_exists {
        files_to_remove.push((
            format!("{} (sqz guidance block only)", agents_md.display()),
            true,
        ));
    }

    // Codex user-level ~/.codex/config.toml: remove [mcp_servers.sqz]
    // while preserving any other MCP servers the user registered.
    let codex_toml = sqz_engine::codex_config_path();
    let codex_toml_exists = codex_toml.exists();
    if codex_toml_exists {
        files_to_remove.push((
            format!("{} ([mcp_servers.sqz] only)", codex_toml.display()),
            true,
        ));
    }

    // Zed: strip the sqz guidance block from .rules / AGENTS.md and
    // remove context_servers.sqz from the user's Zed settings.json.
    let zed_guidance_present = [".rules", "AGENTS.md"].iter().any(|f| {
        let p = project_dir.join(f);
        p.exists()
            && std::fs::read_to_string(&p)
                .map(|s| s.contains("BEGIN sqz-zed-guidance"))
                .unwrap_or(false)
    });
    if zed_guidance_present {
        files_to_remove.push((
            format!("{} (sqz Zed guidance block only)", project_dir.display()),
            true,
        ));
    }
    let zed_settings = sqz_engine::zed_settings_path();
    let zed_settings_has_sqz = zed_settings.exists()
        && std::fs::read_to_string(&zed_settings)
            .map(|s| s.contains("\"sqz\""))
            .unwrap_or(false);
    if zed_settings_has_sqz {
        files_to_remove.push((
            format!("{} (context_servers.sqz only)", zed_settings.display()),
            true,
        ));
    }

    // Kiro steering file, project MCP entry, and the legacy hook file.
    let kiro_steering = sqz_engine::kiro_steering_path(&project_dir);
    let kiro_steering_managed = kiro_steering.exists()
        && std::fs::read_to_string(&kiro_steering)
            .map(|s| s.contains("sqz-kiro-steering"))
            .unwrap_or(false);
    if kiro_steering_managed {
        files_to_remove.push((kiro_steering.display().to_string(), true));
    }
    let kiro_mcp = sqz_engine::kiro_mcp_path(&project_dir);
    let kiro_mcp_has_sqz = kiro_mcp.exists()
        && std::fs::read_to_string(&kiro_mcp)
            .map(|s| s.contains("\"sqz\""))
            .unwrap_or(false);
    if kiro_mcp_has_sqz {
        files_to_remove.push((
            format!("{} (mcpServers.sqz only)", kiro_mcp.display()),
            true,
        ));
    }
    let kiro_legacy = sqz_engine::kiro_legacy_hook_path(&project_dir);
    if kiro_legacy.exists() {
        files_to_remove.push((kiro_legacy.display().to_string(), true));
    }

    // Claude Code user-level ~/.claude/settings.json: surgically remove
    // sqz's hook entries while preserving the user's permissions, env,
    // statusLine and unrelated hooks. This is the symmetric teardown
    // for `sqz init --global`. If the user installed with project scope
    // only, this is a no-op (the file either doesn't exist or has no
    // sqz entries).
    let claude_user_settings = sqz_engine::claude_user_settings_path();
    let claude_user_settings_exists = claude_user_settings
        .as_ref()
        .map(|p| p.exists())
        .unwrap_or(false);
    if let (Some(path), true) = (&claude_user_settings, claude_user_settings_exists) {
        files_to_remove.push((
            format!("{} (sqz hook entries only)", path.display()),
            true,
        ));
    }

    // OpenCode user-level TypeScript plugin. Unlike the other tool
    // configs this lives at `~/.config/opencode/plugins/sqz.ts`, outside
    // any project — so `generate_hook_configs` doesn't produce an entry
    // for it. But it MUST be removed too: OpenCode loads the plugin on
    // startup regardless of what `opencode.json` says. Users reported
    // (follow-up to issue #5) that disabling sqz in config did nothing
    // because the plugin file kept running.
    let opencode_plugin = sqz_engine::opencode_plugin_path();
    let opencode_plugin_exists = opencode_plugin.exists();
    if opencode_plugin_exists {
        files_to_remove.push((opencode_plugin.display().to_string(), true));
    }

    if files_to_remove.is_empty() {
        println!("[sqz] nothing to uninstall — no sqz files found.");
        return;
    }

    println!("\nThe following files will be modified or removed:\n");
    for (path, _) in &files_to_remove {
        println!("  [remove] {path}");
    }
    println!();

    if !skip_confirm {
        print!("Do you want to continue? [Y/n] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            eprintln!("[sqz] could not read input, aborting.");
            std::process::exit(1);
        }
        let answer = answer.trim().to_lowercase();
        if !answer.is_empty() && answer != "y" && answer != "yes" {
            println!("[sqz] aborted.");
            return;
        }
    }

    // Remove shell hook
    if rc_has_hook {
        match hook.uninstall() {
            Ok(true) => println!("[sqz] ✓ hook removed from {}", rc_path.display()),
            Ok(false) => println!("[sqz] ✓ hook not found in {}", rc_path.display()),
            Err(e) => eprintln!("[sqz] ✗ warning: {e}"),
        }
    }

    // Remove AI tool configs
    for config in &tool_configs {
        // OpenCode is handled separately below (surgical edit).
        if config.tool_name == "OpenCode" {
            continue;
        }
        // Codex is also surgical — AGENTS.md and ~/.codex/config.toml.
        if config.tool_name == "Codex" {
            continue;
        }
        // Zed and Kiro share files with user content; their dedicated
        // blocks below remove only sqz's parts.
        if config.tool_name == "Zed" || config.tool_name == "Kiro" {
            continue;
        }
        // Copilot CLI: user-level sqz-owned hook file.
        if config.tool_name == "Copilot CLI" {
            match sqz_engine::remove_copilot_hook() {
                Ok(true) => println!(
                    "[sqz] ✓ removed {}",
                    sqz_engine::copilot_hooks_path().display()
                ),
                Ok(false) => {}
                Err(e) => eprintln!("[sqz] ✗ could not remove Copilot hook: {e}"),
            }
            continue;
        }
        // Windsurf: remove the rules file plus a legacy sqz-authored
        // .windsurfrules from older installs.
        if config.tool_name == "Windsurf" {
            let target = project_dir.join(&config.config_path);
            if target.exists() {
                match std::fs::remove_file(&target) {
                    Ok(()) => println!("[sqz] ✓ removed {}", target.display()),
                    Err(e) => eprintln!("[sqz] ✗ could not remove {}: {e}", target.display()),
                }
            }
            let legacy = project_dir.join(".windsurfrules");
            let legacy_is_sqz = legacy.exists()
                && std::fs::read_to_string(&legacy)
                    .map(|s| s.starts_with("# sqz — Token-Optimized CLI Output"))
                    .unwrap_or(false);
            if legacy_is_sqz {
                match std::fs::remove_file(&legacy) {
                    Ok(()) => println!("[sqz] ✓ removed {}", legacy.display()),
                    Err(e) => eprintln!("[sqz] ✗ could not remove {}: {e}", legacy.display()),
                }
            }
            continue;
        }
        // Cline: .clinerules may be a directory (remove only our sqz.md)
        // or a file (remove only when sqz-authored).
        if config.tool_name == "Cline" {
            let base = project_dir.join(".clinerules");
            let target = if base.is_dir() { base.join("sqz.md") } else { base };
            let is_sqz = target.is_file()
                && std::fs::read_to_string(&target)
                    .map(|s| s.starts_with("# sqz — Token-Optimized CLI Output"))
                    .unwrap_or(false);
            if is_sqz {
                match std::fs::remove_file(&target) {
                    Ok(()) => println!("[sqz] ✓ removed {}", target.display()),
                    Err(e) => eprintln!("[sqz] ✗ could not remove {}: {e}", target.display()),
                }
            }
            continue;
        }
        let full = project_dir.join(&config.config_path);
        if full.exists() {
            match std::fs::remove_file(&full) {
                Ok(()) => println!("[sqz] ✓ removed {}", full.display()),
                Err(e) => eprintln!("[sqz] ✗ could not remove {}: {e}", full.display()),
            }
        }
    }

    // Repo-level CI hook (only when sqz-authored).
    match sqz_engine::remove_ci_hook(&project_dir) {
        Ok(true) => println!(
            "[sqz] ✓ removed {}",
            sqz_engine::ci_hooks_path(&project_dir).display()
        ),
        Ok(false) => {}
        Err(e) => eprintln!("[sqz] ✗ could not remove CI hook: {e}"),
    }

    // Surgically remove sqz entries from the OpenCode project config.
    if opencode_config.is_some() {
        match sqz_engine::remove_sqz_from_opencode_config(&project_dir) {
            Ok(Some((path, true))) => {
                // Either the file was rewritten without sqz's keys, or
                // it was deleted because nothing else remained. The
                // remove_sqz helper handles both paths; we just report
                // what ended up on disk.
                if path.exists() {
                    println!(
                        "[sqz] ✓ removed sqz entries from {}",
                        path.display()
                    );
                } else {
                    println!("[sqz] ✓ removed {}", path.display());
                }
            }
            Ok(Some((path, false))) => {
                println!(
                    "[sqz] ✓ no sqz entries found in {}",
                    path.display()
                );
            }
            Ok(None) => {
                // Shouldn't happen — we only enter this branch when
                // find_opencode_config returned Some during discovery.
            }
            Err(e) => {
                if let Some(path) = &opencode_config_display {
                    eprintln!("[sqz] ✗ could not clean up {path}: {e}");
                } else {
                    eprintln!("[sqz] ✗ could not clean up OpenCode config: {e}");
                }
            }
        }
    }

    // Remove the OpenCode user-level plugin file if it was present.
    if opencode_plugin_exists {
        match std::fs::remove_file(&opencode_plugin) {
            Ok(()) => println!("[sqz] ✓ removed {}", opencode_plugin.display()),
            Err(e) => eprintln!(
                "[sqz] ✗ could not remove {}: {e}",
                opencode_plugin.display()
            ),
        }
    }

    // Surgically strip sqz's block from Codex's project AGENTS.md.
    // Leaves any user-authored rules intact.
    if agents_md_exists {
        match sqz_engine::remove_agents_md_guidance(&project_dir) {
            Ok(Some((path, true))) => {
                if path.exists() {
                    println!("[sqz] ✓ removed sqz block from {}", path.display());
                } else {
                    println!("[sqz] ✓ removed {}", path.display());
                }
            }
            Ok(Some((path, false))) => {
                println!("[sqz] ✓ no sqz block found in {}", path.display());
            }
            Ok(None) => { /* path disappeared between discovery and now — fine */ }
            Err(e) => {
                eprintln!(
                    "[sqz] ✗ could not clean up {}: {e}",
                    agents_md.display()
                );
            }
        }
    }

    // Surgically remove [mcp_servers.sqz] from ~/.codex/config.toml.
    // Other MCP servers and comments are preserved by toml_edit.
    if codex_toml_exists {
        match sqz_engine::remove_codex_mcp_config() {
            Ok(Some((path, true))) => {
                if path.exists() {
                    println!(
                        "[sqz] ✓ removed [mcp_servers.sqz] from {}",
                        path.display()
                    );
                } else {
                    println!("[sqz] ✓ removed {}", path.display());
                }
            }
            Ok(Some((path, false))) => {
                println!(
                    "[sqz] ✓ no [mcp_servers.sqz] entry found in {}",
                    path.display()
                );
            }
            Ok(None) => { /* file disappeared between discovery and now */ }
            Err(e) => {
                eprintln!(
                    "[sqz] ✗ could not clean up {}: {e}",
                    codex_toml.display()
                );
            }
        }
    }

    // Zed: strip the sqz guidance block from .rules / AGENTS.md, then
    // remove context_servers.sqz from the user's Zed settings.json.
    // JSONC settings are left alone with a manual note (issue #6 rule).
    if zed_guidance_present {
        match sqz_engine::remove_zed_guidance(&project_dir) {
            Ok(changed) => {
                for path in changed {
                    if path.exists() {
                        println!("[sqz] ✓ removed sqz Zed block from {}", path.display());
                    } else {
                        println!("[sqz] ✓ removed {}", path.display());
                    }
                }
            }
            Err(e) => eprintln!("[sqz] ✗ could not clean up Zed guidance: {e}"),
        }
    }
    if zed_settings_has_sqz {
        match sqz_engine::remove_zed_mcp_config() {
            Ok(sqz_engine::ZedMcpRemove::Removed) => {
                println!(
                    "[sqz] ✓ removed context_servers.sqz from {}",
                    zed_settings.display()
                );
            }
            Ok(sqz_engine::ZedMcpRemove::SkippedJsonc) => {
                println!(
                    "[sqz] ! {} contains comments (JSONC); remove the \
                     \"sqz\" entry under \"context_servers\" by hand.",
                    zed_settings.display()
                );
            }
            Ok(sqz_engine::ZedMcpRemove::NotPresent) => {}
            Err(e) => eprintln!(
                "[sqz] ✗ could not clean up {}: {e}",
                zed_settings.display()
            ),
        }
    }

    if kiro_steering_managed {
        match sqz_engine::remove_kiro_steering(&project_dir) {
            Ok(true) => println!("[sqz] ✓ removed {}", kiro_steering.display()),
            Ok(false) => {}
            Err(e) => eprintln!("[sqz] ✗ could not clean up {}: {e}", kiro_steering.display()),
        }
    }
    if kiro_mcp_has_sqz {
        match sqz_engine::remove_kiro_mcp_config(&project_dir) {
            Ok(sqz_engine::KiroMcpRemove::Removed) => {
                println!("[sqz] ✓ removed mcpServers.sqz from {}", kiro_mcp.display());
            }
            Ok(sqz_engine::KiroMcpRemove::SkippedUnparseable) => {
                println!(
                    "[sqz] ! {} is not valid JSON; remove the \"sqz\" entry by hand.",
                    kiro_mcp.display()
                );
            }
            Ok(sqz_engine::KiroMcpRemove::NotPresent) => {}
            Err(e) => eprintln!("[sqz] ✗ could not clean up {}: {e}", kiro_mcp.display()),
        }
    }
    if kiro_legacy.exists() {
        match sqz_engine::remove_kiro_legacy_hook(&project_dir) {
            Ok(true) => println!("[sqz] ✓ removed {}", kiro_legacy.display()),
            Ok(false) => {}
            Err(e) => eprintln!("[sqz] ✗ could not clean up {}: {e}", kiro_legacy.display()),
        }
    }

    // Surgically remove sqz's PreToolUse / PreCompact / SessionStart
    // entries from ~/.claude/settings.json. Keeps everything else the
    // user has configured (permissions, env, statusLine, unrelated
    // hooks). If the file ends up empty after stripping, it's removed.
    if claude_user_settings_exists {
        match sqz_engine::remove_claude_global_hook() {
            Ok(Some((path, true))) => {
                if path.exists() {
                    println!(
                        "[sqz] ✓ removed sqz hook entries from {}",
                        path.display()
                    );
                } else {
                    println!("[sqz] ✓ removed {}", path.display());
                }
            }
            Ok(Some((path, false))) => {
                println!(
                    "[sqz] ✓ no sqz hook entries found in {}",
                    path.display()
                );
            }
            Ok(None) => { /* file didn't exist after all — skip */ }
            Err(e) => {
                if let Some(path) = &claude_user_settings {
                    eprintln!(
                        "[sqz] ✗ could not clean up {}: {e}",
                        path.display()
                    );
                } else {
                    eprintln!("[sqz] ✗ could not resolve ~/.claude/settings.json: {e}");
                }
            }
        }
    }

    println!("\n[sqz] uninstall complete.");
}

/// Resolve a `--project` value to a canonical absolute path.
/// "." becomes the current directory; relative paths are resolved.
fn resolve_project_filter(raw: &str) -> String {
    if raw == "list" {
        return raw.to_string();
    }
    let path = if raw == "." {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        let p = std::path::PathBuf::from(raw);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(p)
        }
    };
    path.to_string_lossy().to_string()
}

// ── Terminal color helpers ─────────────────────────────────────────────────

mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const CYAN: &str = "\x1b[36m";
    pub const GREEN: &str = "\x1b[32m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const BLUE: &str = "\x1b[34m";
    pub const WHITE: &str = "\x1b[97m";

    /// Returns true if color output is appropriate (TTY + no NO_COLOR).
    pub fn enabled() -> bool {
        if std::env::var("NO_COLOR").is_ok() {
            return false;
        }
        #[cfg(unix)]
        {
            unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    /// Wrap text in color codes if color is enabled.
    pub fn paint(color: &str, text: &str) -> String {
        if enabled() {
            format!("{color}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    pub fn bold(text: &str) -> String { paint(BOLD, text) }
    pub fn cyan(text: &str) -> String { paint(CYAN, text) }
    pub fn green(text: &str) -> String { paint(GREEN, text) }
    pub fn bright_green(text: &str) -> String { paint(BRIGHT_GREEN, text) }
    pub fn yellow(text: &str) -> String { paint(YELLOW, text) }
    pub fn magenta(text: &str) -> String { paint(MAGENTA, text) }
    pub fn dim(text: &str) -> String { paint(DIM, text) }
    pub fn blue(text: &str) -> String { paint(BLUE, text) }
}

/// `sqz stats [session-id]` — full compression stats report.
fn cmd_stats(session_id: Option<String>, project: Option<String>, breakdown: bool, json: bool, cost_opts: Option<sqz_engine::SavingsAssumptions>) {
    let engine = require_engine();

    // JSON output: emit machine-readable stats and exit.
    if json {
        let project_dir = project.as_deref().map(resolve_project_filter);
        let cs = if let Some(ref dir) = project_dir {
            engine.session_store().compression_stats_for_project(dir).unwrap_or_default()
        } else {
            engine.session_store().compression_stats().unwrap_or_default()
        };
        let cache_entries = engine.session_store()
            .list_cache_entries_lru()
            .unwrap_or_default();
        let cache_size: u64 = cache_entries.iter().map(|(_, sz)| sz).sum();

        let regret = engine.session_store().regret_stats().unwrap_or_default();
        let mut obj = serde_json::json!({
            "totalCompressions": cs.total_compressions,
            "tokensIn": cs.total_tokens_in,
            "tokensOut": cs.total_tokens_out,
            "tokensSaved": cs.tokens_saved(),
            "avgReduction": cs.reduction_pct(),
            "cacheEntries": cache_entries.len(),
            "cacheSize": cache_size,
            "regretReruns": regret.reruns,
            "regretExpands": regret.expands,
        });
        if let Some(ref a) = cost_opts {
            let est = sqz_engine::estimate_saved_cost(cs.tokens_saved(), a);
            obj["estimatedCostSavedUsd"] = serde_json::json!(est.total_usd);
            obj["costAssumptions"] = serde_json::json!({
                "priceInPerMtok": a.price_in_per_mtok,
                "cacheWriteMult": a.cache_write_mult,
                "cacheReadMult": a.cache_read_mult,
                "rereadTurns": a.reread_turns,
            });
        }
        println!("{}", serde_json::to_string(&obj).unwrap());
        return;
    }

    // Handle --project list
    if project.as_deref() == Some("list") {
        let projects = engine.session_store().list_projects().unwrap_or_default();
        if projects.is_empty() {
            println!("{}", colors::yellow("[sqz] No per-project data yet. New compressions will be tagged automatically."));
            return;
        }
        println!();
        println!("  {}", colors::bold(&colors::cyan("📂 Tracked Projects")));
        println!("  {}", colors::dim(&"─".repeat(65)));
        for (dir, count, saved) in &projects {
            let short = std::path::Path::new(dir)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| dir.to_string());
            println!(
                "  {} {:>5} calls  {} {:>7} saved  {}",
                colors::dim("●"),
                colors::bold(&format!("{}", count)),
                colors::green("↓"),
                colors::bright_green(&format!("{}", saved)),
                colors::magenta(&short),
            );
        }
        println!("  {}", colors::dim(&"─".repeat(65)));
        println!();
        println!("  {} sqz stats --project <path>", colors::dim("Use:"));
        println!("       sqz gain --project <path>");
        println!();
        return;
    }

    let project_dir = project.as_deref().map(resolve_project_filter);

    let title = if let Some(ref dir) = project_dir {
        let short = std::path::Path::new(dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dir.clone());
        format!("sqz stats — {}", short)
    } else {
        "sqz compression stats".to_string()
    };

    // Cumulative compression stats (filtered or global)
    let cs = if let Some(ref dir) = project_dir {
        engine.session_store().compression_stats_for_project(dir).unwrap_or_default()
    } else {
        engine.session_store().compression_stats().unwrap_or_default()
    };

    println!();
    println!("  {}", colors::bold(&colors::cyan(&format!("📊 {}", title))));
    println!("  {}", colors::dim(&"─".repeat(50)));
    println!();

    // Big number: tokens saved
    let saved_str = format!("{}", cs.tokens_saved());
    println!("  {}  {}", colors::bright_green(&saved_str), colors::bold("tokens saved"));
    println!("  {}  {:.1}% average reduction", colors::green("↓"), cs.reduction_pct());
    println!();

    // Stats table
    println!("  {:<22} {}", colors::dim("Compressions"), colors::bold(&format!("{}", cs.total_compressions)));
    println!("  {:<22} {}", colors::dim("Tokens in"), format!("{}", cs.total_tokens_in));
    println!("  {:<22} {}", colors::dim("Tokens out"), format!("{}", cs.total_tokens_out));
    println!("  {:<22} {}", colors::dim("Tokens saved"), colors::green(&format!("{}", cs.tokens_saved())));
    println!("  {:<22} {}", colors::dim("Avg reduction"), colors::bright_green(&format!("{:.1}%", cs.reduction_pct())));

    if let Some(ref dir) = project_dir {
        println!("  {:<22} {}", colors::dim("Project"), colors::magenta(dir));
    }

    // Estimated billed-cost saving (opt-in via --cost)
    if let Some(ref a) = cost_opts {
        let est = sqz_engine::estimate_saved_cost(cs.tokens_saved(), a);
        // Two decimals reads best for real totals; small values get four
        // so "write + read = total" doesn't look like $0.00 + $0.00 = $0.01.
        let fmt_usd = |v: f64| if est.total_usd < 0.10 { format!("${v:.4}") } else { format!("${v:.2}") };
        println!();
        println!("  {}", colors::bold(&colors::cyan("💵 Estimated Cost Saved")));
        println!("  {}", colors::dim(&"─".repeat(50)));
        println!("  {:<22} {}", colors::dim("Cache-write savings"), colors::green(&fmt_usd(est.write_usd)));
        println!("  {:<22} {}", colors::dim("Cache-read savings"), colors::green(&fmt_usd(est.reread_usd)));
        println!("  {:<22} {}", colors::dim("Estimated total"), colors::bright_green(&fmt_usd(est.total_usd)));
        println!();
        println!("  {}", colors::dim(&format!(
            "Assumes ${:.2}/MTok input, cache write ×{:.2}, cache read ×{:.2}, ~{:.0} re-read turns.",
            a.price_in_per_mtok, a.cache_write_mult, a.cache_read_mult, a.reread_turns
        )));
        println!("  {}", colors::dim(
            "Models direct token cost under provider prompt caching (the conservative"
        ));
        println!("  {}", colors::dim(
            "case); token reduction alone is not billed-cost reduction (arXiv:2607.12161)."
        ));
    }

    // Compression-regret signals (shown when any exist)
    if let Ok(regret) = engine.session_store().regret_stats() {
        if regret.reruns > 0 || regret.expands > 0 {
            let rate = if cs.total_compressions > 0 {
                regret.reruns as f64 / cs.total_compressions as f64 * 100.0
            } else {
                0.0
            };
            println!();
            println!("  {}", colors::bold(&colors::cyan("🔁 Regret Signals")));
            println!("  {}", colors::dim(&"─".repeat(50)));
            println!(
                "  {:<22} {} {}",
                colors::dim("Quick re-runs"),
                colors::bold(&format!("{}", regret.reruns)),
                colors::dim(&format!("({:.1}% of compressions, avg gap {:.0}s)", rate, regret.avg_rerun_gap_secs)),
            );
            println!(
                "  {:<22} {} {}",
                colors::dim("Ref expands"),
                colors::bold(&format!("{}", regret.expands)),
                colors::dim("(§ref§ tokens recovered to original bytes)"),
            );
            let top = engine.session_store().regret_by_command(3).unwrap_or_default();
            if !top.is_empty() {
                let list = top
                    .iter()
                    .map(|(c, n)| format!("{c} ×{n}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("  {:<22} {}", colors::dim("Most re-run"), colors::magenta(&list));
            }
            println!("  {}", colors::dim(
                "Re-running a command whose output hadn't changed suggests the compressed"
            ));
            println!("  {}", colors::dim(
                "serving missed something. High counts for one command? File an issue."
            ));
        }
    }

    // Session cost section (if session_id provided)
    if let Some(ref sid) = session_id {
        match engine.cost_summary(sid) {
            Ok(cost) => {
                println!();
                println!("  {}", colors::bold(&colors::cyan("💰 Session Cost")));
                println!("  {}", colors::dim(&"─".repeat(50)));
                println!("  {:<22} {}", colors::dim("Session"), colors::magenta(sid));
                println!("  {:<22} {}", colors::dim("Total tokens"), format!("{}", cost.total_tokens));
                println!("  {:<22} {}", colors::dim("Total cost"), colors::yellow(&format!("${:.6}", cost.total_usd)));
                println!("  {:<22} {}", colors::dim("Cache savings"), colors::green(&format!("${:.6}", cost.cache_savings_usd)));
                println!("  {:<22} {}", colors::dim("Compression savings"), colors::green(&format!("${:.6}", cost.compression_savings_usd)));
                if cost.total_usd > 0.0 {
                    let pct = (cost.compression_savings_usd / (cost.total_usd + cost.compression_savings_usd)) * 100.0;
                    println!("  {:<22} {}", colors::dim("Effective reduction"), colors::bright_green(&format!("{:.1}%", pct)));
                }
            }
            Err(e) => {
                println!();
                println!("  {:<22} {}", colors::dim("Session"), sid);
                println!("  {:<22} {}", colors::dim("Error"), format!("{e}"));
            }
        }
    }

    // Cache stats (global only — cache is shared across projects)
    if project_dir.is_none() {
        let cache_entries = engine.session_store()
            .list_cache_entries_lru()
            .unwrap_or_default();
        let cache_size: u64 = cache_entries.iter().map(|(_, sz)| sz).sum();
        println!();
        println!("  {}", colors::bold(&colors::cyan("🗄️  Cache")));
        println!("  {}", colors::dim(&"─".repeat(50)));
        println!("  {:<22} {}", colors::dim("Entries"), format!("{}", cache_entries.len()));
        println!("  {:<22} {}", colors::dim("Size"), format_bytes(cache_size));
    }

    // Per-command breakdown
    if breakdown {
        let cmds = if let Some(ref dir) = project_dir {
            engine.session_store().command_breakdown_for_project(15, dir).unwrap_or_default()
        } else {
            engine.session_store().command_breakdown(15).unwrap_or_default()
        };

        if !cmds.is_empty() {
            println!();
            println!("  {}", colors::bold(&colors::cyan("🔍 Top Token Consumers")));
            println!("  {}", colors::dim(&"─".repeat(70)));
            println!(
                "  {:<20} {:>6} {:>10} {:>10} {:>8}",
                "command", "calls", "tokens in", "out", "saved"
            );
            println!("  {}", colors::dim(&"─".repeat(70)));
            for c in &cmds {
                let pct = c.reduction_pct();
                let pct_str = format!("{:.0}%", pct);
                let pct_colored = if pct > 70.0 {
                    colors::bright_green(&pct_str)
                } else if pct > 30.0 {
                    colors::green(&pct_str)
                } else if pct > 0.0 {
                    colors::yellow(&pct_str)
                } else {
                    colors::dim(&pct_str)
                };

                // Truncate command name to 18 visible chars (char-count,
                // not bytes — command labels can carry non-ASCII).
                let cmd_display = if c.command.chars().count() > 18 {
                    let cut: String = c.command.chars().take(17).collect();
                    format!("{cut}…")
                } else {
                    c.command.clone()
                };
                let cmd_colored = colors::magenta(&cmd_display);
                // Pad to 20 visible chars (accounting for ANSI codes)
                let visible_len = cmd_display.len();
                let pad_needed = if visible_len < 20 { 20 - visible_len } else { 0 };
                let padded_cmd = format!("{}{}", cmd_colored, " ".repeat(pad_needed));

                println!(
                    "  {} {:>6} {:>10} {:>10} {:>8}",
                    padded_cmd,
                    c.invocations,
                    c.tokens_in,
                    c.tokens_out,
                    pct_colored,
                );
            }
            println!("  {}", colors::dim(&"─".repeat(70)));
        }
    }

    println!();
}

/// `sqz gain [--days N]` — show accumulated token savings over time.
fn cmd_gain(days: u32, project: Option<String>) {
    let engine = require_engine();
    let project_dir = project.as_deref().map(resolve_project_filter);

    let gains = if let Some(ref dir) = project_dir {
        engine.session_store().daily_gains_for_project(days, dir).unwrap_or_default()
    } else {
        engine.session_store().daily_gains(days).unwrap_or_default()
    };
    let stats = if let Some(ref dir) = project_dir {
        engine.session_store().compression_stats_for_project(dir).unwrap_or_default()
    } else {
        engine.session_store().compression_stats().unwrap_or_default()
    };

    if gains.is_empty() {
        if stats.total_compressions > 0 {
            // We have data, just none in the window. Tell the user that
            // explicitly so they don't think sqz isn't working.
            if let Some(ref dir) = project_dir {
                println!(
                    "{}",
                    colors::yellow(&format!(
                        "[sqz] No compressions in the last {} days for {}, but \
                         {} total recorded. Try `sqz gain --days 30` for a wider \
                         window, or `sqz stats --project {}` for cumulative.",
                        days, dir, stats.total_compressions, dir,
                    ))
                );
            } else {
                println!(
                    "{}",
                    colors::yellow(&format!(
                        "[sqz] No compressions in the last {} days, but {} total \
                         recorded. Try `sqz gain --days 30` for a wider window, \
                         or `sqz stats` for cumulative.",
                        days, stats.total_compressions,
                    ))
                );
            }
        } else if project_dir.is_some() {
            println!("{}", colors::yellow("[sqz] No compression data for this project yet."));
        } else {
            println!("{}", colors::yellow("[sqz] No compression data yet. Run `sqz compress` to start tracking."));
        }
        return;
    }

    let max_saved = gains.iter().map(|g| g.tokens_saved).max().unwrap_or(1).max(1);
    let bar_width: u64 = 30;

    let header = if let Some(ref dir) = project_dir {
        let short = std::path::Path::new(dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dir.clone());
        format!("📈 Token Savings — {} (last {} days)", short, days)
    } else {
        format!("📈 Token Savings (last {} days)", days)
    };

    println!();
    println!("  {}", colors::bold(&colors::cyan(&header)));
    println!("  {}", colors::dim(&"─".repeat(55)));

    for g in &gains {
        let bar_len = (g.tokens_saved * bar_width / max_saved) as usize;
        let bar_str: String = "█".repeat(bar_len);
        let pad: String = " ".repeat(bar_width as usize - bar_len);

        // Color the bar based on savings intensity
        let colored_bar = if bar_len > 20 {
            colors::bright_green(&bar_str)
        } else if bar_len > 10 {
            colors::green(&bar_str)
        } else if bar_len > 0 {
            colors::blue(&bar_str)
        } else {
            colors::dim(&bar_str)
        };

        let saved_str = if g.tokens_saved > 10000 {
            colors::bright_green(&format!("{:>6}", g.tokens_saved))
        } else if g.tokens_saved > 1000 {
            colors::green(&format!("{:>6}", g.tokens_saved))
        } else {
            format!("{:>6}", g.tokens_saved)
        };

        println!(
            "  {} │{}{}│ {} saved",
            colors::dim(&g.date[5..].to_string()),
            colored_bar,
            pad,
            saved_str,
        );
    }

    println!("  {}", colors::dim(&"─".repeat(55)));

    // Summary line
    let total_saved = colors::bright_green(&format!("{}", stats.tokens_saved()));
    let total_compressions = colors::bold(&format!("{}", stats.total_compressions));
    let reduction = colors::green(&format!("{:.1}%", stats.reduction_pct()));
    println!(
        "  {} {} compressions, {} tokens saved ({} avg)",
        colors::dim("Total:"),
        total_compressions,
        total_saved,
        reduction,
    );
    println!();
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ── Discover ──────────────────────────────────────────────────────────────

fn cmd_discover(days: u32) {
    let engine = require_engine();
    let store = engine.session_store();

    let stats = match store.compression_stats() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[sqz] failed to read stats: {e}");
            return;
        }
    };

    let gains = match store.daily_gains(days) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[sqz] failed to read daily gains: {e}");
            return;
        }
    };

    println!("sqz discover — missed savings analysis (last {} days)", days);
    println!("{}", "─".repeat(50));

    if stats.total_compressions == 0 {
        println!();
        println!("  No compression data found.");
        println!("  sqz hasn't intercepted any commands yet.");
        println!();
        println!("  To start saving tokens:");
        println!("    sqz init          # install shell hooks");
        println!("    # then restart your AI tool");
        println!();
        return;
    }

    let total_original = stats.total_tokens_in;
    let total_compressed = stats.total_tokens_out;
    let total_saved = stats.tokens_saved();
    let avg_reduction = stats.reduction_pct();

    println!();
    println!("  Compressions:    {}", stats.total_compressions);
    println!("  Tokens original: {}", total_original);
    println!("  Tokens after:    {}", total_compressed);
    println!("  Tokens saved:    {} ({:.1}% avg reduction)", total_saved, avg_reduction);
    println!();

    // Estimate what could be saved with better adoption
    let days_with_data = gains.iter().filter(|g| g.tokens_saved > 0).count();
    let days_without = (days as usize).saturating_sub(days_with_data);

    if days_without > 0 && days_with_data > 0 {
        let avg_daily_savings = total_saved / days_with_data.max(1) as u64;
        let missed = avg_daily_savings * days_without as u64;
        println!("  {} days with no sqz activity.", days_without);
        println!("  Estimated missed savings: ~{} tokens", missed);
        println!();
    }

    // Suggest high-value commands
    println!("  High-value commands to route through sqz:");
    println!("    git status/diff/log  → 70-80% reduction");
    println!("    cargo test/build     → 80-90% reduction (failures only)");
    println!("    docker ps/images     → 70-80% reduction");
    println!("    npm test/install     → 60-90% reduction");
    println!("    kubectl get          → 60-70% reduction");
    println!();
}

// ── Resume ────────────────────────────────────────────────────────────────

fn cmd_resume(session_id: Option<String>) {
    let engine = require_engine();
    let store = engine.session_store();

    // If no session ID given, try to find the most recent session
    let sid = match session_id {
        Some(id) => id,
        None => {
            // Find the most recently updated session
            match store.latest_session() {
                Ok(Some(summary)) => summary.id,
                Ok(None) => {
                    eprintln!("[sqz] no sessions found. Start a session first.");
                    return;
                }
                Err(e) => {
                    eprintln!("[sqz] failed to query sessions: {e}");
                    return;
                }
            }
        }
    };

    // Load the session
    let session = match store.load_session(sid.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[sqz] failed to load session '{}': {e}", sid);
            return;
        }
    };

    // Generate a session guide using SessionContinuityManager
    use sqz_engine::SessionContinuityManager;
    let continuity = SessionContinuityManager::new(store);

    // Build a snapshot from the session
    use sqz_engine::{Snapshot, SnapshotEvent, SnapshotEventType};
    let mut events = Vec::new();

    // Add summary as context
    if !session.compressed_summary.is_empty() {
        events.push(SnapshotEvent::new(
            SnapshotEventType::Summary,
            session.compressed_summary.clone(),
        ));
    }

    // Add recent conversation turns
    for (i, turn) in session.conversation.iter().rev().take(5).enumerate() {
        let event_type = if i == 0 && turn.role == sqz_engine::Role::User {
            SnapshotEventType::LastPrompt
        } else {
            SnapshotEventType::Context
        };
        let content = if turn.content.len() > 200 {
            format!("{}...", sqz_engine::text_boundary::truncate_str(&turn.content, 200))
        } else {
            turn.content.clone()
        };
        events.push(SnapshotEvent::new(event_type, content));
    }

    // Add learnings
    for learning in &session.learnings {
        events.push(SnapshotEvent::new(
            SnapshotEventType::Learning,
            format!("{}: {}", learning.key, learning.value),
        ));
    }

    // Add corrections as decisions
    for correction in &session.corrections.entries {
        events.push(SnapshotEvent::new(
            SnapshotEventType::Decision,
            format!("{} → {}", correction.original, correction.correction),
        ));
    }

    let snapshot = Snapshot {
        events,
    };

    let guide = continuity.generate_guide(&snapshot);

    println!("{}", guide.text);
    eprintln!("[sqz] session guide: {} tokens from session '{}'", guide.token_count, sid);
}

// ── Compact command ───────────────────────────────────────────────────────

/// `sqz compact` — proactively evict stale context.
fn cmd_compact() {
    let engine = require_engine();
    let store = engine.session_store();

    // Build context items from known files in the cache
    let known_files = store.known_files().unwrap_or_default();
    let cache_entries = store.list_cache_entries_lru().unwrap_or_default();
    let current_turn = engine.cache_manager().current_turn();

    if known_files.is_empty() && cache_entries.is_empty() {
        println!("[sqz] nothing to compact — no cached content");
        return;
    }

    // Build context items from cache entries
    let items: Vec<sqz_engine::ContextItem> = cache_entries
        .iter()
        .enumerate()
        .map(|(i, (hash, size))| sqz_engine::ContextItem {
            id: format!("cache:{}", &hash[..hash.len().min(12)]),
            content: format!("[cached content, {} bytes]", size),
            last_accessed_turn: current_turn.saturating_sub(cache_entries.len() as u64 - i as u64),
            access_count: 1,
            tokens: (*size as u32 + 3) / 4,
            pinned: false,
        })
        .collect();

    let config = sqz_engine::EvictionConfig::default();
    match sqz_engine::evict(&items, current_turn, &config) {
        Ok(result) => {
            if result.evicted.is_empty() {
                println!("[sqz] compact: nothing to evict (all items are recent)");
            } else {
                // Notify the cache manager that compaction happened
                engine.cache_manager().notify_compaction();

                println!("{}", result.eviction_summary);
                println!(
                    "[sqz] compact: {} → {} tokens ({} freed)",
                    result.tokens_before,
                    result.tokens_after,
                    result.tokens_before - result.tokens_after,
                );
            }
        }
        Err(e) => {
            eprintln!("[sqz] compact error: {e}");
        }
    }
}

/// `sqz reset` — clear cache, stats, or both (issue #17).
///
/// Gives users control over sqz's persistent state when they want to
/// start fresh — switching models, wiping a project, or debugging
/// stale `§ref:HASH§` tokens that confuse the agent.
fn cmd_reset(cache_only: bool, stats_only: bool, project: Option<String>, skip_confirm: bool) {
    use std::io::Write;

    let engine = require_engine();
    let store = engine.session_store();

    let project_dir = project.as_deref().map(resolve_project_filter);

    // Determine what we're clearing
    let (clear_cache, clear_stats) = match (cache_only, stats_only) {
        (true, false) => (true, false),
        (false, true) => (false, true),
        // Default (neither flag): clear both
        _ => (true, true),
    };

    // Show what will be cleared
    println!("[sqz] reset plan:");
    if clear_cache && project_dir.is_none() {
        let count = store.list_cache_entries_lru().map(|v| v.len()).unwrap_or(0);
        println!("  • dedup cache: {} entries will be cleared", count);
    }
    if clear_stats {
        if let Some(ref dir) = project_dir {
            println!("  • compression stats for project: {}", dir);
        } else {
            println!("  • compression stats: all entries will be cleared");
        }
    }
    if !clear_cache && !clear_stats {
        println!("  (nothing to clear)");
        return;
    }
    println!();

    // Confirm
    if !skip_confirm {
        print!("Continue? [Y/n] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            eprintln!("[sqz] could not read input, aborting.");
            std::process::exit(1);
        }
        let answer = answer.trim().to_lowercase();
        if !answer.is_empty() && answer != "y" && answer != "yes" {
            println!("[sqz] aborted.");
            return;
        }
    }

    // Execute
    if clear_cache && project_dir.is_none() {
        match store.clear_cache() {
            Ok(count) => println!("[sqz] ✓ cleared {} cache entries", count),
            Err(e) => eprintln!("[sqz] ✗ cache clear failed: {e}"),
        }
    }
    if clear_stats {
        if let Some(ref dir) = project_dir {
            match store.clear_stats_for_project(dir) {
                Ok(count) => println!("[sqz] ✓ cleared {} stat entries for {}", count, dir),
                Err(e) => eprintln!("[sqz] ✗ stats clear failed: {e}"),
            }
        } else {
            match store.clear_stats() {
                Ok(count) => println!("[sqz] ✓ cleared {} stat entries", count),
                Err(e) => eprintln!("[sqz] ✗ stats clear failed: {e}"),
            }
        }
    }

    println!();
    println!("[sqz] reset complete.");
}

/// `sqz pin` — manage pinned cache entries.
///
/// Pinned entries bypass the 30-minute TTL freshness check so dedup
/// refs to stable knowledge-base content stay valid forever. Useful
/// when the original is persistently re-injected into every agent
/// (e.g. project docs, system prompts, a wiki) and therefore never
/// compacted out of the LLM's context.
fn cmd_pin(action: PinAction) {
    match action {
        PinAction::Add { input } => cmd_pin_add(input),
        PinAction::Mark { hash } => cmd_pin_set(hash, true),
        PinAction::Remove { hash } => cmd_pin_set(hash, false),
        PinAction::List => cmd_pin_list(),
        PinAction::Clear { yes } => cmd_pin_clear(yes),
    }
}

fn cmd_pin_add(input: String) {
    use std::io::Read;

    let content = if input == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("[sqz pin] read stdin: {e}");
            std::process::exit(1);
        }
        buf
    } else {
        let path = std::path::Path::new(&input);
        if path.exists() && path.is_file() {
            match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[sqz pin] read {}: {e}", path.display());
                    std::process::exit(1);
                }
            }
        } else {
            input.clone()
        }
    };

    let engine = require_engine();
    if let Err(e) = engine.compress_with_cache(&content) {
        eprintln!("[sqz pin] compress: {e}");
        std::process::exit(1);
    }

    let hash = sqz_engine::CacheManager::sha256_hex(content.as_bytes());
    match engine.session_store().pin_cache_entry(&hash) {
        Ok(true) => {
            println!("[sqz] pinned {} ({} bytes)", &hash[..16.min(hash.len())], content.len());
        }
        Ok(false) => {
            eprintln!("[sqz pin] cache entry missing after compress — internal error");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[sqz pin] {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_pin_set(hash_or_prefix: String, pinned: bool) {
    let engine = require_engine();
    let store = engine.session_store();

    let full = match store.resolve_cache_hash_by_prefix(&hash_or_prefix) {
        Ok(Some(h)) => h,
        Ok(None) => {
            eprintln!("[sqz pin] no cache entry matches '{hash_or_prefix}'");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[sqz pin] {e}");
            std::process::exit(1);
        }
    };

    let result = if pinned {
        store.pin_cache_entry(&full)
    } else {
        store.unpin_cache_entry(&full)
    };

    let verb = if pinned { "pinned" } else { "unpinned" };
    match result {
        Ok(true) => println!("[sqz] {verb} {}", &full[..16.min(full.len())]),
        Ok(false) => {
            eprintln!("[sqz pin] cache entry vanished mid-operation");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[sqz pin] {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_pin_list() {
    let engine = require_engine();
    let store = engine.session_store();

    let entries = match store.list_pinned_cache_entries() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[sqz pin] {e}");
            std::process::exit(1);
        }
    };

    if entries.is_empty() {
        println!("[sqz] no pinned entries. Use `sqz pin add <file>` to pin content.");
        return;
    }

    println!("HASH              SIZE       LAST ACCESSED");
    for (hash, accessed_at, size) in &entries {
        let short = &hash[..16.min(hash.len())];
        let size_str = format_size_bytes(*size);
        let when = accessed_at.format("%Y-%m-%d %H:%M UTC");
        println!("{short}  {size_str:<10} {when}");
    }
    println!();
    println!("{} pinned entries.", entries.len());
}

fn cmd_pin_clear(skip_confirm: bool) {
    use std::io::Write;

    let engine = require_engine();
    let store = engine.session_store();

    if !skip_confirm {
        print!("Unpin every cache entry? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut buf = String::new();
        if std::io::stdin().read_line(&mut buf).is_err() {
            eprintln!("[sqz pin] aborted.");
            std::process::exit(1);
        }
        let ans = buf.trim().to_lowercase();
        if ans != "y" && ans != "yes" {
            println!("[sqz pin] aborted.");
            return;
        }
    }

    match store.unpin_all_cache_entries() {
        Ok(n) => println!("[sqz] unpinned {n} entries."),
        Err(e) => {
            eprintln!("[sqz pin] {e}");
            std::process::exit(1);
        }
    }
}

fn format_size_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// `sqz print-opencode-plugin` — emit plugin TS for manual install.
///
/// Requested on issue #6: users with heavily-commented `opencode.jsonc`
/// don't want `sqz init` to round-trip their file through serde_json
/// (which drops comments). This lets them drop the plugin in by hand
/// and register the MCP entry themselves without sqz touching the
/// config file at all.
fn cmd_print_opencode_plugin() {
    let sqz_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "sqz".to_string())
        .replace('\\', "/"); //  normalize for Git Bash on Windows
    print!("{}", sqz_engine::generate_opencode_plugin(&sqz_path));
}

/// `sqz vizit` — live terminal dashboard for AI agent sessions.
fn cmd_vizit(refresh_secs: u64, db_path: Option<std::path::PathBuf>, no_color: bool) {
    use crate::vizit::{EventLoop, VizitConfig};

    // If --no-color is passed, set NO_COLOR env var before any rendering.
    if no_color {
        std::env::set_var("NO_COLOR", "1");
    }

    // Validate refresh_secs range
    if refresh_secs < 1 || refresh_secs > 60 {
        eprintln!("[sqz vizit] --refresh must be between 1 and 60, got {refresh_secs}");
        std::process::exit(1);
    }

    // Resolve db_path: use provided path or default to ~/.sqz/sessions.db
    let db_path = db_path.unwrap_or_else(default_vizit_db_path);

    let config = VizitConfig {
        refresh_secs,
        db_path,
    };

    match EventLoop::new(config).and_then(|el| el.run()) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[sqz vizit] error: {e}");
            std::process::exit(1);
        }
    }
}

/// Resolve the default path to the sessions database: `~/.sqz/sessions.db`.
fn default_vizit_db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    home.join(".sqz").join("sessions.db")
}

// ── Hook command ──────────────────────────────────────────────────────────

/// `sqz hook <tool>` — process a PreToolUse hook invocation.
/// Reads JSON from stdin, rewrites bash commands to pipe through sqz.
fn cmd_hook(tool: &str) {
    use std::io::Read;
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("[sqz] hook: stdin read error: {e}");
        // On error, output empty JSON to let the tool proceed unmodified
        println!("{{}}");
        return;
    }

    // Special case: PreCompact hooks are not tool-call rewriters. They fire
    // before the host harness compacts its context window. When that happens
    // our cached §ref:HASH§ tokens may point at content the LLM no longer
    // has in context, so we mark every ref stale. Next read of the same
    // content re-sends the full compressed body instead of a dangling ref.
    //
    // Documented by Anthropic at
    // https://docs.anthropic.com/en/docs/claude-code/hooks-guide —
    // PreCompact fires with matcher "manual" or "auto" depending on whether
    // the user ran /compact or the 95% auto-trigger fired.
    if tool == "precompact" {
        match SqzEngine::new() {
            Ok(engine) => {
                engine.cache_manager().notify_compaction();
                eprintln!("[sqz] precompact: marked cached refs stale");
            }
            Err(e) => {
                // Don't block the host: log and return a benign empty JSON.
                eprintln!("[sqz] precompact: engine init failed: {e}");
            }
        }
        println!("{{}}");
        return;
    }

    let result = match tool {
        "opencode" => sqz_engine::process_opencode_hook(&input),
        "cursor" => sqz_engine::process_hook_cursor(&input),
        "gemini" => sqz_engine::process_hook_gemini(&input),
        "windsurf" => sqz_engine::process_hook_windsurf(&input),
        "kiro" => sqz_engine::process_hook_kiro(&input),
        "copilot" => sqz_engine::process_hook_copilot(&input),
        // "claude" and any other tool use the default Claude Code format
        _ => sqz_engine::process_hook(&input),
    };

    match result {
        Ok(output) => print!("{output}"),
        Err(e) => {
            eprintln!("[sqz] hook: processing error: {e}");
            // On error, pass through the original input unchanged
            print!("{input}");
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn require_engine() -> SqzEngine {
    SqzEngine::new().unwrap_or_else(|e| {
        eprintln!("[sqz] failed to initialise engine: {e}");
        std::process::exit(1);
    })
}

fn default_preset_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    home.join(".sqz").join("presets")
}

/// Install shell completions for the detected shell.
fn install_completions(hook: &ShellHook) {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));

    let (dest, content): (std::path::PathBuf, &str) = match hook {
        ShellHook::Fish => (
            home.join(".config").join("fish").join("completions").join("sqz.fish"),
            include_str!("../completions/sqz.fish"),
        ),
        ShellHook::Zsh => (
            home.join(".zsh").join("completions").join("_sqz"),
            include_str!("../completions/sqz.zsh"),
        ),
        ShellHook::Bash => (
            home.join(".local").join("share").join("bash-completion").join("completions").join("sqz"),
            include_str!("../completions/sqz.bash"),
        ),
        ShellHook::Nushell => (
            home.join(".config").join("nushell").join("completions").join("sqz.nu"),
            include_str!("../completions/sqz.nu"),
        ),
        ShellHook::PowerShell => {
            // Append to PowerShell profile
            let profile = std::env::var("PROFILE")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    home.join("Documents")
                        .join("PowerShell")
                        .join("Microsoft.PowerShell_profile.ps1")
                });
            (profile, include_str!("../completions/sqz.ps1"))
        }
    };

    if let Some(parent) = dest.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return; // silently skip if we can't create the dir
        }
    }

    // For PowerShell, append to profile rather than overwrite
    let write_result = if matches!(hook, ShellHook::PowerShell) {
        let existing = std::fs::read_to_string(&dest).unwrap_or_default();
        if existing.contains("Register-ArgumentCompleter -Native -CommandName sqz") {
            return; // already installed
        }
        use std::io::Write;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&dest)
            .and_then(|mut f| writeln!(f, "\n{content}"))
    } else {
        std::fs::write(&dest, content)
    };

    match write_result {
        Ok(()) => println!("[sqz] completions installed to {}", dest.display()),
        Err(_) => {} // silently skip — completions are optional
    }
}

const DEFAULT_PRESET_TOML: &str = r#"[meta]
name = "default"
version = "1"
description = "Default sqz preset"

[compression]
keep_fields.enabled = false
strip_fields.enabled = false
condense.enabled = true
strip_nulls.enabled = true
flatten.enabled = false
truncate_strings.enabled = false
collapse_arrays.enabled = false
custom_transforms.enabled = false

[budget]
window_size = 200000
warning_threshold = 0.70
ceiling_threshold = 0.85
default_agent_budget = 50000

[terse_mode]
enabled = false
level = "moderate"
"#;
