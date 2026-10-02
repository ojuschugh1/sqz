use super::truncate::{CAP_ERRORS, CAP_WARNINGS};

pub fn format_cargo(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "test" | "nextest" => Some(format_cargo_test(output)),
        "build" | "check" => Some(format_cargo_build(output)),
        "clippy" => Some(format_cargo_clippy(output)),
        _ => None,
    }
}

fn is_noise_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("Compiling")
        || trimmed.starts_with("Checking")
        || trimmed.starts_with("Downloading")
        || trimmed.starts_with("Downloaded")
        || trimmed.starts_with("Fetching")
}

/// What `collect_build_blocks` scraped from a `cargo build` run.
struct BuildBlocks {
    compiled: usize,
    warning_count: usize,
    finished_line: Option<String>,
    error_blocks: Vec<Vec<String>>,
}

/// Walk the build output once: count compiled crates, gather each error/warning
/// diagnostic into its own block, and capture the trailing "Finished" line.
fn collect_build_blocks(output: &str) -> BuildBlocks {
    let mut compiled = 0;
    let mut error_blocks: Vec<Vec<String>> = Vec::new();
    let mut warning_count = 0;
    let mut current_block: Vec<String> = Vec::new();
    let mut in_block = false;
    let mut finished_line: Option<String> = None;

    for line in output.lines() {
        let trimmed = line.trim_start();

        if is_noise_line(line) {
            compiled += 1;
            continue;
        }
        if trimmed.starts_with("Finished") {
            finished_line = Some(trimmed.to_string());
            continue;
        }
        // Skip aggregate noise lines
        if (line.contains("generated") && line.contains("warning"))
            || line.contains("aborting due to")
            || line.contains("could not compile")
        {
            continue;
        }

        let is_error = line.starts_with("error:") || line.starts_with("error[");
        let is_warning = line.starts_with("warning:") || line.starts_with("warning[");

        if is_error || is_warning {
            if in_block && !current_block.is_empty() {
                error_blocks.push(std::mem::take(&mut current_block));
            }
            if is_warning {
                warning_count += 1;
            }
            in_block = true;
            current_block.push(line.to_string());
        } else if in_block {
            if line.trim().is_empty() && current_block.len() > 3 {
                error_blocks.push(std::mem::take(&mut current_block));
                in_block = false;
            } else if current_block.len() < 15 {
                current_block.push(line.to_string());
            }
        }
    }
    if !current_block.is_empty() {
        error_blocks.push(current_block);
    }

    BuildBlocks {
        compiled,
        warning_count,
        finished_line,
        error_blocks,
    }
}

fn format_cargo_build(output: &str) -> String {
    let blocks = collect_build_blocks(output);

    let error_count = blocks
        .error_blocks
        .iter()
        .filter(|b| b.first().map(|l| l.starts_with("error")).unwrap_or(false))
        .count();

    if error_count == 0 && blocks.warning_count == 0 {
        let mut s = format!("ok ({} crates compiled)", blocks.compiled);
        if let Some(ref finished) = blocks.finished_line {
            s = format!("{}\n{}", s, finished);
        }
        return s;
    }

    let mut result = format!(
        "cargo build: {} errors, {} warnings ({} crates)\n",
        error_count, blocks.warning_count, blocks.compiled
    );
    for blk in blocks.error_blocks.iter().take(CAP_ERRORS) {
        result.push_str(&blk.join("\n"));
        result.push('\n');
    }
    if blocks.error_blocks.len() > CAP_ERRORS {
        result.push_str(&format!(
            "...+{} more issues\n",
            blocks.error_blocks.len() - CAP_ERRORS
        ));
    }
    result.trim().to_string()
}

fn format_cargo_test(output: &str) -> String {
    let mut failures: Vec<String> = Vec::new();
    let mut summary_lines: Vec<String> = Vec::new();
    let mut in_failure_section = false;
    let mut in_failure_names = false;
    let mut current_failure: Vec<String> = Vec::new();

    for line in output.lines() {
        if is_noise_line(line) || line.trim_start().starts_with("Finished") {
            continue;
        }
        if line.starts_with("running ") {
            continue;
        }
        if line.starts_with("test ") && line.ends_with("... ok") {
            continue;
        }

        if line == "failures:" {
            if in_failure_section {
                in_failure_names = true;
            }
            in_failure_section = true;
            continue;
        }

        if in_failure_names {
            if line.starts_with("test result:") {
                in_failure_names = false;
                in_failure_section = false;
                summary_lines.push(line.to_string());
            }
            continue;
        }

        if line.starts_with("test result:") {
            summary_lines.push(line.to_string());
            in_failure_section = false;
            continue;
        }

        if in_failure_section {
            if line.starts_with("---- ") {
                if !current_failure.is_empty() {
                    failures.push(current_failure.join("\n"));
                    current_failure.clear();
                }
                current_failure.push(line.to_string());
            } else if line.trim().is_empty() {
                if !current_failure.is_empty() {
                    failures.push(current_failure.join("\n"));
                    current_failure.clear();
                }
            } else if !line.trim().is_empty() {
                current_failure.push(line.to_string());
            }
        }
    }
    if !current_failure.is_empty() {
        failures.push(current_failure.join("\n"));
    }

    if failures.is_empty() {
        if !summary_lines.is_empty() {
            if let Some(agg) = aggregate_test_results(&summary_lines) {
                return agg;
            }
            return summary_lines.join("\n");
        }
        let total = output.lines().filter(|l| l.contains("... ok") || l.contains("passed")).count();
        if total > 0 {
            return format!("ok: {} tests passed", total);
        }

        // Might be compilation errors
        let has_compile_errors = output.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("error[") || t.starts_with("error:")
        });
        if has_compile_errors {
            return format_cargo_build(output);
        }

        return output.to_string();
    }

    let mut result = Vec::new();
    result.push(format!("FAILURES ({}):", failures.len()));
    // One line per failing test, so the list survives the block cap.
    for line in output.lines().filter(|l| l.starts_with("test ") && l.trim_end().ends_with("... FAILED")) {
        result.push(line.to_string());
    }
    for f in failures.iter().take(CAP_WARNINGS) {
        result.push(f.clone());
    }
    if failures.len() > CAP_WARNINGS {
        result.push(format!("...+{} more failures", failures.len() - CAP_WARNINGS));
    }
    if !summary_lines.is_empty() {
        result.push(summary_lines.last().unwrap().clone());
    }
    // `error: test failed, to rerun pass ...`
    for line in output.lines().filter(|l| l.starts_with("error: test failed")) {
        result.push(line.to_string());
    }
    result.join("\n")
}

fn aggregate_test_results(summary_lines: &[String]) -> Option<String> {
    let mut total_passed: usize = 0;
    let mut total_failed: usize = 0;
    let mut total_ignored: usize = 0;
    let mut suites: usize = 0;

    for line in summary_lines {
        if !line.starts_with("test result:") {
            continue;
        }
        suites += 1;
        for part in line.split(';') {
            let trimmed = part.trim();
            if let Some(num) = extract_number_before(trimmed, "passed") {
                total_passed += num;
            } else if let Some(num) = extract_number_before(trimmed, "failed") {
                total_failed += num;
            } else if let Some(num) = extract_number_before(trimmed, "ignored") {
                total_ignored += num;
            }
        }
    }

    if suites == 0 {
        return None;
    }

    let mut parts = vec![format!("{} passed", total_passed)];
    if total_failed > 0 {
        parts.push(format!("{} failed", total_failed));
    }
    if total_ignored > 0 {
        parts.push(format!("{} ignored", total_ignored));
    }

    let suite_text = if suites == 1 {
        "1 suite".to_string()
    } else {
        format!("{} suites", suites)
    };

    Some(format!("cargo test: {} ({})", parts.join(", "), suite_text))
}

fn extract_number_before(text: &str, word: &str) -> Option<usize> {
    let pos = text.find(word)?;
    let before = text[..pos].trim();
    // Get the last whitespace-separated token before the word
    before.rsplit_once(|c: char| !c.is_ascii_digit())
        .map(|(_, n)| n)
        .unwrap_or(before)
        .parse()
        .ok()
}

fn format_cargo_clippy(output: &str) -> String {
    let mut by_rule: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    let mut error_blocks: Vec<Vec<String>> = Vec::new();
    let mut warning_count = 0;
    let mut current_block: Vec<String> = Vec::new();
    let mut current_rule = String::new();
    let mut in_error = false;

    for line in output.lines() {
        if is_noise_line(line) || line.trim_start().starts_with("Finished") {
            if in_error && !current_block.is_empty() {
                error_blocks.push(std::mem::take(&mut current_block));
                in_error = false;
            }
            continue;
        }
        if (line.contains("generated") && line.contains("warning"))
            || line.contains("aborting due to")
            || line.contains("could not compile")
        {
            continue;
        }

        let is_error_line = line.starts_with("error:") || line.starts_with("error[");
        let is_warning_line = line.starts_with("warning:") || line.starts_with("warning[");

        if is_error_line || is_warning_line {
            if in_error && !current_block.is_empty() {
                error_blocks.push(std::mem::take(&mut current_block));
            }
            in_error = false;

            // Extract rule name from brackets
            current_rule = extract_bracket_content(line)
                .unwrap_or_else(|| {
                    let prefix = if is_error_line { "error: " } else { "warning: " };
                    line.strip_prefix(prefix).unwrap_or(line).to_string()
                });

            if is_error_line {
                in_error = true;
                current_block.push(line.to_string());
            } else {
                warning_count += 1;
            }
        } else if line.trim_start().starts_with("--> ") {
            let location = line.trim_start().strip_prefix("--> ").unwrap_or(line).to_string();
            if !current_rule.is_empty() {
                by_rule.entry(current_rule.clone()).or_default().push(location);
            }
            if in_error {
                current_block.push(line.to_string());
            }
        } else if in_error {
            if line.trim().is_empty() {
                if !current_block.is_empty() {
                    error_blocks.push(std::mem::take(&mut current_block));
                }
                in_error = false;
            } else if current_block.len() < 15 {
                current_block.push(line.to_string());
            }
        }
    }
    if in_error && !current_block.is_empty() {
        error_blocks.push(current_block);
    }

    let error_count = error_blocks.len();
    if error_count == 0 && warning_count == 0 {
        return "ok: no issues".to_string();
    }

    let mut result = format!("cargo clippy: {} errors, {} warnings\n", error_count, warning_count);

    if !error_blocks.is_empty() {
        for blk in error_blocks.iter().take(CAP_ERRORS) {
            result.push_str(&blk.join("\n"));
            result.push('\n');
        }
        if error_blocks.len() > CAP_ERRORS {
            result.push_str(&format!("...+{} more errors\n", error_blocks.len() - CAP_ERRORS));
        }
    }

    // Sort warnings by frequency
    let mut rule_counts: Vec<_> = by_rule.iter().collect();
    rule_counts.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    for (rule, locations) in rule_counts.iter().take(CAP_WARNINGS) {
        result.push_str(&format!("  {} ({}x)\n", rule, locations.len()));
        for loc in locations.iter().take(3) {
            result.push_str(&format!("    {}\n", loc));
        }
        if locations.len() > 3 {
            result.push_str(&format!("    ...+{} more\n", locations.len() - 3));
        }
    }
    if rule_counts.len() > CAP_WARNINGS {
        result.push_str(&format!("...+{} more rules\n", rule_counts.len() - CAP_WARNINGS));
    }

    result.trim().to_string()
}

fn extract_bracket_content(line: &str) -> Option<String> {
    let start = line.rfind('[')?;
    let end = line.rfind(']')?;
    if end > start {
        Some(line[start + 1..end].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cargo_build_success() {
        let output = "   Compiling sqz v1.0.0\n   Compiling sqz-engine v1.0.0\n    Finished dev [unoptimized + debuginfo] target(s) in 2.5s\n";
        let result = format_cargo_build(output);
        assert!(result.contains("2 crates compiled"));
        assert!(result.contains("Finished"));
    }

    #[test]
    fn test_cargo_build_errors() {
        let output = "   Compiling sqz v1.0.0\nerror[E0308]: mismatched types\n  --> src/main.rs:10:5\n  |\n10 |     foo()\n  |     ^^^^^ expected u32, found &str\n\nerror: aborting due to 1 previous error\n";
        let result = format_cargo_build(output);
        assert!(result.contains("1 errors"));
        assert!(result.contains("E0308"));
        assert!(result.contains("src/main.rs:10:5"));
        assert!(!result.contains("Compiling"));
        assert!(!result.contains("aborting due to"));
    }

    #[test]
    fn test_cargo_test_all_pass() {
        let output = "   Compiling sqz v1.0.0\n    Finished test [unoptimized + debuginfo] target(s)\n     Running unittests src/lib.rs\n\nrunning 15 tests\ntest a ... ok\ntest b ... ok\ntest result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let result = format_cargo_test(output);
        assert!(result.contains("15 passed"));
        assert!(!result.contains("Compiling"));
        assert!(!result.contains("FAILURES"));
    }

    #[test]
    fn test_cargo_test_multi_suite() {
        let output = "running 5 tests\ntest a ... ok\ntest result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n\nrunning 3 tests\ntest b ... ok\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let result = format_cargo_test(output);
        assert!(result.contains("8 passed"));
        assert!(result.contains("2 suites"));
    }

    #[test]
    fn test_cargo_test_with_failure() {
        let output = "running 3 tests\ntest a ... ok\ntest b ... FAILED\ntest c ... ok\n\nfailures:\n\n---- b stdout ----\nthread 'b' panicked at 'assertion failed'\nnote: run with RUST_BACKTRACE=1\n\nfailures:\n    b\n\ntest result: FAILED. 2 passed; 1 failed; 0 ignored\n";
        let result = format_cargo_test(output);
        assert!(result.contains("FAILURES (1):"));
        assert!(result.contains("assertion failed"));
    }

    #[test]
    fn test_cargo_clippy_clean() {
        let output = "    Checking sqz v1.0.0\n    Finished dev profile\n";
        let result = format_cargo_clippy(output);
        assert_eq!(result, "ok: no issues");
    }

    #[test]
    fn test_cargo_clippy_warnings() {
        let output = "warning: unused variable: `x`\n  --> src/lib.rs:10:9\n  |\n10 |     let x = 5;\n  |         ^ help: use `_x`\n  |\n  = note: `#[warn(unused_variables)]` on by default [unused_variables]\n\nwarning: `sqz` (lib) generated 1 warning\n    Finished dev profile\n";
        let result = format_cargo_clippy(output);
        assert!(result.contains("0 errors, 1 warnings"));
    }

    #[test]
    fn test_extract_number_before() {
        assert_eq!(extract_number_before("15 passed", "passed"), Some(15));
        assert_eq!(extract_number_before("ok. 3 failed", "failed"), Some(3));
        assert_eq!(extract_number_before("no number here", "passed"), None);
    }
}
