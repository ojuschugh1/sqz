//! Exit-status trailer protocol between hook-rewritten commands and
//! `sqz compress`.
//!
//! A pipeline exits with the status of its last command, so
//! `cmd 2>&1 | sqz compress` would report sqz's status, not cmd's. The
//! rewrite sends the real status through the pipe as a trailer line
//! that sqz strips and exits with:
//!
//! ```sh
//! { cmd 2>&1; printf '\n__SQZ_EXIT_%d__\n' "$?"; } | sqz compress --cmd cmd
//! ```
//!
//! Plain POSIX, so it works in bash/zsh/dash/ash/ksh where `pipefail`
//! (rejected by dash/ash) and `PIPESTATUS` (bash-only) would not.
//! PowerShell gets its own form. Input without a trailer keeps the old
//! exit-0 behavior, so manual pipes and legacy shell functions are
//! unaffected.

/// Build the POSIX form of a rewritten command that carries its exit
/// status to sqz. `escaped_label` must already be shell-escaped.
pub fn posix_rewrite(command: &str, escaped_label: &str) -> String {
    format!(
        "{{ {command} 2>&1; printf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; }} | sqz compress --cmd {escaped_label}"
    )
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
/// producer added) removed, plus the parsed status. When no marker is
/// present the input is returned unchanged with `None`.
///
/// The match is deliberately strict: the marker must be the final
/// non-empty line, consist of nothing but the literal frame and 1–10
/// digits, and sit at the very end of the stream. A command whose real
/// output happens to contain the string mid-stream is left alone.
pub fn strip_exit_marker(input: &str) -> (String, Option<i32>) {
    const PREFIX: &str = "__SQZ_EXIT_";
    const SUFFIX: &str = "__";

    // Peel trailing newlines (the producer emits a trailing \n; PowerShell
    // may emit \r\n) to expose the marker line.
    let trimmed = input.trim_end_matches(['\n', '\r']);
    let line_start = trimmed.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let last_line = trimmed[line_start..].trim_end_matches('\r');

    let Some(rest) = last_line.strip_prefix(PREFIX) else {
        return (input.to_string(), None);
    };
    let Some(digits) = rest.strip_suffix(SUFFIX) else {
        return (input.to_string(), None);
    };
    if digits.is_empty() || digits.len() > 10 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return (input.to_string(), None);
    }
    let Ok(code) = digits.parse::<i32>() else {
        return (input.to_string(), None);
    };

    // Drop the marker line plus the single separator newline the producer
    // prepended (`printf '\n…'`), restoring the original bytes exactly.
    let mut head = &trimmed[..line_start];
    head = head.strip_suffix("\r\n").or_else(|| head.strip_suffix('\n')).unwrap_or(head);
    (head.to_string(), Some(code))
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
            "x\n__SQZ_EXIT_5_\n",
            "x\nSQZ_EXIT_5__\n",
        ] {
            let (out, code) = strip_exit_marker(input);
            assert_eq!(out, input, "must not strip: {input:?}");
            assert_eq!(code, None);
        }
    }

    #[test]
    fn posix_rewrite_shape() {
        let r = posix_rewrite("git status", "'git status'");
        assert_eq!(
            r,
            "{ git status 2>&1; printf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; } | sqz compress --cmd 'git status'"
        );
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
