pub fn format_git(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "status" => Some(format_git_status(output)),
        "log" => Some(format_git_log(output)),
        "diff" => Some(format_git_diff(output)),
        "show" => Some(format_git_show(output)),
        "stash" => Some(format_git_stash(output)),
        "remote" => Some(format_git_remote(output)),
        "fetch" => Some(format_git_fetch(output)),
        "add" | "commit" | "push" | "pull" | "checkout" | "switch" | "branch" => {
            Some(format_git_short(subcmd.unwrap(), output))
        }
        _ => None,
    }
}

/// Porcelain / short-status line: two status columns, space, path (locale-independent).
fn is_status_porcelain_line(line: &str) -> bool {
    let b = line.as_bytes();
    if line.len() < 4 || b[2] != b' ' {
        return false;
    }
    let valid = |c: u8| matches!(c, b' ' | b'?' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U');
    valid(b[0]) && valid(b[1]) && !line[3..].trim().is_empty()
}

fn starts_with_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

const STAGED_NEW_PREFIXES: &[&str] = &[
    "new file:",
    "nuovo file:",       // it
    "neue Datei:",       // de
    "nouveau fichier:",  // fr
    "nuevo archivo:",    // es
    "novo arquivo:",     // pt
];

const MODIFIED_PREFIXES: &[&str] = &[
    "modified:",
    "modificato:",   // it
    "modifiziert:",  // de
    "modifié:",      // fr
    "modificado:",   // es/pt
];

const DELETED_PREFIXES: &[&str] = &[
    "deleted:",
    "eliminato:",  // it
    "gelöscht:",   // de
    "supprimé:",   // fr
    "eliminado:",  // es/pt
];

const UNTRACKED_SECTION_HEADERS: &[&str] = &[
    "Untracked files:",
    "File non tracciati:",           // it
    "Unversionierte Dateien:",       // de
    "Fichiers non suivis:",          // fr
    "Archivos sin seguimiento:",     // es
    "Arquivos não rastreados:",      // pt
];

const CLEAN_MARKERS: &[&str] = &[
    "nothing to commit",
    "working tree clean",
    "non c'è nulla di cui eseguire il commit", // it
    "albero di lavoro pulito",                 // it
    "nichts zu committen",                     // de
    "rien à valider",                          // fr
    "nada para hacer commit",                  // es
];

fn parse_porcelain_status(output: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();

    for line in output.lines() {
        if !is_status_porcelain_line(line) {
            continue;
        }
        let index = line.as_bytes()[0];
        let worktree = line.as_bytes()[1];
        let path = line[3..].trim().to_string();

        if index == b'?' && worktree == b'?' {
            untracked.push(path);
        } else if index != b' ' {
            staged.push(format!("{} {}", index as char, path));
        } else if worktree != b' ' {
            modified.push(format!("{} {}", worktree as char, path));
        }
    }

    (staged, modified, untracked)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusSection {
    None,
    Staged,
    Unstaged,
    Untracked,
}

fn classify_section_header(line: &str) -> Option<StatusSection> {
    let trimmed = line.trim();
    if starts_with_any(trimmed, UNTRACKED_SECTION_HEADERS) {
        return Some(StatusSection::Untracked);
    }
    if !trimmed.ends_with(':') || line.starts_with('\t') {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("not staged")
        || lower.contains("non verrà")
        || lower.contains("nicht zum commit")
        || lower.contains("pas indexées")
        || lower.contains("sin añadir")
    {
        return Some(StatusSection::Unstaged);
    }
    if lower.contains("to be committed")
        || lower.contains("verrà eseguito")
        || lower.contains("zum commit vorgemerkte")
        || lower.contains("seront validées")
        || lower.contains("serán confirmados")
    {
        return Some(StatusSection::Staged);
    }
    None
}

fn parse_long_status(output: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();
    let mut section = StatusSection::None;

    for line in output.lines() {
        if let Some(next) = classify_section_header(line) {
            section = next;
            continue;
        }

        if !line.starts_with('\t') || line.trim().starts_with("(use") {
            continue;
        }

        let trimmed = line.trim();
        if starts_with_any(trimmed, STAGED_NEW_PREFIXES) {
            staged.push(trimmed.to_string());
        } else if section == StatusSection::Untracked {
            untracked.push(trimmed.to_string());
        } else if starts_with_any(trimmed, MODIFIED_PREFIXES) || starts_with_any(trimmed, DELETED_PREFIXES) {
            if section == StatusSection::Staged {
                staged.push(trimmed.to_string());
            } else {
                modified.push(trimmed.to_string());
            }
        }
    }

    (staged, modified, untracked)
}


fn format_git_status(output: &str) -> String {
    if CLEAN_MARKERS.iter().any(|m| output.contains(m)) {
        return "clean".to_string();
    }

    let (staged, modified, untracked) = if output.lines().any(is_status_porcelain_line) {
        parse_porcelain_status(output)
    } else {
        parse_long_status(output)
    };

    if staged.is_empty() && modified.is_empty() && untracked.is_empty() {
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
            result.push(format!(
                "untracked({}): {}, ...+{}",
                untracked.len(),
                untracked[..3].join(", "),
                untracked.len() - 3
            ));
        } else {
            result.push(format!("untracked({}): {}", untracked.len(), untracked.join(", ")));
        }
    }
    result.join("\n")
}

fn format_git_log(output: &str) -> String {
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
        return output.to_string();
    }
    commits.join("\n")
}

fn format_git_diff(output: &str) -> String {
    // Build a file summary header
    let mut files: Vec<&str> = Vec::new();
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for line in output.lines() {
        if line.starts_with("diff --git") {
            if let Some(b_path) = line.split(" b/").last() {
                files.push(b_path);
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }

    let mut result = Vec::new();
    if !files.is_empty() {
        result.push(format!("{} files +{} -{}", files.len(), additions, deletions));
    }

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
            context_count += 1;
            if context_count <= 1 {
                result.push(line.to_string());
            }
        }
    }
    result.join("\n")
}

fn format_git_show(output: &str) -> String {
    // git show is commit info + diff; compress the diff part
    let mut header_lines = Vec::new();
    let mut diff_started = false;
    let mut diff_output = String::new();

    for line in output.lines() {
        if line.starts_with("diff --git") {
            diff_started = true;
        }
        if diff_started {
            diff_output.push_str(line);
            diff_output.push('\n');
        } else {
            let trimmed = line.trim();
            // Keep commit, author (first line only), subject
            if line.starts_with("commit ") {
                header_lines.push(format!("{}", &line[7..line.len().min(14 + 7)]));
            } else if line.starts_with("Author:") {
                // skip
            } else if line.starts_with("Date:") {
                // skip
            } else if !trimmed.is_empty() && !trimmed.starts_with("Merge:") {
                header_lines.push(trimmed.to_string());
            }
        }
    }

    if diff_output.is_empty() {
        return header_lines.join("\n");
    }

    let compressed_diff = format_git_diff(&diff_output);
    format!("{}\n{}", header_lines.join("\n"), compressed_diff)
}

fn format_git_stash(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() {
        return "ok: no stashes".to_string();
    }
    // git stash list — compact to count + first few
    if lines.len() > 5 {
        let first_three: Vec<&str> = lines[..3].to_vec();
        format!("{} stashes:\n{}\n...+{} more", lines.len(), first_three.join("\n"), lines.len() - 3)
    } else {
        output.to_string()
    }
}

fn format_git_remote(output: &str) -> String {
    // git remote -v has duplicate lines (fetch/push), deduplicate
    let mut seen = std::collections::BTreeMap::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        // "origin  git@github.com:foo/bar.git (fetch)"
        if let Some(name_end) = trimmed.find(|c: char| c.is_whitespace()) {
            let name = &trimmed[..name_end];
            let rest = trimmed[name_end..].trim();
            let url = rest.split_whitespace().next().unwrap_or(rest);
            seen.entry(name.to_string()).or_insert_with(|| url.to_string());
        }
    }
    if seen.is_empty() {
        return output.to_string();
    }
    seen.iter().map(|(name, url)| format!("{}\t{}", name, url)).collect::<Vec<_>>().join("\n")
}

fn format_git_fetch(output: &str) -> String {
    if output.trim().is_empty() {
        return "ok: up-to-date".to_string();
    }
    // Keep lines with branch updates, skip "remote: Counting objects" noise
    let meaningful: Vec<&str> = output.lines().filter(|l| {
        let t = l.trim();
        !t.starts_with("remote: Counting")
            && !t.starts_with("remote: Compressing")
            && !t.starts_with("remote: Total")
            && !t.starts_with("Receiving objects")
            && !t.starts_with("Resolving deltas")
            && !t.is_empty()
    }).collect();
    if meaningful.is_empty() {
        return "ok: up-to-date".to_string();
    }
    meaningful.join("\n")
}

fn format_git_short(subcmd: &str, output: &str) -> String {
    match subcmd {
        "add" => {
            if output.trim().is_empty() { return "ok".to_string(); }
            output.to_string()
        }
        "commit" => {
            for line in output.lines() {
                if line.contains(']') && line.contains('[') {
                    return format!("ok {}", line.trim());
                }
            }
            if output.trim().is_empty() { return "ok".to_string(); }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_status_clean() {
        let output = "On branch main\nnothing to commit, working tree clean\n";
        assert_eq!(format_git_status(output), "clean");
    }

    #[test]
    fn test_git_status_italian_staged() {
        let output = "Sul branch master\n\nNon ci sono ancora commit\n\n\
Modifiche di cui verrà eseguito il commit:\n\
  (usa \"git rm --cached <file>...\" per rimuovere gli elementi dall'area di staging)\n\
\tnuovo file:             file1.txt\n\
\tnuovo file:             file2.txt\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(2)"));
        assert!(result.contains("file1.txt"));
        assert!(result.contains("file2.txt"));
        assert!(!result.contains("M ifiche"));
    }

    #[test]
    fn test_git_status_italian_header_not_corrupted() {
        let output = "Sul branch master\n\nModifiche di cui verrà eseguito il commit:\n";
        let result = format_git_status(output);
        assert_eq!(result, output);
        assert!(!result.contains("modified("));
    }

    #[test]
    fn test_git_status_porcelain() {
        let output = "A  file1.txt\nA  file2.txt\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(2)"));
        assert!(result.contains("file1.txt"));
    }

    #[test]
    fn test_git_status_short_format() {
        let output = "M  src/main.rs\n?? new.txt\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(1)"));
        assert!(result.contains("untracked(1)"));
    }

    #[test]
    fn test_git_status_english_staged() {
        let output = "On branch main\n\nChanges to be committed:\n\
  (use \"git restore --staged <file>...\" to unstage)\n\
\tnew file:   README.md\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(1)"));
        assert!(result.contains("README.md"));
    }

    #[test]
    fn test_git_status_english_unstaged_modified() {
        let output = "On branch main\n\nChanges not staged for commit:\n\
  (use \"git add <file>...\" to update what will be committed)\n\
\tmodified:   README.md\n";
        let result = format_git_status(output);
        assert!(result.contains("modified(1)"));
        assert!(result.contains("README.md"));
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
    fn test_git_diff_summary_header() {
        let output = "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n+    println!(\"hello\");\n }\n";
        let result = format_git_diff(output);
        assert!(result.starts_with("1 files +1 -0"));
    }

    #[test]
    fn test_git_stash_many() {
        let mut lines = Vec::new();
        for i in 0..10 {
            lines.push(format!("stash@{{{}}}: WIP on main: abc{} msg{}", i, i, i));
        }
        let output = lines.join("\n");
        let result = format_git_stash(&output);
        assert!(result.contains("10 stashes"));
        assert!(result.contains("...+7 more"));
    }

    #[test]
    fn test_git_remote_dedup() {
        let output = "origin\tgit@github.com:user/repo.git (fetch)\norigin\tgit@github.com:user/repo.git (push)\nupstream\tgit@github.com:org/repo.git (fetch)\nupstream\tgit@github.com:org/repo.git (push)\n";
        let result = format_git_remote(output);
        assert!(result.contains("origin"));
        assert!(result.contains("upstream"));
        // Should only appear once each
        assert_eq!(result.matches("origin").count(), 1);
    }

    #[test]
    fn test_git_fetch_empty() {
        assert_eq!(format_git_fetch(""), "ok: up-to-date");
    }

    #[test]
    fn test_git_fetch_strips_noise() {
        let output = "remote: Counting objects: 5, done.\nremote: Compressing objects: 100% (3/3), done.\nremote: Total 5\nReceiving objects: 100%\nResolving deltas: 100%\nFrom github.com:user/repo\n   abc..def  main -> origin/main\n";
        let result = format_git_fetch(output);
        assert!(!result.contains("Counting objects"));
        assert!(result.contains("main -> origin/main"));
    }
}
