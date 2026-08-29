//! Generic aligned-column table compactor.
//!
//! Many CLIs pad output into aligned columns (`docker ps`, `kubectl get`,
//! `ps aux`, database shells). The padding is pure token waste: the
//! alignment carries no information a model needs. This stage detects a
//! shared column grid and collapses each separator run to exactly two
//! spaces, preserving every cell byte-for-byte — cells keep their single
//! internal spaces, so columns stay parseable.
//!
//! The detector is deliberately strict so indentation-significant content
//! (source code, YAML, diffs, pretty JSON) never triggers it:
//!
//! * at least 5 non-empty lines, none starting with whitespace
//! * a column position must be gap-covered by every line (or past the
//!   line's end) to count as a separator
//! * at least one separator band where 80% of lines have a real run of
//!   two or more spaces

/// Result of a successful compaction.
#[derive(Debug, Clone)]
pub struct TableCompactResult {
    pub text: String,
    /// Number of lines that had at least one separator collapsed.
    pub rows_compacted: usize,
}

const MIN_ROWS: usize = 5;
const SEPARATOR: &str = "  ";

/// Detect an aligned-column table in `text` and collapse the alignment
/// padding. Returns `None` when the content does not look like a table
/// or when compaction would not shrink it.
pub fn compact_aligned_table(text: &str) -> Option<TableCompactResult> {
    let lines: Vec<&str> = text.lines().collect();
    let non_empty: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if non_empty.len() < MIN_ROWS {
        return None;
    }
    // Leading whitespace anywhere = indentation-significant content
    // (code, YAML, diff context lines, pretty JSON). Never touch it.
    if non_empty
        .iter()
        .any(|l| l.starts_with(' ') || l.starts_with('\t'))
    {
        return None;
    }
    // Embedded tabs make column positions unreliable.
    if non_empty.iter().any(|l| l.contains('\t')) {
        return None;
    }

    let rows: Vec<Vec<char>> = non_empty.iter().map(|l| l.chars().collect()).collect();
    let max_len = rows.iter().map(|r| r.len()).max()?;

    // A column position is a separator candidate when EVERY line either
    // ended before it or has a 2+ space gap covering it.
    let gap_maps: Vec<Vec<bool>> = rows.iter().map(|r| gap_map(r)).collect();
    let mut is_separator = vec![false; max_len];
    for (p, sep) in is_separator.iter_mut().enumerate() {
        *sep = rows.iter().zip(&gap_maps).all(|(row, gaps)| {
            p >= row.len() || gaps[p]
        });
    }

    // Group consecutive separator positions into bands; a band must not
    // start at column 0 (that would be leading indentation, excluded
    // above anyway) and must be at least 2 columns wide.
    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for p in 0..max_len {
        match (is_separator[p], start) {
            (true, None) => start = Some(p),
            (false, Some(s)) => {
                if p - s >= 2 && s > 0 {
                    bands.push((s, p));
                }
                start = None;
            }
            _ => {}
        }
    }
    // A band running to max_len is trailing padding shared by every
    // line; collapsing it saves nothing meaningful, so drop it.
    if bands.is_empty() {
        return None;
    }
    // Require at least one band that is a REAL gap (2+ spaces present,
    // not just end-of-line) in 80% of lines — otherwise ragged prose
    // with one accidental alignment could sneak through.
    let strong_band = bands.iter().any(|&(s, e)| {
        let real = rows
            .iter()
            .zip(&gap_maps)
            .filter(|(row, gaps)| {
                (s..e).any(|p| p < row.len() && gaps[p])
            })
            .count();
        real * 10 >= rows.len() * 8
    });
    if !strong_band {
        return None;
    }

    // Rewrite every line: any run of 2+ spaces that overlaps a band
    // collapses to the two-space separator. Runs elsewhere are left
    // alone (they are inside a cell, wherever a CLI allows that).
    let mut rows_compacted = 0usize;
    let rewritten: Vec<String> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                return (*line).to_string();
            }
            let chars: Vec<char> = line.chars().collect();
            let gaps = gap_map(&chars);
            let mut out = String::with_capacity(line.len());
            let mut i = 0;
            let mut changed = false;
            while i < chars.len() {
                if gaps[i] {
                    let run_start = i;
                    while i < chars.len() && gaps[i] {
                        i += 1;
                    }
                    let overlaps_band = bands
                        .iter()
                        .any(|&(s, e)| run_start < e && i > s);
                    if overlaps_band {
                        out.push_str(SEPARATOR);
                        if i - run_start > SEPARATOR.len() {
                            changed = true;
                        }
                    } else {
                        out.extend(&chars[run_start..i]);
                    }
                } else {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            if changed {
                rows_compacted += 1;
            }
            out
        })
        .collect();

    if rows_compacted == 0 {
        return None;
    }
    let mut text_out = rewritten.join("\n");
    if text.ends_with('\n') {
        text_out.push('\n');
    }
    if text_out.len() >= text.len() {
        return None;
    }
    Some(TableCompactResult {
        text: text_out,
        rows_compacted,
    })
}

/// Per-position map of "this char is part of a 2+ space run".
fn gap_map(chars: &[char]) -> Vec<bool> {
    let mut map = vec![false; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            let start = i;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if i - start >= 2 {
                for slot in &mut map[start..i] {
                    *slot = true;
                }
            }
        } else {
            i += 1;
        }
    }
    map
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKER_PS: &str = "\
CONTAINER ID   IMAGE          COMMAND                  STATUS          PORTS                    NAMES
3f4a5b6c7d8e   nginx:latest   \"/docker-entrypoint.\"   Up 2 hours      0.0.0.0:8080->80/tcp     web-frontend
9a8b7c6d5e4f   postgres:16    \"docker-entrypoint.s\"   Up 2 hours      5432/tcp                 db-primary
1c2d3e4f5a6b   redis:7        \"docker-entrypoint.s\"   Up 45 minutes   6379/tcp                 cache
7e8f9a0b1c2d   grafana/graf   \"/run.sh\"                Up 12 minutes   0.0.0.0:3000->3000/tcp   metrics-ui
5a4b3c2d1e0f   alpine:3.20    \"sleep infinity\"         Up 3 seconds    <none>                   scratchpad
";

    #[test]
    fn docker_ps_compacts_and_preserves_cells() {
        let result = compact_aligned_table(DOCKER_PS).expect("docker ps must compact");
        assert!(result.text.len() < DOCKER_PS.len());
        assert!(result.rows_compacted >= 5);
        // Every cell value survives byte-exact, including cells with
        // single internal spaces.
        for cell in [
            "CONTAINER ID",
            "Up 2 hours",
            "Up 45 minutes",
            "0.0.0.0:8080->80/tcp",
            "web-frontend",
            "db-primary",
            "\"sleep infinity\"",
        ] {
            assert!(result.text.contains(cell), "lost cell {cell:?}:\n{}", result.text);
        }
        // Separators collapsed to exactly two spaces: no 3+ space runs remain.
        assert!(!result.text.contains("   "), "residual padding:\n{}", result.text);
    }

    #[test]
    fn kubectl_get_pods_compacts() {
        let input = "\
NAME                          READY   STATUS    RESTARTS   AGE
api-6d4cf56db6-2m5xk          1/1     Running   0          4d20h
api-6d4cf56db6-9lq2v          1/1     Running   0          4d20h
worker-7c9f8b5d44-abcde       1/1     Running   2          12h
worker-7c9f8b5d44-fghij       0/1     Pending   0          3m
ingress-nginx-controller-x    1/1     Running   0          30d
";
        let result = compact_aligned_table(input).expect("kubectl output must compact");
        assert!(result.text.contains("api-6d4cf56db6-2m5xk  1/1  Running  0  4d20h"));
    }

    #[test]
    fn source_code_never_triggers() {
        let input = "\
fn main() {
    let x = 1;
    let y = 2;
    if x < y {
        println!(\"{}\", x);
    }
    let aligned    = 3;
    let alignment  = 4;
}
";
        assert!(compact_aligned_table(input).is_none(), "indented code must not compact");
    }

    #[test]
    fn pretty_json_never_triggers() {
        let input = "\
{
  \"name\": \"sqz\",
  \"version\": \"1.6.1\",
  \"nested\": {
    \"a\":  1,
    \"b\":  2,
    \"c\":  3
  }
}
";
        assert!(compact_aligned_table(input).is_none());
    }

    #[test]
    fn diff_context_never_triggers() {
        let input = "\
+added line one with content
-removed line with content
 context line kept as-is
+added line two with content
 another context line here
 third context line present
";
        assert!(compact_aligned_table(input).is_none(), "diff context lines start with a space");
    }

    #[test]
    fn too_few_rows_never_triggers() {
        let input = "\
NAME     AGE
first    1d
second   2d
";
        assert!(compact_aligned_table(input).is_none());
    }

    #[test]
    fn prose_without_shared_grid_never_triggers() {
        let input = "\
This is an ordinary paragraph of text without any columns.
It has several lines, more than the minimum row count requires.
None of them share  a consistent gap position with the others.
Some lines have a  double space here and there, but never aligned.
The detector must not touch this content at all, ever.
Final line to push the count over the minimum threshold.
";
        assert!(compact_aligned_table(input).is_none());
    }

    #[test]
    fn ragged_last_column_still_compacts() {
        let input = "\
PID    COMMAND         TIME
101    /usr/bin/a      0:01
2      b               12:44
30303  /opt/long/c     0:59
44     dd              1:02
5      e               100:00
";
        let result = compact_aligned_table(input).expect("ragged table must compact");
        assert!(result.text.contains("101  /usr/bin/a  0:01"));
        assert!(result.text.contains("30303  /opt/long/c  0:59"));
    }
}
