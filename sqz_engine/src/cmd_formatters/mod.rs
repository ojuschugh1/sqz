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
mod gh;
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

    dispatch(cmd, input)
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
        "gh" => gh::format_gh(parts.get(1).copied(), output),

        // Rust
        "cargo" => cargo::format_cargo(parts.get(1).copied(), output),

        // JavaScript/TypeScript
        "npm" | "npx" => npm::format_npm(parts.get(1).copied(), output),
        "pnpm" => npm::format_pnpm(parts.get(1).copied(), output),
        "yarn" | "bun" => npm::format_npm(parts.get(1).copied(), output),
        "tsc" => Some(js::format_tsc(output)),
        "eslint" | "biome" => Some(lint::format_lint(output)),

        // Python
        "pytest" => Some(test_output::format_test_failures(output)),
        "python" | "python3" if cmd.contains("pytest") || cmd.contains("-m pytest") => {
            Some(test_output::format_test_failures(output))
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

        // Cloud CLIs
        "aws" => cloud::format_aws(parts.get(1).copied(), output),
        "terraform" | "tf" => cloud::format_terraform(parts.get(1).copied(), output),
        "gcloud" => cloud::format_gcloud(output),

        // System utilities
        "ls" => Some(system::format_ls(output)),
        "find" | "fd" => Some(system::format_find(output)),
        "grep" | "rg" | "ag" => Some(system::format_grep(output)),
        "tree" => Some(system::format_tree(output)),
        "curl" | "wget" => Some(system::format_curl(output)),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(format_command("gh pr list", "[]").is_some());
        assert!(format_command("ruff check", "All checks passed!").is_some());
        assert!(format_command("go test ./...", "ok").is_some());
        assert!(format_command("grep foo bar.txt", "bar.txt:1:foo").is_some());
        assert!(format_command("tree", ".\n├── src\n└── Cargo.toml").is_some());
        assert!(format_command("curl http://example.com", "response").is_some());
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
        assert!(format_command("aws s3 ls", "2024-01-01 bucket/file.txt").is_some());
        assert!(format_command("terraform plan", "No changes. Infrastructure is up-to-date.").is_some());
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
