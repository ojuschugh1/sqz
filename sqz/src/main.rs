mod cli_proxy;
mod shell_hook;
mod tests;

use clap::{Parser, Subcommand};
use sqz_engine::SqzEngine;
use sqz_engine::{EntropyAnalyzer, InfoLevel};
use sqz_engine::{TeeManager, TeeMode};
use sqz_engine::{DashboardConfig, DashboardMetrics, DashboardServer};

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
    Status,

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
    },

    /// Show accumulated token savings over time.
    Gain {
        /// Number of days to show (default: 7).
        #[arg(long, default_value_t = 7)]
        days: u32,
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

        Some(Command::Init { yes, global }) => cmd_init(yes, global),
        Some(Command::Compress { text, mode, verify }) => cmd_compress(text, &mode, verify),
        Some(Command::Export { session_id }) => cmd_export(&session_id),
        Some(Command::Import { file }) => cmd_import(&file),
        Some(Command::Status) => cmd_status(),
        Some(Command::Cost { session_id }) => cmd_cost(&session_id),
        Some(Command::Analyze { file, high, low }) => cmd_analyze(file, high, low),
        Some(Command::Tee { action }) => cmd_tee(action),
        Some(Command::Dashboard { port }) => cmd_dashboard(port),
        Some(Command::Proxy { port }) => cmd_proxy(port),
        Some(Command::Uninstall { yes }) => cmd_uninstall(yes),
        Some(Command::Stats { session_id }) => cmd_stats(session_id),
        Some(Command::Gain { days }) => cmd_gain(days),
        Some(Command::Discover { days }) => cmd_discover(days),
        Some(Command::Resume { session_id }) => cmd_resume(session_id),
        Some(Command::Hook { tool }) => cmd_hook(&tool),
        Some(Command::Compact) => cmd_compact(),
    }
}

// ── Command implementations ───────────────────────────────────────────────

/// `sqz init` — detect shell, install hook, create default preset.
fn cmd_init(skip_confirm: bool, global: bool) {
    use std::io::Write;

    let hook = ShellHook::detect();
    let rc_path = hook.rc_path();
    let preset_dir = default_preset_dir();
    let preset_path = preset_dir.join("default.toml");
    let sqz_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "sqz".to_string());
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
        // OpenCode is special: the installer merges into whichever of
        // opencode.json / opencode.jsonc already exists, rather than
        // blindly creating a parallel opencode.json. Report the
        // accurate target in the plan.
        if config.tool_name == "OpenCode" {
            match &opencode_existing {
                Some(path) => {
                    // Existing file — we'll merge sqz entries into it.
                    let note = if opencode_jsonc_has_comments {
                        format!(
                            "{} hook config (merge sqz entries — comments in .jsonc \
                             will be dropped)",
                            config.tool_name
                        )
                    } else {
                        format!("{} hook config (merge sqz entries)", config.tool_name)
                    };
                    plan.push((path.display().to_string(), note, false));
                }
                None => {
                    // No existing config — we'll create a fresh opencode.json.
                    let full_path = project_dir.join(&config.config_path);
                    plan.push((
                        full_path.display().to_string(),
                        format!("{} hook config", config.tool_name),
                        true,
                    ));
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
        if !full_path.exists() {
            plan.push((
                full_path.display().to_string(),
                format!("{} hook config", config.tool_name),
                true,
            ));
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
    let installed_tools = sqz_engine::install_tool_hooks_scoped(&project_dir, &sqz_path, scope);
    for tool in &installed_tools {
        println!("[sqz] ✓ {} hook installed", tool);
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

/// `sqz compress [text] [--mode safe|default|aggressive|auto] [--verify]`
fn cmd_compress(text: Option<String>, mode: &str, show_verify: bool) {
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

    // When reading from stdin in auto mode (the shell hook path), route
    // through CliProxy to get dedup cache, per-command formatters, context
    // refs, and predictive pre-caching. The SQZ_CMD env var carries the
    // original command name from the shell hook.
    if mode == "auto" && is_stdin {
        let cmd = std::env::var("SQZ_CMD").unwrap_or_else(|_| "stdin".to_string());
        let proxy = match CliProxy::new() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[sqz] proxy init error: {e}");
                print!("{input}");
                return;
            }
        };
        let compressed = proxy.intercept_output(&cmd, &input);
        print!("{}", compressed);
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
            let _ = engine.session_store().log_compression(
                c.tokens_original,
                c.tokens_compressed,
                &c.stages_applied,
                mode,
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
fn cmd_status() {
    let engine = require_engine();
    let report = engine.usage_report("default");
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
    let config = DashboardConfig { port };
    let metrics = std::sync::Arc::new(std::sync::Mutex::new(DashboardMetrics::default()));
    let server = DashboardServer::new(config, metrics);

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
                let _ = engine.session_store().log_compression(
                    stats.tokens_original,
                    stats.tokens_compressed,
                    &["proxy".to_string()],
                    &format!("proxy:{:?}", format),
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
        .unwrap_or_else(|_| "sqz".to_string());
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
        let full = project_dir.join(&config.config_path);
        if full.exists() {
            files_to_remove.push((full.display().to_string(), true));
        }
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
        let full = project_dir.join(&config.config_path);
        if full.exists() {
            match std::fs::remove_file(&full) {
                Ok(()) => println!("[sqz] ✓ removed {}", full.display()),
                Err(e) => eprintln!("[sqz] ✗ could not remove {}: {e}", full.display()),
            }
        }
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

/// `sqz stats [session-id]` — full compression stats report.
fn cmd_stats(session_id: Option<String>) {
    let engine = require_engine();

    // Table drawing helpers
    let bar = "├─────────────────────────┼──────────────────┤";
    let top = "┌─────────────────────────┬──────────────────┐";
    let bot = "└─────────────────────────┴──────────────────┘";
    let row = |label: &str, val: &str| {
        println!("│ {:<23} │ {:>16} │", label, val);
    };

    println!();
    println!("{top}");
    println!("│ {:^42} │", "sqz compression stats");
    println!("{bar}");

    // Cumulative compression stats
    let cs = engine.session_store().compression_stats().unwrap_or_default();
    row("Total compressions", &format!("{}", cs.total_compressions));
    row("Tokens in (total)", &format!("{}", cs.total_tokens_in));
    row("Tokens out (total)", &format!("{}", cs.total_tokens_out));
    row("Tokens saved", &format!("{}", cs.tokens_saved()));
    row("Avg reduction", &format!("{:.1}%", cs.reduction_pct()));

    // Session cost section (if session_id provided)
    if let Some(ref sid) = session_id {
        match engine.cost_summary(sid) {
            Ok(cost) => {
                println!("{bar}");
                row("Session", sid);
                row("Total tokens", &format!("{}", cost.total_tokens));
                row("Total cost", &format!("${:.6}", cost.total_usd));
                row("Cache savings", &format!("${:.6}", cost.cache_savings_usd));
                row("Compression savings", &format!("${:.6}", cost.compression_savings_usd));
                if cost.total_usd > 0.0 {
                    let pct = (cost.compression_savings_usd / (cost.total_usd + cost.compression_savings_usd)) * 100.0;
                    row("Effective reduction", &format!("{:.1}%", pct));
                }
            }
            Err(e) => {
                println!("{bar}");
                row("Session", sid);
                row("Error", &format!("{e}"));
            }
        }
    }

    // Cache stats
    let cache_entries = engine.session_store()
        .list_cache_entries_lru()
        .unwrap_or_default();
    let cache_size: u64 = cache_entries.iter().map(|(_, sz)| sz).sum();
    println!("{bar}");
    row("Cache entries", &format!("{}", cache_entries.len()));
    row("Cache size", &format_bytes(cache_size));

    println!("{bot}");
    println!();
}

/// `sqz gain [--days N]` — show accumulated token savings over time.
fn cmd_gain(days: u32) {
    let engine = require_engine();
    let gains = engine.session_store().daily_gains(days).unwrap_or_default();
    let stats = engine.session_store().compression_stats().unwrap_or_default();

    if gains.is_empty() {
        println!("[sqz] No compression data yet. Run `sqz compress` to start tracking.");
        return;
    }

    let max_saved = gains.iter().map(|g| g.tokens_saved).max().unwrap_or(1).max(1);
    let bar_width: u64 = 30;

    println!();
    println!("  sqz token savings (last {} days)", days);
    println!("  {}", "─".repeat(50));

    for g in &gains {
        let bar_len = (g.tokens_saved * bar_width / max_saved) as usize;
        let bar: String = "█".repeat(bar_len);
        let pad: String = " ".repeat(bar_width as usize - bar_len);
        println!(
            "  {} │{}{}│ {} saved",
            &g.date[5..], // MM-DD
            bar,
            pad,
            g.tokens_saved,
        );
    }

    println!("  {}", "─".repeat(50));
    println!(
        "  Total: {} compressions, {} tokens saved ({:.1}% avg reduction)",
        stats.total_compressions,
        stats.tokens_saved(),
        stats.reduction_pct(),
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
            format!("{}...", &turn.content[..200])
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
