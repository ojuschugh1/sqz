//! The sqz-mcp binary over real stdio: command-line flags, line framing,
//! and how the proxy treats its upstream.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

fn sqz_mcp() -> &'static str {
    env!("CARGO_BIN_EXE_sqz-mcp")
}

/// `sqz-mcp <args>` with its database in `dir` and every stream piped.
fn spawn(dir: &Path, args: &[&str]) -> Child {
    Command::new(sqz_mcp())
        .args(args)
        .current_dir(dir)
        .env("SQZ_DB_PATH", dir.join("sessions.db"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// The child's exit status, or `None` (after killing it) when it is still
/// running `secs` seconds later.
fn wait_for(child: &mut Child, secs: u64) -> Option<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// Stdout lines as they arrive, so a test can time out instead of hanging.
fn lines(stdout: ChildStdout) -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn reply(rx: &Receiver<String>) -> serde_json::Value {
    let line = rx.recv_timeout(Duration::from_secs(30)).expect("no reply");
    serde_json::from_str(&line).unwrap()
}

#[test]
fn version_and_help_print_and_exit() {
    let dir = tempfile::tempdir().unwrap();
    for flag in ["--version", "-V"] {
        let mut child = spawn(dir.path(), &[flag]);
        let status = wait_for(&mut child, 10).unwrap_or_else(|| panic!("{flag}: still running"));
        assert!(status.success(), "{flag}");
        let mut out = String::new();
        child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        assert_eq!(out, format!("sqz-mcp {}\n", env!("CARGO_PKG_VERSION")), "{flag}");
    }
    let mut child = spawn(dir.path(), &["--help"]);
    assert!(wait_for(&mut child, 10).is_some_and(|s| s.success()));
    let mut err = String::new();
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(err.starts_with("Usage: sqz-mcp ") && err.contains("--version"), "{err}");
}

const PASSTHROUGH_LATIN1: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"passthrough\",\"arguments\":{\"text\":\"caf\xe9\"}}}\n";

#[test]
fn a_line_that_is_not_utf8_gets_a_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn(dir.path(), &[]);
    let mut stdin = child.stdin.take().unwrap();
    let rx = lines(child.stdout.take().unwrap());
    stdin.write_all(PASSTHROUGH_LATIN1).unwrap();
    let first = reply(&rx);
    assert_eq!(first["error"]["code"], -32700, "{first}");
    assert!(first["id"].is_null(), "{first}");
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    assert!(reply(&rx)["result"]["tools"].is_array());
    drop(stdin);
    assert!(wait_for(&mut child, 10).is_some_and(|s| s.success()));
}

#[test]
fn proxy_passes_on_a_client_line_that_is_not_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn(dir.path(), &["proxy", "--", sqz_mcp()]);
    let mut stdin = child.stdin.take().unwrap();
    let rx = lines(child.stdout.take().unwrap());
    stdin.write_all(PASSTHROUGH_LATIN1).unwrap();
    let first = reply(&rx);
    assert_eq!(first["result"]["content"][0]["text"], "caf\u{fffd}", "{first}");
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    assert_eq!(reply(&rx)["id"], 2);
    drop(stdin);
    assert!(wait_for(&mut child, 10).is_some_and(|s| s.success()));
}

#[cfg(unix)]
#[test]
fn proxy_passes_on_an_upstream_line_that_is_not_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = r#"read -r line; printf '{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"caf\351"}]}}\n'; read -r line; printf '{"jsonrpc":"2.0","id":2,"result":{}}\n'"#;
    let mut child = spawn(dir.path(), &["proxy", "--", "sh", "-c", upstream]);
    let mut stdin = child.stdin.take().unwrap();
    let rx = lines(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"t","arguments":{{}}}}}}"#).unwrap();
    let first = reply(&rx);
    assert_eq!(first["result"]["content"][0]["text"], "caf\u{fffd}", "{first}");
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    assert_eq!(reply(&rx)["id"], 2);
    drop(stdin);
    assert!(wait_for(&mut child, 10).is_some_and(|s| s.success()));
}

#[cfg(unix)]
#[test]
fn proxy_stops_an_upstream_that_ignores_end_of_input() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("upstream.pid");
    let upstream = format!("echo $$ > '{}'; exec sleep 30", pid_file.display());
    let mut child = spawn(dir.path(), &["proxy", "--", "sh", "-c", &upstream]);
    let started = Instant::now();
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pid_file).ok().filter(|s| s.ends_with('\n')) {
            break pid.trim().to_string();
        }
        assert!(started.elapsed() < Duration::from_secs(10), "upstream never started");
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(child.stdin.take());
    let status = wait_for(&mut child, 15);
    assert!(status.is_some_and(|s| s.success()), "proxy still running after its input closed");
    let running = Command::new("kill").args(["-0", &pid]).stderr(Stdio::null()).status().unwrap().success();
    assert!(!running, "upstream {pid} outlived the proxy");
}

#[test]
fn unknown_flags_still_start_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn(dir.path(), &["--transport", "stdio", "--from-some-client"]);
    let mut stdin = child.stdin.take().unwrap();
    let rx = lines(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#).unwrap();
    assert_eq!(reply(&rx)["result"]["serverInfo"]["name"], "sqz-mcp");
    drop(stdin);
    assert!(wait_for(&mut child, 10).is_some_and(|s| s.success()));
}
