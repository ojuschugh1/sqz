//! dotnet CLI formatter (MSBuild + VSTest output).

use super::truncate::CAP_ERRORS;

pub fn format_dotnet(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "build" | "publish" => Some(format_build(output)),
        "test" => Some(format_test(output)),
        _ => None,
    }
}

fn format_build(output: &str) -> String {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings = 0usize;

    for line in output.lines() {
        let trimmed = line.trim();
        // "/src/File.cs(12,34): error CS1002: ; expected [/src/App.csproj]"
        if trimmed.contains(": error ") {
            errors.push(trimmed.to_string());
        } else if trimmed.contains(": warning ") {
            warnings += 1;
        }
    }

    if output.contains("Build succeeded") && errors.is_empty() {
        let elapsed = output
            .lines()
            .find(|l| l.trim().starts_with("Time Elapsed"))
            .map(|l| l.trim().trim_start_matches("Time Elapsed").trim().to_string());
        let mut out = "dotnet build: succeeded".to_string();
        if warnings > 0 {
            out.push_str(&format!(", {warnings} warnings"));
        }
        if let Some(t) = elapsed {
            out.push_str(&format!(" ({t})"));
        }
        return out;
    }

    if errors.is_empty() {
        return output
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ok")
            .trim()
            .to_string();
    }

    let mut out = format!(
        "dotnet build: FAILED — {} errors, {} warnings\n",
        errors.len(),
        warnings
    );
    // MSBuild repeats each diagnostic once per target framework; dedup.
    errors.dedup();
    for e in errors.iter().take(CAP_ERRORS) {
        out.push_str(&format!("  {e}\n"));
    }
    if errors.len() > CAP_ERRORS {
        out.push_str(&format!("  ...+{} more errors\n", errors.len() - CAP_ERRORS));
    }
    out.trim_end().to_string()
}

fn format_test(output: &str) -> String {
    let mut summary: Option<String> = None;
    let mut failed_blocks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;

    for line in output.lines() {
        let trimmed = line.trim_end();
        let t = trimmed.trim();
        // "Passed!  - Failed: 0, Passed: 12, ..." / "Failed! - Failed: 2, ..."
        if (t.starts_with("Passed!") || t.starts_with("Failed!")) && t.contains("Total:") {
            summary = Some(t.to_string());
            continue;
        }
        if t.starts_with("Failed ") && t.ends_with(']') {
            if let Some(block) = current.take() {
                failed_blocks.push(block);
            }
            current = Some(t.to_string());
            continue;
        }
        if let Some(block) = current.as_mut() {
            if t.starts_with("Error Message:")
                || t.starts_with("Stack Trace:")
                || t.starts_with("Assert.")
                || t.starts_with("Expected:")
                || t.starts_with("Actual:")
                || t.starts_with("at ")
            {
                block.push_str(&format!("\n    {t}"));
            } else if t.is_empty() {
                failed_blocks.push(current.take().unwrap());
            }
        }
    }
    if let Some(block) = current.take() {
        failed_blocks.push(block);
    }

    let mut out = String::new();
    match summary {
        Some(s) => out.push_str(&format!("dotnet test: {s}\n")),
        None => {
            return output
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("ok")
                .trim()
                .to_string();
        }
    }
    for block in failed_blocks.iter().take(CAP_ERRORS) {
        out.push_str(&format!("  {block}\n"));
    }
    if failed_blocks.len() > CAP_ERRORS {
        out.push_str(&format!(
            "  ...+{} more failed tests\n",
            failed_blocks.len() - CAP_ERRORS
        ));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_failure_keeps_diagnostics_dedups_frameworks() {
        let output = "\
  Determining projects to restore...
  Restored /src/App/App.csproj (in 312 ms).
  App -> /src/App/bin/Debug/net8.0/App.dll
/src/App/Login.cs(42,13): error CS1002: ; expected [/src/App/App.csproj]
/src/App/Login.cs(42,13): error CS1002: ; expected [/src/App/App.csproj]
/src/App/Feed.cs(7,9): warning CS0168: The variable 'x' is declared but never used [/src/App/App.csproj]

Build FAILED.

    1 Warning(s)
    2 Error(s)

Time Elapsed 00:00:02.84
";
        let result = format_dotnet(Some("build"), output).unwrap();
        assert!(result.starts_with("dotnet build: FAILED"));
        assert_eq!(result.matches("CS1002").count(), 1, "per-framework duplicate must fold");
        assert!(!result.contains("Determining projects"), "restore noise dropped");
        assert!(result.len() < output.len());
    }

    #[test]
    fn build_success_is_one_line() {
        let output = "\
  Determining projects to restore...
  All projects are up-to-date for restore.
  App -> /src/App/bin/Debug/net8.0/App.dll

Build succeeded.
    0 Warning(s)
    0 Error(s)

Time Elapsed 00:00:01.10
";
        let result = format_dotnet(Some("build"), output).unwrap();
        assert_eq!(result, "dotnet build: succeeded (00:00:01.10)");
    }

    #[test]
    fn test_failures_keep_message_and_summary() {
        let output = "\
  Determining projects to restore...
Passed TestLoginHappyPath [42 ms]
Failed TestSessionRefresh [118 ms]
  Error Message:
   Assert.Equal() Failure
  Expected: 200
  Actual:   401

Failed! - Failed:     1, Passed:    11, Skipped:     0, Total:    12, Duration: 512 ms - App.Tests.dll (net8.0)
";
        let result = format_dotnet(Some("test"), output).unwrap();
        assert!(result.starts_with("dotnet test: Failed! - Failed:     1"));
        assert!(result.contains("TestSessionRefresh"));
        assert!(result.contains("Expected: 200"));
        assert!(!result.contains("TestLoginHappyPath"), "passing tests dropped");
    }

    #[test]
    fn unknown_subcommand_falls_through() {
        assert!(format_dotnet(Some("run"), "hello").is_none());
        assert!(format_dotnet(None, "hello").is_none());
    }
}
