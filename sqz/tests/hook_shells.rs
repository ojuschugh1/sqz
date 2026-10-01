//! The hook's rewrites, run in real shells: same output and exit status as
//! the original command, and a leading `cd` still applies afterwards.
#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn sqz() -> &'static str {
    env!("CARGO_BIN_EXE_sqz")
}

fn rewrite(command: &str) -> Option<String> {
    let input = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}}).to_string();
    let mut child = Command::new(sqz())
        .args(["hook", "claude"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    parsed
        .pointer("/hookSpecificOutput/updatedInput/command")?
        .as_str()
        .map(str::to_string)
}

#[derive(Debug, PartialEq)]
struct Run {
    out: Vec<u8>,
    status: i32,
    cwd: String,
}

/// Run `command` the way an agent's shell tool does, stderr merged, and
/// record the working directory it leaves behind.
fn run(shell: &str, command: &str, dir: &Path, home: &Path) -> Run {
    let script = format!("exec 2>&1\n{command}\nst=$?\nprintf '\\n[cwd=%s]' \"$PWD\"\nexit $st");
    let bin_dir = Path::new(sqz()).parent().unwrap();
    let path = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap_or_default());
    let out = Command::new(shell)
        .arg("-c")
        .arg(&script)
        .current_dir(dir)
        .env("PATH", path)
        .env("HOME", home)
        .env("SQZ_DB_PATH", home.join("sessions.db"))
        .env("SQZ_QUIET", "1")
        .env("SQZ_NO_DEDUP", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let mut text = out.stdout;
    let trailer = b"\n[cwd=";
    let at = text
        .windows(trailer.len())
        .rposition(|w| w == trailer)
        .unwrap_or_else(|| panic!("{shell}: no cwd trailer for {command:?}: {}", String::from_utf8_lossy(&text)));
    let cwd = String::from_utf8_lossy(&text[at + trailer.len()..text.len() - 1]).to_string();
    text.truncate(at);
    Run { out: text, status: out.status.code().unwrap_or(-1), cwd }
}

fn shells() -> Vec<&'static str> {
    let mut found = vec!["bash", "sh"];
    for (name, paths) in [("zsh", ["/bin/zsh", "/usr/bin/zsh"]), ("dash", ["/bin/dash", "/usr/bin/dash"])] {
        if paths.iter().any(|p| Path::new(p).exists()) {
            found.push(name);
        }
    }
    found
}

const CASES: &[&str] = &[
    "echo hello",
    "printf 'a\\nb\\n' | sort -r",
    "false || echo recovered",
    "true && false",
    "sh -c 'echo out; echo err >&2; exit 3'",
    "for i in 1 2 3; do echo \"n$i\"; done",
    "echo one # trailing comment",
    "cat <<'EOF'\nline | with ; ops &\nEOF",
    "cat <<EOF | tr a-z A-Z\nshout\nEOF",
    "cd sub && pwd",
    "cd missing-dir && echo never",
    "cd sub; ls ..",
    "printf 'no newline at end'",
    "seq 3 | grep 2; echo \"status $?\"",
    "echo $(echo nested | tr a-z A-Z)",
    "if [ -d sub ]; then echo yes; else echo no; fi",
    "(cd sub && pwd)",
    "grep -c alpha notes.txt || true",
    "echo first\necho second",
    "echo done;",
    "printf '\\377\\376raw\\000bytes\\n'",
    "cat notes.txt | while read -r l; do echo \"got $l\"; done",
    "nonexistent-command-xyz",
    "sh -c 'exit 7' | cat",
    "sh -c 'exit 7'",
];

#[test]
fn rewrites_behave_like_the_original() {
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture.path().join("work");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("notes.txt"), "alpha\nbeta\n").unwrap();
    let home = fixture.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    for shell in shells() {
        for command in CASES {
            let rewritten = rewrite(command).unwrap_or_else(|| panic!("not rewritten: {command:?}"));
            assert!(rewritten.contains("sqz compress"), "{rewritten}");
            let original = run(shell, command, &dir, &home);
            let piped = run(shell, &rewritten, &dir, &home);
            assert_eq!(
                piped,
                original,
                "{shell}: {command:?}\nrewritten: {rewritten}\nout: {:?}\nwant: {:?}",
                String::from_utf8_lossy(&piped.out),
                String::from_utf8_lossy(&original.out)
            );
        }
    }
}

#[test]
fn state_changes_are_not_rewritten() {
    for command in ["export X=1 && make", "make && cd build", "npm run build &", "tail -f log.txt", "set -e; make"] {
        assert_eq!(rewrite(command), None, "{command:?}");
    }
}
