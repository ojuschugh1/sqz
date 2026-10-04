use std::collections::{HashMap, HashSet};

use aho_corasick::{AhoCorasick, AhoCorasickKind};

const FEW: usize = 8;
const CHUNK_BYTES: usize = 64 << 10;

pub(crate) fn present(haystack: &str, needles: &[&str]) -> Vec<bool> {
    if needles.len() <= FEW {
        return needles.iter().map(|n| haystack.contains(n)).collect();
    }
    let lines: HashSet<&str> = haystack.lines().map(str::trim).collect();
    let mut found: Vec<bool> = needles
        .iter()
        .map(|n| n.is_empty() || lines.contains(n))
        .collect();

    let mut patterns: Vec<&str> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    for (i, &n) in needles.iter().enumerate() {
        if !found[i] {
            let p = *index.entry(n).or_insert_with(|| {
                patterns.push(n);
                patterns.len() - 1
            });
            open.push((i, p));
        }
    }
    let mut hits = Vec::with_capacity(patterns.len());
    let mut start = 0;
    let mut bytes = 0;
    for (end, p) in patterns.iter().enumerate() {
        if bytes + p.len() > CHUNK_BYTES && end > start {
            hits.extend(scan(haystack, &patterns[start..end]));
            start = end;
            bytes = 0;
        }
        bytes += p.len();
    }
    hits.extend(scan(haystack, &patterns[start..]));
    for (i, p) in open {
        found[i] = hits[p];
    }
    found
}

fn scan(haystack: &str, patterns: &[&str]) -> Vec<bool> {
    if patterns.len() <= FEW {
        return patterns.iter().map(|p| haystack.contains(p)).collect();
    }
    let Ok(ac) = AhoCorasick::builder()
        .kind(Some(AhoCorasickKind::ContiguousNFA))
        .build(patterns)
    else {
        return patterns.iter().map(|p| haystack.contains(p)).collect();
    };
    let mut hits = vec![false; patterns.len()];
    let mut left = patterns.len();
    for m in ac.find_overlapping_iter(haystack) {
        let p = m.pattern().as_usize();
        if !hits[p] {
            hits[p] = true;
            left -= 1;
            if left == 0 {
                break;
            }
        }
    }
    hits
}

#[cfg(test)]
pub(crate) fn time_ratio(small: impl Fn(), large: impl Fn()) -> f64 {
    use std::time::{Duration, Instant};
    let (mut s, mut l) = (Duration::MAX, Duration::MAX);
    for _ in 0..5 {
        let t = Instant::now();
        small();
        s = s.min(t.elapsed());
        let t = Instant::now();
        large();
        l = l.min(t.elapsed());
    }
    l.as_secs_f64() / s.as_secs_f64().max(1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn check(hay: &str, needles: &[&str]) {
        let want: Vec<bool> = needles.iter().map(|n| hay.contains(n)).collect();
        assert_eq!(
            present(hay, needles),
            want,
            "haystack {hay:?}, needles {needles:?}"
        );
    }

    #[test]
    fn edge_cases_match_contains() {
        let hay = "alpha beta\n  gamma delta  \r\nepsilon\nab\n日本語 テキスト\n\nlast";
        let mut needles = vec![
            "",
            "alpha beta",
            "gamma delta",
            "  gamma delta  ",
            "gamma delta  \r",
            "ta\nep",
            "beta\n  gamma",
            "beta",
            "zeta",
            "a",
            "ab",
            "b\nab",
            "日本語",
            "本語 テ",
            "テキスト\n\nla",
            "語\n",
            "\r\n",
            "\n\n",
            "last",
            "last\n",
            "ab\n日",
            "alpha beta\n  gamma delta  \r\nepsilon",
            "日本語 テキスト",
        ];
        check(hay, &needles);
        check(hay, &needles[..FEW]);
        needles.extend([
            "aa",
            "a",
            "ab",
            "ab",
            "eps",
            "psi",
            "silon",
            "epsilon\nab",
            "lon\na",
        ]);
        check(hay, &needles);
        let run = "aaaaaaaa";
        check(
            run,
            &[
                "a",
                "aa",
                "aaa",
                "aaaa",
                "aaaaaaaa",
                "aaaaaaaaa",
                "b",
                "ab",
                "ba",
                "",
            ],
        );
        check("", &["", "a", "", "b", "c", "d", "e", "f", "g", "h"]);
        check("x", &[]);
    }

    #[test]
    fn many_needles_match_contains() {
        let hay: String = (0..300)
            .map(|i| format!("  line {i}: error: thing {} failed\n", i * 7))
            .collect();
        let owned: Vec<String> = (0..400)
            .flat_map(|i| {
                [
                    format!("line {i}: error: thing {} failed", i * 7),
                    format!("thing {} fail", i * 7),
                    format!("{i}: error"),
                    format!("failed\n  line {}", i + 1),
                    format!("line {i}: error: thing {} failed", i * 7 + 1),
                ]
            })
            .collect();
        let needles: Vec<&str> = owned.iter().map(String::as_str).collect();
        check(&hay, &needles);
    }

    #[test]
    fn chunked_needles_match_contains() {
        let hay: String = (0..3000)
            .map(|i| format!("> {i}: error: request {} timed out\n", i * 13))
            .collect();
        let mut owned: Vec<String> = (0..3000)
            .flat_map(|i| {
                [
                    format!("{i}: error: request {} timed out", i * 13),
                    format!("{i}: error: request {} timed out", i * 13 + 1),
                ]
            })
            .collect();
        owned.push(hay[..CHUNK_BYTES + 1].to_string());
        owned.push(format!("{}!", &hay[..CHUNK_BYTES]));
        let needles: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert!(needles.iter().map(|n| n.len()).sum::<usize>() > 4 * CHUNK_BYTES);
        check(&hay, &needles);
    }

    proptest! {
        #[test]
        fn prop_matches_contains(
            hay in "[ab \n\té日]{0,120}",
            extra in prop::collection::vec("[ab \n日]{0,5}", 0..30),
            cuts in prop::collection::vec((0usize..130, 0usize..12), 0..20),
        ) {
            let starts: Vec<usize> = hay.char_indices().map(|(i, _)| i).collect();
            let mut needles: Vec<String> = extra;
            needles.extend(hay.lines().map(|l| l.trim().to_string()));
            if !starts.is_empty() {
                for (start, len) in cuts {
                    let s = start % starts.len();
                    let end = starts.get(s + len).copied().unwrap_or(hay.len());
                    needles.push(hay[starts[s]..end].to_string());
                }
            }
            let refs: Vec<&str> = needles.iter().map(String::as_str).collect();
            let want: Vec<bool> = refs.iter().map(|n| hay.contains(n)).collect();
            prop_assert_eq!(present(&hay, &refs), want);
        }
    }
}
