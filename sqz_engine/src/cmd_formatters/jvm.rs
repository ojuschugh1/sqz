use super::truncate::CAP_ERRORS;

pub fn format_gradle(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "build" | "assemble" | "compileJava" | "compileKotlin" => Some(format_gradle_build(output)),
        "test" => Some(format_gradle_test(output)),
        _ => None,
    }
}

pub fn format_maven(subcmd: Option<&str>, output: &str) -> Option<String> {
    // Maven doesn't have clean subcommands the same way, look at output patterns
    let _ = subcmd;
    Some(format_mvn(output))
}

fn format_gradle_build(output: &str) -> String {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings = 0;
    let mut tasks_executed = 0;
    let mut actionable = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("e: ") || trimmed.starts_with("ERROR:") {
            errors.push(trimmed.to_string());
        } else if trimmed.starts_with("w: ") || trimmed.starts_with("WARNING:") {
            warnings += 1;
        } else if trimmed.contains("actionable task") {
            actionable = Some(trimmed);
        }
        if trimmed.starts_with("> Task :") {
            tasks_executed += 1;
        }
    }

    if errors.is_empty() {
        if output.contains("BUILD SUCCESSFUL") {
            if let Some(line) = actionable {
                return line.to_string();
            }
            let mut result = "BUILD SUCCESSFUL".to_string();
            if tasks_executed > 0 {
                result.push_str(&format!(" ({} tasks)", tasks_executed));
            }
            if warnings > 0 {
                result.push_str(&format!(", {} warnings", warnings));
            }
            return result;
        }
        return output.to_string();
    }

    let mut result = format!("BUILD FAILED: {} errors, {} warnings\n", errors.len(), warnings);
    for e in errors.iter().take(CAP_ERRORS) {
        result.push_str(&format!("  {}\n", e));
    }
    if errors.len() > CAP_ERRORS {
        result.push_str(&format!("  ...+{} more errors\n", errors.len() - CAP_ERRORS));
    }
    result.trim().to_string()
}

fn format_gradle_test(output: &str) -> String {
    let mut total = 0;
    let mut failed = 0;
    let mut failures: Vec<String> = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        // "X tests completed, Y failed"
        if trimmed.contains("tests completed") {
            for part in trimmed.split(',') {
                let p = part.trim();
                if p.contains("completed") {
                    total = extract_leading_number(p);
                } else if p.contains("failed") {
                    failed = extract_leading_number(p);
                }
            }
        }
        // Individual failure lines
        if trimmed.contains("FAILED") && !trimmed.contains("BUILD") {
            failures.push(trimmed.to_string());
        }
    }

    if output.contains("BUILD SUCCESSFUL") && failed == 0 {
        if total > 0 {
            return format!("ok: {} tests passed", total);
        }
        return "BUILD SUCCESSFUL".to_string();
    }

    if failed > 0 || !failures.is_empty() {
        let mut result = format!("FAILED: {} of {} tests\n", failed, total);
        for f in failures.iter().take(CAP_ERRORS) {
            result.push_str(&format!("  {}\n", f));
        }
        return result.trim().to_string();
    }

    output.to_string()
}

fn format_mvn(output: &str) -> String {
    // Strip download noise
    let mut errors: Vec<String> = Vec::new();
    let mut build_result: Option<String> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        // Skip download progress
        if trimmed.starts_with("Downloading") || trimmed.starts_with("Downloaded")
            || trimmed.starts_with("Progress") {
            continue;
        }
        if trimmed.contains("[ERROR]") {
            errors.push(trimmed.to_string());
        }
        if trimmed.contains("BUILD SUCCESS") || trimmed.contains("BUILD FAILURE") {
            build_result = Some(trimmed.to_string());
        }
    }

    if let Some(ref result_line) = build_result {
        if errors.is_empty() {
            return result_line.clone();
        }
        let mut result = format!("{}\n", result_line);
        for e in errors.iter().take(CAP_ERRORS) {
            result.push_str(&format!("  {}\n", e));
        }
        if errors.len() > CAP_ERRORS {
            result.push_str(&format!("  ...+{} more errors\n", errors.len() - CAP_ERRORS));
        }
        return result.trim().to_string();
    }

    if errors.is_empty() {
        return output.to_string();
    }

    format!("{} errors:\n{}", errors.len(),
        errors.iter().take(CAP_ERRORS).cloned().collect::<Vec<_>>().join("\n"))
}

fn extract_leading_number(s: &str) -> usize {
    let num_str: String = s.chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    num_str.parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gradle_build_success() {
        let output = "> Task :compileJava\n> Task :classes\n\nBUILD SUCCESSFUL in 3s\n2 actionable tasks: 2 executed\n";
        let result = format_gradle_build(output);
        assert!(result.contains("2 actionable tasks"));
    }

    #[test]
    fn test_gradle_build_errors() {
        let output = "> Task :compileJava\ne: src/Main.java:10: error: cannot find symbol\ne: src/Main.java:15: error: incompatible types\n\nBUILD FAILED\n";
        let result = format_gradle_build(output);
        assert!(result.contains("2 errors"));
    }

    #[test]
    fn test_gradle_test_pass() {
        let output = "> Task :test\n\nBUILD SUCCESSFUL in 5s\n50 tests completed, 0 failed\n";
        let result = format_gradle_test(output);
        assert!(result.contains("50 tests passed"));
    }

    #[test]
    fn test_mvn_strips_downloads() {
        let output = "Downloading from central: https://repo.maven.org/maven2/foo.jar\nDownloaded from central: https://repo.maven.org/maven2/foo.jar\n[INFO] BUILD SUCCESS\n";
        let result = format_mvn(output);
        assert_eq!(result, "[INFO] BUILD SUCCESS");
        assert!(!result.contains("Downloading"));
    }

    #[test]
    fn test_mvn_errors() {
        let output = "[ERROR] src/Main.java:[10,5] cannot find symbol\n[ERROR] src/Main.java:[15,3] incompatible types\n[INFO] BUILD FAILURE\n";
        let result = format_mvn(output);
        assert!(result.contains("BUILD FAILURE"));
        assert!(result.contains("cannot find symbol"));
    }
}
