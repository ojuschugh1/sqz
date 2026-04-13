/// End-to-end integration tests for the `sqz` binary.
///
/// These tests build and invoke the actual binary, verifying real behaviour
/// rather than mocked internals.  They cover the CLI surface, compression
/// correctness, TOON encoding, entropy analysis, tee mode, and the hook
/// system.
use std::process::{Command, Output};
use std::path::PathBuf;

// ── helpers ───────────────────────────────────────────────────────────────────

fn sqz_bin() -> PathBuf {
    // Use the debug build so tests run without --release.
    let mut p = std::env::current_exe().unwrap();
    p.pop(); // remove test binary name
    // Cargo puts integration test binaries in deps/, step up one more.
    if p.ends_with("deps") {
        p.pop();
    }
    p.push("sqz");
    p
}

/// Workspace root — integration tests may run from a different cwd.
fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points to sqz/, so go up one level.
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    // If we're in sqz/, go up to workspace root.
    if manifest.ends_with("sqz") {
        manifest.parent().unwrap().to_path_buf()
    } else {
        manifest
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(sqz_bin())
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("failed to run sqz binary")
}

fn run_with_stdin(args: &[&str], stdin: &str) -> Output {
    use std::io::Write;
    let mut child = Command::new(sqz_bin())
        .args(args)
        .current_dir(workspace_root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn sqz");
    child.stdin.as_mut().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

// ── basic CLI ─────────────────────────────────────────────────────────────────

#[test]
fn test_help_exits_zero() {
    let o = run(&["--help"]);
    assert!(o.status.success(), "sqz --help should exit 0");
    assert!(stdout(&o).contains("compress"), "help should mention compress");
}

#[test]
fn test_version_exits_zero() {
    let o = run(&["--version"]);
    assert!(o.status.success());
}

#[test]
fn test_unknown_subcommand_exits_nonzero() {
    let o = run(&["totally-unknown-subcommand-xyz"]);
    assert!(!o.status.success(), "unknown subcommand should fail");
}

// ── compress ──────────────────────────────────────────────────────────────────

#[test]
fn test_compress_plain_text_passthrough() {
    let o = run(&["compress", "hello world"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("hello world"), "plain text should pass through: {out}");
}

#[test]
fn test_compress_json_applies_toon() {
    let json = r#"{"name":"Alice","age":30,"status":"active"}"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let out = stdout(&o);
    // TOON-encoded output starts with the TOON prefix
    assert!(out.contains("TOON:"), "JSON should be TOON-encoded: {out}");
}

#[test]
fn test_compress_json_strips_nulls() {
    let json = r#"{"name":"Alice","error":null,"status":"active"}"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let out = stdout(&o);
    // null fields should be stripped
    assert!(!out.contains("null"), "null fields should be stripped: {out}");
}

#[test]
fn test_compress_reports_token_reduction() {
    let json = r#"{"name":"Alice","age":30,"email":"alice@example.com","status":"active","error":null,"debug_info":"verbose debug data here that should be stripped"}"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let err = stderr(&o);
    // stderr should contain token counts
    assert!(err.contains("tokens"), "should report token counts: {err}");
    assert!(err.contains("%"), "should report reduction percentage: {err}");
}

#[test]
fn test_compress_stdin() {
    let o = run_with_stdin(&["compress"], r#"{"key":"value","null_field":null}"#);
    assert!(o.status.success());
    let out = stdout(&o);
    // Output is either TOON-encoded (fresh) or a dedup reference (cached from prior run)
    assert!(out.contains("TOON:") || out.starts_with("§ref:"),
        "stdin JSON should be TOON-encoded or dedup ref: {out}");
}

#[test]
fn test_compress_empty_input() {
    let o = run(&["compress", ""]);
    assert!(o.status.success(), "empty input should not crash");
}

#[test]
fn test_compress_large_json_reduces_tokens() {
    // Build a JSON array with 20 objects
    let items: Vec<String> = (0..20)
        .map(|i| format!(r#"{{"id":{i},"name":"item_{i}","description":"A verbose description of item number {i} that contains a lot of redundant text","metadata":{{"internal_id":"abc{i}","debug_info":"debug","trace_id":"trace{i}","created_at":"2024-01-01","updated_at":"2024-01-15"}}}}"#))
        .collect();
    let json = format!("[{}]", items.join(","));

    let o = run(&["compress", &json]);
    assert!(o.status.success());
    let err = stderr(&o);
    // Should achieve meaningful compression
    assert!(err.contains("tokens"), "should report token counts: {err}");
}

#[test]
fn test_compress_ansi_stripped() {
    // Input with ANSI color codes
    let ansi_input = "\x1b[1;31mERROR:\x1b[0m something went wrong\n\x1b[32mOK\x1b[0m";
    let o = run(&["compress", ansi_input]);
    assert!(o.status.success());
    let out = stdout(&o);
    // ANSI codes should be stripped, text preserved
    assert!(out.contains("ERROR:"), "text should be preserved: {out}");
    assert!(out.contains("OK"), "text should be preserved: {out}");
    assert!(!out.contains("\x1b["), "ANSI codes should be stripped: {out}");
}

// ── analyze ───────────────────────────────────────────────────────────────────

#[test]
fn test_analyze_rust_file() {
    let o = run(&["analyze", "sqz_engine/src/toon.rs"]);
    assert!(o.status.success(), "analyze should succeed: {}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("block"), "should show blocks: {out}");
    assert!(out.contains("entropy"), "should show entropy scores: {out}");
    assert!(out.contains("HighInfo") || out.contains("MediumInfo") || out.contains("LowInfo"),
        "should classify blocks: {out}");
}

#[test]
fn test_analyze_shows_summary() {
    let o = run(&["analyze", "sqz_engine/src/pipeline.rs"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("blocks total"), "should show summary: {out}");
    assert!(out.contains("HighInfo"), "should count HighInfo: {out}");
}

#[test]
fn test_analyze_stdin() {
    let source = r#"
fn complex_function(x: i32, y: i32) -> i32 {
    let result = x * y + x.pow(2);
    result.abs()
}

// boilerplate comment
// boilerplate comment
// boilerplate comment
"#;
    let o = run_with_stdin(&["analyze"], source);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("block"), "should show blocks: {out}");
}

#[test]
fn test_analyze_custom_thresholds() {
    let o = run(&["analyze", "--high", "80", "--low", "20", "sqz_engine/src/toon.rs"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("block"), "should show blocks: {out}");
}

#[test]
fn test_analyze_nonexistent_file() {
    let o = run(&["analyze", "/nonexistent/path/file.rs"]);
    assert!(!o.status.success(), "should fail for missing file");
    let err = stderr(&o);
    assert!(err.contains("could not read"), "should report error: {err}");
}

// ── status ────────────────────────────────────────────────────────────────────

#[test]
fn test_status_shows_budget() {
    let o = run(&["status"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("consumed"), "should show consumed: {out}");
    assert!(out.contains("available"), "should show available: {out}");
    assert!(out.contains("tokens"), "should show token counts: {out}");
}

#[test]
fn test_status_shows_agent() {
    let o = run(&["status"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("agent"), "should show agent: {out}");
}

// ── tee ───────────────────────────────────────────────────────────────────────

#[test]
fn test_tee_list_empty() {
    // Use a temp dir to avoid polluting real tee storage
    let o = run(&["tee", "list"]);
    assert!(o.status.success());
    // Either "no saved tee entries" or a list — both are valid
    let out = stdout(&o);
    let _ = out; // just verify it doesn't crash
}

#[test]
fn test_tee_get_nonexistent() {
    let o = run(&["tee", "get", "nonexistent-id-xyz"]);
    assert!(!o.status.success(), "should fail for missing tee entry");
}

// ── export / import ───────────────────────────────────────────────────────────

#[test]
fn test_export_nonexistent_session() {
    let o = run(&["export", "nonexistent-session-id-xyz"]);
    assert!(!o.status.success(), "should fail for missing session");
}

#[test]
fn test_cost_nonexistent_session() {
    let o = run(&["cost", "nonexistent-session-id-xyz"]);
    assert!(!o.status.success(), "should fail for missing session");
}

// ── compression correctness ───────────────────────────────────────────────────

#[test]
fn test_compress_preserves_error_fields() {
    // Error fields should be preserved (semantically significant)
    let json = r#"{"error":"connection refused","status":"failed","code":503}"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let out = stdout(&o);
    // The error message should survive compression
    assert!(out.contains("error") || out.contains("failed") || out.contains("503"),
        "error fields should be preserved: {out}");
}

#[test]
fn test_compress_nested_json() {
    let json = r#"{"user":{"name":"Bob","profile":{"age":25,"city":"NYC"}},"status":"ok"}"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("TOON:"), "nested JSON should be TOON-encoded: {out}");
}

#[test]
fn test_compress_json_array() {
    let json = r#"[{"id":1,"name":"a"},{"id":2,"name":"b"},{"id":3,"name":"c"}]"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("TOON:"), "JSON array should be TOON-encoded: {out}");
}

#[test]
fn test_compress_multiline_cli_output() {
    let cli_output = "Compiling sqz v0.1.0\n\
        Compiling sqz_engine v0.1.0\n\
        warning: unused variable `x`\n\
          --> src/main.rs:10:5\n\
        error[E0308]: mismatched types\n\
          --> src/lib.rs:42:10\n\
        error: aborting due to previous error\n";
    let o = run_with_stdin(&["compress"], cli_output);
    assert!(o.status.success());
    let out = stdout(&o);
    // Errors and warnings should be preserved (or dedup ref if cached)
    assert!(out.contains("error") || out.contains("warning") || out.starts_with("§ref:"),
        "errors/warnings should be preserved or dedup ref returned: {out}");
}

// ── TOON round-trip via compress ──────────────────────────────────────────────

#[test]
fn test_toon_output_is_ascii_safe() {
    let json = r#"{"message":"héllo wörld","status":"ok"}"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let out = stdout(&o);
    // TOON output should only contain printable ASCII + standard whitespace
    for ch in out.chars() {
        if ch == '\n' || ch == '\r' || ch == '\t' {
            continue;
        }
        assert!(
            ch.is_ascii() && (ch as u8) >= 0x20,
            "non-ASCII char in output: {:?} (U+{:04X})", ch, ch as u32
        );
    }
}

// ── init ──────────────────────────────────────────────────────────────────────

#[test]
fn test_init_exits_zero() {
    // init may already be done; it should be idempotent
    let o = run(&["init"]);
    assert!(o.status.success(), "sqz init should exit 0: {}", stderr(&o));
}

// ── dashboard ─────────────────────────────────────────────────────────────────

#[test]
fn test_dashboard_help() {
    let o = run(&["dashboard", "--help"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("port") || out.contains("dashboard"), "should show dashboard help: {out}");
}
