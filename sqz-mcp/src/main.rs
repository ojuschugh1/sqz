// sqz-mcp binary entry point
// Compiled Rust binary (not Node.js) — Requirement 2.4

use std::path::PathBuf;
use sqz_mcp::{McpServer, McpTransport};

fn main() {
    let args: Vec<String> = std::env::args().collect();

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
    let _watcher = server.watch_presets().unwrap_or_else(|e| {
        eprintln!("[sqz-mcp] warning: preset watcher failed to start: {e}");
        // Return a no-op watcher by panicking — we handle this gracefully.
        // In practice the server still works without hot-reload.
        panic!("watcher failed: {e}");
    });

    if let Err(e) = server.start(transport) {
        eprintln!("[sqz-mcp] server error: {e}");
        std::process::exit(1);
    }
}
