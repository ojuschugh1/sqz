//! Exit-status trailer protocol between hook-rewritten commands and
//! `sqz compress`.
//!
//! A pipeline exits with the status of its last command, so
//! `cmd 2>&1 | sqz compress` would report sqz's status, not cmd's. The
//! rewrite sends the real status through the pipe as a trailer line
//! that sqz strips and exits with:
//!
//! ```sh
//! { cmd; printf '\n__SQZ_EXIT_%d__\n' "$?"; } 2>&1 | sqz compress --cmd cmd
//! ```
//!
//! Plain POSIX, so it works in bash/zsh/dash/ash/ksh where `pipefail`
//! (rejected by dash/ash) and `PIPESTATUS` (bash-only) would not.
//! PowerShell gets its own form. Input without a trailer keeps the old
//! exit-0 behavior, so manual pipes and legacy shell functions are
//! unaffected.

/// Build the POSIX form of a rewritten command that carries its exit
/// status to sqz. `escaped_label` must already be shell-escaped. The
/// `2>&1` sits on the group, so a redirect of the command's own, such as
/// `2>/dev/null` or `2> err.log`, still wins.
pub fn posix_rewrite(command: &str, escaped_label: &str) -> String {
    format!(
        "{{ {command}; printf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; }} 2>&1 | sqz compress --cmd {escaped_label}"
    )
}

/// The form for a list or pipeline. The newline before `printf` ends a
/// trailing comment or heredoc, and the group's `2>&1` covers every
/// command in it.
pub fn posix_rewrite_list(body: &str, escaped_label: &str) -> String {
    format!(
        "{{ {body}\nprintf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; }} 2>&1 | sqz compress --cmd {escaped_label}"
    )
}

/// The ungrouped form for bash and zsh: each marked command gets its own
/// pipe, and `pipefail` makes each pipe exit with the command's status, so
/// `&&` and `||` behave as before and builtins run in the shell itself.
/// The shell must not keep state between commands, or `pipefail` leaks.
pub fn posix_flat_rewrite(command: &str, cmds: &[crate::shell_scan::FlatCmd]) -> String {
    let command = command.trim_end();
    let mut out = String::from("set -o pipefail; ");
    let mut at = 0;
    for cmd in cmds {
        out.push_str(&command[at..cmd.end]);
        if let Some(label) = &cmd.label {
            if cmd.merge_stderr {
                out.push_str(" 2>&1");
            }
            out.push_str(" | sqz compress --cmd ");
            if label.contains(' ') {
                out.push('\'');
                out.push_str(label);
                out.push('\'');
            } else {
                out.push_str(label);
            }
        }
        at = cmd.end;
    }
    out.push_str(&command[at..]);
    out
}

/// Build the PowerShell form. Native commands set `$LASTEXITCODE`;
/// cmdlets set only `$?`, so fall back to that. The `$(...)`
/// subexpression keeps the trailing `__` out of the variable name.
pub fn powershell_rewrite(command: &str, escaped_label: &str) -> String {
    format!(
        "& {{ {command} 2>&1; \"__SQZ_EXIT_$(if ($null -ne $LASTEXITCODE) {{ $LASTEXITCODE }} elseif ($?) {{ 0 }} else {{ 1 }})__\" }} | sqz compress --cmd {escaped_label}"
    )
}

/// Split a trailing `__SQZ_EXIT_<n>__` marker off `input`.
///
/// Returns the input with the marker (and the separator newline the
/// producer added) removed, plus the status to exit with. When no marker
/// is present the input is returned unchanged with `None`.
///
/// The match is deliberately strict: the marker must be the final
/// non-empty line, consist of nothing but the literal frame, an optional
/// `-` and 1 to 10 digits, and sit at the very end of the stream. A
/// command whose real output happens to contain the string mid-stream is
/// left alone.
pub fn strip_exit_marker(input: &str) -> (String, Option<i32>) {
    let (head, code) = strip_exit_marker_bytes(input.as_bytes());
    // `head` is a prefix of `input` that ends before an ASCII newline.
    (input[..head.len()].to_string(), code)
}

/// [`strip_exit_marker`] for output that isn't valid UTF-8.
pub fn strip_exit_marker_bytes(input: &[u8]) -> (&[u8], Option<i32>) {
    const PREFIX: &[u8] = b"__SQZ_EXIT_";
    const SUFFIX: &[u8] = b"__";

    // Peel trailing newlines (the producer emits a trailing \n; PowerShell
    // may emit \r\n) to expose the marker line.
    let mut end = input.len();
    while end > 0 && matches!(input[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    let trimmed = &input[..end];
    let line_start = trimmed.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let last_line = &trimmed[line_start..];

    let Some(digits) = last_line.strip_prefix(PREFIX).and_then(|r| r.strip_suffix(SUFFIX)) else {
        return (input, None);
    };
    let unsigned = digits.strip_prefix(b"-").unwrap_or(digits);
    if unsigned.is_empty() || unsigned.len() > 10 || !unsigned.iter().all(u8::is_ascii_digit) {
        return (input, None);
    }
    let Some(code) = std::str::from_utf8(digits).ok().and_then(|d| d.parse::<i64>().ok()) else {
        return (input, None);
    };

    // Drop the marker line plus the single separator newline the producer
    // prepended (`printf '\n…'`), restoring the original bytes exactly.
    let mut head = &trimmed[..line_start];
    head = head
        .strip_suffix(b"\r\n")
        .or_else(|| head.strip_suffix(b"\n"))
        .unwrap_or(head);
    (head, Some(exit_status(code)))
}

/// What a process can exit with for `code`: all 32 bits on Windows, where
/// PowerShell reports a crash as a negative `$LASTEXITCODE`, and the low
/// byte on Unix. A failure never comes out as 0.
fn exit_status(code: i64) -> i32 {
    let kept = if cfg!(windows) { code as i32 } else { (code & 0xFF) as i32 };
    if kept == 0 && code != 0 {
        1
    } else {
        kept
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_marker_and_restores_exact_bytes() {
        let (out, code) = strip_exit_marker("hello\nworld\n\n__SQZ_EXIT_2__\n");
        assert_eq!(out, "hello\nworld\n");
        assert_eq!(code, Some(2));
    }

    #[test]
    fn output_without_trailing_newline() {
        let (out, code) = strip_exit_marker("no newline\n__SQZ_EXIT_42__\n");
        assert_eq!(out, "no newline");
        assert_eq!(code, Some(42));
    }

    #[test]
    fn empty_output_just_marker() {
        let (out, code) = strip_exit_marker("\n__SQZ_EXIT_0__\n");
        assert_eq!(out, "");
        assert_eq!(code, Some(0));
    }

    #[test]
    fn crlf_from_powershell() {
        let (out, code) = strip_exit_marker("line one\r\n__SQZ_EXIT_1__\r\n");
        assert_eq!(out, "line one");
        assert_eq!(code, Some(1));
    }

    #[test]
    fn no_marker_passes_through() {
        let input = "plain output\nwith lines\n";
        let (out, code) = strip_exit_marker(input);
        assert_eq!(out, input);
        assert_eq!(code, None);
    }

    #[test]
    fn marker_mid_stream_is_not_a_trailer() {
        let input = "__SQZ_EXIT_7__\nreal output after\n";
        let (out, code) = strip_exit_marker(input);
        assert_eq!(out, input);
        assert_eq!(code, None);
    }

    #[test]
    fn malformed_markers_left_alone() {
        for input in [
            "x\n__SQZ_EXIT___\n",
            "x\n__SQZ_EXIT_12a__\n",
            "x\n__SQZ_EXIT_12345678901__\n",
            "x\n__SQZ_EXIT_-__\n",
            "x\n__SQZ_EXIT_--5__\n",
            "x\n__SQZ_EXIT_+5__\n",
            "x\n__SQZ_EXIT_5_\n",
            "x\nSQZ_EXIT_5__\n",
        ] {
            let (out, code) = strip_exit_marker(input);
            assert_eq!(out, input, "must not strip: {input:?}");
            assert_eq!(code, None);
        }
    }

    #[test]
    fn negative_and_wide_codes_still_fail() {
        let (out, code) = strip_exit_marker("fatal: crashed\n\n__SQZ_EXIT_-1073741819__\n");
        assert_eq!(out, "fatal: crashed\n");
        assert_eq!(code, Some(if cfg!(windows) { -1073741819 } else { 5 }));
        let (_, code) = strip_exit_marker("x\n__SQZ_EXIT_3221225477__\n");
        assert_eq!(code, Some(if cfg!(windows) { -1073741819 } else { 5 }));
        let (_, code) = strip_exit_marker("x\n__SQZ_EXIT_256__\n");
        assert_eq!(code, Some(if cfg!(windows) { 256 } else { 1 }));
        assert_eq!(strip_exit_marker("x\n__SQZ_EXIT_4294967296__\n").1, Some(1));
        assert_eq!(strip_exit_marker("x\n__SQZ_EXIT_-0__\n").1, Some(0));
        let (_, code) = strip_exit_marker_bytes(b"\xff\n__SQZ_EXIT_-1__\n");
        assert_eq!(code, Some(if cfg!(windows) { -1 } else { 255 }));
    }

    #[test]
    fn posix_rewrite_shape() {
        let r = posix_rewrite("git status", "'git status'");
        assert_eq!(
            r,
            "{ git status; printf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; } 2>&1 | sqz compress --cmd 'git status'"
        );
    }

    #[test]
    fn strips_marker_from_binary_output() {
        let input = b"\xff\xfe\x00raw\n\n__SQZ_EXIT_3__\n";
        assert_eq!(strip_exit_marker_bytes(input), (&b"\xff\xfe\x00raw\n"[..], Some(3)));
        let plain = b"\xff no marker";
        assert_eq!(strip_exit_marker_bytes(plain), (&plain[..], None));
    }

    #[test]
    fn list_rewrite_shape() {
        assert_eq!(
            posix_rewrite_list("ls # all", "'sh: ls # all'"),
            "{ ls # all\nprintf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; } 2>&1 | sqz compress --cmd 'sh: ls # all'"
        );
    }

    #[test]
    fn flat_rewrite_shape() {
        let cmd = "cd sub && cargo test 2>&1; git log -3 # recent\n";
        let parts = crate::shell_scan::flat(cmd).unwrap();
        assert_eq!(
            posix_flat_rewrite(cmd, &parts),
            "set -o pipefail; cd sub && cargo test 2>&1 | sqz compress --cmd 'cargo test'; \
             git log -3 2>&1 | sqz compress --cmd 'git log -3' # recent"
        );
        let parts = crate::shell_scan::flat("ls").unwrap();
        assert_eq!(posix_flat_rewrite("ls", &parts), "set -o pipefail; ls 2>&1 | sqz compress --cmd ls");
    }

    #[test]
    fn round_trip_posix_semantics() {
        // Simulate what the shell group produces for a failing command.
        let produced = format!("ls: cannot access '/nope': No such file or directory\n\n__SQZ_EXIT_{}__\n", 2);
        let (out, code) = strip_exit_marker(&produced);
        assert_eq!(out, "ls: cannot access '/nope': No such file or directory\n");
        assert_eq!(code, Some(2));
    }
}
