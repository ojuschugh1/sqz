use super::truncate::{CAP_ERRORS, CAP_WARNINGS};

pub fn format_python(cmd: &str, subcmd: Option<&str>, output: &str) -> Option<String> {
    let base = cmd.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("");

    match base {
        "pytest" => Some(format_pytest(output)),
        "python" | "python3" if cmd.contains("pytest") || cmd.contains("-m pytest") => {
            Some(format_pytest(output))
        }
        "ruff" => Some(format_ruff(subcmd, output)),
        "mypy" => Some(format_mypy(output)),
        "pip" | "pip3" => format_pip(subcmd, output),
        _ => None,
    }
}

/// Keeps the final counts, every short-summary line, and per failure the
/// failing source line (`>`), assertion detail (`E`) and `path:line` from the
/// FAILURES section. Progress dots, headers and unchanged source lines go.
pub fn format_pytest(output: &str) -> String {
    let is_banner = |l: &str| l.starts_with("===") && l.trim_end().ends_with("===");
    let strip_banner = |l: &str| l.trim().trim_matches('=').trim().to_string();

    let mut totals = String::new();
    let mut summary: Vec<String> = Vec::new();
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    let mut section = "";

    for line in output.lines() {
        if is_banner(line) {
            let inner = strip_banner(line);
            let lower = inner.to_ascii_lowercase();
            section = if lower == "failures" || lower == "errors" {
                "failures"
            } else if lower.contains("short test summary") {
                "summary"
            } else if lower.contains("passed") || lower.contains("failed") || lower.contains("error") || lower.contains("no tests ran") {
                totals = inner;
                ""
            } else {
                ""
            };
            continue;
        }
        match section {
            "failures" => {
                if line.starts_with("___") && line.trim_end().ends_with("___") {
                    blocks.push((line.trim().trim_matches('_').trim().to_string(), Vec::new()));
                } else if let Some((_, details)) = blocks.last_mut() {
                    let t = line.trim_start();
                    let is_location = !line.starts_with(' ')
                        && !t.is_empty()
                        && t.split(':').nth(1).is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty());
                    if line.starts_with('>') || line.starts_with("E ") || is_location {
                        details.push(line.trim_end().to_string());
                    }
                }
            }
            "summary" => {
                if line.starts_with("FAILED") || line.starts_with("ERROR") {
                    summary.push(line.trim_end().to_string());
                }
            }
            _ => {}
        }
    }

    if totals.is_empty() && summary.is_empty() && blocks.is_empty() {
        return super::test_output::format_test_failures(output);
    }

    let mut out = Vec::new();
    if !totals.is_empty() {
        out.push(totals);
    }
    if summary.is_empty() {
        for (title, _) in &blocks {
            summary.push(format!("FAILED {title}"));
        }
    }
    for line in summary.iter().take(CAP_ERRORS) {
        out.push(line.clone());
        let name = line.split_whitespace().nth(1).unwrap_or("");
        if let Some((_, details)) = blocks.iter().find(|(title, _)| {
            let dotted = title.replace('.', "::");
            name.ends_with(&format!("::{title}")) || name.ends_with(&format!("::{dotted}")) || name == title
        }) {
            let mut shown = 0;
            for d in details {
                if line.contains(d.trim_start_matches("E ").trim()) {
                    continue;
                }
                if shown == 12 {
                    out.push(format!("    ...+{} more lines", details.len() - shown));
                    break;
                }
                out.push(format!("    {d}"));
                shown += 1;
            }
        }
    }
    if summary.len() > CAP_ERRORS {
        out.push(format!("...+{} more failures", summary.len() - CAP_ERRORS));
    }
    out.join("\n")
}

fn format_ruff(subcmd: Option<&str>, output: &str) -> String {
    // Try JSON first (ruff check --output-format=json)
    if output.trim_start().starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(output) {
            return format_ruff_json(&arr);
        }
    }

    let is_format = subcmd == Some("format");
    if is_format {
        return format_ruff_format(output);
    }

    // Human-readable: "file.py:10:5: E501 Line too long"
    let mut by_file: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    let mut total = 0;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        // Pattern: "path:line:col: CODE message"
        if let Some(colon1) = trimmed.find(':') {
            let after = &trimmed[colon1 + 1..];
            if after.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                let file = trimmed[..colon1].to_string();
                by_file.entry(file).or_default().push(trimmed.to_string());
                total += 1;
            }
        }
    }

    if total == 0 {
        if output.contains("All checks passed") || output.trim().is_empty() {
            return "ok: 0 issues".to_string();
        }
        return output.to_string();
    }

    let mut result = format!("{} issues in {} files:\n", total, by_file.len());
    let mut shown = 0;
    for (file, issues) in &by_file {
        if shown >= CAP_ERRORS { break; }
        result.push_str(&format!("  {} ({}):\n", file, issues.len()));
        for issue in issues.iter().take(5) {
            result.push_str(&format!("    {}\n", issue));
            shown += 1;
        }
        if issues.len() > 5 {
            result.push_str(&format!("    ...+{} more\n", issues.len() - 5));
        }
    }
    if total > CAP_ERRORS {
        result.push_str(&format!("...+{} more issues\n", total - CAP_ERRORS));
    }
    result.trim().to_string()
}

fn format_ruff_json(diagnostics: &[serde_json::Value]) -> String {
    if diagnostics.is_empty() {
        return "ok: 0 issues".to_string();
    }

    // Group by code
    let mut by_code: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for d in diagnostics {
        let code = d.get("code").and_then(|v| v.as_str()).unwrap_or("?");
        *by_code.entry(code.to_string()).or_insert(0) += 1;
    }

    let mut result = format!("{} issues:\n", diagnostics.len());
    let mut sorted: Vec<_> = by_code.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (code, count) in sorted.iter().take(CAP_WARNINGS) {
        result.push_str(&format!("  {} ({}x)\n", code, count));
    }
    if sorted.len() > CAP_WARNINGS {
        result.push_str(&format!("  ...+{} more rules\n", sorted.len() - CAP_WARNINGS));
    }
    result.trim().to_string()
}

fn format_ruff_format(output: &str) -> String {
    let changed: Vec<&str> = output.lines()
        .filter(|l| !l.trim().is_empty() && !l.contains("file") && !l.contains("left unchanged"))
        .collect();

    if changed.is_empty() || output.contains("left unchanged") {
        if let Some(line) = output.lines().find(|l| l.contains("file")) {
            return line.trim().to_string();
        }
        return "ok: no changes".to_string();
    }

    format!("{} files reformatted", changed.len())
}

fn format_mypy(output: &str) -> String {
    let mut by_code: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    let mut total = 0;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.contains(": error") || trimmed.contains(": note") {
            total += 1;
            // Extract error code from brackets [code]
            let code = if let Some(bracket_start) = trimmed.rfind('[') {
                if let Some(bracket_end) = trimmed.rfind(']') {
                    trimmed[bracket_start + 1..bracket_end].to_string()
                } else {
                    "other".to_string()
                }
            } else {
                "other".to_string()
            };
            by_code.entry(code).or_default().push(trimmed.to_string());
        }
    }

    // Check for summary line
    if let Some(summary) = output.lines().find(|l| l.contains("Found") && l.contains("error")) {
        if total == 0 {
            return summary.trim().to_string();
        }
    }

    if total == 0 {
        return "ok: no errors".to_string();
    }

    let mut result = format!("mypy: {} errors\n", total);
    let mut sorted: Vec<_> = by_code.iter().collect();
    sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    for (code, locations) in sorted.iter().take(CAP_WARNINGS) {
        result.push_str(&format!("  [{}] ({}x)\n", code, locations.len()));
        for loc in locations.iter().take(3) {
            result.push_str(&format!("    {}\n", loc));
        }
        if locations.len() > 3 {
            result.push_str(&format!("    ...+{} more\n", locations.len() - 3));
        }
    }
    result.trim().to_string()
}

fn format_pip(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "install" => Some(format_pip_install(output)),
        "freeze" | "list" => Some(format_pip_list(output)),
        _ => None,
    }
}

fn format_pip_install(output: &str) -> String {
    // Keep only "Successfully installed" or "Requirement already satisfied" summary
    for line in output.lines().rev() {
        if line.starts_with("Successfully installed") {
            let packages: Vec<&str> = line.strip_prefix("Successfully installed ")
                .unwrap_or(line)
                .split_whitespace()
                .collect();
            if packages.len() > 10 {
                return format!("installed {} packages: {}, ...+{}", packages.len(),
                    packages[..5].join(", "), packages.len() - 5);
            }
            return line.to_string();
        }
    }
    if output.contains("already satisfied") {
        return "ok: already satisfied".to_string();
    }
    "ok".to_string()
}

fn format_pip_list(output: &str) -> String {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= 30 {
        return output.to_string();
    }
    format!("{} packages installed\n{}\n...+{} more",
        lines.len() - 2,  // subtract header lines
        lines[..12].join("\n"),
        lines.len() - 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PYTEST_FAIL: &str = "============================= test session starts ==============================\n\
platform linux -- Python 3.12.4, pytest-8.3.2, pluggy-1.5.0\n\
rootdir: /work/api\n\
collected 42 items\n\n\
tests/test_auth.py ..........                                            [ 23%]\n\
tests/test_billing.py ........F...                                       [ 52%]\n\n\
=================================== FAILURES ===================================\n\
_______________________ test_refund_exceeds_charge_rejected ____________________\n\n\
    def test_refund_exceeds_charge_rejected():\n\
        charge = Charge(amount_cents=5000)\n\
>       with pytest.raises(RefundError):\n\
E       Failed: DID NOT RAISE <class 'billing.errors.RefundError'>\n\n\
tests/test_billing.py:71: Failed\n\
=========================== short test summary info ============================\n\
FAILED tests/test_billing.py::test_refund_exceeds_charge_rejected - Failed: DID NOT RAISE <class 'billing.errors.RefundError'>\n\
========================= 1 failed, 41 passed in 3.42s =========================\n";

    #[test]
    fn test_pytest_failure_keeps_totals_location_and_source_line() {
        let result = format_pytest(PYTEST_FAIL);
        let mut lines = result.lines();
        assert_eq!(lines.next(), Some("1 failed, 41 passed in 3.42s"));
        assert_eq!(
            lines.next(),
            Some("FAILED tests/test_billing.py::test_refund_exceeds_charge_rejected - Failed: DID NOT RAISE <class 'billing.errors.RefundError'>")
        );
        assert!(result.contains("    >       with pytest.raises(RefundError):"), "{result}");
        assert!(result.contains("    tests/test_billing.py:71: Failed"), "{result}");
        assert!(!result.contains("tests/test_auth.py .........."));
        assert!(!result.contains("charge = Charge(amount_cents=5000)"));
        assert_eq!(result.matches("DID NOT RAISE").count(), 1, "E line duplicating the summary is dropped");
    }

    #[test]
    fn test_pytest_all_pass_collapses_to_totals() {
        let output = "============================= test session starts ==============================\n\
collected 3 items\n\ntests/test_a.py ...                                                      [100%]\n\n\
============================== 3 passed in 0.12s ===============================\n";
        assert_eq!(format_pytest(output), "3 passed in 0.12s");
    }

    #[test]
    fn test_pytest_class_method_block_matches_summary() {
        let output = "=================================== FAILURES ===================================\n\
__________________________ TestBilling.test_refund_zero ________________________\n\n\
>       assert svc.refund(charge, 0) is None\n\
E       assert Refund(id=1) is None\n\n\
tests/test_billing.py:12: AssertionError\n\
=========================== short test summary info ============================\n\
FAILED tests/test_billing.py::TestBilling::test_refund_zero - assert Refund(id=1) is None\n\
============================== 1 failed in 0.05s ===============================\n";
        let result = format_pytest(output);
        assert!(result.contains("    tests/test_billing.py:12: AssertionError"), "{result}");
        assert!(result.contains("    >       assert svc.refund(charge, 0) is None"), "{result}");
    }

    #[test]
    fn test_pytest_unstructured_falls_back() {
        let output = "FAILED tests/test_foo.py::test_bar - AssertionError\n1 failed, 5 passed\n";
        let result = format_pytest(output);
        assert!(result.contains("test_bar"));
    }

    #[test]
    fn test_ruff_clean() {
        let result = format_ruff(Some("check"), "All checks passed!\n");
        assert_eq!(result, "ok: 0 issues");
    }

    #[test]
    fn test_ruff_human_readable() {
        let output = "src/main.py:10:5: E501 Line too long (120 > 100)\nsrc/main.py:15:1: F401 Unused import\nsrc/util.py:3:1: E302 Expected 2 blank lines\n";
        let result = format_ruff(Some("check"), output);
        assert!(result.contains("3 issues in 2 files"));
    }

    #[test]
    fn test_ruff_json() {
        let json = r#"[{"code":"E501","message":"Line too long","filename":"a.py","location":{"row":1,"column":1}},{"code":"E501","message":"Line too long","filename":"b.py","location":{"row":2,"column":1}}]"#;
        let result = format_ruff(Some("check"), json);
        assert!(result.contains("2 issues"));
        assert!(result.contains("E501 (2x)"));
    }

    #[test]
    fn test_mypy_clean() {
        let result = format_mypy("Success: no issues found in 10 source files\n");
        assert_eq!(result, "ok: no errors");
    }

    #[test]
    fn test_mypy_errors() {
        let output = "src/main.py:10: error: Incompatible types [assignment]\nsrc/main.py:15: error: Missing return [return]\nFound 2 errors in 1 file\n";
        let result = format_mypy(output);
        assert!(result.contains("mypy: 2 errors"));
        assert!(result.contains("[assignment]"));
    }

    #[test]
    fn test_pip_install() {
        let output = "Collecting requests\nDownloading requests-2.28.0.tar.gz\nSuccessfully installed requests-2.28.0 urllib3-1.26.9\n";
        let result = format_pip_install(output);
        assert!(result.starts_with("Successfully installed"));
    }

    #[test]
    fn test_pip_already_satisfied() {
        let output = "Requirement already satisfied: requests in ./venv/lib\n";
        let result = format_pip_install(output);
        assert_eq!(result, "ok: already satisfied");
    }
}
