// ls and find output is left to the generic pipeline, which keeps every entry.

/// `path:line:text` matches in ripgrep's heading layout: each file's path
/// once, then its `line:text` matches, a blank line between files. Every
/// match stays; order is unchanged. Returns None unless at least 80% of the
/// lines are matches and the layout saves 10%.
pub fn format_grep(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let parsed: Vec<Option<(&str, &str)>> = lines.iter().map(|l| split_match(l)).collect();
    let matches = parsed.iter().filter(|p| p.is_some()).count();
    if lines.is_empty() || matches * 5 < lines.len() * 4 {
        return None;
    }

    let mut out = String::with_capacity(output.len());
    let mut current: Option<&str> = None;
    for (line, parsed) in lines.iter().zip(&parsed) {
        match parsed {
            Some((path, rest)) => {
                if current != Some(*path) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(path);
                    out.push('\n');
                    current = Some(path);
                }
                out.push_str(rest);
                out.push('\n');
            }
            None => {
                if current.is_some() {
                    out.push('\n');
                }
                out.push_str(line);
                out.push('\n');
                current = None;
            }
        }
    }
    let out = out.trim_end().to_string();
    if out.len() * 10 > output.trim_end().len() * 9 {
        return None;
    }
    Some(out)
}

/// `src/a.rs:12:text` -> (`src/a.rs`, `12:text`). The path has no spaces
/// and looks like a file (a `/` or `.`), so `10:22:03` in a log line isn't
/// read as one.
fn split_match(line: &str) -> Option<(&str, &str)> {
    let (path, rest) = line.split_once(':')?;
    let digits = rest.find(':')?;
    if path.is_empty()
        || path.contains(char::is_whitespace)
        || !(path.contains('/') || path.contains('.'))
        || digits == 0
        || !rest[..digits].bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((path, rest))
}

pub fn format_tree(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= 30 { return output.to_string(); }

    // Filter out noise directories and collapse deep trees
    let noise_dirs = ["node_modules", ".git", "target", "__pycache__",
                      ".next", "dist", "build", ".cache", "vendor",
                      "venv", ".venv", ".tox"];

    let mut result = Vec::new();
    let mut skipped = 0;
    let mut in_noise = false;
    let mut noise_depth = 0;

    for line in &lines {
        let content = line.trim_start_matches(|c: char| c == '│' || c == ' ' || c == '├' || c == '└' || c == '─' || c == '─');
        let depth = line.len() - content.len();

        if noise_dirs.iter().any(|d| content.trim() == *d || content.trim().ends_with(d)) {
            in_noise = true;
            noise_depth = depth;
            result.push(format!("{} [collapsed]", line));
            continue;
        }

        if in_noise && depth > noise_depth {
            skipped += 1;
            continue;
        }
        in_noise = false;

        result.push(line.to_string());
    }

    if skipped > 0 {
        result.push(format!("({} entries collapsed)", skipped));
    }

    result.join("\n")
}

/// Response body without curl's progress meter. Everything else, blank
/// lines included, stays.
pub fn format_curl(output: &str) -> String {
    let lines: Vec<&str> = output.lines().filter(|l| !is_curl_progress(l)).collect();
    if lines.iter().all(|l| l.trim().is_empty()) {
        return output.to_string();
    }
    lines.join("\n")
}

fn is_curl_progress(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("% Total") && t.contains("% Received") {
        return true;
    }
    if t.starts_with("Dload") && t.contains("Upload") {
        return true;
    }
    // `100  1270  100  1270    0     0   6350      0 --:--:-- --:--:-- --:--:--  6350`
    let fields: Vec<&str> = t.split_whitespace().collect();
    let is_time = |f: &str| {
        f == "--:--:--" || (f.len() >= 7 && f.split(':').count() == 3 && f.bytes().all(|b| b.is_ascii_digit() || b == b':'))
    };
    fields.len() == 12
        && fields[0].bytes().all(|b| b.is_ascii_digit())
        && fields[8..11].iter().all(|f| is_time(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reverse the heading layout back to `path:line:text` lines.
    fn unheading(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut path: Option<&str> = None;
        for line in s.lines() {
            if line.is_empty() {
                path = None;
            } else if let Some(p) = path {
                out.push(format!("{p}:{line}"));
            } else if split_match(line).is_some() {
                out.push(line.to_string());
            } else {
                path = Some(line);
            }
        }
        out
    }

    #[test]
    fn test_grep_heading_keeps_every_match_in_order() {
        let mut lines = Vec::new();
        for (file, n) in [("src/main.rs", 30), ("src/lib.rs", 12), ("tests/it.rs", 8)] {
            for i in 0..n {
                lines.push(format!("{file}:{}:    let x = compute({i});", i * 3 + 1));
            }
        }
        let output = lines.join("\n") + "\n";
        let result = format_grep(&output).expect("heading layout saves here");
        assert!(result.starts_with("src/main.rs\n1:    let x = compute(0);\n"), "{result}");
        assert_eq!(unheading(&result), lines, "every match, same order");
    }

    #[test]
    fn test_grep_one_match_per_file_passes_through() {
        let output = (0..40).map(|i| format!("src/f{i}.rs:3:hit\n")).collect::<String>();
        assert!(format_grep(&output).is_none(), "no repeated paths to save");
    }

    #[test]
    fn test_grep_other_shapes_pass_through() {
        assert!(format_grep("src/lib.rs\nsrc/main.rs\n").is_none(), "-l");
        assert!(format_grep("src/lib.rs:4\nsrc/main.rs:9\n").is_none(), "-c");
        let log = "2026-09-14 10:22:03 INFO start\n2026-09-14 10:22:04 INFO next\n".repeat(20);
        assert!(format_grep(&log).is_none(), "timestamps are not paths");
    }

    #[test]
    fn test_tree_collapses_noise_and_keeps_the_rest() {
        let mut lines = vec![
            ".".to_string(),
            "├── src".to_string(),
            "│   └── main.rs".to_string(),
            "├── node_modules".to_string(),
        ];
        for i in 0..50 {
            lines.push(format!("│   ├── package_{}", i));
        }
        for i in 0..60 {
            lines.push(format!("├── doc_{}.md", i));
        }
        lines.push("└── Cargo.toml".to_string());
        let output = lines.join("\n");
        let result = format_tree(&output);
        assert!(result.contains("[collapsed]"));
        assert!(result.contains("entries collapsed"));
        assert!(!result.contains("package_49"));
        assert!(result.contains("doc_59.md"), "entries outside noise dirs all stay");
        assert!(result.contains("Cargo.toml"));
    }

    #[test]
    fn test_tree_short_passthrough() {
        let output = ".\n├── src\n│   └── main.rs\n└── Cargo.toml\n";
        assert_eq!(format_tree(output), output);
    }

    #[test]
    fn test_curl_strips_progress_only() {
        let output = "  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current\n                                 Dload  Upload   Total   Spent    Left  Speed\n100  1270  100  1270    0     0   6350      0 --:--:-- --:--:-- --:--:--  6350\n# Title\n\n%PDF-like line stays\nsecond paragraph\n";
        let result = format_curl(output);
        assert_eq!(result, "# Title\n\n%PDF-like line stays\nsecond paragraph");
    }
}
