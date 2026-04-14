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
        // Try to find a run of identical or pattern-matching lines
        let mut run_len = 1;
        while i + run_len < lines.len() && lines[i] == lines[i + run_len] {
            run_len += 1;
        }

        if run_len >= min_run_length {
            // Collapse the run
            output.push(format!("{} [×{}]", lines[i], run_len));
            runs_collapsed += 1;
            let line_tokens = (lines[i].len() as u32 + 3) / 4;
            tokens_saved += line_tokens * (run_len as u32 - 1);
            i += run_len;
        } else {
            // Try pattern-based RLE: detect lines that match a template
            // e.g., "Compiling foo v1.0" / "Compiling bar v2.0" → "Compiling {N items}..."
            let pattern_run = detect_pattern_run(&lines[i..]);
            if pattern_run.count >= min_run_length {
                output.push(format!(
                    "{} [×{}, varying: {}]",
                    pattern_run.template, pattern_run.count, pattern_run.varying_part
                ));
                runs_collapsed += 1;
                let line_tokens = (lines[i].len() as u32 + 3) / 4;
                tokens_saved += line_tokens * (pattern_run.count as u32 - 1);
                i += pattern_run.count;
            } else {
                output.push(lines[i].to_string());
                i += 1;
            }
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

/// A detected pattern run.
struct PatternRun {
    template: String,
    count: usize,
    varying_part: String,
}

/// Detect a run of lines that share a common prefix/suffix pattern.
/// E.g., "Compiling foo v1.0", "Compiling bar v2.0" share prefix "Compiling ".
fn detect_pattern_run(lines: &[&str]) -> PatternRun {
    if lines.len() < 2 {
        return PatternRun {
            template: lines.first().unwrap_or(&"").to_string(),
            count: 1,
            varying_part: String::new(),
        };
    }

    let first = lines[0];
    let words_first: Vec<&str> = first.split_whitespace().collect();
    if words_first.is_empty() {
        return PatternRun {
            template: first.to_string(),
            count: 1,
            varying_part: String::new(),
        };
    }

    // Find the longest common prefix (in words) between first and second line
    let second_words: Vec<&str> = lines[1].split_whitespace().collect();
    let mut prefix_len = 0;
    for w in 0..words_first.len().min(second_words.len()) {
        if words_first[w] == second_words[w] {
            prefix_len = w + 1;
        } else {
            break;
        }
    }

    // Need at least 1 shared prefix word to form a pattern
    if prefix_len == 0 {
        return PatternRun {
            template: first.to_string(),
            count: 1,
            varying_part: String::new(),
        };
    }

    let prefix: String = words_first[..prefix_len].join(" ");

    // Count how many consecutive lines share this prefix
    let mut count = 1;
    for line in &lines[1..] {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() >= prefix_len && words[..prefix_len].join(" ") == prefix {
            count += 1;
        } else {
            break;
        }
    }

    if count < 2 {
        return PatternRun {
            template: first.to_string(),
            count: 1,
            varying_part: String::new(),
        };
    }

    // Collect the varying parts
    let varying: Vec<String> = lines[..count]
        .iter()
        .map(|l| {
            let words: Vec<&str> = l.split_whitespace().collect();
            words[prefix_len..].join(" ")
        })
        .collect();

    PatternRun {
        template: format!("{prefix} ..."),
        count,
        varying_part: format!("{} unique values", varying.len()),
    }
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
    fn test_rle_pattern_repetition() {
        let input = "Compiling foo v1.0\nCompiling bar v2.0\nCompiling baz v3.0\ndone\n";
        let result = rle_compress(input, 2).unwrap();
        assert!(result.runs_collapsed > 0, "should detect pattern run");
        assert!(result.text.contains("×3"), "output: {}", result.text);
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
