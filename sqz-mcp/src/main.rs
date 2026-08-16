// sqz-mcp binary entry point
// Compiled Rust binary (not Node.js) — Requirement 2.4

use std::path::PathBuf;
use sqz_mcp::{McpServer, McpTransport};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // `sqz-mcp proxy [--no-desc] [--no-cache] -- <upstream command...>`
    // wraps another MCP server and compresses its responses.
    if args.get(1).map(|a| a.as_str()) == Some("proxy") {
        let mut compress_descriptions = true;
        let mut no_cache = false;
        let mut lazy_tools = false;
        let mut upstream: Vec<String> = Vec::new();
        let mut past_separator = false;
        for arg in &args[2..] {
            if past_separator {
                upstream.push(arg.clone());
                continue;
            }
            match arg.as_str() {
                "--" => past_separator = true,
                "--no-desc" => compress_descriptions = false,
                "--no-cache" => no_cache = true,
                "--lazy-tools" => lazy_tools = true,
                "--help" | "-h" => {
                    eprintln!("Usage: sqz-mcp proxy [--no-desc] [--no-cache] [--lazy-tools] -- <upstream command...>");
                    std::process::exit(0);
                }
                other => upstream.push(other.to_string()),
            }
        }
        if let Err(e) = sqz_mcp::proxy::run_proxy(&upstream, compress_descriptions, no_cache, lazy_tools) {
            eprintln!("[sqz-mcp] proxy error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Parse --transport and --port flags.
    let mut transport = McpTransport::Stdio;
    let mut preset_dir = PathBuf::from(".");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--transport" | "-t" => {
                i += 1;
                if i < args.len() {
                    match args[i].as_str() {
                        "sse" => {
                            // Default SSE port; may be overridden by --port.
                            transport = McpTransport::Sse { port: 3000 };
                        }
                        "stdio" | _ => {
                            transport = McpTransport::Stdio;
                        }
                    }
                }
            }
            "--port" | "-p" => {
                i += 1;
                if i < args.len() {
                    let port: u16 = args[i].parse().unwrap_or(3000);
                    transport = McpTransport::Sse { port };
                }
            }
            "--preset-dir" | "-d" => {
                i += 1;
                if i < args.len() {
                    preset_dir = PathBuf::from(&args[i]);
                }
            }
            "--help" | "-h" => {
                eprintln!("Usage: sqz-mcp [--transport stdio|sse] [--port PORT] [--preset-dir DIR]");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    let server = McpServer::new(&preset_dir).unwrap_or_else(|e| {
        eprintln!("[sqz-mcp] failed to initialize server: {e}");
        std::process::exit(1);
    });

    // Start preset hot-reload watcher (keep handle alive for the process lifetime).
    // If the watcher fails (e.g. invalid/unwatchable preset dir), degrade gracefully —
    // the server still works, just without hot-reload.
    let _watcher = match server.watch_presets() {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("[sqz-mcp] warning: preset watcher failed to start: {e}");
            eprintln!("[sqz-mcp] continuing without hot-reload support");
            None
        }
    };

    if let Err(e) = server.start(transport) {
        eprintln!("[sqz-mcp] server error: {e}");
        std::process::exit(1);
    }
}
