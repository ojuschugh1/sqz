/// Minimum Description Length (MDL) stage selection.
///
/// MDL principle (Rissanen 1978): the best compression is the one that
/// minimizes description_length(model) + description_length(data|model).
///
/// Instead of running all 16 stages on every input, MDL selects the optimal
/// subset of stages for each content type. Stages that add overhead (headers,
/// legends) without sufficient compression are skipped.

use crate::error::Result;

/// A stage candidate with its estimated cost and benefit.
#[derive(Debug, Clone)]
pub struct StageCandidate {
    /// Stage name.
    pub name: String,
    /// Estimated overhead in tokens (headers, legends, markers).
    pub overhead_tokens: u32,
    /// Estimated savings in tokens.
    pub savings_tokens: u32,
    /// Whether this stage is applicable to the content type.
    pub applicable: bool,
}

impl StageCandidate {
    /// Net benefit: savings minus overhead. Negative means the stage hurts.
    pub fn net_benefit(&self) -> i32 {
        self.savings_tokens as i32 - self.overhead_tokens as i32
    }
}

/// Result of MDL stage selection.
#[derive(Debug, Clone)]
pub struct MdlSelection {
    /// Stages to enable (in order).
    pub enabled_stages: Vec<String>,
    /// Stages skipped because they would add overhead.
    pub skipped_stages: Vec<String>,
    /// Total estimated net savings.
    pub estimated_net_savings: i32,
}

/// Content characteristics used for stage selection.
#[derive(Debug, Clone)]
pub struct ContentProfile {
    /// Is the content JSON?
    pub is_json: bool,
    /// Content length in bytes.
    pub length: usize,
    /// Estimated token count.
    pub tokens: u32,
    /// Does the content have repeated lines?
    pub has_repetition: bool,
    /// Does the content look like a diff?
    pub is_diff: bool,
    /// Does the content look like prose?
    pub is_prose: bool,
    /// Does the content look like log output?
    pub is_log: bool,
    /// Number of null fields (JSON only).
    pub null_count: usize,
    /// Number of array elements (JSON only).
    pub array_element_count: usize,
}

/// Select the optimal subset of compression stages for the given content.
///
/// Uses the MDL principle: only enable stages where the expected savings
/// exceed the overhead. This avoids the problem where stages like dict
/// compression or RLE add headers that make small payloads larger.
pub fn select_stages(profile: &ContentProfile) -> MdlSelection {
    let mut candidates = build_candidates(profile);

    // Sort by net benefit descending
    candidates.sort_by(|a, b| b.net_benefit().cmp(&a.net_benefit()));

    let mut enabled = Vec::new();
    let mut skipped = Vec::new();
    let mut total_net = 0i32;

    for candidate in &candidates {
        if !candidate.applicable {
            skipped.push(candidate.name.clone());
            continue;
        }

        if candidate.net_benefit() > 0 {
            enabled.push(candidate.name.clone());
            total_net += candidate.net_benefit();
        } else {
            skipped.push(candidate.name.clone());
        }
    }

    // Always include ansi_strip (zero overhead, always beneficial)
    if !enabled.contains(&"ansi_strip".to_string()) {
        enabled.insert(0, "ansi_strip".to_string());
    }

    MdlSelection {
        enabled_stages: enabled,
        skipped_stages: skipped,
        estimated_net_savings: total_net,
    }
}

/// Build stage candidates with estimated costs and benefits.
fn build_candidates(p: &ContentProfile) -> Vec<StageCandidate> {
    vec![
        StageCandidate {
            name: "strip_nulls".to_string(),
            overhead_tokens: 0,
            savings_tokens: if p.is_json { (p.null_count as u32) * 3 } else { 0 },
            applicable: p.is_json && p.null_count > 0,
        },
        StageCandidate {
            name: "condense".to_string(),
            overhead_tokens: 0,
            savings_tokens: if p.has_repetition { p.tokens / 4 } else { 0 },
            applicable: !p.is_json && p.has_repetition,
        },
        StageCandidate {
            name: "git_diff_fold".to_string(),
            overhead_tokens: 2, // "[N unchanged lines]" markers
            savings_tokens: if p.is_diff { p.tokens / 3 } else { 0 },
            applicable: p.is_diff,
        },
        StageCandidate {
            name: "collapse_arrays".to_string(),
            overhead_tokens: 5, // summary string or table header
            savings_tokens: if p.is_json && p.array_element_count > 10 {
                (p.array_element_count as u32 - 5) * 4
            } else {
                0
            },
            applicable: p.is_json && p.array_element_count > 10,
        },
        StageCandidate {
            name: "flatten".to_string(),
            overhead_tokens: 0,
            savings_tokens: if p.is_json { p.tokens / 20 } else { 0 },
            applicable: p.is_json && p.tokens > 50,
        },
        StageCandidate {
            name: "truncate_strings".to_string(),
            overhead_tokens: 1, // "..." per truncation
            savings_tokens: if p.is_json { p.tokens / 10 } else { 0 },
            applicable: p.is_json && p.tokens > 100,
        },
        StageCandidate {
            name: "rle".to_string(),
            overhead_tokens: 3, // "[×N]" markers
            savings_tokens: if p.has_repetition && !p.is_json { p.tokens / 5 } else { 0 },
            applicable: !p.is_json && p.has_repetition && p.length > 200,
        },
        StageCandidate {
            name: "sliding_window_dedup".to_string(),
            overhead_tokens: 2, // "[→LN]" markers
            savings_tokens: if p.has_repetition && !p.is_json { p.tokens / 8 } else { 0 },
            applicable: !p.is_json && p.has_repetition && p.length > 300,
        },
        StageCandidate {
            name: "entropy_truncate".to_string(),
            overhead_tokens: 3, // "[N segments omitted]"
            savings_tokens: if p.is_prose && p.tokens > 100 { p.tokens / 6 } else { 0 },
            applicable: !p.is_json && p.is_prose && p.length > 500,
        },
        StageCandidate {
            name: "token_prune".to_string(),
            overhead_tokens: 0,
            savings_tokens: if p.is_prose { p.tokens / 10 } else { 0 },
            applicable: !p.is_json && p.is_prose && p.length > 100,
        },
        StageCandidate {
            name: "dict_compress".to_string(),
            overhead_tokens: 15, // §dict§ header
            savings_tokens: if p.is_json && p.tokens > 50 { p.tokens / 8 } else { 0 },
            applicable: p.is_json && p.tokens > 120, // only worth it for larger JSON
        },
        StageCandidate {
            name: "toon_encode".to_string(),
            overhead_tokens: 2, // "TOON:" prefix
            savings_tokens: if p.is_json { p.tokens / 5 } else { 0 },
            applicable: p.is_json,
        },
        StageCandidate {
            name: "textrank".to_string(),
            overhead_tokens: 0,
            savings_tokens: if p.is_prose && p.tokens > 200 { p.tokens / 3 } else { 0 },
            applicable: p.is_prose && p.tokens > 200,
        },
    ]
}

/// Profile content to determine its characteristics.
pub fn profile_content(text: &str) -> ContentProfile {
    let is_json = text.trim().starts_with('{') || text.trim().starts_with('[');
    let lines: Vec<&str> = text.lines().collect();
    let tokens = (text.len() as u32 + 3) / 4;

    // Check for repetition
    let mut has_repetition = false;
    if lines.len() > 2 {
        let mut prev = "";
        let mut run = 0;
        for line in &lines {
            if *line == prev {
                run += 1;
                if run >= 2 {
                    has_repetition = true;
                    break;
                }
            } else {
                run = 0;
            }
            prev = line;
        }
    }

    let is_diff = text.contains("\n+") && text.contains("\n-") && text.contains("@@");
    let is_log = lines.iter().take(10).any(|l| {
        l.contains("[INFO]") || l.contains("[ERROR]") || l.contains("[WARN]")
    });

    // Prose heuristic
    let mut prose_lines = 0;
    let mut code_lines = 0;
    for line in lines.iter().take(20) {
        let t = line.trim();
        if t.is_empty() { continue; }
        if t.ends_with('{') || t.ends_with(';') || t.contains("::") || t.contains("->") {
            code_lines += 1;
        } else {
            prose_lines += 1;
        }
    }
    let is_prose = prose_lines > code_lines && !is_json && !is_diff && !is_log;

    // JSON-specific metrics
    let null_count = if is_json { text.matches(":null").count() + text.matches(": null").count() } else { 0 };
    let array_element_count = if is_json {
        text.matches('{').count().saturating_sub(1) // rough estimate
    } else {
        0
    };

    ContentProfile {
        is_json,
        length: text.len(),
        tokens,
        has_repetition,
        is_diff,
        is_prose,
        is_log,
        null_count,
        array_element_count,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_profile() {
        let p = profile_content(r#"{"a":1,"b":null,"c":null}"#);
        assert!(p.is_json);
        assert_eq!(p.null_count, 2);
        assert!(!p.is_prose);
    }

    #[test]
    fn test_prose_profile() {
        let p = profile_content("This is a normal sentence about something interesting and important.");
        assert!(p.is_prose);
        assert!(!p.is_json);
    }

    #[test]
    fn test_diff_profile() {
        let p = profile_content("@@ -1,5 +1,5 @@\n-old\n+new\n context\n");
        assert!(p.is_diff);
    }

    #[test]
    fn test_log_profile() {
        let p = profile_content("[INFO] Started\n[ERROR] Failed\n[WARN] Slow\n");
        assert!(p.is_log);
    }

    #[test]
    fn test_select_stages_json() {
        let p = ContentProfile {
            is_json: true,
            length: 500,
            tokens: 125,
            has_repetition: false,
            is_diff: false,
            is_prose: false,
            is_log: false,
            null_count: 5,
            array_element_count: 0,
        };
        let sel = select_stages(&p);
        assert!(sel.enabled_stages.contains(&"strip_nulls".to_string()));
        assert!(sel.enabled_stages.contains(&"toon_encode".to_string()));
        assert!(!sel.enabled_stages.contains(&"condense".to_string()));
    }

    #[test]
    fn test_select_stages_prose() {
        let p = ContentProfile {
            is_json: false,
            length: 1000,
            tokens: 250,
            has_repetition: false,
            is_diff: false,
            is_prose: true,
            is_log: false,
            null_count: 0,
            array_element_count: 0,
        };
        let sel = select_stages(&p);
        assert!(sel.enabled_stages.contains(&"token_prune".to_string()));
        assert!(sel.enabled_stages.contains(&"textrank".to_string()));
        assert!(!sel.enabled_stages.contains(&"strip_nulls".to_string()));
    }

    #[test]
    fn test_select_stages_always_includes_ansi_strip() {
        let p = ContentProfile {
            is_json: false,
            length: 10,
            tokens: 3,
            has_repetition: false,
            is_diff: false,
            is_prose: false,
            is_log: false,
            null_count: 0,
            array_element_count: 0,
        };
        let sel = select_stages(&p);
        assert!(sel.enabled_stages.contains(&"ansi_strip".to_string()));
    }

    #[test]
    fn test_select_stages_skips_overhead_for_small_json() {
        let p = ContentProfile {
            is_json: true,
            length: 50,
            tokens: 12,
            has_repetition: false,
            is_diff: false,
            is_prose: false,
            is_log: false,
            null_count: 0,
            array_element_count: 0,
        };
        let sel = select_stages(&p);
        // dict_compress has 15 token overhead — not worth it for 12 tokens
        assert!(!sel.enabled_stages.contains(&"dict_compress".to_string()));
    }

    #[test]
    fn test_net_benefit_calculation() {
        let c = StageCandidate {
            name: "test".to_string(),
            overhead_tokens: 5,
            savings_tokens: 20,
            applicable: true,
        };
        assert_eq!(c.net_benefit(), 15);

        let c2 = StageCandidate {
            name: "test".to_string(),
            overhead_tokens: 20,
            savings_tokens: 5,
            applicable: true,
        };
        assert_eq!(c2.net_benefit(), -15);
    }

    #[test]
    fn test_repetitive_log_profile() {
        let p = profile_content("[INFO] ok\n[INFO] ok\n[INFO] ok\n[ERROR] fail\n");
        assert!(p.is_log);
        assert!(p.has_repetition);
    }
}
