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

        // Compute confidence: ratio of passed checks to total checks
        let total = passed.len() + failed.len();
        let confidence = if total == 0 {
            1.0
        } else {
            passed.len() as f64 / total as f64
        };

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
}
