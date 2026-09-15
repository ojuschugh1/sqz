use super::truncate::CAP_LIST;

pub fn format_npm(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "test" | "run" if output.contains("FAIL") || output.contains("error") => {
            Some(super::test_output::format_test_failures(output))
        }
        "install" | "i" | "add" | "ci" => Some(format_install(output)),
        "audit" => Some(format_audit(output)),
        "outdated" => Some(format_outdated(output)),
        "run" => {
            if output.trim().is_empty() { return Some("ok".to_string()); }
            None
        }
        _ => None,
    }
}

pub fn format_pnpm(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "install" | "i" | "add" => Some(format_install(output)),
        "test" | "run" if output.contains("FAIL") || output.contains("ERR") => {
            Some(super::test_output::format_test_failures(output))
        }
        "audit" => Some(format_audit(output)),
        "outdated" => Some(format_outdated(output)),
        _ => None,
    }
}

fn format_install(output: &str) -> String {
    let mut summary_parts = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.contains("added") && trimmed.contains("packages") {
            summary_parts.push(trimmed.to_string());
            continue;
        }
        // pnpm: "Packages: +42"
        if trimmed.starts_with("Packages:") {
            summary_parts.push(trimmed.to_string());
        }
        // pnpm: "Progress: resolved X, reused Y, downloaded Z, added W"
        if trimmed.starts_with("Progress:") && trimmed.contains("added") {
            summary_parts.push(trimmed.to_string());
        }
        if trimmed.contains("vulnerabilities") || trimmed.starts_with("npm audit fix") {
            summary_parts.push(trimmed.to_string());
        }
    }

    if !summary_parts.is_empty() {
        return summary_parts.join("\n");
    }

    // pnpm "Already up to date" or empty output
    if output.contains("Already up to date") || output.contains("up to date") {
        return "ok: up to date".to_string();
    }

    "ok".to_string()
}

fn format_audit(output: &str) -> String {
    // Try to parse JSON audit output (npm audit --json)
    if output.trim_start().starts_with('{') {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
            return format_audit_json(&json);
        }
    }

    // Human-readable: look for summary lines
    let mut vulns = Vec::new();
    let mut total = 0;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.contains("vulnerabilities") {
            return trimmed.to_string();
        }
        if trimmed.contains("severity") || trimmed.contains("Critical") || trimmed.contains("High") {
            vulns.push(trimmed.to_string());
            total += 1;
        }
    }

    if total == 0 {
        return "ok: 0 vulnerabilities".to_string();
    }
    if vulns.len() > CAP_LIST {
        let omitted = vulns.len() - CAP_LIST;
        vulns.truncate(CAP_LIST);
        vulns.push(format!("...+{} more", omitted));
    }
    format!("{} vulnerabilities:\n{}", total, vulns.join("\n"))
}

fn format_audit_json(json: &serde_json::Value) -> String {
    if let Some(meta) = json.get("metadata") {
        let vulns = meta.get("vulnerabilities").and_then(|v| v.as_object());
        if let Some(v) = vulns {
            let total: u64 = v.values().filter_map(|n| n.as_u64()).sum();
            if total == 0 {
                return "ok: 0 vulnerabilities".to_string();
            }
            let mut parts = Vec::new();
            for severity in &["critical", "high", "moderate", "low"] {
                if let Some(count) = v.get(*severity).and_then(|n| n.as_u64()) {
                    if count > 0 {
                        parts.push(format!("{} {}", count, severity));
                    }
                }
            }
            return format!("{} vulnerabilities: {}", total, parts.join(", "));
        }
    }
    "audit: unable to parse".to_string()
}

fn format_outdated(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() || output.trim().is_empty() {
        return "ok: all up to date".to_string();
    }

    // npm outdated outputs a table: Package | Current | Wanted | Latest
    let data_lines: Vec<&str> = lines.iter()
        .filter(|l| !l.is_empty() && !l.starts_with("Package"))
        .copied()
        .collect();

    if data_lines.is_empty() {
        return "ok: all up to date".to_string();
    }

    if data_lines.len() > CAP_LIST {
        let mut result = format!("{} outdated packages:\n", data_lines.len());
        for line in data_lines.iter().take(CAP_LIST) {
            result.push_str(line);
            result.push('\n');
        }
        result.push_str(&format!("...+{} more", data_lines.len() - CAP_LIST));
        result
    } else {
        format!("{} outdated packages:\n{}", data_lines.len(), data_lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_npm_install_compact() {
        let output = "added 42 packages in 3s\n2 vulnerabilities\n";
        let result = format_install(output);
        assert!(result.contains("added 42 packages"));
        assert!(result.contains("2 vulnerabilities"));
    }

    #[test]
    fn test_npm_install_keeps_audit_summary_and_fix_hint() {
        let output = "npm warn deprecated glob@7.2.3: Glob versions prior to v9 are no longer supported\n\n\
added 412 packages, and audited 413 packages in 9s\n\n58 packages are looking for funding\n  run `npm fund` for details\n\n\
3 vulnerabilities (1 moderate, 2 high)\n\nTo address all issues, run:\n  npm audit fix\n\nRun `npm audit` for details.\n";
        let result = format_install(output);
        assert_eq!(result, "added 412 packages, and audited 413 packages in 9s\n3 vulnerabilities (1 moderate, 2 high)\nnpm audit fix");
    }

    #[test]
    fn test_pnpm_install_up_to_date() {
        let output = "Already up to date\n";
        let result = format_install(output);
        assert_eq!(result, "ok: up to date");
    }

    #[test]
    fn test_audit_clean() {
        let result = format_audit("found 0 vulnerabilities\n");
        assert!(result.contains("0 vulnerabilities"));
    }

    #[test]
    fn test_audit_json() {
        let json = r#"{"metadata":{"vulnerabilities":{"critical":1,"high":2,"moderate":0,"low":3}}}"#;
        let result = format_audit(json);
        assert!(result.contains("6 vulnerabilities"));
        assert!(result.contains("1 critical"));
        assert!(result.contains("2 high"));
    }

    #[test]
    fn test_outdated_empty() {
        assert_eq!(format_outdated(""), "ok: all up to date");
    }

    #[test]
    fn test_outdated_with_packages() {
        let output = "Package  Current  Wanted  Latest\nreact    17.0.0   17.0.2  18.2.0\nlodash   4.17.0   4.17.4  4.17.21\n";
        let result = format_outdated(output);
        assert!(result.contains("2 outdated packages"));
    }
}
