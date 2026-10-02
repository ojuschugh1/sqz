mod preprocess;
mod truncate;
mod test_output;
mod git;
mod cargo;
mod npm;
mod docker;
mod kubectl;
mod system;
mod js;
mod lint;
mod python;
mod go;
mod cloud;
mod jvm;
mod ruby;
mod xcode;
mod android;
mod dotnet;
#[cfg(test)]
mod props;

pub fn format_command(cmd: &str, output: &str) -> Option<String> {
    let cleaned = preprocess::clean(output);
    let input = if cleaned.len() < output.len() { &cleaned } else { output };

    let formatted = dispatch(cmd, input)?;
    if drops_error_lines(input, &formatted) {
        return None;
    }
    Some(formatted)
}

/// A formatter may summarize success, but every line reporting a failure
/// has to come through, unless the formatter says it capped them. Lines
/// that only restate the failure (`Build FAILED.`, `could not compile`) may
/// go as long as the output still says it failed.
fn drops_error_lines(input: &str, formatted: &str) -> bool {
    if formatted.contains("...+") && !input.contains("...+") {
        return false;
    }
    let lower = formatted.to_ascii_lowercase();
    let says_failed = lower.contains("fail") || lower.contains("error");
    input.lines().filter(|l| is_error_line(l)).any(|l| {
        let t = l.trim();
        if is_failure_summary(t) {
            !says_failed
        } else {
            !formatted.contains(t)
        }
    })
}

fn is_failure_summary(line: &str) -> bool {
    const SUMMARIES: &[&str] = &[
        "could not compile", "aborting due to", "previous error", "build failed",
        "build failure", "a complete log of this run", "test result: failed",
    ];
    let lower = line.to_ascii_lowercase();
    (line.len() <= 40 && (lower.contains("failed") || lower.contains("failure")))
        || SUMMARIES.iter().any(|s| lower.contains(s))
}

/// Narrower than a substring match on "error", so `Compiling thiserror`
/// and `7 passed; 0 failed` don't count.
fn is_error_line(line: &str) -> bool {
    let t = line.trim_start();
    const STARTS: &[&str] = &[
        "error", "Error", "ERROR", "fatal", "Fatal", "FATAL", "FAILURE", "FAILED", "panic",
        "Traceback", "Exception", "npm ERR!", "npm error", "ERR_", "CONFLICT", "! [rejected]",
        "! [remote rejected]", "denied:", "[ERROR]",
    ];
    const CONTAINS: &[&str] = &[
        ": error", ": fatal", "error[E", " FAILED", "Execution failed", "failed to ",
        "Failed to ", "panicked at", "did not complete successfully", "An error occurred",
        "Exception in thread", "Automatic merge failed",
    ];
    // Whole words only: `Errors  Files` and `ExceptionHandler` don't count.
    let starts = STARTS.iter().any(|p| {
        t.starts_with(p)
            && !(p.ends_with(|c: char| c.is_ascii_alphabetic())
                && t[p.len()..].starts_with(|c: char| c.is_ascii_alphanumeric()))
    });
    starts || CONTAINS.iter().any(|p| t.contains(p))
}

fn dispatch(cmd: &str, output: &str) -> Option<String> {
    let mut parts: Vec<&str> = cmd.split_whitespace().collect();

    // `bundle exec rspec ...` / `bin/rails ...` — unwrap the bundler prefix so
    // the real Ruby tool is dispatched. `bundle install/update` is left intact.
    if parts.first().map(|s| s.rsplit('/').next().unwrap_or(s)) == Some("bundle")
        && parts.get(1) == Some(&"exec")
    {
        parts.drain(0..2);
    }

    let base = parts.first().map(|s| s.rsplit('/').next().unwrap_or(s)).unwrap_or("");

    match base {
        // Version control
        "git" => git::format_git(parts.get(1).copied(), output),

        // Rust
        "cargo" => cargo::format_cargo(parts.get(1).copied(), output),

        // JavaScript/TypeScript
        "npm" | "npx" => npm::format_npm(parts.get(1).copied(), output),
        "pnpm" => npm::format_pnpm(parts.get(1).copied(), output),
        "yarn" | "bun" => npm::format_npm(parts.get(1).copied(), output),
        "tsc" => Some(js::format_tsc(output)),
        "eslint" | "biome" => Some(lint::format_lint(output)),

        // Python
        "pytest" => Some(python::format_pytest(output)),
        "python" | "python3" if cmd.contains("pytest") || cmd.contains("-m pytest") => {
            Some(python::format_pytest(output))
        }
        "ruff" => python::format_python(cmd, parts.get(1).copied(), output),
        "mypy" => python::format_python(cmd, None, output),
        "pip" | "pip3" => python::format_python(cmd, parts.get(1).copied(), output),

        // Go
        "go" => go::format_go(parts.get(1).copied(), output),
        "golangci-lint" => Some(lint::format_lint(output)),

        // JVM
        "gradle" | "gradlew" | "./gradlew" => jvm::format_gradle(parts.get(1).copied(), output),
        "mvn" | "maven" => jvm::format_maven(parts.get(1).copied(), output),

        // Apple / Android / .NET
        "xcodebuild" => Some(xcode::format_xcodebuild(output)),
        "adb" if parts.get(1) == Some(&"logcat") => Some(android::format_logcat(output)),
        "logcat" => Some(android::format_logcat(output)),
        "dotnet" => dotnet::format_dotnet(parts.get(1).copied(), output),
        // Ruby
        "rspec" => Some(ruby::format_rspec(output)),
        "rubocop" => Some(ruby::format_rubocop(output)),
        "rake" | "rails" => ruby::format_rake(parts.get(1).copied(), output),
        "bundle" | "bundler" => ruby::format_bundle(parts.get(1).copied(), output),

        // Containers / orchestration
        "docker" | "podman" => docker::format_docker(parts.get(1).copied(), output),
        "kubectl" => kubectl::format_kubectl(parts.get(1).copied(), output),

        // Infrastructure
        "terraform" | "tf" => cloud::format_terraform(parts.get(1).copied(), output),

        // System utilities
        "grep" | "rg" | "ag" => system::format_grep(output),
        "tree" => Some(system::format_tree(output)),
        "curl" | "wget" => Some(system::format_curl(output)),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_lines_are_whole_words() {
        for line in [
            "error[E0425]: cannot find value `x` in this scope",
            "Error from server (NotFound): namespaces \"prod\" not found",
            "npm error code ERESOLVE",
            "fatal: not a git repository",
            "src/a.ts(3,7): error TS2322: Type 'string' is not assignable",
            "thread 'main' panicked at src/main.rs:3:5:",
            " ! [rejected]        main -> main (fetch first)",
        ] {
            assert!(is_error_line(line), "{line}");
        }
        for line in [
            "   Compiling thiserror v1.0.63",
            "Errors  Files",
            "test result: ok. 7 passed; 0 failed",
            "ExceptionHandler.java compiled",
        ] {
            assert!(!is_error_line(line), "{line}");
        }
    }

    #[test]
    fn test_formatter_that_drops_an_error_is_not_used() {
        // A summary formatter that answers "ok" would hide the failure.
        let failed = "Collecting torch==9.0\nERROR: No matching distribution found for torch==9.0\n";
        assert!(drops_error_lines(failed, "ok"));
        assert!(!drops_error_lines(failed, "ERROR: No matching distribution found for torch==9.0"));
        // Restating lines may go when the output still says it failed.
        let build = "x.cs(1,1): error CS0103: nope\nBuild FAILED.\n";
        assert!(!drops_error_lines(build, "dotnet build: FAILED\n  x.cs(1,1): error CS0103: nope"));
        assert!(drops_error_lines("Build FAILED.\n", "done"));
    }

    #[test]
    fn test_format_command_routing() {
        assert!(format_command("git status", "nothing to commit").is_some());
        assert!(format_command("cargo test", "test result: ok").is_some());
        assert!(format_command("unknown_tool", "output").is_none());
    }

    #[test]
    fn test_ansi_stripped_before_dispatch() {
        let output = "\x1b[32mOn branch main\x1b[0m\nnothing to commit, working tree clean\n";
        let result = format_command("git status", output);
        assert_eq!(result, Some("clean".to_string()));
    }

    #[test]
    fn test_new_commands_dispatch() {
        assert!(format_command("ruff check", "All checks passed!").is_some());
        assert!(format_command("go test ./...", "ok").is_some());
        let grep: String = (0..10).map(|i| format!("src/main.rs:{i}:let x = compute();\n")).collect();
        assert!(format_command("grep -rn compute src", &grep).is_some());
        assert!(format_command("tree", ".\n├── src\n└── Cargo.toml").is_some());
        assert!(format_command("curl http://example.com", "response").is_some());
    }

    #[test]
    fn test_data_commands_are_left_to_the_pipeline() {
        // Their output is the data the agent asked for: JSON, YAML, tables,
        // diffs, file lists. The generic pipeline keeps all of it.
        for (cmd, out) in [
            ("gh pr view 12 --json body", r#"{"body":"long text","number":12}"#),
            ("gh pr diff 12", "diff --git a/x b/x\n"),
            ("aws ec2 describe-instances", r#"{"Reservations":[]}"#),
            ("gcloud compute instances list", "NAME ZONE\nweb us-east1-b\n"),
            ("kubectl get pods -o json", r#"{"kind":"PodList","items":[]}"#),
            ("ls -la", "total 0\n-rw-r--r--  1 dev  staff  0 Sep 1 12:00 a\n"),
            ("find . -name '*.rs'", "./a.rs\n./b.rs\n"),
        ] {
            assert!(format_command(cmd, out).is_none(), "{cmd}");
        }
    }

    #[test]
    fn test_mobile_dotnet_dispatch() {
        assert!(format_command("xcodebuild -scheme App", "** BUILD SUCCEEDED **").is_some());
        assert!(format_command("adb logcat -d", "08-29 10:00:01.000  1  1 E Tag: boom").is_some());
        assert!(format_command("logcat", "08-29 10:00:01.000  1  1 E Tag: boom").is_some());
        assert!(format_command("dotnet build", "Build succeeded.").is_some());
        assert!(format_command("dotnet test", "Passed! - Failed: 0, Passed: 1, Total: 1").is_some());
        assert!(format_command("adb devices", "List of devices attached").is_none());
        assert!(format_command("dotnet run", "hello").is_none());
    }

    #[test]
    fn test_phase4_commands_dispatch() {
        assert!(format_command(
            "terraform plan",
            "aws_instance.web: Refreshing state... [id=i-1]\nNo changes. Infrastructure is up-to-date."
        )
        .is_some());
        assert!(format_command("gradle build", "BUILD SUCCESSFUL in 3s").is_some());
        assert!(format_command("mvn compile", "[INFO] BUILD SUCCESS").is_some());
        assert!(format_command("kubectl describe pod nginx", "Name: nginx").is_some());
    }

    #[test]
    fn test_ruby_commands_dispatch() {
        assert!(format_command("rspec", "1 example, 0 failures").is_some());
        assert!(
            format_command("rubocop", "10 files inspected, no offenses detected").is_some()
        );
        assert!(format_command(
            "rake test",
            "8 runs, 9 assertions, 0 failures, 0 errors, 0 skips"
        )
        .is_some());
        assert!(format_command(
            "bundle install",
            "Bundle complete! 85 dependencies"
        )
        .is_some());
        // `bundle exec` prefix unwraps to the real tool.
        assert!(format_command("bundle exec rspec", "1 example, 0 failures").is_some());
        // Non-test rake tasks fall through to generic compression.
        assert!(format_command("rake db:migrate", "== migrating ==").is_none());
    }
}
