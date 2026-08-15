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

// Localized section-header markers, verified against git's own translation
// files (po/it.po, po/de.po, po/fr.po, po/es.po, po/pt_PT.po on git master).
// Matched case-insensitively via `contains` on the ASCII-lowercased header.
// Unstaged markers are checked BEFORE staged ones: French "…qui ne seront pas
// validées :" contains the staged marker "seront validées" as a substring.
//
// Only the section headers need translation. File entries inside a section
// are classified by the section alone — the localized label ("nuovo file:",
// "geändert:", "renommé :", …) is kept verbatim in the summary, so we never
// need a per-label table and unknown labels can't be silently dropped.

const UNSTAGED_HEADER_MARKERS: &[&str] = &[
    "not staged",                    // en: Changes not staged for commit:
    "non nell'area di staging",      // it: Modifiche non nell'area di staging per il commit:
    "nicht zum commit vorgemerkt",   // de: Änderungen, die nicht zum Commit vorgemerkt sind:
    "ne seront pas validées",        // fr: Modifications qui ne seront pas validées :
    "no rastreados para el commit",  // es: Cambios no rastreados para el commit:
    "por encenar para memória",      // pt_PT: Alterações por encenar para memória:
];

const STAGED_HEADER_MARKERS: &[&str] = &[
    "to be committed",               // en: Changes to be committed:
    "di cui verrà eseguito il commit", // it: Modifiche di cui verrà eseguito il commit:
    "zum commit vorgemerkte",        // de: Zum Commit vorgemerkte Änderungen:
    "seront validées",               // fr: Modifications qui seront validées :
    "a ser confirmados",             // es: Cambios a ser confirmados:
    "para serem memorizadas",        // pt_PT: Alterações para serem memorizadas:
];

const UNTRACKED_HEADER_PREFIXES: &[&str] = &[
    "Untracked files",               // en
    "File non tracciati",            // it
    "Unversionierte Dateien",        // de
    "Fichiers non suivis",           // fr
    "Archivos sin seguimiento",      // es
    "Ficheiros desmonitorizados",    // pt_PT
];

const CLEAN_MARKERS: &[&str] = &[
    "nothing to commit",
    "working tree clean",
    "non c'è nulla di cui eseguire il commit",  // it
    "nichts zu committen",                       // de
    "rien à valider",                            // fr
    "nada para hacer commit",                    // es
    "nada a memorizar",                          // pt_PT
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
    /// A section header we don't have translations for ("Unmerged paths:",
    /// an unlisted locale, …). File entries under it force a raw passthrough
    /// rather than a partial — and therefore wrong — summary.
    Unknown,
}

fn classify_section_header(line: &str) -> Option<StatusSection> {
    let trimmed = line.trim();
    if UNTRACKED_HEADER_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return Some(StatusSection::Untracked);
    }
    if !trimmed.ends_with(':') {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    // Unstaged first — the French staged marker is a substring of the
    // unstaged header (see table comment above).
    if UNSTAGED_HEADER_MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(StatusSection::Unstaged);
    }
    if STAGED_HEADER_MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(StatusSection::Staged);
    }
    None
}

/// Parse localized long-format `git status`. Returns `None` when the output
/// contains file entries we can't confidently place in a section — the
/// caller then passes the raw output through instead of emitting a partial
/// summary. That covers every locale git ships beyond the tables above
/// (ru, zh_CN, ko, pl, sv, tr, uk, vi, …) as well as sections we don't
/// summarize, like "Unmerged paths:" during a conflict.
fn parse_long_status(output: &str) -> Option<(Vec<String>, Vec<String>, Vec<String>)> {
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();
    let mut section = StatusSection::None;

    for line in output.lines() {
        // File entries are tab-indented; hint lines ("  (use …") are
        // space-indented; headers and prose are flush-left.
        if let Some(rest) = line.strip_prefix('\t') {
            let entry = rest.trim();
            if entry.is_empty() {
                continue;
            }
            match section {
                StatusSection::Staged => staged.push(entry.to_string()),
                StatusSection::Unstaged => modified.push(entry.to_string()),
                StatusSection::Untracked => untracked.push(entry.to_string()),
                // A file entry outside any recognized section means we're
                // looking at a locale or section we don't understand.
                // Refuse to guess.
                StatusSection::None | StatusSection::Unknown => return None,
            }
            continue;
        }
        // Localized hint lines: "  (usa \"git add <file>...\" …)". Git indents
        // them with two spaces, but tolerate flush-left parenthesized lines
        // too — skipping a hint must never end the open section.
        if line.starts_with(' ') || line.trim_start().starts_with('(') {
            continue;
        }
        if let Some(next) = classify_section_header(line) {
            section = next;
        } else if line.trim_end().ends_with(':') {
            // Header-shaped line we can't classify (unknown locale or an
            // unsummarized section like "Unmerged paths:").
            section = StatusSection::Unknown;
        } else {
            // Prose ("On branch main", branch-tracking info, …) ends any
            // open section.
            section = StatusSection::None;
        }
    }

    Some((staged, modified, untracked))
}


fn format_git_status(output: &str) -> String {
    let parsed = if output.lines().any(is_status_porcelain_line) {
        Some(parse_porcelain_status(output))
    } else {
        parse_long_status(output)
    };

    // Unparseable (unknown locale / unknown section with file entries):
    // pass the raw output through rather than emit a wrong summary.
    let Some((staged, modified, untracked)) = parsed else {
        return output.to_string();
    };

    if staged.is_empty() && modified.is_empty() && untracked.is_empty() {
        // Only report "clean" when we also see a known clean marker; the
        // check runs after parsing so untracked-only output ("nothing added
        // to commit but untracked files present") can never hit it.
        if CLEAN_MARKERS.iter().any(|m| output.contains(m)) {
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

    // The localized fixtures below use the exact msgstr values from git's
    // own translation files (po/*.po on git master) — not guessed
    // translations. If one of these fails after a table edit, re-check the
    // .po file before "fixing" the test.

    /// German mixed status. Real strings: staged header "Zum Commit
    /// vorgemerkte Änderungen:", unstaged header "Änderungen, die nicht zum
    /// Commit vorgemerkt sind:", modified label "geändert:" (NOT
    /// "modifiziert:"). Regression: modified entries must not be silently
    /// dropped, and unstaged must not leak into staged.
    #[test]
    fn test_git_status_german_mixed() {
        let output = "Auf Branch main\n\n\
Zum Commit vorgemerkte Änderungen:\n\
  (benutzen Sie \"git restore --staged <Datei>...\" zum Entfernen aus der Staging-Area)\n\
\tneue Datei:     neu.txt\n\
\n\
Änderungen, die nicht zum Commit vorgemerkt sind:\n\
  (benutzen Sie \"git add <Datei>...\", um die Änderungen zum Commit vorzumerken)\n\
\tgeändert:       alt.txt\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(1)"), "staged section lost: {result}");
        assert!(result.contains("neu.txt"));
        assert!(result.contains("modified(1)"), "unstaged geändert: entry dropped or misfiled: {result}");
        assert!(result.contains("alt.txt"));
    }

    /// Italian mixed status. Real unstaged header is "Modifiche non
    /// nell'area di staging per il commit:". Regression: unstaged entries
    /// must land in modified(), not be counted as staged.
    #[test]
    fn test_git_status_italian_mixed() {
        let output = "Sul branch master\n\n\
Modifiche di cui verrà eseguito il commit:\n\
  (usa \"git restore --staged <file>...\" per rimuovere gli elementi dall'area di staging)\n\
\tnuovo file:     nuovo.txt\n\
\n\
Modifiche non nell'area di staging per il commit:\n\
  (usa \"git add <file>...\" per aggiornare gli elementi di cui verrà eseguito il commit)\n\
\tmodificato:     vecchio.txt\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(1)"), "wrong staged count: {result}");
        assert!(result.contains("nuovo.txt"));
        assert!(result.contains("modified(1)"), "unstaged entry misfiled as staged: {result}");
        assert!(result.contains("vecchio.txt"));
    }

    /// French uses a no-break space before the colon in file labels
    /// ("modifié\u{a0}:") and the staged header marker is a substring of the
    /// unstaged header ("…qui ne seront pas validées :"). Both sections must
    /// classify correctly.
    #[test]
    fn test_git_status_french_mixed_nbsp() {
        let output = "Sur la branche main\n\n\
Modifications qui seront validées :\n\
  (utilisez \"git restore --staged <fichier>...\" pour désindexer)\n\
\tnouveau fichier\u{a0}: ajout.txt\n\
\n\
Modifications qui ne seront pas validées :\n\
  (utilisez \"git add <fichier>...\" pour mettre à jour ce qui sera validé)\n\
\tmodifié\u{a0}:         change.txt\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(1)"), "french staged lost: {result}");
        assert!(result.contains("ajout.txt"));
        assert!(result.contains("modified(1)"), "french unstaged misfiled as staged: {result}");
        assert!(result.contains("change.txt"));
    }

    /// Spanish (real strings: "Cambios a ser confirmados:", plural labels
    /// "nuevos archivos:").
    #[test]
    fn test_git_status_spanish_staged() {
        let output = "En la rama main\n\n\
Cambios a ser confirmados:\n\
  (usa \"git restore --staged <archivo>...\" para sacar del área de stage)\n\
\tnuevos archivos: hola.txt\n";
        let result = format_git_status(output);
        assert!(result.contains("staged(1)"), "spanish staged lost: {result}");
        assert!(result.contains("hola.txt"));
    }

    /// A locale with no marker-table entry (Russian) must pass the raw
    /// output through — never a partial or misclassified summary.
    #[test]
    fn test_git_status_unknown_locale_passthrough() {
        let output = "На ветке main\n\n\
Изменения, которые будут включены в коммит:\n\
\tновый файл:     файл.txt\n";
        let result = format_git_status(output);
        assert_eq!(result, output, "unknown locale must pass through raw");
    }

    /// Merge conflicts add an "Unmerged paths:" section we don't summarize.
    /// Its file entries must force raw passthrough, not vanish.
    #[test]
    fn test_git_status_unmerged_passthrough() {
        let output = "On branch main\n\n\
Unmerged paths:\n\
  (use \"git add <file>...\" to mark resolution)\n\
\tboth modified:   conflicted.txt\n";
        let result = format_git_status(output);
        assert_eq!(result, output, "conflict output must pass through raw");
    }

    /// Untracked-only output must summarize as untracked — and must not hit
    /// the "clean" branch even though git prints "nothing added to commit…".
    #[test]
    fn test_git_status_untracked_only() {
        let output = "On branch main\n\n\
Untracked files:\n\
  (use \"git add <file>...\" to include in what will be committed)\n\
\tstray.txt\n\
\n\
nothing added to commit but untracked files present (use \"git add\" to track)\n";
        let result = format_git_status(output);
        assert!(result.contains("untracked(1)"), "untracked lost: {result}");
        assert!(result.contains("stray.txt"));
        assert_ne!(result, "clean");
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
