//! The hook's rewrites, run in real shells: same output and exit status as
//! the original command, and a leading `cd` still applies afterwards.
#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn sqz() -> &'static str {
    env!("CARGO_BIN_EXE_sqz")
}

/// Run `sqz hook <tool>` on `payload` for a user whose shell is `shell`,
/// with Claude Code's settings pointed at `config` (or an empty
/// directory), returning the rewritten command.
fn hook(tool: &str, payload: serde_json::Value, shell: &str, config: &Path) -> Option<String> {
    let mut child = Command::new(sqz())
        .args(["hook", tool])
        .env("SHELL", shell)
        .env("CLAUDE_CONFIG_DIR", config)
        .env("CLAUDE_PROJECT_DIR", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    parsed
        .pointer("/hookSpecificOutput/updatedInput/command")
        .or_else(|| parsed.pointer("/hookSpecificOutput/tool_input/command"))?
        .as_str()
        .map(str::to_string)
}

/// The grouped form, as Gemini CLI and the other POSIX hosts get it.
fn grouped(command: &str, shell: &str, config: &Path) -> Option<String> {
    let payload = serde_json::json!({"tool_name": "run_shell_command", "tool_input": {"command": command}});
    hook("gemini", payload, shell, config)
}

fn claude(command: &str, mode: &str, shell: &str, config: &Path) -> Option<String> {
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "permission_mode": mode,
        "tool_input": {"command": command},
    });
    hook("claude", payload, shell, config)
}

#[derive(Debug, PartialEq)]
struct Run {
    out: Vec<u8>,
    status: i32,
    cwd: Option<String>,
}

/// Run `command` the way an agent's shell tool does, stderr merged, and
/// record the working directory it leaves behind: `None` when the shell
/// stops first, as it does on a fatal expansion error.
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
        .env_remove("X")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let mut text = out.stdout;
    let status = out.status.code().unwrap_or(-1);
    let trailer = b"\n[cwd=";
    let Some(at) = text.windows(trailer.len()).rposition(|w| w == trailer) else {
        return Run { out: text, status, cwd: None };
    };
    let cwd = String::from_utf8_lossy(&text[at + trailer.len()..text.len() - 1]).to_string();
    text.truncate(at);
    Run { out: text, status, cwd: Some(cwd) }
}

fn installed(name: &str) -> bool {
    [format!("/bin/{name}"), format!("/usr/bin/{name}")].iter().any(|p| Path::new(p).exists())
}

fn shells() -> Vec<&'static str> {
    let mut found = vec!["bash", "sh"];
    for name in ["zsh", "dash"] {
        if installed(name) {
            found.push(name);
        }
    }
    found
}

/// The shells Claude Code runs its Bash tool in.
fn claude_shells() -> Vec<&'static str> {
    let mut found = vec!["bash"];
    if installed("zsh") {
        found.push("zsh");
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
    "sh -c 'echo out; echo err >&2' 2>/dev/null",
    "sh -c 'echo kept; echo logged >&2' 2>err.log; cat err.log",
    "echo first\necho second",
    "echo done;",
    "printf '\\377\\376raw\\000bytes\\n'",
    "cat notes.txt | while read -r l; do echo \"got $l\"; done",
    "nonexistent-command-xyz",
    "sh -c 'exit 7' | cat",
    "sh -c 'exit 7'",
    "ls ${X:?not set}",
    "for f in *.nomatchxyz; do echo $f; done",
    "ls ${X:?not set}; echo next",
    "ls ${X:?not set} || echo next",
    "ls $((1/0)); echo next",
    "ls =nosuchcmdxyz; echo next",
    "ls *.nomatchxyz; echo next",
];

/// Lists only the ungrouped form takes: builtins that must keep running
/// in the shell itself, around commands that get their own pipe.
const FLAT_CASES: &[&str] = &[
    "export X=1 && sh -c 'echo x=$X'",
    "cd sub && ls && cd .. && pwd",
    "X=2; sh -c 'echo $0' \"$X\"",
    "sh -c 'exit 4' && echo skipped",
    "sh -c 'exit 4' || sh -c 'echo handled'",
    "grep beta notes.txt 2>/dev/null; grep missing notes.txt",
    "ls sub > listing.txt && cat listing.txt",
    "sh -c 'echo to-stderr >&2' 2>/dev/null",
];

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture.path().join("work");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("notes.txt"), "alpha\nbeta\n").unwrap();
    let home = fixture.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    (fixture, dir, home)
}

/// The rewrite prints the same bytes, exits with the same status and leaves
/// the same directory. A shell that stops picks its own status and leaves
/// the agent where it was, so then the rewrite only has to fail too, in the
/// directory it started in.
fn assert_same(shell: &str, command: &str, rewritten: &str, dir: &Path, home: &Path) {
    assert!(rewritten.contains("sqz compress"), "{rewritten}");
    let _ = std::fs::remove_file(dir.join("listing.txt"));
    let original = run(shell, command, dir, home);
    let _ = std::fs::remove_file(dir.join("listing.txt"));
    let mut piped = run(shell, rewritten, dir, home);
    if original.cwd.is_none()
        && original.status != 0
        && piped.status != 0
        && piped.cwd == run(shell, "true", dir, home).cwd
    {
        piped.status = original.status;
        piped.cwd = None;
    }
    assert_eq!(
        piped,
        original,
        "{shell}: {command:?}\nrewritten: {rewritten}\nout: {:?}\nwant: {:?}",
        String::from_utf8_lossy(&piped.out),
        String::from_utf8_lossy(&original.out)
    );
}

#[test]
fn rewrites_behave_like_the_original() {
    let (fixture, dir, home) = fixture();
    for shell in shells() {
        for command in CASES {
            let rewritten = grouped(command, shell, fixture.path())
                .unwrap_or_else(|| panic!("not rewritten: {command:?}"));
            assert_same(shell, command, &rewritten, &dir, &home);
        }
    }
}

#[test]
fn claude_rewrites_behave_like_the_original() {
    let (fixture, dir, home) = fixture();
    for shell in claude_shells() {
        for command in CASES.iter().chain(FLAT_CASES) {
            let rewritten = claude(command, "bypassPermissions", shell, fixture.path())
                .unwrap_or_else(|| panic!("not rewritten: {command:?}"));
            assert_same(shell, command, &rewritten, &dir, &home);
        }
    }
}

#[test]
fn claude_default_mode_needs_the_allow_rules() {
    let (fixture, _, _) = fixture();
    assert_eq!(claude("cargo test", "default", "zsh", fixture.path()), None);
    std::fs::write(
        fixture.path().join("settings.json"),
        serde_json::json!({"permissions": {"allow": ["Bash(set -o pipefail)", "Bash(sqz compress --cmd *)"]}})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        claude("cargo test", "default", "zsh", fixture.path()).as_deref(),
        Some("set -o pipefail; cargo test 2>&1 | sqz compress --cmd 'cargo test'")
    );
    // Grouped forms never match an allow rule, so they wait for bypass mode.
    assert_eq!(claude("git log | head", "default", "zsh", fixture.path()), None);
    assert!(claude("git log | head", "bypassPermissions", "zsh", fixture.path()).is_some());
    // Piping a command alone would let a fatal expansion in it run the rest.
    assert_eq!(claude("ls ${X:?not set}; echo next", "default", "bash", fixture.path()), None);
    assert_eq!(claude("ls *.nomatchxyz; echo next", "default", "zsh", fixture.path()), None);
    assert!(claude("ls *.nomatchxyz; echo next", "default", "bash", fixture.path()).is_some());
    assert!(claude("ls ${X:?not set} && echo next", "default", "zsh", fixture.path()).is_some());
}

#[test]
fn state_changes_are_not_grouped() {
    let (fixture, _, _) = fixture();
    for command in ["export X=1 && make", "make && cd build", "npm run build &", "tail -f log.txt", "set -e; make"] {
        assert_eq!(grouped(command, "zsh", fixture.path()), None, "{command:?}");
    }
}
