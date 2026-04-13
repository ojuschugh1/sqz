/// Per-command output formatters — hand-tuned compression for the top 10
/// most common CLI commands in AI coding sessions.
///
/// Each formatter takes raw command output and returns a compact version
/// that preserves all actionable information while stripping noise.
/// Formatters are stateless and infallible (return original on any issue).

/// Route command output to the appropriate formatter.
/// Returns `None` if no specialized formatter matches (use generic pipeline).
pub fn format_command(cmd: &str, output: &str) -> Option<String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let base = parts.first().map(|s| s.rsplit('/').next().unwrap_or(s)).unwrap_or("");

    match base {
        "git" => format_git(parts.get(1).copied(), output),
        "cargo" => format_cargo(parts.get(1).copied(), output),
        "npm" | "npx" => format_npm(parts.get(1).copied(), output),
        "pytest" | "python" if cmd.contains("pytest") => Some(format_test_failures(output)),
        "go" if parts.get(1).copied() == Some("test") => Some(format_test_failures(output)),
        "docker" => format_docker(parts.get(1).copied(), output),
        "kubectl" => format_kubectl(parts.get(1).copied(), output),
        "ls" => Some(format_ls(output)),
        "find" | "fd" => Some(format_find(output)),
        "tsc" => Some(format_tsc(output)),
        "eslint" | "biome" => Some(format_lint(output)),
        _ => None,
    }
}

// ── git ───────────────────────────────────────────────────────────────────

fn format_git(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "status" => Some(format_git_status(output)),
        "log" => Some(format_git_log(output)),
        "diff" => Some(format_git_diff(output)),
        "add" | "commit" | "push" | "pull" | "checkout" | "switch" | "branch" => {
            Some(format_git_short(subcmd.unwrap(), output))
        }
        _ => None,
    }
}

fn format_git_status(output: &str) -> String {
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("new file:") || trimmed.starts_with("modified:") && line.starts_with('\t') {
            // In the staged section (after "Changes to be committed:")
            staged.push(trimmed.to_string());
        } else if trimmed.starts_with("modified:") || trimmed.starts_with("deleted:") {
            modified.push(trimmed.to_string());
        } else if line.starts_with("\t") && !trimmed.starts_with("(use") {
            // Could be untracked or staged depending on section
            if output[..output.find(line).unwrap_or(0)].contains("Untracked files:") {
                untracked.push(trimmed.to_string());
            }
        }
    }

    // Also handle short-format status (git status -s)
    if staged.is_empty() && modified.is_empty() && untracked.is_empty() {
        let mut short_staged = Vec::new();
        let mut short_modified = Vec::new();
        let mut short_untracked = Vec::new();
        for line in output.lines() {
            if line.len() < 3 { continue; }
            let (idx, rest) = (line.get(..2), line.get(3..));
            if let (Some(idx), Some(rest)) = (idx, rest) {
                match idx.trim() {
                    "M" | "A" | "D" | "R" => short_staged.push(format!("{} {}", idx.trim(), rest)),
                    "??" => short_untracked.push(rest.to_string()),
                    _ if idx.contains('M') => short_modified.push(format!("M {}", rest)),
                    _ => {}
                }
            }
        }
        if !short_staged.is_empty() || !short_modified.is_empty() || !short_untracked.is_empty() {
            staged = short_staged;
            modified = short_modified;
            untracked = short_untracked;
        }
    }

    if staged.is_empty() && modified.is_empty() && untracked.is_empty() {
        if output.contains("nothing to commit") {
            return "clean".to_string();
        }
        return output.to_string();
    }

    let mut result = Vec::new();
    if !staged.is_empty() {
        result.push(format!("staged({}): {}", staged.len(), staged.join(", ")));
    }
    if !modified.is_empty() {
        result.push(format!("modified({}): {}", modified.len(), modified.join(", ")));
    }
    if !untracked.is_empty() {
        if untracked.len() > 5 {
            result.push(format!("untracked({}): {}, ...+{}", untracked.len(),
                untracked[..3].join(", "), untracked.len() - 3));
        } else {
            result.push(format!("untracked({}): {}", untracked.len(), untracked.join(", ")));
        }
    }
    result.join("\n")
}

fn format_git_log(output: &str) -> String {
    // Compact: one line per commit — hash + subject
    let mut commits = Vec::new();
    let mut current_hash = String::new();
    let mut current_subject = String::new();

    for line in output.lines() {
        if line.starts_with("commit ") {
            if !current_hash.is_empty() {
                commits.push(format!("{} {}", &current_hash[..current_hash.len().min(7)], current_subject.trim()));
            }
            current_hash = line.strip_prefix("commit ").unwrap_or("").trim().to_string();
            current_subject.clear();
        } else if line.starts_with("Author:") || line.starts_with("Date:") || line.starts_with("Merge:") {
            // Skip
        } else {
            let trimmed = line.trim();
            if !trimmed.is_empty() && current_subject.is_empty() {
                current_subject = trimmed.to_string();
            }
        }
    }
    if !current_hash.is_empty() {
        commits.push(format!("{} {}", &current_hash[..current_hash.len().min(7)], current_subject.trim()));
    }

    if commits.is_empty() {
        // Might already be --oneline format
        return output.to_string();
    }
    commits.join("\n")
}

fn format_git_diff(output: &str) -> String {
    // Keep hunk headers and changed lines, strip context to 1 line
    let mut result = Vec::new();
    let mut context_count = 0;

    for line in output.lines() {
        if line.starts_with("diff --git") || line.starts_with("---") || line.starts_with("+++") {
            result.push(line.to_string());
            context_count = 0;
        } else if line.starts_with("@@") {
            result.push(line.to_string());
            context_count = 0;
        } else if line.starts_with('+') || line.starts_with('-') {
            result.push(line.to_string());
            context_count = 0;
        } else {
            // Context line — keep max 1
            context_count += 1;
            if context_count <= 1 {
                result.push(line.to_string());
            }
        }
    }
    result.join("\n")
}

fn format_git_short(subcmd: &str, output: &str) -> String {
    match subcmd {
        "add" => {
            if output.trim().is_empty() { return "ok".to_string(); }
            output.to_string()
        }
        "commit" => {
            // Extract short hash and subject
            for line in output.lines() {
                if line.contains(']') && line.contains('[') {
                    return format!("ok {}", line.trim());
                }
            }
            if output.trim().is_empty() { return "ok".to_string(); }
            // First non-empty line
            output.lines().find(|l| !l.trim().is_empty()).unwrap_or("ok").to_string()
        }
        "push" => {
            for line in output.lines() {
                if line.contains("->") {
                    return format!("ok {}", line.trim());
                }
            }
            "ok".to_string()
        }
        "pull" => {
            let mut files_changed = 0;
            let mut insertions = 0;
            let mut deletions = 0;
            for line in output.lines() {
                if line.contains("files changed") || line.contains("file changed") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    for (i, p) in parts.iter().enumerate() {
                        if *p == "file" || p.starts_with("file") { files_changed = parts.get(i-1).and_then(|n| n.parse().ok()).unwrap_or(0); }
                        if p.starts_with("insertion") { insertions = parts.get(i-1).and_then(|n| n.parse().ok()).unwrap_or(0); }
                        if p.starts_with("deletion") { deletions = parts.get(i-1).and_then(|n| n.parse().ok()).unwrap_or(0); }
                    }
                }
            }
            if files_changed > 0 {
                format!("ok {} files +{} -{}", files_changed, insertions, deletions)
            } else if output.contains("Already up to date") {
                "ok up-to-date".to_string()
            } else {
                "ok".to_string()
            }
        }
        _ => output.lines().take(3).collect::<Vec<_>>().join("\n"),
    }
}

// ── cargo ─────────────────────────────────────────────────────────────────

fn format_cargo(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "test" => Some(format_test_failures(output)),
        "build" | "check" => Some(format_cargo_build(output)),
        "clippy" => Some(format_lint(output)),
        _ => None,
    }
}

fn format_cargo_build(output: &str) -> String {
    let errors: Vec<&str> = output.lines()
        .filter(|l| l.starts_with("error") || l.contains("error[E") || l.starts_with("warning"))
        .collect();

    if errors.is_empty() {
        // Success — find the Finished/Compiling summary
        for line in output.lines().rev() {
            if line.contains("Finished") || line.contains("Compiling") {
                return line.trim().to_string();
            }
        }
        return "ok".to_string();
    }

    // Group errors by file
    let mut grouped: Vec<String> = Vec::new();
    let current_file = String::new();
    let file_errors: Vec<String> = Vec::new();

    for line in output.lines() {
        if line.starts_with("error") || line.contains("error[E") {
            // Extract file path from " --> file:line:col"
            grouped.push(line.to_string());
        } else if line.trim().starts_with("-->") {
            grouped.push(format!("  {}", line.trim()));
        }
    }

    if grouped.is_empty() {
        return output.to_string();
    }

    let _ = (current_file, file_errors); // suppress unused warnings
    format!("ERRORS: {}\n{}", errors.len(), grouped.join("\n"))
}

// ── test runners (generic) ────────────────────────────────────────────────

fn format_test_failures(output: &str) -> String {
    let mut failures = Vec::new();
    let mut summary_line = String::new();
    let mut in_failure = false;
    let mut failure_buf = Vec::new();

    for line in output.lines() {
        // Rust test result line
        if line.starts_with("test result:") || line.starts_with("Tests:") {
            summary_line = line.to_string();
        }
        // Rust: "---- test_name stdout ----" marks failure start
        if line.starts_with("---- ") && line.ends_with(" ----") {
            if !failure_buf.is_empty() {
                failures.push(failure_buf.join("\n"));
                failure_buf.clear();
            }
            in_failure = true;
            failure_buf.push(line.to_string());
            continue;
        }
        // Rust: "failures:" section
        if line == "failures:" {
            in_failure = true;
            continue;
        }
        // pytest: "FAILED" lines
        if line.contains("FAILED") || line.contains("FAIL:") {
            failures.push(line.to_string());
        }
        // go test: "--- FAIL:"
        if line.starts_with("--- FAIL:") {
            failures.push(line.to_string());
        }
        // Collect failure details
        if in_failure {
            if line.trim().is_empty() && !failure_buf.is_empty() {
                failures.push(failure_buf.join("\n"));
                failure_buf.clear();
                in_failure = false;
            } else {
                failure_buf.push(line.to_string());
            }
        }
        // "test ... FAILED" individual lines
        if line.contains("... FAILED") || line.contains("FAILED") && line.starts_with("test ") {
            if !failures.iter().any(|f| f.contains(line)) {
                failures.push(line.to_string());
            }
        }
    }
    if !failure_buf.is_empty() {
        failures.push(failure_buf.join("\n"));
    }

    // If all tests passed, return compact summary
    if failures.is_empty() {
        if !summary_line.is_empty() {
            return summary_line;
        }
        // Count test lines
        let total = output.lines().filter(|l| l.contains("... ok") || l.contains("PASSED") || l.contains("passed")).count();
        if total > 0 {
            return format!("ok: {} tests passed", total);
        }
        return output.to_string();
    }

    let mut result = Vec::new();
    if !summary_line.is_empty() {
        result.push(summary_line);
    }
    result.push(format!("FAILURES ({}):", failures.len()));
    for f in &failures {
        result.push(f.clone());
    }
    result.join("\n")
}

// ── npm ───────────────────────────────────────────────────────────────────

fn format_npm(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "test" => Some(format_test_failures(output)),
        "run" if output.contains("error") || output.contains("FAIL") => Some(format_test_failures(output)),
        "install" | "i" | "add" => Some(format_npm_install(output)),
        _ => None,
    }
}

fn format_npm_install(output: &str) -> String {
    let mut vulns = String::new();
    for line in output.lines() {
        if line.contains("added") && line.contains("packages") {
            return line.trim().to_string();
        }
        if line.contains("vulnerabilities") {
            vulns = line.trim().to_string();
        }
    }
    if !vulns.is_empty() {
        return format!("ok ({})", vulns);
    }
    "ok".to_string()
}

// ── docker ────────────────────────────────────────────────────────────────

fn format_docker(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "ps" => Some(format_docker_ps(output)),
        "images" => Some(format_docker_images(output)),
        "logs" => None, // Use generic condense for log dedup
        _ => None,
    }
}

fn format_docker_ps(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() { return output.to_string(); }

    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            // Header — keep only NAME, IMAGE, STATUS
            result.push("NAME | IMAGE | STATUS".to_string());
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            // CONTAINER_ID IMAGE COMMAND CREATED STATUS ... NAMES
            let name = parts.last().unwrap_or(&"");
            let image = parts.get(1).unwrap_or(&"");
            let status_parts: Vec<&&str> = parts.iter().skip(4).take_while(|p| !p.starts_with("0.0.0.0")).collect();
            let status = status_parts.iter().map(|s| **s).collect::<Vec<_>>().join(" ");
            result.push(format!("{} | {} | {}", name, image, status));
        } else {
            result.push(line.to_string());
        }
    }
    result.join("\n")
}

fn format_docker_images(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() { return output.to_string(); }

    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            result.push("REPO:TAG | SIZE".to_string());
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            let repo = parts[0];
            let tag = parts[1];
            let size = parts.last().unwrap_or(&"");
            result.push(format!("{}:{} | {}", repo, tag, size));
        }
    }
    result.join("\n")
}

// ── kubectl ───────────────────────────────────────────────────────────────

fn format_kubectl(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "get" => Some(format_kubectl_get(output)),
        _ => None,
    }
}

fn format_kubectl_get(output: &str) -> String {
    // Keep header + data but strip AGE column and collapse whitespace
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() { return output.to_string(); }

    let mut result = Vec::new();
    for line in &lines {
        // Collapse multiple spaces to single
        let collapsed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        result.push(collapsed);
    }
    result.join("\n")
}

// ── ls ────────────────────────────────────────────────────────────────────

fn format_ls(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= 20 { return output.to_string(); }

    // Group by directory structure
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("total") { continue; }
        if trimmed.starts_with('d') || trimmed.ends_with('/') {
            dirs.push(trimmed.split_whitespace().last().unwrap_or(trimmed).to_string());
        } else {
            files.push(trimmed.split_whitespace().last().unwrap_or(trimmed).to_string());
        }
    }

    let mut result = Vec::new();
    if !dirs.is_empty() {
        result.push(format!("dirs({}): {}", dirs.len(), dirs.join(", ")));
    }
    if files.len() > 10 {
        result.push(format!("files({}): {}, ...+{}", files.len(),
            files[..5].join(", "), files.len() - 5));
    } else if !files.is_empty() {
        result.push(format!("files({}): {}", files.len(), files.join(", ")));
    }
    result.join("\n")
}

// ── find ──────────────────────────────────────────────────────────────────

fn format_find(output: &str) -> String {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= 20 { return output.to_string(); }

    // Group by parent directory
    let mut by_dir: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for line in &lines {
        let path = std::path::Path::new(line.trim());
        let parent = path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        by_dir.entry(parent).or_default().push(name);
    }

    let mut result = Vec::new();
    result.push(format!("{} files found:", lines.len()));
    for (dir, files) in &by_dir {
        if files.len() > 5 {
            result.push(format!("  {}/ ({} files)", dir, files.len()));
        } else {
            result.push(format!("  {}/ {}", dir, files.join(", ")));
        }
    }
    result.join("\n")
}

// ── tsc ───────────────────────────────────────────────────────────────────

fn format_tsc(output: &str) -> String {
    // Group TypeScript errors by file
    let mut by_file: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    let mut error_count = 0;

    for line in output.lines() {
        // TS errors: "src/file.ts(10,5): error TS2345: ..."
        if line.contains("): error TS") || line.contains("): warning TS") {
            error_count += 1;
            if let Some(paren_pos) = line.find('(') {
                let file = &line[..paren_pos];
                by_file.entry(file.to_string()).or_default().push(line.to_string());
            } else {
                by_file.entry("unknown".to_string()).or_default().push(line.to_string());
            }
        }
    }

    if error_count == 0 {
        if output.contains("Found 0 errors") || output.trim().is_empty() {
            return "ok: 0 errors".to_string();
        }
        return output.to_string();
    }

    let mut result = Vec::new();
    result.push(format!("ERRORS: {} in {} files", error_count, by_file.len()));
    for (file, errors) in &by_file {
        result.push(format!("  {} ({}):", file, errors.len()));
        for e in errors.iter().take(5) {
            result.push(format!("    {}", e));
        }
        if errors.len() > 5 {
            result.push(format!("    ...+{} more", errors.len() - 5));
        }
    }
    result.join("\n")
}

// ── lint (eslint/biome) ───────────────────────────────────────────────────

fn format_lint(output: &str) -> String {
    let mut by_rule: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut total = 0;

    for line in output.lines() {
        // ESLint: "  10:5  error  Description  rule-name"
        // Clippy: "warning: description"
        let trimmed = line.trim();
        if trimmed.contains("error") || trimmed.contains("warning") {
            total += 1;
            // Try to extract rule name (last word in ESLint format)
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if let Some(rule) = parts.last() {
                *by_rule.entry(rule.to_string()).or_insert(0) += 1;
            }
        }
    }

    if total == 0 {
        return "ok: 0 issues".to_string();
    }

    let mut result = Vec::new();
    result.push(format!("{} issues:", total));
    let mut sorted: Vec<_> = by_rule.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (rule, count) in sorted.iter().take(10) {
        result.push(format!("  {} ({}x)", rule, count));
    }
    if sorted.len() > 10 {
        result.push(format!("  ...+{} more rules", sorted.len() - 10));
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_status_clean() {
        let output = "On branch main\nnothing to commit, working tree clean\n";
        assert_eq!(format_git_status(output), "clean");
    }

    #[test]
    fn test_git_log_compact() {
        let output = "commit abc1234567890\nAuthor: Test <test@test.com>\nDate:   Mon Apr 13\n\n    feat: Add feature\n\ncommit def5678901234\nAuthor: Test <test@test.com>\nDate:   Sun Apr 12\n\n    fix: Bug fix\n";
        let result = format_git_log(output);
        assert!(result.contains("abc1234"));
        assert!(result.contains("feat: Add feature"));
        assert!(!result.contains("Author:"));
    }

    #[test]
    fn test_git_push_compact() {
        let output = "Enumerating objects: 5, done.\nCounting objects: 100% (5/5), done.\nDelta compression using up to 8 threads\n   abc1234..def5678  main -> main\n";
        let result = format_git_short("push", output);
        assert!(result.starts_with("ok"));
    }

    #[test]
    fn test_cargo_test_all_pass() {
        let output = "running 15 tests\ntest a ... ok\ntest b ... ok\ntest result: ok. 15 passed; 0 failed; 0 ignored\n";
        let result = format_test_failures(output);
        assert!(result.contains("ok") || result.contains("passed"));
        assert!(!result.contains("FAILURES"));
    }

    #[test]
    fn test_cargo_test_with_failure() {
        let output = "running 3 tests\ntest a ... ok\ntest b ... FAILED\ntest c ... ok\n\nfailures:\n\n---- b stdout ----\nassertion failed\n\ntest result: FAILED. 2 passed; 1 failed\n";
        let result = format_test_failures(output);
        assert!(result.contains("FAIL"));
    }

    #[test]
    fn test_docker_ps_compact() {
        let output = "CONTAINER ID   IMAGE     COMMAND   CREATED   STATUS    PORTS     NAMES\nabc123def456   nginx     \"nginx\"   2h ago    Up 2h     80/tcp    web\n";
        let result = format_docker_ps(output);
        assert!(result.contains("NAME | IMAGE | STATUS"));
        assert!(result.contains("web"));
    }

    #[test]
    fn test_tsc_no_errors() {
        let result = format_tsc("Found 0 errors.\n");
        assert_eq!(result, "ok: 0 errors");
    }

    #[test]
    fn test_format_command_routing() {
        assert!(format_command("git status", "nothing to commit").is_some());
        assert!(format_command("cargo test", "test result: ok").is_some());
        assert!(format_command("unknown_tool", "output").is_none());
    }

    #[test]
    fn test_ls_short_passthrough() {
        let output = "file1.rs\nfile2.rs\n";
        assert_eq!(format_ls(output), output);
    }

    #[test]
    fn test_npm_install_compact() {
        let output = "added 42 packages in 3s\n2 vulnerabilities\n";
        let result = format_npm_install(output);
        assert!(result.contains("added 42 packages"));
    }
}
