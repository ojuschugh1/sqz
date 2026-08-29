//! xcodebuild formatter. Apple's build tool prints every compiler
//! invocation, environment export, and phase banner — routinely tens of
//! thousands of tokens for a build whose useful content is a handful of
//! diagnostics and one verdict line.

use super::truncate::CAP_ERRORS;

pub fn format_xcodebuild(output: &str) -> String {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings = 0usize;
    let mut failed_commands: Vec<String> = Vec::new();
    let mut in_failed_block = false;
    let mut test_summary: Option<String> = None;
    let mut failed_tests: Vec<String> = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("The following build commands failed:") {
            in_failed_block = true;
            continue;
        }
        if in_failed_block {
            if trimmed.is_empty() || trimmed.starts_with('(') {
                in_failed_block = false;
            } else if failed_commands.len() < CAP_ERRORS {
                failed_commands.push(trimmed.to_string());
            }
            continue;
        }

        // "/path/File.swift:12:5: error: message" and bare "error: message"
        if trimmed.contains(": error:") || trimmed.starts_with("error: ") {
            errors.push(trimmed.to_string());
        } else if trimmed.contains(": warning:") || trimmed.starts_with("warning: ") {
            warnings += 1;
        } else if trimmed.starts_with("Executed ") && trimmed.contains(" test") {
            // "Executed 12 tests, with 2 failures (0 unexpected) in 0.421 seconds"
            test_summary = Some(trimmed.to_string());
        } else if trimmed.starts_with("Test Case '") && trimmed.contains("' failed") {
            failed_tests.push(trimmed.to_string());
        }
    }

    let succeeded = output.contains("** BUILD SUCCEEDED **");
    let failed = output.contains("** BUILD FAILED **")
        || output.contains("** TEST FAILED **")
        || !errors.is_empty();

    let mut out = String::new();
    if failed {
        out.push_str(&format!(
            "xcodebuild: FAILED — {} errors, {} warnings\n",
            errors.len(),
            warnings
        ));
        for e in errors.iter().take(CAP_ERRORS) {
            out.push_str(&format!("  {e}\n"));
        }
        if errors.len() > CAP_ERRORS {
            out.push_str(&format!("  ...+{} more errors\n", errors.len() - CAP_ERRORS));
        }
        if !failed_commands.is_empty() {
            out.push_str("  failed commands:\n");
            for c in &failed_commands {
                out.push_str(&format!("    {c}\n"));
            }
        }
    } else if succeeded {
        out.push_str("xcodebuild: BUILD SUCCEEDED");
        if warnings > 0 {
            out.push_str(&format!(" ({warnings} warnings)"));
        }
        out.push('\n');
    }

    if let Some(summary) = test_summary {
        out.push_str(&format!("{summary}\n"));
        for t in failed_tests.iter().take(CAP_ERRORS) {
            out.push_str(&format!("  {t}\n"));
        }
    }

    if out.is_empty() {
        // Nothing recognized — return the last non-empty line rather
        // than guessing.
        return output
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ok")
            .trim()
            .to_string();
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_build_keeps_errors_and_failed_commands() {
        let output = "\
Build settings from command line:
    SDKROOT = iphonesimulator17.5

CompileSwift normal arm64 /Users/dev/App/Sources/Login.swift (in target 'App' from project 'App')
    cd /Users/dev/App
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
/Users/dev/App/Sources/Login.swift:42:17: error: value of type 'Session' has no member 'tokn'
/Users/dev/App/Sources/Login.swift:12:5: warning: initialization of immutable value 'x' was never used
CompileSwift normal arm64 /Users/dev/App/Sources/Feed.swift
Ld /Users/dev/build/App.build/Binary/App normal
** BUILD FAILED **

The following build commands failed:
\tCompileSwift normal arm64 /Users/dev/App/Sources/Login.swift
(1 failure)
";
        let result = format_xcodebuild(output);
        assert!(result.starts_with("xcodebuild: FAILED — 1 errors, 1 warnings"));
        assert!(result.contains("no member 'tokn'"));
        assert!(result.contains("failed commands:"));
        assert!(result.contains("CompileSwift normal arm64 /Users/dev/App/Sources/Login.swift"));
        assert!(!result.contains("export DEVELOPER_DIR"), "env noise must be dropped");
        assert!(result.len() < output.len());
    }

    #[test]
    fn successful_build_collapses_to_verdict() {
        let output = "\
CompileSwift normal arm64 /Users/dev/App/Sources/Main.swift
    cd /Users/dev/App
CodeSign /Users/dev/build/App.app
/Users/dev/App/Sources/Main.swift:3:5: warning: unused variable 'y'
** BUILD SUCCEEDED **
";
        let result = format_xcodebuild(output);
        assert_eq!(result, "xcodebuild: BUILD SUCCEEDED (1 warnings)");
    }

    #[test]
    fn test_run_keeps_summary_and_failures() {
        let output = "\
Test Suite 'All tests' started at 2026-08-29 10:00:00.000
Test Case '-[AppTests testLoginHappyPath]' passed (0.021 seconds).
Test Case '-[AppTests testSessionRefresh]' failed (0.180 seconds).
Executed 12 tests, with 1 failure (0 unexpected) in 0.421 (0.430) seconds
** TEST FAILED **
";
        let result = format_xcodebuild(output);
        assert!(result.contains("Executed 12 tests, with 1 failure"));
        assert!(result.contains("testSessionRefresh"));
        assert!(!result.contains("testLoginHappyPath"), "passing tests are noise");
    }
}
