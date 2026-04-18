/// Two-pass compression verifier.
///
/// After compression, the verifier checks that critical information was
/// preserved. If confidence is below the threshold, it signals the caller
/// to fall back to a safer (less aggressive) compression mode.
///
/// Checks performed:
/// 1. Required JSON keys present (if original was JSON)
/// 2. Numeric fields unchanged (no value corruption)
/// 3. Error/warning lines retained (critical signal preservation)
/// 4. Diff hunk headers present (if input was a git diff)
/// 5. File paths preserved (no path truncation)
/// 6. Minimum content retention (output not too short vs input)
/// 7. Identifier/path/URL preservation — deterministic token scan covering
///    filesystem paths, URLs, backtick-quoted code identifiers, environment
///    variable names, and version numbers. Added after the sessions that
///    produced the `packages → pkgs` / `configuration/` / `repository/` bug
///    class — the idea (post-compression preservation check) was prompted
///    by caveman-compress's validate.py, but the scan mechanism, inputs,
///    and integration point are sqz-specific.

use crate::types::VerifyResult;

/// Confidence threshold below which fallback is triggered.
const FALLBACK_THRESHOLD: f64 = 0.6;

pub struct Verifier;

impl Verifier {
    /// Run all invariant checks on `original` → `compressed`.
    ///
    /// Returns a `VerifyResult` with confidence score and check details.
    /// If `result.confidence < FALLBACK_THRESHOLD`, the caller should
    /// re-compress with a safer preset.
    pub fn verify(original: &str, compressed: &str) -> VerifyResult {
        let mut passed = Vec::new();
        let mut failed = Vec::new();

        // Check 1: Minimum content retention (output must be ≥ 10% of input length)
        let retention = if original.is_empty() {
            1.0
        } else {
            compressed.len() as f64 / original.len() as f64
        };
        if retention >= 0.10 {
            passed.push("min_retention".to_string());
        } else {
            failed.push((
                "min_retention".to_string(),
                format!("output is only {:.1}% of input length", retention * 100.0),
            ));
        }

        // Check 2: Error/warning lines retained
        let error_lines: Vec<&str> = original
            .lines()
            .filter(|l| {
                let lower = l.to_lowercase();
                lower.contains("error:") || lower.contains("warning:") || lower.contains("fatal:")
                    || lower.contains("panic:") || lower.contains("exception:")
            })
            .collect();
        if error_lines.is_empty() {
            passed.push("error_lines".to_string());
        } else {
            let missing: Vec<&str> = error_lines
                .iter()
                .filter(|&&line| !compressed.contains(line.trim()))
                .copied()
                .collect();
            if missing.is_empty() {
                passed.push("error_lines".to_string());
            } else {
                failed.push((
                    "error_lines".to_string(),
                    format!("{} error/warning line(s) missing from output", missing.len()),
                ));
            }
        }

        // Check 3: File paths preserved (lines containing / or \ with extension)
        let path_lines: Vec<&str> = original
            .lines()
            .filter(|l| {
                (l.contains('/') || l.contains('\\'))
                    && l.chars().any(|c| c == '.')
                    && l.len() < 200 // skip very long lines
            })
            .take(20) // only check first 20 path-like lines
            .collect();
        if path_lines.is_empty() {
            passed.push("file_paths".to_string());
        } else {
            let missing_paths = path_lines
                .iter()
                .filter(|&&line| {
                    // Extract the path-like token and check it's in the output
                    let token = line.split_whitespace()
                        .find(|t| t.contains('/') || t.contains('\\'))
                        .unwrap_or("");
                    !token.is_empty() && !compressed.contains(token)
                })
                .count();
            if missing_paths == 0 {
                passed.push("file_paths".to_string());
            } else {
                failed.push((
                    "file_paths".to_string(),
                    format!("{missing_paths} file path(s) missing from output"),
                ));
            }
        }

        // Check 4: JSON key preservation (if original is JSON)
        // We check that the compressed output contains at least 50% of the
        // original top-level keys. Intentionally stripped keys are expected to be absent.
        let orig_trimmed = original.trim();
        if orig_trimmed.starts_with('{') || orig_trimmed.starts_with('[') {
            if let Ok(orig_val) = serde_json::from_str::<serde_json::Value>(orig_trimmed) {
                let orig_keys = collect_top_level_keys(&orig_val);
                if orig_keys.is_empty() {
                    passed.push("json_keys".to_string());
                } else {
                    let present: usize = orig_keys
                        .iter()
                        .filter(|&&k| compressed.contains(k))
                        .count();
                    let retention_ratio = present as f64 / orig_keys.len() as f64;
                    // Pass if at least 50% of original keys are present
                    if retention_ratio >= 0.5 {
                        passed.push("json_keys".to_string());
                    } else {
                        let missing: Vec<&str> = orig_keys
                            .iter()
                            .filter(|&&k| !compressed.contains(k))
                            .copied()
                            .collect();
                        failed.push((
                            "json_keys".to_string(),
                            format!("only {:.0}% of JSON keys retained; missing: {:?}",
                                retention_ratio * 100.0,
                                &missing[..missing.len().min(5)]),
                        ));
                    }
                }
            } else {
                passed.push("json_keys".to_string()); // not valid JSON, skip
            }
        } else {
            passed.push("json_keys".to_string()); // not JSON, skip
        }

        // Check 5: Diff hunk headers preserved (if input is a git diff)
        let hunk_headers: Vec<&str> = original
            .lines()
            .filter(|l| l.starts_with("@@"))
            .collect();
        if hunk_headers.is_empty() {
            passed.push("diff_hunks".to_string());
        } else {
            let missing_hunks = hunk_headers
                .iter()
                .filter(|&&h| !compressed.contains(h))
                .count();
            if missing_hunks == 0 {
                passed.push("diff_hunks".to_string());
            } else {
                failed.push((
                    "diff_hunks".to_string(),
                    format!("{missing_hunks} diff hunk header(s) missing"),
                ));
            }
        }

        // Check 6: Numeric values preserved (spot-check first 10 numbers)
        let numbers: Vec<&str> = original
            .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
            .filter(|s| !s.is_empty() && s.len() >= 2 && s.parse::<f64>().is_ok())
            .take(10)
            .collect();
        if numbers.is_empty() {
            passed.push("numeric_values".to_string());
        } else {
            let missing_nums = numbers
                .iter()
                .filter(|&&n| !compressed.contains(n))
                .count();
            if missing_nums == 0 {
                passed.push("numeric_values".to_string());
            } else {
                failed.push((
                    "numeric_values".to_string(),
                    format!("{missing_nums} numeric value(s) missing from output"),
                ));
            }
        }

        // Check 7: Preservation tokens — identifier-shaped substrings the
        // model may dereference (paths, URLs, backticked code, env vars,
        // version numbers). See the session fixes for `packages/`,
        // `configuration/`, `repository/` — this check catches that bug
        // class deterministically rather than waiting for user reports.
        //
        // We require at least 85% of preservation tokens to survive. Lower
        // than 100% because: (1) dedup may have collapsed a repeated path
        // into a `§ref:…§` marker intentionally, and (2) the scanner is
        // heuristic and may flag a token that the pipeline legitimately
        // rewrote (e.g. long base64 truncated by entropy_truncate).
        let preservation_tokens = extract_preservation_tokens(original);
        if preservation_tokens.is_empty() {
            passed.push("preservation".to_string());
        } else {
            let present = preservation_tokens
                .iter()
                .filter(|t| compressed.contains(t.as_str()))
                .count();
            let total = preservation_tokens.len();
            let ratio = present as f64 / total as f64;
            if ratio >= 0.85 {
                passed.push("preservation".to_string());
            } else {
                let missing: Vec<&str> = preservation_tokens
                    .iter()
                    .filter(|t| !compressed.contains(t.as_str()))
                    .take(5)
                    .map(|t| t.as_str())
                    .collect();
                failed.push((
                    "preservation".to_string(),
                    format!(
                        "only {}/{} preservation tokens retained ({:.0}%); missing: {:?}",
                        present, total, ratio * 100.0, missing,
                    ),
                ));
            }
        }

        // Compute confidence: ratio of passed checks to total checks.
        //
        // Special case: preservation is a "sentinel" check. If it fails, the
        // LLM will likely try to dereference a token (filename, URL, identifier)
        // that no longer exists in the compressed output, which causes
        // cascading failures in agent sessions (the exact bug class that
        // produced fixes fd4603d, f6dc86c, b8bd0d7). Cap confidence at 0.5
        // when preservation fails so that the fallback kicks in even when
        // the other 6 checks pass.
        let total = passed.len() + failed.len();
        let mut confidence = if total == 0 {
            1.0
        } else {
            passed.len() as f64 / total as f64
        };
        let preservation_failed = failed.iter().any(|(k, _)| k == "preservation");
        if preservation_failed {
            confidence = confidence.min(0.5);
        }

        let fallback_triggered = confidence < FALLBACK_THRESHOLD;

        VerifyResult {
            passed: failed.is_empty(),
            confidence,
            checks_passed: passed,
            checks_failed: failed,
            fallback_triggered,
        }
    }

    /// Check if a verify result warrants fallback to safer compression.
    pub fn should_fallback(result: &VerifyResult) -> bool {
        result.fallback_triggered
    }
}

fn collect_top_level_keys(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::Object(map) => map.keys().map(|k| k.as_str()).collect(),
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Preservation-token extractor
// ---------------------------------------------------------------------------
//
// Pulls out identifier-shaped substrings from input that the LLM may try to
// dereference. The verifier requires these tokens to appear somewhere in the
// compressed output — if too many go missing, the compression is rejected
// and the caller falls back to Safe mode.
//
// Design notes:
// - Byte-level ASCII scan. No regex crate dependency; avoids ReDoS.
// - Conservative by design: false positives (over-preservation) cause at
//   worst a missed compression opportunity, whereas false negatives cause
//   silent data loss. We err toward flagging more.
// - Deduplicates tokens (each unique token counted once).
// - Caps the scan at 1 MB of input to bound worst-case runtime. Larger
//   inputs are truncated for scanning only; the actual compression still
//   processes the full input.

const MAX_SCAN_BYTES: usize = 1024 * 1024;
const MAX_TOKENS: usize = 500;

/// Scan input for preservation tokens: filesystem paths, URLs, backtick-quoted
/// code identifiers, environment variable names, and version numbers.
fn extract_preservation_tokens(input: &str) -> Vec<String> {
    let scan = &input[..input.len().min(MAX_SCAN_BYTES)];
    let bytes = scan.as_bytes();
    let mut tokens: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    let mut i = 0;
    while i < bytes.len() && tokens.len() < MAX_TOKENS {
        let b = bytes[i];

        // Backtick-quoted identifier: `foo_bar`, `Type::method`, `api::v1`
        if b == b'`' {
            if let Some(end) = find_closing(bytes, i + 1, b'`') {
                let slice = &scan[i + 1..end];
                // Must look like an identifier (not prose): contain at least
                // one non-space char and no spaces unless it's path-like.
                if !slice.is_empty()
                    && slice.len() <= 200
                    && is_identifier_or_path_content(slice)
                {
                    tokens.insert(slice.to_string());
                }
                i = end + 1;
                continue;
            }
        }

        // Environment variable: $HOME, $PATH, ${FOO}
        if b == b'$' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'{' {
                if let Some(end) = find_closing(bytes, i + 2, b'}') {
                    let slice = &scan[i + 2..end];
                    if is_env_name(slice) {
                        tokens.insert(format!("${{{}}}", slice));
                    }
                    i = end + 1;
                    continue;
                }
            } else if next.is_ascii_uppercase() || next == b'_' {
                // Bare $VARNAME — consume uppercase/underscore/digit run
                let start = i + 1;
                let mut j = start;
                while j < bytes.len()
                    && (bytes[j].is_ascii_uppercase()
                        || bytes[j] == b'_'
                        || bytes[j].is_ascii_digit())
                {
                    j += 1;
                }
                if j > start {
                    tokens.insert(format!("${}", &scan[start..j]));
                    i = j;
                    continue;
                }
            }
        }

        // URL: detect common protocol prefixes
        if is_url_start(bytes, i) {
            let end = scan_url_end(bytes, i);
            if end > i + 8 {
                // min "http://x"
                tokens.insert(scan[i..end].to_string());
                i = end;
                continue;
            }
        }

        // Path or path-like token. Must have at least one '/' with alphanum
        // on both sides. Starts with '/', '.', or alphanum; ends at whitespace
        // or unambiguous terminator.
        if is_path_start(bytes, i) {
            let end = scan_path_end(bytes, i);
            if end > i {
                let slice = &scan[i..end];
                // Require at least one '/' for path-ness.
                if slice.contains('/') && is_plausible_path(slice) {
                    tokens.insert(slice.to_string());
                    i = end;
                    continue;
                }
            }
        }

        // Version number: digit.digit(.digit)+, optional 'v' prefix, optional suffix
        if b.is_ascii_digit() || (b == b'v' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) {
            let end = scan_version_end(bytes, i);
            if end > i {
                let slice = &scan[i..end];
                if is_version(slice) {
                    tokens.insert(slice.to_string());
                    i = end;
                    continue;
                }
            }
        }

        i += 1;
    }

    tokens.into_iter().collect()
}

fn find_closing(bytes: &[u8], start: usize, target: u8) -> Option<usize> {
    // Look up to 256 bytes forward for the closing char. Anything longer
    // than that is almost certainly not a single token — give up.
    let end = (start + 256).min(bytes.len());
    bytes[start..end].iter().position(|&b| b == target).map(|off| start + off)
}

fn is_identifier_or_path_content(s: &str) -> bool {
    // Reject if it's mostly whitespace or contains multiple spaces (prose).
    // Allow single spaces (e.g. `some command --flag` in docs is prose, but
    // `cargo test` in docs is a command; we treat both conservatively as
    // preserve-worthy since splitting would lose the intent).
    let space_count = s.bytes().filter(|&b| b == b' ').count();
    if space_count > 3 {
        return false;
    }
    // Must contain at least one "identifier-ish" byte.
    s.bytes().any(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'/' || b == b':')
}

fn is_env_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

fn is_url_start(bytes: &[u8], i: usize) -> bool {
    const PREFIXES: &[&[u8]] = &[
        b"https://", b"http://", b"git://", b"ssh://", b"ftp://",
        b"file://", b"git@", b"ws://", b"wss://",
    ];
    PREFIXES.iter().any(|p| bytes[i..].starts_with(p))
}

fn scan_url_end(bytes: &[u8], start: usize) -> usize {
    // URL ends at whitespace, quote, angle bracket, comma followed by space,
    // or closing paren/bracket at end-of-sentence. Cap at 2KB.
    let cap = (start + 2048).min(bytes.len());
    let mut i = start;
    while i < cap {
        let b = bytes[i];
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'
            || b == b'"' || b == b'\'' || b == b'<' || b == b'>'
            || b == b'`'
        {
            break;
        }
        i += 1;
    }
    // Trim trailing punctuation that's usually sentence, not URL.
    while i > start {
        let last = bytes[i - 1];
        if last == b'.' || last == b',' || last == b';' || last == b':'
            || last == b')' || last == b']' || last == b'!' || last == b'?'
        {
            i -= 1;
        } else {
            break;
        }
    }
    i
}

fn is_path_start(bytes: &[u8], i: usize) -> bool {
    if i >= bytes.len() {
        return false;
    }
    // Path start can't follow an alphanum (otherwise we'd split identifiers).
    if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return false;
    }
    let b = bytes[i];
    b == b'/' || b == b'.' || b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn scan_path_end(bytes: &[u8], start: usize) -> usize {
    // Path characters: alphanum, '_', '-', '.', '/', and nothing else.
    // Cap at 512 bytes to avoid pathological inputs.
    let cap = (start + 512).min(bytes.len());
    let mut i = start;
    while i < cap {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.' || b == b'/' {
            i += 1;
        } else {
            break;
        }
    }
    // Trim trailing '.' that's probably sentence punctuation.
    while i > start && bytes[i - 1] == b'.' {
        i -= 1;
    }
    i
}

fn is_plausible_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    let slash = match bytes.iter().position(|&b| b == b'/') {
        Some(p) => p,
        None => return false,
    };
    // Require: something before the slash (so "/" alone isn't a path), OR
    // content after the slash (so a path can start with '/'). Also require
    // at least one alphabetic byte anywhere, to reject "3/4" fractions.
    let has_before = slash > 0;
    let has_after = slash + 1 < bytes.len();
    if !has_before && !has_after {
        return false;
    }
    bytes.iter().any(|b| b.is_ascii_alphabetic())
}

fn scan_version_end(bytes: &[u8], start: usize) -> usize {
    let cap = (start + 64).min(bytes.len());
    let mut i = start;
    while i < cap {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' {
            i += 1;
        } else {
            break;
        }
    }
    i
}

fn is_version(s: &str) -> bool {
    // Semver-ish: needs at least two '.' with digits on both sides.
    // Optional leading 'v'. Optional pre-release suffix.
    let trimmed = s.strip_prefix('v').unwrap_or(s);
    let dots = trimmed.bytes().filter(|&b| b == b'.').count();
    if dots < 2 {
        return false;
    }
    // First segment must be digits.
    let first_segment: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if first_segment.is_empty() {
        return false;
    }
    // Second segment must start with digit (after first '.').
    let after_first: &str = &trimmed[first_segment.len() + 1..];
    after_first.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_identical_passes_all() {
        let text = "error: something went wrong\nfile: src/main.rs\n";
        let result = Verifier::verify(text, text);
        assert!(result.passed);
        assert!((result.confidence - 1.0).abs() < f64::EPSILON);
        assert!(!result.fallback_triggered);
    }

    #[test]
    fn verify_empty_input_passes() {
        let result = Verifier::verify("", "");
        assert!(result.passed);
    }

    #[test]
    fn verify_detects_missing_error_line() {
        let original = "error: connection refused\nsome other content here\n";
        let compressed = "some other content here\n"; // error line stripped
        let result = Verifier::verify(original, compressed);
        assert!(!result.passed);
        assert!(result.checks_failed.iter().any(|(k, _)| k == "error_lines"));
    }

    #[test]
    fn verify_detects_over_compression() {
        // Use content with multiple checkable markers so more checks fail
        let original = "error: critical failure at line 42\n@@ -1,5 +1,5 @@\n/path/to/file.rs\nvalue: 12345\n".repeat(20);
        let compressed = "x"; // almost nothing retained
        let result = Verifier::verify(&original, compressed);
        assert!(!result.passed);
        assert!(result.checks_failed.iter().any(|(k, _)| k == "min_retention"));
        assert!(result.fallback_triggered, "should trigger fallback: confidence={:.2}", result.confidence);
    }

    #[test]
    fn verify_json_keys_preserved() {
        let original = r#"{"id":1,"name":"Alice","status":"active"}"#;
        let compressed = r#"TOON:{id:1,name:"Alice",status:"active"}"#;
        let result = Verifier::verify(original, compressed);
        assert!(result.checks_passed.contains(&"json_keys".to_string()));
    }

    #[test]
    fn verify_detects_missing_json_keys() {
        let original = r#"{"id":1,"name":"Alice","status":"active","role":"admin","email":"a@b.com","created":"2024-01-01"}"#;
        let compressed = r#"TOON:{id:1}"#; // only 1 of 6 keys retained (17%)
        let result = Verifier::verify(original, compressed);
        assert!(result.checks_failed.iter().any(|(k, _)| k == "json_keys"),
            "should fail json_keys when <50% of keys retained");
    }

    #[test]
    fn verify_diff_hunks_preserved() {
        let original = "@@ -1,5 +1,5 @@\n-old\n+new\n context\n";
        let compressed = "@@ -1,5 +1,5 @@\n-old\n+new\n";
        let result = Verifier::verify(original, compressed);
        assert!(result.checks_passed.contains(&"diff_hunks".to_string()));
    }

    #[test]
    fn verify_detects_missing_diff_hunks() {
        let original = "@@ -1,5 +1,5 @@\n-old\n+new\n";
        let compressed = "-old\n+new\n"; // hunk header stripped
        let result = Verifier::verify(original, compressed);
        assert!(result.checks_failed.iter().any(|(k, _)| k == "diff_hunks"));
    }

    #[test]
    fn fallback_threshold_triggers_correctly() {
        // Create a result that fails most checks
        let original = "error: critical failure\n@@ -1,5 +1,5 @@\n/path/to/file.rs:42\n";
        let compressed = "x"; // almost nothing retained
        let result = Verifier::verify(original, compressed);
        assert!(result.fallback_triggered, "should trigger fallback on low confidence");
    }

    // ── Real-world coding session patterns ────────────────────────────────

    #[test]
    fn verify_cargo_test_output_preserved() {
        let original = "running 47 tests\ntest engine::tests::test_compress ... ok\ntest pipeline::tests::compress_json ... ok\ntest result: ok. 47 passed; 0 failed; 0 ignored; finished in 2.34s\n";
        let compressed = "47 tests\ntest result: ok. 47 passed; 0 failed; finished in 2.34s\n";
        let result = Verifier::verify(original, compressed);
        // Should pass — key info retained, no error lines, no JSON
        assert!(result.confidence >= 0.7, "cargo test output should verify well: {:.2}", result.confidence);
    }

    #[test]
    fn verify_rust_compile_error_preserved() {
        let original = "error[E0308]: mismatched types\n --> src/main.rs:42:5\n  |\n42 |     let x: i32 = \"hello\";\n  |                  ^^^^^^^ expected `i32`, found `&str`\n\nerror: aborting due to previous error\n";
        let compressed = "error[E0308]: mismatched types\n --> src/main.rs:42:5\nerror: aborting due to previous error\n";
        let result = Verifier::verify(original, compressed);
        // Error lines must be retained
        assert!(result.checks_passed.contains(&"error_lines".to_string()),
            "error lines should be preserved");
    }

    #[test]
    fn verify_git_log_output() {
        let original = "commit a1b2c3d4\nAuthor: Ojus Chugh <ojuschugh@gmail.com>\nDate:   Sun Apr 12 10:00:00 2026\n\n    feat: Add compression engine\n\ncommit b2c3d4e5\nAuthor: Ojus Chugh <ojuschugh@gmail.com>\nDate:   Sat Apr 11 15:30:00 2026\n\n    fix: Handle edge case\n";
        let compressed = "commit a1b2c3d4\n    feat: Add compression engine\ncommit b2c3d4e5\n    fix: Handle edge case\n";
        let result = Verifier::verify(original, compressed);
        assert!(result.confidence >= 0.7, "git log should verify well: {:.2}", result.confidence);
    }

    #[test]
    fn verify_json_api_with_stripped_nulls() {
        // Simulates what the pipeline does: strip null fields, TOON encode
        let original = r#"{"id":1,"name":"Alice","debug_info":null,"trace_id":null,"status":"active"}"#;
        let compressed = r#"TOON:{id:1,name:"Alice",status:"active"}"#;
        let result = Verifier::verify(original, compressed);
        // 3 of 5 keys retained (60%) — should pass the 50% threshold
        assert!(result.checks_passed.contains(&"json_keys".to_string()),
            "60% key retention should pass: {:?}", result.checks_failed);
    }

    // ── Preservation-token extractor tests ──────────────────────────────

    #[test]
    fn extract_detects_absolute_paths() {
        let tokens = extract_preservation_tokens("see /etc/myapp/config.yml for details");
        assert!(tokens.contains(&"/etc/myapp/config.yml".to_string()),
            "absolute path should be extracted: {:?}", tokens);
    }

    #[test]
    fn extract_detects_relative_paths() {
        let tokens = extract_preservation_tokens("edit src/main.rs and tests/util.rs");
        assert!(tokens.contains(&"src/main.rs".to_string()), "{:?}", tokens);
        assert!(tokens.contains(&"tests/util.rs".to_string()), "{:?}", tokens);
    }

    #[test]
    fn extract_detects_directory_listing_entries() {
        // The Reddit case: `packages/` directory listing
        let input = "drwxr-xr-x  user staff  192 Apr 18 packages/\n\
                     drwxr-xr-x  user staff   96 Apr 18 configuration/\n";
        let tokens = extract_preservation_tokens(input);
        // These should appear as path-like tokens (they have `/`)
        assert!(tokens.iter().any(|t| t.contains("packages")),
            "should extract packages: {:?}", tokens);
        assert!(tokens.iter().any(|t| t.contains("configuration")),
            "should extract configuration: {:?}", tokens);
    }

    #[test]
    fn extract_detects_urls() {
        let input = "clone from https://github.com/example/repository and \
                     read https://docs.example.com/guide.";
        let tokens = extract_preservation_tokens(input);
        assert!(tokens.contains(&"https://github.com/example/repository".to_string()),
            "{:?}", tokens);
        assert!(tokens.iter().any(|t| t.starts_with("https://docs.example.com")),
            "{:?}", tokens);
    }

    #[test]
    fn extract_detects_backtick_identifiers() {
        let tokens = extract_preservation_tokens(
            "use `SqzEngine::new` and `CompressionPipeline::compress`"
        );
        assert!(tokens.contains(&"SqzEngine::new".to_string()), "{:?}", tokens);
        assert!(tokens.contains(&"CompressionPipeline::compress".to_string()), "{:?}", tokens);
    }

    #[test]
    fn extract_detects_env_vars() {
        let tokens = extract_preservation_tokens("set $HOME and ${FOO_BAR} and $PATH");
        assert!(tokens.contains(&"$HOME".to_string()), "{:?}", tokens);
        assert!(tokens.contains(&"${FOO_BAR}".to_string()), "{:?}", tokens);
        assert!(tokens.contains(&"$PATH".to_string()), "{:?}", tokens);
    }

    #[test]
    fn extract_detects_version_numbers() {
        let tokens = extract_preservation_tokens(
            "upgrade to 1.2.3 from v0.7.0 and pin 2.0.0-beta.1"
        );
        assert!(tokens.iter().any(|t| t.starts_with("1.2.3")), "{:?}", tokens);
        assert!(tokens.iter().any(|t| t.starts_with("v0.7.0")), "{:?}", tokens);
    }

    #[test]
    fn extract_ignores_prose() {
        // Plain prose without paths/URLs/identifiers should produce no tokens
        let tokens = extract_preservation_tokens(
            "The quick brown fox jumps over the lazy dog. Lorem ipsum dolor sit amet."
        );
        assert!(tokens.is_empty(), "prose should yield no preservation tokens: {:?}", tokens);
    }

    #[test]
    fn extract_ignores_fractions_in_prose() {
        // "3/4 of the way" is not a path
        let tokens = extract_preservation_tokens("We completed 3/4 of the tasks");
        assert!(tokens.iter().all(|t| !t.contains("3/4")),
            "fraction should not be extracted as path: {:?}", tokens);
    }

    #[test]
    fn extract_caps_at_max_tokens() {
        // Generate input with far more than MAX_TOKENS paths
        let mut input = String::new();
        for i in 0..1000 {
            input.push_str(&format!("file_{}/sub_{}.txt ", i, i));
        }
        let tokens = extract_preservation_tokens(&input);
        assert!(tokens.len() <= MAX_TOKENS, "should cap at {MAX_TOKENS}, got {}", tokens.len());
    }

    // ── Preservation check integration tests (regression for session bugs) ─

    #[test]
    fn verify_rejects_packages_to_pkgs_rewrite() {
        // Reddit bug: `packages` directory renamed to `pkgs` in output
        let original = "drwxr-xr-x  user staff  192 Apr 18 packages/\n\
                        drwxr-xr-x  user staff  128 Apr 18 documentation/\n";
        let compressed = "drwxr-xr-x  user staff  192 Apr 18 pkgs/\n\
                          drwxr-xr-x  user staff  128 Apr 18 docs/\n";
        let result = Verifier::verify(original, compressed);
        assert!(
            result.checks_failed.iter().any(|(k, _)| k == "preservation"),
            "should fail preservation when packages→pkgs: {:?}", result.checks_failed
        );
    }

    #[test]
    fn verify_rejects_config_path_rewrite() {
        // /etc/myapp/configuration/ → /etc/myapp/config/
        let original = "check /etc/myapp/configuration/default.yml for errors";
        let compressed = "check /etc/myapp/config/default.yml for errors";
        let result = Verifier::verify(original, compressed);
        assert!(
            result.checks_failed.iter().any(|(k, _)| k == "preservation"),
            "should fail preservation when path segment rewritten: {:?}", result.checks_failed
        );
    }

    #[test]
    fn verify_rejects_github_repo_rewrite() {
        // github.com/.../repository → github.com/.../repo
        let original = "origin  https://github.com/example/repository (fetch)";
        let compressed = "origin  https://github.com/example/repo (fetch)";
        let result = Verifier::verify(original, compressed);
        assert!(
            result.checks_failed.iter().any(|(k, _)| k == "preservation"),
            "should fail preservation when URL path rewritten: {:?}", result.checks_failed
        );
    }

    #[test]
    fn verify_rejects_drops_filenames_entirely() {
        // RLE pattern-run bug: 4 directory lines collapsed to a summary
        let original = "drwxr-xr-x packages/\n\
                        drwxr-xr-x configuration/\n\
                        drwxr-xr-x documentation/\n\
                        drwxr-xr-x environment/\n";
        let compressed = "drwxr-xr-x ... [×4, varying: 4 unique values]\n";
        let result = Verifier::verify(original, compressed);
        assert!(
            result.checks_failed.iter().any(|(k, _)| k == "preservation"),
            "should fail preservation when filenames dropped: {:?}", result.checks_failed
        );
    }

    #[test]
    fn verify_accepts_lossless_dedup_output() {
        // Legitimate dedup: original content is fully represented in output
        let original = "see /etc/myapp/default.yml and src/main.rs";
        let compressed = "see /etc/myapp/default.yml and src/main.rs";
        let result = Verifier::verify(original, compressed);
        assert!(
            result.checks_passed.contains(&"preservation".to_string()),
            "identical content must pass preservation: {:?}", result.checks_failed
        );
    }

    #[test]
    fn verify_accepts_json_null_stripped() {
        // stripping null fields from JSON should leave paths in values intact
        let original = r#"{"path":"/etc/foo.yml","debug":null,"log":"/var/log/app.log"}"#;
        let compressed = r#"TOON:{path:"/etc/foo.yml",log:"/var/log/app.log"}"#;
        let result = Verifier::verify(original, compressed);
        assert!(
            result.checks_passed.contains(&"preservation".to_string()),
            "null-stripping must not trip preservation: {:?}", result.checks_failed
        );
    }

    #[test]
    fn verify_accepts_empty_input() {
        // No preservation tokens in empty input → check passes vacuously
        let result = Verifier::verify("", "");
        assert!(result.checks_passed.contains(&"preservation".to_string()));
    }

    #[test]
    fn preservation_failure_triggers_fallback() {
        // Even if all other checks pass, a preservation failure must drop
        // confidence below the fallback threshold. This is the defense we
        // set up against the bug class from the April 18 audit session
        // (fd4603d / f6dc86c / b8bd0d7).
        let original = "commit: check /etc/myapp/configuration/default.yml\n\
                        file: src/main.rs line 42\n";
        let compressed = "commit: check /etc/myapp/config/default.yml\n\
                          file: src/main.rs line 42\n";
        let result = Verifier::verify(original, compressed);
        assert!(
            result.checks_failed.iter().any(|(k, _)| k == "preservation"),
            "preservation should fail"
        );
        assert!(
            result.fallback_triggered,
            "preservation failure alone must trigger fallback (confidence={:.2})",
            result.confidence
        );
    }
}
