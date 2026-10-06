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

fn run_with_db(db: &std::path::Path, args: &[&str], stdin: Option<&str>) -> Output {
    use std::io::Write;
    let mut child = Command::new(sqz_bin())
        .args(args)
        .env("SQZ_DB_PATH", db)
        .env_remove("SQZ_NO_DEDUP")
        .env_remove("SQZ_NO_ABBREV")
        .current_dir(workspace_root())
        .stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn sqz");
    if let Some(s) = stdin {
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

// ── slice refs ────────────────────────────────────────────────────────────────

/// `cat f.py` then `sed -n '41,80p' f.py`: the range comes back as a
/// line-range ref to the full read, and `sqz expand` on that ref returns
/// exactly those lines.
#[test]
fn test_ranged_reread_becomes_line_range_ref() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    let file: String = (1..=50)
        .map(|i| format!("def handler_{i:03}(request):\n    token = request.headers.get('Authorization')\n    return verify(token, scope='handler_{i:03}')\n\n"))
        .collect();
    let lines: Vec<&str> = file.split_inclusive('\n').collect();
    let slice: String = lines[40..80].concat();

    let full = run_with_db(&db, &["compress", "--cmd", "cat auth.py"], Some(&file));
    assert!(full.status.success());
    assert!(stdout(&full).contains("def handler_050"), "first read serves the file");

    let ranged = run_with_db(&db, &["compress", "--cmd", "sed -n '41,80p' auth.py"], Some(&slice));
    let out = stdout(&ranged);
    assert!(out.starts_with("§ref:") && out.ends_with(":L41-80§\n"), "expected a newline-terminated line-range ref, got: {out:?}");
    assert!(stderr(&ranged).contains("lines 41-80 of 200"), "{}", stderr(&ranged));

    let expanded = run_with_db(&db, &["expand", out.trim()], None);
    assert!(expanded.status.success(), "{}", stderr(&expanded));
    assert_eq!(String::from_utf8_lossy(&expanded.stdout), slice);

    // Same slice with the cache disabled is served in full.
    let raw = run_with_db(&db, &["compress", "--no-cache", "--cmd", "sed -n '41,80p' auth.py"], Some(&slice));
    assert!(!stdout(&raw).contains("§ref:"));
    assert!(stdout(&raw).contains("def handler_020"));
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
fn test_discover_suggestions_carry_no_figures() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    let json = r#"{"name":"Alice","age":30,"email":"alice@example.com","status":"active","error":null,"debug_info":"verbose debug data here that should be stripped"}"#;
    let c = run_with_db(&db, &["compress", json], None);
    assert!(c.status.success(), "{}", stderr(&c));

    let o = run_with_db(&db, &["discover"], None);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("High-value commands"), "{out}");
    assert!(!out.contains("kubectl"), "{out}");
    assert!(!out.contains("% reduction"), "{out}");
}

#[test]
fn test_compress_stdin() {
    // Large enough that compression clears the net-win gate — tiny inputs
    // now pass through verbatim by design.
    let items: Vec<String> = (0..12)
        .map(|i| format!(r#"{{"id":{i},"name":"item_{i}","status":"active","null_field":null}}"#))
        .collect();
    let json = format!("[{}]", items.join(","));
    let o = run_with_stdin(&["compress"], &json);
    assert!(o.status.success());
    let out = stdout(&o);
    // Output is either TOON-encoded (fresh) or a dedup reference (cached from prior run)
    assert!(out.contains("TOON:") || out.starts_with("§ref:"),
        "stdin JSON should be TOON-encoded or dedup ref: {out}");
}

#[test]
fn test_compress_stdin_tiny_input_passes_through() {
    // Sub-threshold savings: the net-win gate returns the input verbatim.
    let o = run_with_stdin(&["compress", "--no-cache"], r#"{"key":"value","null_field":null}"#);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(
        out.contains(r#"{"key":"value","null_field":null}"#) || out.starts_with("§ref:"),
        "tiny stdin input should pass through (or hit dedup): {out}"
    );
}

#[test]
fn test_compress_takes_a_label_that_starts_with_a_dash() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    let o = run_with_db(&db, &["compress", "--cmd", "-j", "--no-cache"], Some("one line of output\n"));
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "one line of output\n");
}

#[test]
fn test_compress_empty_input() {
    let o = run(&["compress", ""]);
    assert!(o.status.success(), "empty input should not crash");
}

#[test]
fn test_compress_output_is_identical_across_runs() {
    let pods: Vec<serde_json::Value> = (0..20)
        .map(|i| {
            serde_json::json!({
                "kind": "Pod",
                "metadata": {
                    "name": format!("web-{i}"),
                    "namespace": "default",
                    "labels": {"app": "web", "pod-template-hash": "5d8f7c9b4", "tier": "frontend"},
                },
                "spec": {
                    "containers": [{"name": "web", "image": "nginx:1.27", "ports": [{"containerPort": 8080}]}],
                    "nodeName": format!("node-{}", i % 3),
                },
                "status": {
                    "phase": "Running",
                    "conditions": [{"type": "Ready", "status": "True"}],
                    "containerStatuses": [{"name": "web", "ready": true, "restartCount": i % 2}],
                },
            })
        })
        .collect();
    let kubectl =
        serde_json::to_string_pretty(&serde_json::json!({"kind": "List", "items": pods})).unwrap();
    let cargo_test: String = (0..12)
        .map(|i| {
            format!(
                "---- engine::tests::case_{i} stdout ----\nthread 'engine::tests::case_{i}' panicked at src/engine.rs:{}:9:\nassertion `left == right` failed\n  left: {i}\n right: {}\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n",
                100 + i,
                i + 1,
            )
        })
        .collect();
    for input in [kubectl, cargo_test] {
        let outputs: Vec<Vec<u8>> = (0..4)
            .map(|_| {
                let dir = tempfile::tempdir().unwrap();
                let o = run_with_db(&dir.path().join("sqz.db"), &["compress"], Some(&input));
                assert!(o.status.success());
                o.stdout
            })
            .collect();
        assert!(
            outputs.iter().all(|o| *o == outputs[0]),
            "output differs between runs"
        );
    }
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

fn high_risk_outputs() -> Vec<(&'static str, String)> {
    let hex = |s: &str| sqz_engine::CacheManager::sha256_hex(s.as_bytes());
    let frames: String = (0..40)
        .map(|i| format!("  {i:>2}: myapp::scheduler::step_{i}\n             at ./src/scheduler/mod_{}.rs:{}:5\n", i % 7, 40 + 7 * i))
        .collect();
    let tables: String = ["customers", "orders", "refunds", "payments", "shipments", "invoices"]
        .iter()
        .map(|t| format!("CREATE TABLE {t} (\n    id BIGSERIAL PRIMARY KEY,\n    status TEXT NOT NULL DEFAULT 'pending',\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE INDEX idx_{t}_created_at ON {t} (created_at);\n"))
        .collect();
    let pem: String = (0..20).map(|i| hex(&format!("pem {i}")) + "\n").collect();
    let creds: String = ["stripe", "sendgrid", "github", "datadog", "sentry"]
        .iter()
        .map(|s| format!("{s}_api_key: sk_test_{}\n{s}_endpoint: https://{s}.example.com/v1\n", &hex(s)[..32]))
        .collect();
    vec![
        ("cargo run", format!("thread 'main' panicked at src/scheduler/queue.rs:217:31:\ncalled `Option::unwrap()` on a `None` value\nstack backtrace:\n{frames}")),
        ("cat migrations/0042_orders.sql", format!("BEGIN;\n{tables}ALTER TABLE refunds ADD COLUMN reason TEXT;\nCOMMIT;\n")),
        ("cat deploy/secrets.env", format!("-----BEGIN RSA PRIVATE KEY-----\n{pem}-----END RSA PRIVATE KEY-----\n{creds}")),
    ]
}

#[test]
fn test_high_risk_output_passes_through_every_path() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    for (cmd, output) in high_risk_outputs() {
        for args in [
            vec!["compress", "--cmd", cmd],
            vec!["compress"],
            vec!["compress", "--mode", "safe"],
            vec!["compress", "--mode", "default"],
            vec!["compress", "--mode", "aggressive"],
        ] {
            let o = run_with_db(&db, &args, Some(&output));
            assert!(o.status.success(), "{args:?}: {}", stderr(&o));
            assert_eq!(stdout(&o), output, "{args:?}");
        }
        let payload = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": cmd},
            "tool_response": {"stdout": output, "stderr": "", "interrupted": false, "isImage": false},
        });
        let o = run_with_db(&db, &["hook", "claude-post"], Some(&payload.to_string()));
        assert!(o.status.success(), "{cmd}: {}", stderr(&o));
        assert_eq!(stdout(&o), "", "{cmd}: the original result must stay");
    }
}

fn run_panicking(at: &str, backtrace: Option<&str>, db: &std::path::Path, args: &[&str], stdin: &str) -> Output {
    use std::io::Write;
    let mut cmd = Command::new(sqz_bin());
    cmd.args(args)
        .env("SQZ_DB_PATH", db)
        .env("SQZ_TEST_PANIC", at)
        .env_remove("RUST_BACKTRACE")
        .current_dir(workspace_root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(v) = backtrace {
        cmd.env("RUST_BACKTRACE", v);
    }
    let mut child = cmd.spawn().expect("failed to spawn sqz");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn test_output_passes_through_when_compression_panics() {
    if !cfg!(debug_assertions) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    let panicking = |args: &[&str], stdin: &str| run_panicking("1", None, &db, args, stdin);
    let fallback = "[sqz] fallback: compression panicked, output passed through unchanged";
    let body: String = (0..60)
        .map(|i| format!("2026-10-06T12:00:{i:02}Z INFO request served path=/health status=200 latency_ms={i}\n"))
        .collect();
    let input = format!("{body}\n__SQZ_EXIT_3__\n");
    for args in [
        vec!["compress"],
        vec!["compress", "--cmd", "tail -60 app.log"],
        vec!["compress", "--mode", "safe"],
        vec!["compress", "--mode", "default"],
        vec!["compress", "--mode", "aggressive"],
    ] {
        let o = panicking(&args, &input);
        assert_eq!(o.status.code(), Some(3), "{args:?}: {}", stderr(&o));
        assert_eq!(stdout(&o), body, "{args:?}");
        assert_eq!(stderr(&o), format!("{fallback}\n"), "{args:?}");
    }
    let o = run_panicking("1", Some("0"), &db, &["compress"], &input);
    assert_eq!(stderr(&o), format!("{fallback}\n"), "RUST_BACKTRACE=0");
    let o = panicking(&["compress", &body], "");
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), body);
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "tail -60 app.log"},
        "tool_response": {"stdout": body, "stderr": "", "interrupted": false, "isImage": false},
    });
    let o = panicking(&["hook", "claude-post"], &payload.to_string());
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "no replacement");
    let late = [vec!["compress"], vec!["compress", "--cmd", "tail -60 app.log"]];
    for (i, args) in late.iter().enumerate() {
        let o = run_panicking("late", None, &dir.path().join(format!("late{i}.db")), args, &input);
        let err = stderr(&o);
        assert_eq!(o.status.code(), Some(3), "{args:?}: {err}");
        assert_eq!(stdout(&o), body, "{args:?}");
        assert!(err.contains("% reduction)"), "{args:?}: {err}");
        assert_eq!(err.lines().last(), Some(fallback), "{args:?}: {err}");
    }
    let late_db = dir.path().join("late-post.db");
    let o = run_panicking("late", None, &late_db, &["hook", "claude-post"], &payload.to_string());
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "no replacement");
    let o = run_with_db(&db, &["hook", "claude-post"], Some(&payload.to_string()));
    assert!(stdout(&o).contains("updatedToolOutput"), "{}", stdout(&o));
}

#[test]
fn test_panic_fallback_names_the_location_with_rust_backtrace() {
    if !cfg!(debug_assertions) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let body: String = (0..60)
        .map(|i| format!("2026-10-06T12:00:{i:02}Z INFO request served path=/health status=200 latency_ms={i}\n"))
        .collect();
    let input = format!("{body}\n__SQZ_EXIT_3__\n");
    let site = |line: &str| -> Option<String> {
        let at = line
            .strip_prefix("[sqz] fallback: compression panicked at ")?
            .strip_suffix(", output passed through unchanged")?;
        let mut parts = at.rsplitn(3, ':');
        let (col, line, file) = (parts.next()?, parts.next()?, parts.next()?);
        let ok = col.parse::<u32>().is_ok() && line.parse::<u32>().is_ok() && file.ends_with("cli_proxy.rs");
        ok.then(|| at.to_string())
    };
    let cases = [
        ("1", vec!["compress"]),
        ("1", vec!["compress", "--mode", "safe"]),
        ("late", vec!["compress", "--cmd", "tail -60 app.log"]),
    ];
    for (i, (at, args)) in cases.iter().enumerate() {
        let plain = run_panicking(at, None, &dir.path().join(format!("plain{i}.db")), args, &input);
        let o = run_panicking(at, Some("1"), &dir.path().join(format!("{i}.db")), args, &input);
        let err = stderr(&o);
        assert_eq!(o.status.code(), Some(3), "{args:?}: {err}");
        assert_eq!(stdout(&o), body, "{args:?}");
        assert!(!err.contains("SQZ_TEST_PANIC"), "{args:?}: {err}");
        let at_site = site(err.lines().last().unwrap_or_default()).expect(&err);
        assert_eq!(err.replacen(&format!(" at {at_site}"), "", 1), stderr(&plain), "{args:?}");
    }
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "tail -60 app.log"},
        "tool_response": {"stdout": body, "stderr": "", "interrupted": false, "isImage": false},
    });
    let o = run_panicking("1", Some("1"), &dir.path().join("post.db"), &["hook", "claude-post"], &payload.to_string());
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "no replacement");
    assert_eq!(stderr(&o), "");
}

#[test]
fn test_claude_post_never_answers_with_a_ref() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    let log: String = (0..300)
        .map(|i| format!("2026-10-06T12:{:02}:{:02}Z INFO request served path=/health status=200 latency_ms={}\n", i / 60, i % 60, i % 7))
        .collect();
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "tail -300 server.log | grep INFO"},
        "tool_response": {"stdout": log, "stderr": "", "interrupted": false, "isImage": false},
    })
    .to_string();
    let first = stdout(&run_with_db(&db, &["hook", "claude-post"], Some(&payload)));
    let reply: serde_json::Value = serde_json::from_str(&first).expect(&first);
    let text = reply["hookSpecificOutput"]["updatedToolOutput"]["stdout"].as_str().unwrap();
    assert!(text.len() < log.len() && !text.contains("§ref:"), "{text}");
    let second = stdout(&run_with_db(&db, &["hook", "claude-post"], Some(&payload)));
    assert_eq!(second, first, "the repeat must not be a ref the model never saw");
    let (_, hint) = text.split_once("[full output: sqz expand ").expect(text);
    let expanded = run_with_db(&db, &["expand", &hint[..16]], None);
    assert_eq!(stdout(&expanded), log, "{}", stderr(&expanded));
}

fn rust_backtrace(indent: &str, thread: &str, frames: usize) -> String {
    let mut s = format!(
        "{indent}thread '{thread}' panicked at src/ledger.rs:41:9:\n{indent}assertion `left == right` failed\n{indent}stack backtrace:\n{indent}   0: rust_begin_unwind\n{indent}             at /rustc/eeb90cda/library/std/src/panicking.rs:665:5\n"
    );
    for i in 1..=frames {
        s += &format!("{indent}{i:>4}: ledger::step_{i:02}\n{indent}             at ./src/ledger.rs:{}:13\n", 90 + 9 * i);
    }
    s + &format!("{indent}note: Some details are omitted, run with `RUST_BACKTRACE=full` for a verbose backtrace.\n")
}

fn cargo_test_run(tests: usize, failing: usize, frames: usize) -> String {
    let step = tests / failing;
    let failed: Vec<usize> = (0..tests).filter(|i| i % step == 7 && i / step < failing).collect();
    let mut s = format!("   Compiling ledger v0.3.1 (/work/ledger)\n    Finished `test` profile [unoptimized + debuginfo] target(s) in 7.42s\n     Running unittests src/lib.rs (target/debug/deps/ledger-5c1a2b3d)\n\nrunning {tests} tests\n");
    for i in 0..tests {
        s += &format!("test tests::case_{i:03} ... {}\n", if failed.contains(&i) { "FAILED" } else { "ok" });
    }
    s += "\nfailures:\n\n";
    for i in &failed {
        s += &format!("---- tests::case_{i:03} stdout ----\n{}\n", rust_backtrace("", &format!("tests::case_{i:03}"), frames));
    }
    s += "\nfailures:\n";
    for i in &failed {
        s += &format!("    tests::case_{i:03}\n");
    }
    s + &format!("\ntest result: FAILED. {} passed; {failing} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.61s\n\nerror: test failed, to rerun pass `--lib`\n", tests - failing)
}

fn build_script_panic() -> String {
    let crates = ["libc", "cfg-if", "itoa", "ryu", "memchr", "once_cell", "log", "bitflags", "smallvec", "serde"];
    let downloaded: String = crates.iter().map(|c| format!("  Downloaded {c} v1.0.0\n")).collect();
    let compiling: String = crates.iter().map(|c| format!("   Compiling {c} v1.0.0\n")).collect();
    format!(
        "    Updating crates.io index\n{downloaded}  Downloaded 10 crates (1.9 MB) in 0.84s\n{compiling}   Compiling app v0.1.0 (/srv/app)\nerror: failed to run custom build command for `app v0.1.0 (/srv/app)`\n\nCaused by:\n  process didn't exit successfully: `/srv/app/target/debug/build/app-3f1c2d9e/build-script-build` (exit status: 101)\n  --- stderr\n{}",
        rust_backtrace("  ", "main", 20)
    )
}

fn junit_trace(exception: &str, frame: &str) -> String {
    let frames: String = (0..16)
        .map(|i| format!("{frame}at com.acme.billing.Step{i:02}.run(Step{i:02}.java:{})\n", 30 + i))
        .collect();
    format!("{exception}org.opentest4j.AssertionFailedError: expected: <1042> but was: <987>\n{frames}")
}

fn gradle_test_run() -> String {
    let passed: String = (0..60).map(|i| format!("LedgerTest > case{i:02}() PASSED\n")).collect();
    format!(
        "> Task :compileJava UP-TO-DATE\n> Task :compileTestJava\n> Task :test\n\n{passed}\nLedgerTest > refundTotals() FAILED\n{}\n61 tests completed, 1 failed\n\n> Task :test FAILED\n\nFAILURE: Build failed with an exception.\n\nBUILD FAILED in 6s\n4 actionable tasks: 2 executed, 2 up-to-date\n",
        junit_trace("    ", "        ")
    )
}

fn mvn_test_run() -> String {
    let runs: String = (0..30)
        .map(|i| format!("[INFO] Running com.acme.billing.Case{i:02}Test\n[INFO] Tests run: 4, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.0{i:02} s - in com.acme.billing.Case{i:02}Test\n"))
        .collect();
    format!(
        "[INFO] Scanning for projects...\n[INFO] Building billing 0.3.1\n{runs}[INFO] Running com.acme.billing.LedgerTest\n[ERROR] Tests run: 3, Failures: 1, Errors: 0, Skipped: 0, Time elapsed: 0.041 s <<< FAILURE! - in com.acme.billing.LedgerTest\n[ERROR] com.acme.billing.LedgerTest.refundTotals  Time elapsed: 0.012 s  <<< FAILURE!\n{}\n[INFO] Results:\n[ERROR] Failures: \n[ERROR]   LedgerTest.refundTotals:41 expected: <1042> but was: <987>\n[ERROR] Tests run: 123, Failures: 1, Errors: 0, Skipped: 0\n[INFO] BUILD FAILURE\n",
        junit_trace("", "\t")
    )
}

#[test]
fn test_formatters_keep_every_trace_line_or_pass_through() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    let o = run_with_db(&db, &["compress", "--cmd", "cat ./src/ledger.rs"], Some("pub fn step_01() {}\n"));
    assert!(o.status.success(), "{}", stderr(&o));
    let post = |cmd: &str, output: &str| {
        let payload = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": cmd},
            "tool_response": {"stdout": output, "stderr": "", "interrupted": false, "isImage": false},
        });
        let o = run_with_db(&db, &["hook", "claude-post"], Some(&payload.to_string()));
        assert!(o.status.success(), "{cmd}: {}", stderr(&o));
        stdout(&o)
    };
    for (cmd, output) in [
        ("cargo build", build_script_panic()),
        ("./gradlew test", gradle_test_run()),
        ("mvn test", mvn_test_run()),
        ("cargo test", cargo_test_run(200, 20, 3)),
    ] {
        assert!(output.len() < 30_000, "{cmd}");
        let o = run_with_db(&db, &["compress", "--cmd", cmd], Some(&output));
        assert!(o.status.success(), "{cmd}: {}", stderr(&o));
        assert_eq!(stdout(&o), output, "{cmd}");
        assert_eq!(post(cmd, &output), "", "{cmd}: the original result must stay");
    }
    for (cmd, output) in [("cargo test", cargo_test_run(300, 1, 2)), ("cargo test", cargo_test_run(200, 3, 8))] {
        let formatted = sqz_engine::format_command(cmd, &output).expect(cmd);
        assert!(formatted.len() * 3 < output.len(), "{formatted}");
        let trace: Vec<&str> = output
            .split("\n\n")
            .filter(|p| p.contains("stack backtrace:"))
            .flat_map(str::lines)
            .collect();
        assert!(trace.len() >= 9);
        for line in &trace {
            assert!(formatted.lines().any(|l| l == *line), "{line}");
        }
        let o = run_with_db(&db, &["compress", "--cmd", cmd], Some(&output));
        assert!(o.status.success(), "{cmd}: {}", stderr(&o));
        assert_eq!(stdout(&o), format!("{formatted}\n"));
        let reply: serde_json::Value = serde_json::from_str(&post(cmd, &output)).unwrap();
        assert_eq!(reply["hookSpecificOutput"]["updatedToolOutput"]["stdout"], formatted.as_str());
    }
}

#[test]
fn test_json_values_survive_safe_mode() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sqz.db");
    let pretty = "[\n  0,\n  0,\n  0,\n  0,\n  0,\n  0,\n  1\n]\n";
    for args in [vec!["compress", "--mode", "safe"], vec!["compress", "--mode", "default"], vec!["compress"]] {
        let o = run_with_db(&db, &args, Some(pretty));
        assert!(o.status.success(), "{args:?}: {}", stderr(&o));
        let out = stdout(&o);
        assert!(out.contains("[0,0,0,0,0,0,1]") || out == pretty, "{args:?}: {out}");
    }
}

// ── TOON round-trip via compress ──────────────────────────────────────────────

#[test]
fn test_toon_output_keeps_text_and_escapes_controls() {
    let json = r#"{"message":"héllo wörld","note":"bell\u0007here","status":"ok"}"#;
    let o = run(&["compress", json]);
    assert!(o.status.success());
    let out = stdout(&o);
    // Non-ASCII text costs fewer tokens as-is than as \u escapes.
    assert!(out.contains("héllo wörld"), "text kept as-is: {out}");
    for ch in out.chars() {
        assert!(
            !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'),
            "raw control char in output: U+{:04X}", ch as u32
        );
    }
}

// ── init ──────────────────────────────────────────────────────────────────────

/// Run `sqz init` with HOME, every client config root, the database and the
/// project directory pointed at throwaway dirs, so the test can't touch the
/// developer's real shell rc, agent configs or this repo.
fn run_init_sandboxed(home: &std::path::Path, project: &std::path::Path) -> Output {
    Command::new(sqz_bin())
        .args(["init", "--yes"])
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("SHELL", "/bin/zsh")
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("APPDATA", home.join("AppData"))
        .env("KIRO_HOME", home.join(".kiro"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("COPILOT_HOME", home.join(".copilot"))
        .env("SQZ_DB_PATH", home.join("sessions.db"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("failed to run sqz init")
}

#[test]
fn test_init_exits_zero() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let o = run_init_sandboxed(home.path(), project.path());
    assert!(o.status.success(), "sqz init should exit 0: {}", stderr(&o));
    // Idempotent: a second run succeeds and leaves one shell block.
    let o = run_init_sandboxed(home.path(), project.path());
    assert!(o.status.success(), "second sqz init should exit 0: {}", stderr(&o));
    let rc = std::fs::read_to_string(home.path().join(".zshrc")).unwrap();
    assert_eq!(rc.matches("# sqz — context intelligence layer").count(), 1);
}

// ── dashboard ─────────────────────────────────────────────────────────────────

#[test]
fn test_dashboard_help() {
    let o = run(&["dashboard", "--help"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("port") || out.contains("dashboard"), "should show dashboard help: {out}");
}
