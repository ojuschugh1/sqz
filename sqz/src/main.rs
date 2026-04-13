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
    Init,

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

    /// Remove sqz shell hooks from the RC file.
    Uninstall,

    /// Show a full compression stats report for a session.
    Stats {
        /// Session ID. If omitted, shows aggregate stats for the default agent.
        session_id: Option<String>,
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

        Some(Command::Init) => cmd_init(),
        Some(Command::Compress { text, mode, verify }) => cmd_compress(text, &mode, verify),
        Some(Command::Export { session_id }) => cmd_export(&session_id),
        Some(Command::Import { file }) => cmd_import(&file),
        Some(Command::Status) => cmd_status(),
        Some(Command::Cost { session_id }) => cmd_cost(&session_id),
        Some(Command::Analyze { file, high, low }) => cmd_analyze(file, high, low),
        Some(Command::Tee { action }) => cmd_tee(action),
        Some(Command::Dashboard { port }) => cmd_dashboard(port),
        Some(Command::Proxy { port }) => cmd_proxy(port),
        Some(Command::Uninstall) => cmd_uninstall(),
        Some(Command::Stats { session_id }) => cmd_stats(session_id),
    }
}

// ── Command implementations ───────────────────────────────────────────────

/// `sqz init` — detect shell, install hook, create default preset.
fn cmd_init() {
    let hook = ShellHook::detect();
    println!("[sqz] detected shell: {:?}", hook);

    match hook.install() {
        Ok(true) => println!("[sqz] hook installed to {}", hook.rc_path().display()),
        Ok(false) => println!("[sqz] hook already present in {}", hook.rc_path().display()),
        Err(e) => {
            // Requirement 1.5: log error and continue without aborting.
            eprintln!("[sqz] warning: {e}");
            eprintln!("[sqz] shell hook installation failed; output will pass uncompressed.");
        }
    }

    // Install shell completions
    install_completions(&hook);

    // Create default preset directory and file.
    let preset_dir = default_preset_dir();
    if let Err(e) = std::fs::create_dir_all(&preset_dir) {
        eprintln!("[sqz] warning: could not create preset dir {}: {e}", preset_dir.display());
    } else {
        let preset_path = preset_dir.join("default.toml");
        if !preset_path.exists() {
            match std::fs::write(&preset_path, DEFAULT_PRESET_TOML) {
                Ok(()) => println!("[sqz] default preset written to {}", preset_path.display()),
                Err(e) => eprintln!("[sqz] warning: could not write preset: {e}"),
            }
        } else {
            println!("[sqz] default preset already exists at {}", preset_path.display());
        }
    }

    println!("[sqz] init complete. Restart your shell or source the RC file.");
}

/// `sqz compress [text] [--mode safe|default|aggressive|auto] [--verify]`
fn cmd_compress(text: Option<String>, mode: &str, show_verify: bool) {
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

/// `sqz proxy [--port N]` — transparent HTTP proxy (coming in a future release).
fn cmd_proxy(port: u16) {
    eprintln!(
        "[sqz] proxy mode is not yet available in this release.\n\
         \n\
         The API proxy will sit between your application and OpenAI/Anthropic/Google AI,\n\
         compressing every request transparently — no code changes required.\n\
         \n\
         Planned for v0.2.0. Track progress at:\n\
         https://github.com/ojuschugh1/sqz/issues\n\
         \n\
         (requested port: {port})"
    );
    std::process::exit(1);
}

/// `sqz uninstall` — remove sqz shell hooks from the RC file.
fn cmd_uninstall() {
    let hook = ShellHook::detect();
    println!("[sqz] detected shell: {:?}", hook);

    match hook.uninstall() {
        Ok(true) => println!("[sqz] hook removed from {}", hook.rc_path().display()),
        Ok(false) => println!("[sqz] hook not found in {} — nothing to remove", hook.rc_path().display()),
        Err(e) => {
            eprintln!("[sqz] warning: {e}");
            eprintln!("[sqz] could not remove hook; you may need to edit {} manually", hook.rc_path().display());
        }
    }
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

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
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
