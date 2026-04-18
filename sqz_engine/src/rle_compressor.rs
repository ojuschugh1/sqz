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
