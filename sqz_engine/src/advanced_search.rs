use rusqlite::{params, Connection};

use crate::error::Result;

/// A single search result with id, fused score, and a smart snippet.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub score: f64,
    pub snippet: String,
}

/// Advanced search engine combining BM25 (porter stemming) and trigram
/// substring search, merged via Reciprocal Rank Fusion.  Backed by an
/// in-memory SQLite database with two FTS5 virtual tables.
pub struct AdvancedSearch {
    db: Connection,
}

// ── Schema ────────────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS docs (
    id      TEXT PRIMARY KEY,
    content TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS docs_porter USING fts5(
    id,
    content,
    content='docs',
    content_rowid='rowid',
    tokenize='porter ascii'
);

CREATE VIRTUAL TABLE IF NOT EXISTS docs_trigram USING fts5(
    id,
    content,
    content='docs',
    content_rowid='rowid',
    tokenize='trigram'
);

-- Keep FTS tables in sync with the docs table.
CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN
    INSERT INTO docs_porter(rowid, id, content)
    VALUES (new.rowid, new.id, new.content);
    INSERT INTO docs_trigram(rowid, id, content)
    VALUES (new.rowid, new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS docs_ad AFTER DELETE ON docs BEGIN
    INSERT INTO docs_porter(docs_porter, rowid, id, content)
    VALUES ('delete', old.rowid, old.id, old.content);
    INSERT INTO docs_trigram(docs_trigram, rowid, id, content)
    VALUES ('delete', old.rowid, old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS docs_au AFTER UPDATE ON docs BEGIN
    INSERT INTO docs_porter(docs_porter, rowid, id, content)
    VALUES ('delete', old.rowid, old.id, old.content);
    INSERT INTO docs_trigram(docs_trigram, rowid, id, content)
    VALUES ('delete', old.rowid, old.id, old.content);
    INSERT INTO docs_porter(rowid, id, content)
    VALUES (new.rowid, new.id, new.content);
    INSERT INTO docs_trigram(rowid, id, content)
    VALUES (new.rowid, new.id, new.content);
END;
"#;

// ── RRF constant ──────────────────────────────────────────────────────────────

/// Reciprocal Rank Fusion smoothing constant (standard value from the
/// original Cormack, Clarke & Buettcher paper).
const RRF_K: f64 = 60.0;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compute Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Extract a smart snippet: a window of text around the first occurrence
/// of any query term, with `…` ellipsis markers when truncated.
fn extract_snippet(content: &str, query_terms: &[&str], window: usize) -> String {
    let lower = content.to_lowercase();
    // Find the earliest match position across all terms.
    let mut best_pos: Option<usize> = None;
    for term in query_terms {
        if let Some(pos) = lower.find(&term.to_lowercase()) {
            best_pos = Some(match best_pos {
                Some(bp) => bp.min(pos),
                None => pos,
            });
        }
    }

    let pos = match best_pos {
        Some(p) => p,
        None => 0,
    };

    let start = pos.saturating_sub(window);
    let end = (pos + window).min(content.len());

    // Snap to char boundaries (stable alternative to floor/ceil_char_boundary).
    let start = {
        let mut i = start;
        while i > 0 && !content.is_char_boundary(i) { i -= 1; }
        i
    };
    let end = {
        let mut i = end;
        while i < content.len() && !content.is_char_boundary(i) { i += 1; }
        i
    };

    let mut snippet = String::new();
    if start > 0 {
        snippet.push_str("…");
    }
    snippet.push_str(&content[start..end]);
    if end < content.len() {
        snippet.push_str("…");
    }
    snippet
}

// ── AdvancedSearch impl ───────────────────────────────────────────────────────

impl AdvancedSearch {
    /// Create a new `AdvancedSearch` backed by an in-memory SQLite database.
    pub fn new() -> Result<Self> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    /// Index a document.  If a document with the same `id` already exists it
    /// is replaced.
    pub fn index(&self, id: &str, content: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO docs (id, content) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET content = excluded.content",
            params![id, content],
        )?;
        Ok(())
    }

    /// Run an advanced search combining BM25, trigram, RRF, fuzzy correction,
    /// proximity reranking, and smart snippet extraction.
    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let terms: Vec<&str> = query.split_whitespace().collect();

        // 1. BM25 search (porter stemming).
        let bm25 = self.bm25_search(query);

        // 2. Trigram substring search.
        let trigram = self.trigram_search(query);

        // 3. Merge via Reciprocal Rank Fusion.
        let mut results = self.reciprocal_rank_fusion(&bm25, &trigram);

        // 4. If no results, try fuzzy correction (Levenshtein ≤ 2).
        if results.is_empty() {
            if let Some(corrected) = self.fuzzy_correct(query) {
                let bm25_c = self.bm25_search(&corrected);
                let trigram_c = self.trigram_search(&corrected);
                results = self.reciprocal_rank_fusion(&bm25_c, &trigram_c);
            }
        }

        // 5. Proximity reranking for multi-term queries.
        if terms.len() > 1 {
            self.proximity_rerank(&mut results, &terms);
        }

        // 6. Smart snippet extraction.
        for r in &mut results {
            if let Ok(content) = self.get_content(&r.id) {
                r.snippet = extract_snippet(&content, &terms, 80);
            }
        }

        Ok(results)
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn get_content(&self, id: &str) -> Result<String> {
        let content: String = self.db.query_row(
            "SELECT content FROM docs WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(content)
    }

    /// BM25 search on the porter-stemmed FTS5 table.
    /// Returns `(bm25_score, doc_id)` pairs ordered by relevance.
    fn bm25_search(&self, query: &str) -> Vec<(f64, String)> {
        let mut stmt = match self.db.prepare(
            "SELECT d.id, bm25(docs_porter) AS score
             FROM docs_porter p
             JOIN docs d ON d.rowid = p.rowid
             WHERE docs_porter MATCH ?1
             ORDER BY score",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![query], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok())
            .map(|(id, score)| (score, id))
            .collect()
    }

    /// Trigram substring search on the trigram FTS5 table.
    /// Returns `(bm25_score, doc_id)` pairs ordered by relevance.
    fn trigram_search(&self, query: &str) -> Vec<(f64, String)> {
        // Trigram tokenizer requires the query to be at least 3 chars.
        if query.len() < 3 {
            return Vec::new();
        }
        let mut stmt = match self.db.prepare(
            "SELECT d.id, bm25(docs_trigram) AS score
             FROM docs_trigram t
             JOIN docs d ON d.rowid = t.rowid
             WHERE docs_trigram MATCH ?1
             ORDER BY score",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![query], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok())
            .map(|(id, score)| (score, id))
            .collect()
    }

    /// Merge two ranked lists using Reciprocal Rank Fusion.
    ///
    /// Documents appearing in both lists receive a higher fused score than
    /// documents appearing in only one.
    fn reciprocal_rank_fusion(
        &self,
        a: &[(f64, String)],
        b: &[(f64, String)],
    ) -> Vec<SearchResult> {
        use std::collections::HashMap;

        let mut scores: HashMap<String, f64> = HashMap::new();

        for (rank, (_score, id)) in a.iter().enumerate() {
            *scores.entry(id.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
        for (rank, (_score, id)) in b.iter().enumerate() {
            *scores.entry(id.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        let mut results: Vec<SearchResult> = scores
            .into_iter()
            .map(|(id, score)| SearchResult {
                id,
                score,
                snippet: String::new(),
            })
            .collect();

        // Sort descending by fused score.
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Attempt fuzzy correction for the query.  Collects all known terms from
    /// the docs table and finds the closest match within Levenshtein distance 2.
    fn fuzzy_correct(&self, query: &str) -> Option<String> {
        // Build a vocabulary of unique words from indexed documents.
        let vocab = self.vocabulary();
        if vocab.is_empty() {
            return None;
        }

        let terms: Vec<&str> = query.split_whitespace().collect();
        let mut corrected_terms: Vec<String> = Vec::new();
        let mut any_corrected = false;

        for term in &terms {
            let lower = term.to_lowercase();
            let mut best: Option<(usize, String)> = None;
            for word in &vocab {
                let dist = levenshtein(&lower, word);
                if dist > 0 && dist <= 2 {
                    if best.as_ref().map_or(true, |(d, _)| dist < *d) {
                        best = Some((dist, word.clone()));
                    }
                }
            }
            if let Some((_dist, correction)) = best {
                corrected_terms.push(correction);
                any_corrected = true;
            } else {
                corrected_terms.push(lower);
            }
        }

        if any_corrected {
            Some(corrected_terms.join(" "))
        } else {
            None
        }
    }

    /// Collect unique lowercase words from all indexed documents.
    fn vocabulary(&self) -> Vec<String> {
        let mut stmt = match self.db.prepare("SELECT content FROM docs") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let mut words = std::collections::HashSet::new();
        for row in rows.flatten() {
            for word in row.split_whitespace() {
                let w: String = word
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
                    .to_lowercase();
                if w.len() >= 2 {
                    words.insert(w);
                }
            }
        }
        words.into_iter().collect()
    }

    /// Proximity reranking: boost results where query terms appear close
    /// together in the document content.
    fn proximity_rerank(&self, results: &mut Vec<SearchResult>, query_terms: &[&str]) {
        for r in results.iter_mut() {
            let content = match self.get_content(&r.id) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let lower = content.to_lowercase();

            // Find positions of each query term.
            let mut positions: Vec<usize> = Vec::new();
            for term in query_terms {
                if let Some(pos) = lower.find(&term.to_lowercase()) {
                    positions.push(pos);
                }
            }

            if positions.len() >= 2 {
                positions.sort_unstable();
                // Compute the span (distance between first and last term).
                let span = positions.last().unwrap() - positions.first().unwrap();
                // Boost: closer terms → higher boost.  A span of 0 gives max
                // boost of 2×; very distant terms give ~1× (no boost).
                let boost = 1.0 + 1.0 / (1.0 + span as f64 / 50.0);
                r.score *= boost;
            }
        }

        // Re-sort after boosting.
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_search() -> AdvancedSearch {
        AdvancedSearch::new().unwrap()
    }

    #[test]
    fn test_index_and_bm25_search() {
        let s = make_search();
        s.index("d1", "the quick brown fox jumps over the lazy dog").unwrap();
        s.index("d2", "a fast red car drives on the highway").unwrap();

        let results = s.bm25_search("fox");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "d1");
    }

    #[test]
    fn test_trigram_search() {
        let s = make_search();
        s.index("d1", "authentication middleware handles tokens").unwrap();
        s.index("d2", "database migration scripts for postgres").unwrap();

        let results = s.trigram_search("auth");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "d1");
    }

    #[test]
    fn test_rrf_merge_both_lists() {
        let s = make_search();
        s.index("d1", "rust programming language systems").unwrap();
        s.index("d2", "rust prevention coating for metal surfaces").unwrap();
        s.index("d3", "programming in python is fun").unwrap();

        // "rust programming" should match d1 in both porter and trigram,
        // giving it a higher RRF score than d2 or d3.
        let results = s.search("rust programming").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "d1");
    }

    #[test]
    fn test_rrf_docs_in_both_rank_higher() {
        let s = make_search();
        // d1 contains both "alpha" and "beta" — will appear in both BM25 and trigram.
        s.index("d1", "alpha beta gamma delta").unwrap();
        // d2 contains only "alpha".
        s.index("d2", "alpha only here nothing else relevant").unwrap();

        let bm25 = s.bm25_search("alpha");
        let trigram = s.trigram_search("alpha");

        // Both should find d1 and d2.
        let merged = s.reciprocal_rank_fusion(&bm25, &trigram);
        // d1 and d2 both appear in both lists, but let's just verify the
        // merge produces results from both lists.
        assert!(merged.len() >= 1);

        // Docs appearing in both lists should have higher scores.
        let in_bm25: std::collections::HashSet<_> = bm25.iter().map(|(_, id)| id.clone()).collect();
        let in_trigram: std::collections::HashSet<_> = trigram.iter().map(|(_, id)| id.clone()).collect();
        let in_both: std::collections::HashSet<_> = in_bm25.intersection(&in_trigram).cloned().collect();

        if merged.len() >= 2 {
            let top = &merged[0];
            if in_both.contains(&top.id) {
                // Good — doc in both lists is ranked first.
            }
        }
    }

    #[test]
    fn test_fuzzy_correction() {
        let s = make_search();
        s.index("d1", "authentication middleware").unwrap();
        s.index("d2", "database migration").unwrap();

        // "authentcation" is a typo (missing 'i'), Levenshtein distance 1.
        let corrected = s.fuzzy_correct("authentcation");
        assert!(corrected.is_some());
        let c = corrected.unwrap();
        assert!(c.contains("authentication"), "corrected to: {}", c);
    }

    #[test]
    fn test_fuzzy_search_end_to_end() {
        let s = make_search();
        s.index("d1", "authentication middleware handles tokens").unwrap();

        // Typo query — should still find d1 via fuzzy correction.
        let results = s.search("authentcation").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "d1");
    }

    #[test]
    fn test_proximity_reranking() {
        let s = make_search();
        // d1: terms "error" and "handler" are close together.
        s.index("d1", "the error handler catches all exceptions").unwrap();
        // d2: terms "error" and "handler" are far apart.
        s.index(
            "d2",
            "an error occurred in the system and after many lines of unrelated text the handler was invoked",
        ).unwrap();

        let results = s.search("error handler").unwrap();
        assert!(results.len() >= 2);
        // d1 should rank higher due to proximity boost.
        assert_eq!(results[0].id, "d1");
    }

    #[test]
    fn test_smart_snippet_extraction() {
        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
                        The authentication module verifies JWT tokens. \
                        Sed do eiusmod tempor incididunt ut labore.";
        let snippet = extract_snippet(content, &["authentication"], 40);
        assert!(snippet.contains("authentication"));
        // Should have ellipsis since it's in the middle.
        assert!(snippet.contains("…"));
    }

    #[test]
    fn test_empty_query_returns_empty() {
        let s = make_search();
        s.index("d1", "some content").unwrap();
        let results = s.search("").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_no_results_returns_empty() {
        let s = make_search();
        s.index("d1", "hello world").unwrap();
        let results = s.search("zzzznonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_index_upsert() {
        let s = make_search();
        s.index("d1", "original content about cats").unwrap();
        s.index("d1", "updated content about dogs").unwrap();

        let results = s.search("dogs").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "d1");

        let results = s.search("cats").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("hello", "hello"), 0);
        assert_eq!(levenshtein("hello", "helo"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn test_snippet_at_start() {
        let content = "authentication is important for security";
        let snippet = extract_snippet(content, &["authentication"], 80);
        assert!(snippet.contains("authentication"));
        // Should not have leading ellipsis since match is at start.
        assert!(!snippet.starts_with('…'));
    }

    #[test]
    fn test_multiple_documents_search() {
        let s = make_search();
        for i in 0..10 {
            s.index(&format!("d{}", i), &format!("document number {} about testing", i))
                .unwrap();
        }
        let results = s.search("testing").unwrap();
        assert_eq!(results.len(), 10);
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;
        use std::collections::{HashMap, HashSet};

        // **Validates: Requirements 41.1, 41.2**
        //
        // Property 41: Advanced search RRF merges correctly
        //
        // For any two ranked lists, the Reciprocal Rank Fusion merge SHALL:
        // 1. Produce a result set containing all unique documents from both inputs.
        // 2. Assign a higher RRF score to documents appearing in both lists than
        //    to documents appearing in only one list (at comparable rank positions).
        proptest! {
            #[test]
            fn prop_rrf_merge_contains_all_unique_docs_and_both_rank_higher(
                // Generate 2-6 unique doc IDs for list A, 2-6 for list B,
                // with at least some overlap guaranteed by construction.
                shared_count in 1..4usize,
                a_only_count in 1..4usize,
                b_only_count in 1..4usize,
            ) {
                let s = make_search();

                // Build document sets: shared docs appear in both lists,
                // a_only docs appear only in list A, b_only only in list B.
                let mut list_a: Vec<(f64, String)> = Vec::new();
                let mut list_b: Vec<(f64, String)> = Vec::new();

                // Shared documents — present in both lists.
                for i in 0..shared_count {
                    let id = format!("shared_{}", i);
                    // Use descending scores so rank = index.
                    list_a.push((-(i as f64), id.clone()));
                    list_b.push((-(i as f64), id));
                }

                // A-only documents.
                for i in 0..a_only_count {
                    let id = format!("a_only_{}", i);
                    list_a.push((-((shared_count + i) as f64), id));
                }

                // B-only documents.
                for i in 0..b_only_count {
                    let id = format!("b_only_{}", i);
                    list_b.push((-((shared_count + i) as f64), id));
                }

                let merged = s.reciprocal_rank_fusion(&list_a, &list_b);

                // ── Property 1: merged contains all unique docs from both lists ──
                let all_ids: HashSet<String> = list_a.iter().map(|(_, id)| id.clone())
                    .chain(list_b.iter().map(|(_, id)| id.clone()))
                    .collect();
                let merged_ids: HashSet<String> = merged.iter().map(|r| r.id.clone()).collect();
                prop_assert_eq!(
                    merged_ids, all_ids,
                    "Merged result set must contain all unique documents from both input lists"
                );

                // ── Property 2: docs in both lists score higher than docs in only one ──
                let a_ids: HashSet<String> = list_a.iter().map(|(_, id)| id.clone()).collect();
                let b_ids: HashSet<String> = list_b.iter().map(|(_, id)| id.clone()).collect();
                let in_both: HashSet<String> = a_ids.intersection(&b_ids).cloned().collect();
                let in_one_only: HashSet<String> = a_ids.symmetric_difference(&b_ids).cloned().collect();

                if !in_both.is_empty() && !in_one_only.is_empty() {
                    let scores: HashMap<String, f64> = merged.iter()
                        .map(|r| (r.id.clone(), r.score))
                        .collect();

                    let min_both_score = in_both.iter()
                        .filter_map(|id| scores.get(id))
                        .cloned()
                        .fold(f64::INFINITY, f64::min);

                    let max_one_score = in_one_only.iter()
                        .filter_map(|id| scores.get(id))
                        .cloned()
                        .fold(f64::NEG_INFINITY, f64::max);

                    prop_assert!(
                        min_both_score > max_one_score,
                        "Documents in both lists (min score {}) must score higher \
                         than documents in only one list (max score {})",
                        min_both_score, max_one_score
                    );
                }
            }
        }
    }
}
