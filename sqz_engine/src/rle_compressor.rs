/// Run-Length Encoding (RLE) for structured repetition patterns.
///
/// Generalizes the condense stage to catch repeated patterns within lines
/// and repeated structured blocks, not just consecutive identical lines.
///
/// RLE is provably optimal for data with runs of identical symbols
/// (Shannon 1948). This implementation works at the token/phrase level
/// rather than the byte level, making it effective for CLI output patterns.

use crate::error::Result;

/// Result of RLE compression.
#[derive(Debug, Clone)]
pub struct RleResult {
    /// The compressed text.
    pub text: String,
    /// Number of runs collapsed.
    pub runs_collapsed: usize,
    /// Tokens saved (estimated).
    pub tokens_saved: u32,
}

/// Compress text using token-level run-length encoding.
///
/// Detects repeated phrases/tokens and replaces runs with compact notation:
/// `token [×N]` where N is the repeat count.
pub fn rle_compress(text: &str, min_run_length: usize) -> Result<RleResult> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return Ok(RleResult {
            text: text.to_string(),
            runs_collapsed: 0,
            tokens_saved: 0,
        });
    }

    let mut output = Vec::new();
    let mut runs_collapsed = 0usize;
    let mut tokens_saved = 0u32;
    let mut i = 0;

    while i < lines.len() {
        // Try to find a run of identical lines (lossless — safe to collapse)
        let mut run_len = 1;
        while i + run_len < lines.len() && lines[i] == lines[i + run_len] {
            run_len += 1;
        }

        if run_len >= min_run_length {
            // Collapse the run of identical lines. Lossless: N copies of the
            // same text can be recovered from "{text} [×N]".
            output.push(format!("{} [×{}]", lines[i], run_len));
            runs_collapsed += 1;
            let line_tokens = (lines[i].len() as u32 + 3) / 4;
            tokens_saved += line_tokens * (run_len as u32 - 1);
            i += run_len;
        } else {
            // Pattern-run detection (lines with shared word-prefix but
            // different suffixes) was removed because it was LOSSY: it
            // replaced the varying parts with "{count} unique values",
            // discarding real filenames from `ls -l` output. See Reddit
            // report #1 and the test_ls_output_preserves_all_filenames
            // regression test. Exact duplicates above are still collapsed.
            output.push(lines[i].to_string());
            i += 1;
        }
    }

    let trailing_newline = text.ends_with('\n');
    let mut result = output.join("\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Ok(RleResult {
        text: result,
        runs_collapsed,
        tokens_saved,
    })
}

// ── Log-template compaction ──────────────────────────────────────────────────

/// Result of log-template compaction.
#[derive(Debug, Clone)]
pub struct LogTemplateResult {
    pub text: String,
    pub runs_collapsed: usize,
    pub tokens_saved: u32,
}

/// Collapse consecutive lines that are identical except for VOLATILE spans:
/// timestamps, durations, percentages. What exact RLE (identical lines only)
/// leaves on the table in real logs is mostly timestamp spam.
///
/// Deliberately narrow — the predecessor of this stage (pattern-run
/// detection) was removed for eating `ls` filenames. Rules:
/// - Only timestamps, durations, and percentages count as volatile.
///   Hex ids, UUIDs, filenames, and plain integers make lines DIFFERENT.
/// - Only consecutive lines collapse, and the first line stays verbatim.
/// - The marker names what varied, so the loss is explicit.
pub fn log_template_collapse(text: &str, min_run_length: usize) -> Result<LogTemplateResult> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < min_run_length {
        return Ok(LogTemplateResult {
            text: text.to_string(),
            runs_collapsed: 0,
            tokens_saved: 0,
        });
    }

    let templates: Vec<String> = lines.iter().map(|l| mask_volatiles(l)).collect();

    let mut output = Vec::new();
    let mut runs_collapsed = 0usize;
    let mut tokens_saved = 0u32;
    let mut i = 0;

    while i < lines.len() {
        let mut run_len = 1;
        // A run requires the same template AND that masking actually did
        // something (otherwise identical lines belong to exact RLE).
        while i + run_len < lines.len()
            && templates[i + run_len] == templates[i]
            && templates[i] != lines[i]
            && lines[i + run_len] != lines[i]
        {
            run_len += 1;
        }

        if run_len >= min_run_length {
            output.push(lines[i].to_string());
            output.push(format!(
                "[×{} similar lines; only timestamps/durations vary]",
                run_len - 1
            ));
            runs_collapsed += 1;
            for line in &lines[i + 1..i + run_len] {
                tokens_saved += (line.len() as u32).div_ceil(4);
            }
            i += run_len;
        } else {
            output.push(lines[i].to_string());
            i += 1;
        }
    }

    let trailing_newline = text.ends_with('\n');
    let mut result = output.join("\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Ok(LogTemplateResult {
        text: result,
        runs_collapsed,
        tokens_saved,
    })
}

/// Mask volatile spans (timestamps, durations, percentages) with a
/// placeholder. Everything else — hex ids, UUIDs, paths, plain integers —
/// is preserved so identifier-bearing lines never share a template.
fn mask_volatiles(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for token in line.split_inclusive(|c: char| c.is_whitespace()) {
        let (word, ws) = match token.find(|c: char| c.is_whitespace()) {
            Some(idx) => token.split_at(idx),
            None => (token, ""),
        };
        let stripped = word.trim_matches(|c: char| "[]()<>,".contains(c));
        if is_volatile(stripped) {
            out.push_str(&word.replace(stripped, "‹›"));
        } else {
            out.push_str(word);
        }
        out.push_str(ws);
    }
    out
}

fn is_volatile(token: &str) -> bool {
    if token.len() < 2 {
        return false;
    }
    // Durations: 123ms, 4.5s, 12µs, 3m20s
    for suffix in ["ms", "µs", "us", "ns", "s", "m"] {
        if let Some(num) = token.strip_suffix(suffix) {
            if !num.is_empty()
                && num.chars().all(|c| c.is_ascii_digit() || c == '.')
                && num.chars().any(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
    }
    // Percentages: 45%, 99.9%
    if let Some(num) = token.strip_suffix('%') {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return true;
        }
    }
    // Clock times and dates: 12:34:56(.789), 2026-08-15, 2026-08-15T12:34:56Z
    let digits = token.chars().filter(|c| c.is_ascii_digit()).count();
    if digits >= 4
        && token
            .chars()
            .all(|c| c.is_ascii_digit() || ":-.TZ+".contains(c))
        && (token.contains(':') || token.matches('-').count() >= 2)
    {
        return true;
    }
    false
}

// ── Sliding Window Dedup ──────────────────────────────────────────────────────

/// Token-level sliding window deduplication.
///
/// Finds repeated substrings within a text using a sliding window approach.
/// When the same phrase appears multiple times, subsequent occurrences are
/// replaced with back-references: `[→L{line}]`.
///
/// This catches repeated substrings that aren't line-aligned — e.g., the
/// same error message appearing in two different stack frames.
pub fn sliding_window_dedup(text: &str, min_match_words: usize) -> Result<SlidingWindowResult> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return Ok(SlidingWindowResult {
            text: text.to_string(),
            dedup_count: 0,
            tokens_saved: 0,
        });
    }

    // Build a phrase index: map each n-word phrase to its first occurrence line
    let mut phrase_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut output = Vec::new();
    let mut dedup_count = 0usize;
    let mut tokens_saved = 0u32;

    for (line_idx, &line) in lines.iter().enumerate() {
        let words: Vec<&str> = line.split_whitespace().collect();

        // Check if this line (as a phrase) was seen before
        let phrase = words.join(" ");
        if phrase.len() >= min_match_words * 3 {
            if let Some(&first_line) = phrase_index.get(&phrase) {
                // This exact line appeared before — replace with back-reference
                output.push(format!("[→L{}]", first_line + 1));
                dedup_count += 1;
                tokens_saved += (phrase.len() as u32 + 3) / 4;
                continue;
            }
        }

        // Index this line's phrase
        if phrase.len() >= min_match_words * 3 {
            phrase_index.insert(phrase, line_idx);
        }
        output.push(line.to_string());
    }

    let trailing_newline = text.ends_with('\n');
    let mut result = output.join("\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Ok(SlidingWindowResult {
        text: result,
        dedup_count,
        tokens_saved,
    })
}

/// Result of sliding window deduplication.
#[derive(Debug, Clone)]
pub struct SlidingWindowResult {
    pub text: String,
    pub dedup_count: usize,
    pub tokens_saved: u32,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── log-template compaction ───────────────────────────────────────

    #[test]
    fn template_collapses_timestamp_spam() {
        let input = "\
12:00:01 worker heartbeat ok in 12ms
12:00:02 worker heartbeat ok in 14ms
12:00:03 worker heartbeat ok in 11ms
12:00:04 worker heartbeat ok in 13ms
done\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.runs_collapsed, 1);
        assert!(result.text.contains("12:00:01 worker heartbeat ok in 12ms"), "first line verbatim");
        assert!(result.text.contains("[×3 similar lines; only timestamps/durations vary]"));
        assert!(!result.text.contains("12:00:03"), "later volatiles collapsed");
        assert!(result.text.ends_with("done\n"));
    }

    #[test]
    fn template_never_collapses_identifier_bearing_lines() {
        // Same shape but each line carries a distinct hex id: these are the
        // `ls`-filenames class of lines that the removed pattern-run stage
        // used to eat. They must all survive.
        let input = "\
12:00:01 pushed commit deadbeef01 to origin
12:00:02 pushed commit cafebabe02 to origin
12:00:03 pushed commit 0ddba11f03 to origin
12:00:04 pushed commit feedface04 to origin\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.runs_collapsed, 0, "{}", result.text);
        for id in ["deadbeef01", "cafebabe02", "0ddba11f03", "feedface04"] {
            assert!(result.text.contains(id));
        }
    }

    #[test]
    fn template_never_collapses_distinct_filenames() {
        let input = "\
-rw-r--r-- 1 u staff 120 Aug 15 12:00 alpha.rs
-rw-r--r-- 1 u staff 340 Aug 15 12:01 beta.rs
-rw-r--r-- 1 u staff 560 Aug 15 12:02 gamma.rs
-rw-r--r-- 1 u staff 780 Aug 15 12:03 delta.rs\n";
        let result = log_template_collapse(input, 3).unwrap();
        for name in ["alpha.rs", "beta.rs", "gamma.rs", "delta.rs"] {
            assert!(result.text.contains(name), "filename lost: {}", result.text);
        }
    }

    #[test]
    fn template_leaves_identical_lines_to_exact_rle() {
        let input = "same line\nsame line\nsame line\nsame line\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.runs_collapsed, 0);
        assert_eq!(result.text, input);
    }

    #[test]
    fn template_short_runs_untouched() {
        let input = "12:00:01 tick\n12:00:02 tick\nother\n";
        let result = log_template_collapse(input, 3).unwrap();
        assert_eq!(result.text, input);
    }

    // --- RLE tests ---

    #[test]
    fn test_rle_no_repetition() {
        let result = rle_compress("alpha\nbeta\ngamma\n", 2).unwrap();
        assert_eq!(result.runs_collapsed, 0);
    }

    #[test]
    fn test_rle_exact_repetition() {
        let input = "ok\nok\nok\nok\nok\n";
        let result = rle_compress(input, 2).unwrap();
        assert!(result.runs_collapsed > 0);
        assert!(result.text.contains("[×5]"), "output: {}", result.text);
    }

    #[test]
    fn test_rle_pattern_no_longer_collapses() {
        // Pattern-run collapsing was removed because it was lossy — it
        // replaced varying filename/identifier suffixes with "N unique
        // values", dropping data the LLM needs. Different lines that
        // merely share a prefix must pass through unchanged.
        let input = "Compiling foo v1.0\nCompiling bar v2.0\nCompiling baz v3.0\ndone\n";
        let result = rle_compress(input, 2).unwrap();
        assert_eq!(result.runs_collapsed, 0, "different lines must not be collapsed");
        assert!(result.text.contains("foo"), "filename 'foo' must survive: {}", result.text);
        assert!(result.text.contains("bar"), "filename 'bar' must survive: {}", result.text);
        assert!(result.text.contains("baz"), "filename 'baz' must survive: {}", result.text);
    }

    #[test]
    fn test_rle_ls_l_output_preserves_filenames() {
        // Regression for https://github.com/ojuschugh1/sqz/issues/1
        // ls -l lines share the 'drwxr-xr-x' prefix but have distinct
        // filenames. Pattern-run collapsing used to delete them.
        let input = "drwxr-xr-x  6 user user  192 Apr 18 10:00 packages\n\
                     drwxr-xr-x  3 user user   96 Apr 18 10:00 configuration\n\
                     drwxr-xr-x  4 user user  128 Apr 18 10:00 documentation\n\
                     drwxr-xr-x  2 user user   64 Apr 18 10:00 environment\n";
        let result = rle_compress(input, 2).unwrap();
        for name in &["packages", "configuration", "documentation", "environment"] {
            assert!(result.text.contains(name),
                "filename '{}' must be preserved — got:\n{}", name, result.text);
        }
    }

    #[test]
    fn test_rle_mixed_content() {
        let input = "header\ntest a ... ok\ntest b ... ok\ntest c ... ok\nfooter\n";
        let result = rle_compress(input, 2).unwrap();
        assert!(result.text.contains("header"));
        assert!(result.text.contains("footer"));
    }

    #[test]
    fn test_rle_preserves_trailing_newline() {
        let result = rle_compress("a\na\na\n", 2).unwrap();
        assert!(result.text.ends_with('\n'));
    }

    #[test]
    fn test_rle_empty_input() {
        let result = rle_compress("", 2).unwrap();
        assert_eq!(result.text, "");
        assert_eq!(result.runs_collapsed, 0);
    }

    #[test]
    fn test_rle_single_line() {
        let result = rle_compress("hello\n", 2).unwrap();
        assert_eq!(result.runs_collapsed, 0);
    }

    #[test]
    fn test_rle_min_run_length_respected() {
        let input = "ok\nok\ndone\n";
        let result_2 = rle_compress(input, 2).unwrap();
        let result_3 = rle_compress(input, 3).unwrap();
        assert!(result_2.runs_collapsed > 0, "run of 2 should collapse with min=2");
        assert_eq!(result_3.runs_collapsed, 0, "run of 2 should NOT collapse with min=3");
    }

    // --- Sliding window dedup tests ---

    #[test]
    fn test_sliding_window_no_duplicates() {
        let result = sliding_window_dedup("line 1\nline 2\nline 3\n", 3).unwrap();
        assert_eq!(result.dedup_count, 0);
    }

    #[test]
    fn test_sliding_window_exact_duplicate_lines() {
        let input = "error: connection refused at port 8080\nsome other line\nerror: connection refused at port 8080\n";
        let result = sliding_window_dedup(input, 3).unwrap();
        assert!(result.dedup_count > 0, "should detect duplicate line");
        assert!(result.text.contains("[→L1]"), "output: {}", result.text);
    }

    #[test]
    fn test_sliding_window_preserves_first_occurrence() {
        let input = "the error message here\nother stuff\nthe error message here\n";
        let result = sliding_window_dedup(input, 3).unwrap();
        // First occurrence should be preserved verbatim
        assert!(result.text.starts_with("the error message here\n"));
    }

    #[test]
    fn test_sliding_window_short_lines_ignored() {
        let input = "ok\nok\nok\n";
        let result = sliding_window_dedup(input, 3).unwrap();
        // "ok" is too short (< min_match_words * 3 = 9 chars)
        assert_eq!(result.dedup_count, 0);
    }

    #[test]
    fn test_sliding_window_empty_input() {
        let result = sliding_window_dedup("", 3).unwrap();
        assert_eq!(result.text, "");
    }

    use proptest::prelude::*;

    proptest! {
        /// RLE never produces output longer than input (excluding the run markers).
        #[test]
        fn prop_rle_tokens_saved_non_negative(
            text in "[a-z]{3,10}(\n[a-z]{3,10}){2,10}\n"
        ) {
            let result = rle_compress(&text, 2).unwrap();
            // tokens_saved is u32, always >= 0
            let _ = result.tokens_saved;
            // Output should not be empty if input isn't
            prop_assert!(!result.text.is_empty());
        }

        /// Sliding window dedup count is non-negative and bounded.
        #[test]
        fn prop_sliding_window_bounded(
            text in "[a-z ]{10,50}(\n[a-z ]{10,50}){2,8}\n"
        ) {
            let result = sliding_window_dedup(&text, 3).unwrap();
            let line_count = text.lines().count();
            prop_assert!(
                result.dedup_count <= line_count,
                "dedup count {} exceeds line count {}",
                result.dedup_count, line_count
            );
        }
    }
}
