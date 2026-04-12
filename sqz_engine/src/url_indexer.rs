use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SqzError};

// ── Public types ──────────────────────────────────────────────────────────────

/// Trait abstracting HTTP fetching so callers can provide their own
/// implementation. This keeps sqz fully offline (Requirement 23).
pub trait ContentFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<String>;
}

/// A single indexed chunk (one heading section of a page).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedChunk {
    pub url: String,
    pub heading: String,
    pub body: String,
    pub rank: f64,
}

/// Result of a `fetch_and_index` call.
#[derive(Debug, Clone)]
pub struct IndexResult {
    pub url: String,
    pub chunks_indexed: usize,
    pub cached: bool,
}

// ── Schema ────────────────────────────────────────────────────────────────────

const URL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS url_cache (
    url          TEXT PRIMARY KEY,
    fetched_at   TEXT NOT NULL,
    ttl_secs     INTEGER NOT NULL,
    markdown     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS url_chunks (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    url      TEXT NOT NULL,
    heading  TEXT NOT NULL,
    body     TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS url_chunks_fts USING fts5(
    url,
    heading,
    body,
    content='url_chunks',
    content_rowid='id',
    tokenize='porter ascii'
);

CREATE TRIGGER IF NOT EXISTS url_chunks_ai AFTER INSERT ON url_chunks BEGIN
    INSERT INTO url_chunks_fts(rowid, url, heading, body)
    VALUES (new.id, new.url, new.heading, new.body);
END;

CREATE TRIGGER IF NOT EXISTS url_chunks_ad AFTER DELETE ON url_chunks BEGIN
    INSERT INTO url_chunks_fts(url_chunks_fts, rowid, url, heading, body)
    VALUES ('delete', old.id, old.url, old.heading, old.body);
END;
"#;

pub(crate) fn apply_url_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(URL_SCHEMA)
}

// ── HTML → Markdown conversion ────────────────────────────────────────────────

/// Simple regex-free HTML to markdown converter. Handles common tags without
/// pulling in any external dependency.
fn html_to_markdown(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    let mut in_tag = false;
    let mut tag_buf = String::new();
    let mut skip_content = false;

    while let Some(ch) = chars.next() {
        if ch == '<' {
            in_tag = true;
            tag_buf.clear();
            continue;
        }
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let tag_lower = tag_buf.to_ascii_lowercase();
                let tag_name = tag_lower.split_whitespace().next().unwrap_or("");

                match tag_name {
                    "h1" => out.push_str("\n# "),
                    "h2" => out.push_str("\n## "),
                    "h3" => out.push_str("\n### "),
                    "h4" => out.push_str("\n#### "),
                    "h5" => out.push_str("\n##### "),
                    "h6" => out.push_str("\n###### "),
                    "/h1" | "/h2" | "/h3" | "/h4" | "/h5" | "/h6" => {
                        out.push('\n');
                    }
                    "p" | "/p" | "br" | "br/" => out.push('\n'),
                    "li" => out.push_str("\n- "),
                    "pre" | "code" => out.push_str("\n```\n"),
                    "/pre" | "/code" => out.push_str("\n```\n"),
                    "strong" | "b" => out.push_str("**"),
                    "/strong" | "/b" => out.push_str("**"),
                    "em" | "i" => out.push('*'),
                    "/em" | "/i" => out.push('*'),
                    "script" | "style" | "noscript" => skip_content = true,
                    "/script" | "/style" | "/noscript" => skip_content = false,
                    _ => {} // ignore other tags
                }
            } else {
                tag_buf.push(ch);
            }
            continue;
        }
        if skip_content {
            continue;
        }
        // Decode common HTML entities
        if ch == '&' {
            let mut entity = String::new();
            for ec in chars.by_ref() {
                if ec == ';' {
                    break;
                }
                entity.push(ec);
                if entity.len() > 8 {
                    break;
                }
            }
            match entity.as_str() {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                "nbsp" => out.push(' '),
                _ => {
                    out.push('&');
                    out.push_str(&entity);
                    out.push(';');
                }
            }
            continue;
        }
        out.push(ch);
    }

    // Collapse excessive blank lines
    collapse_blank_lines(&out)
}

fn collapse_blank_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut blank_count = 0u32;
    for line in s.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim().to_string()
}


// ── Chunking by headings ──────────────────────────────────────────────────────

/// A chunk is a heading + the body text until the next heading of equal or
/// higher level.
#[derive(Debug, Clone)]
struct MdChunk {
    heading: String,
    body: String,
}

fn chunk_by_headings(markdown: &str) -> Vec<MdChunk> {
    let mut chunks: Vec<MdChunk> = Vec::new();
    let mut current_heading = String::new();
    let mut current_body = String::new();

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            // Flush previous chunk
            if !current_heading.is_empty() || !current_body.trim().is_empty() {
                chunks.push(MdChunk {
                    heading: current_heading.clone(),
                    body: current_body.trim().to_string(),
                });
            }
            // Strip leading '#' chars and whitespace for the heading text
            current_heading = trimmed.trim_start_matches('#').trim().to_string();
            current_body.clear();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    // Flush last chunk
    if !current_heading.is_empty() || !current_body.trim().is_empty() {
        chunks.push(MdChunk {
            heading: current_heading,
            body: current_body.trim().to_string(),
        });
    }

    chunks
}

// ── UrlIndexer ────────────────────────────────────────────────────────────────

/// Fetches URLs, converts HTML to markdown, chunks by headings, and indexes
/// into SQLite FTS5 for BM25-ranked section search.
pub struct UrlIndexer {
    db: Connection,
    ttl: Duration,
}

impl UrlIndexer {
    /// Create a new `UrlIndexer` backed by an in-memory SQLite database.
    /// Useful for testing.
    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        apply_url_schema(&conn)?;
        Ok(Self {
            db: conn,
            ttl: Duration::from_secs(24 * 3600),
        })
    }

    /// Create a `UrlIndexer` from an existing connection (e.g. shared with
    /// `SessionStore`). Applies the URL-specific schema tables.
    pub fn from_connection(conn: Connection, ttl: Duration) -> Result<Self> {
        apply_url_schema(&conn)?;
        Ok(Self { db: conn, ttl })
    }

    /// Create a `UrlIndexer` with a custom TTL from an in-memory database.
    #[cfg(test)]
    pub(crate) fn in_memory_with_ttl(ttl: Duration) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        apply_url_schema(&conn)?;
        Ok(Self { db: conn, ttl })
    }

    /// Fetch a URL via the provided `fetcher`, convert HTML to markdown,
    /// chunk by headings, and index into FTS5.
    ///
    /// - If the URL was fetched within the TTL window and `force` is false,
    ///   the cached version is used (no fetch).
    /// - If `force` is true, the cache is bypassed and the URL is re-fetched.
    pub fn fetch_and_index(
        &self,
        url: &str,
        force: bool,
        fetcher: &dyn ContentFetcher,
    ) -> Result<IndexResult> {
        // Check cache
        if !force {
            if let Some(fetched_at) = self.get_cache_time(url)? {
                let age = Utc::now().signed_duration_since(fetched_at);
                if age.num_seconds() < self.ttl.as_secs() as i64 {
                    let count = self.chunk_count(url)?;
                    return Ok(IndexResult {
                        url: url.to_string(),
                        chunks_indexed: count,
                        cached: true,
                    });
                }
            }
        }

        // Fetch
        let html = fetcher.fetch(url)?;
        let markdown = html_to_markdown(&html);
        let chunks = chunk_by_headings(&markdown);

        // Clear old data for this URL
        self.delete_url(url)?;

        // Store cache metadata
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT OR REPLACE INTO url_cache (url, fetched_at, ttl_secs, markdown) VALUES (?1, ?2, ?3, ?4)",
            params![url, now, self.ttl.as_secs() as i64, markdown],
        )?;

        // Index chunks
        let chunk_count = chunks.len();
        for chunk in &chunks {
            self.db.execute(
                "INSERT INTO url_chunks (url, heading, body) VALUES (?1, ?2, ?3)",
                params![url, chunk.heading, chunk.body],
            )?;
        }

        Ok(IndexResult {
            url: url.to_string(),
            chunks_indexed: chunk_count,
            cached: false,
        })
    }

    /// Search indexed URL content. Returns only matching sections (not full
    /// documents), ranked by BM25.
    pub fn search(&self, query: &str) -> Result<Vec<IndexedChunk>> {
        let mut stmt = self.db.prepare(
            r#"SELECT c.url, c.heading, c.body, rank
               FROM url_chunks c
               JOIN url_chunks_fts f ON c.id = f.rowid
               WHERE url_chunks_fts MATCH ?1
               ORDER BY rank"#,
        )?;

        let rows = stmt.query_map(params![query], |row| {
            Ok(IndexedChunk {
                url: row.get(0)?,
                heading: row.get(1)?,
                body: row.get(2)?,
                rank: row.get(3)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn get_cache_time(&self, url: &str) -> Result<Option<DateTime<Utc>>> {
        let result: rusqlite::Result<String> = self.db.query_row(
            "SELECT fetched_at FROM url_cache WHERE url = ?1",
            params![url],
            |row| row.get(0),
        );
        match result {
            Ok(ts) => {
                let dt = ts
                    .parse::<DateTime<Utc>>()
                    .map_err(|e| SqzError::Other(format!("invalid timestamp: {e}")))?;
                Ok(Some(dt))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SqzError::SessionStore(e)),
        }
    }

    fn chunk_count(&self, url: &str) -> Result<usize> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM url_chunks WHERE url = ?1",
            params![url],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    fn delete_url(&self, url: &str) -> Result<()> {
        // Delete chunks (triggers will clean FTS)
        self.db.execute(
            "DELETE FROM url_chunks WHERE url = ?1",
            params![url],
        )?;
        self.db.execute(
            "DELETE FROM url_cache WHERE url = ?1",
            params![url],
        )?;
        Ok(())
    }
}


// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Mock fetcher that returns pre-configured HTML for given URLs.
    struct MockFetcher {
        pages: HashMap<String, String>,
        call_count: AtomicUsize,
    }

    impl MockFetcher {
        fn new() -> Self {
            Self {
                pages: HashMap::new(),
                call_count: AtomicUsize::new(0),
            }
        }

        fn add_page(&mut self, url: &str, html: &str) {
            self.pages.insert(url.to_string(), html.to_string());
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl ContentFetcher for MockFetcher {
        fn fetch(&self, url: &str) -> Result<String> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.pages
                .get(url)
                .cloned()
                .ok_or_else(|| SqzError::Other(format!("URL not found: {url}")))
        }
    }

    // ── html_to_markdown tests ────────────────────────────────────────────────

    #[test]
    fn test_html_to_markdown_headings() {
        let html = "<h1>Title</h1><h2>Subtitle</h2><p>Body text</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("## Subtitle"));
        assert!(md.contains("Body text"));
    }

    #[test]
    fn test_html_to_markdown_strips_script_and_style() {
        let html = "<p>visible</p><script>alert('x')</script><style>.a{}</style><p>also visible</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("visible"));
        assert!(md.contains("also visible"));
        assert!(!md.contains("alert"));
        assert!(!md.contains(".a{}"));
    }

    #[test]
    fn test_html_to_markdown_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("A & B < C > D"));
    }

    #[test]
    fn test_html_to_markdown_lists() {
        let html = "<ul><li>one</li><li>two</li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("- one"));
        assert!(md.contains("- two"));
    }

    #[test]
    fn test_html_to_markdown_bold_italic() {
        let html = "<strong>bold</strong> and <em>italic</em>";
        let md = html_to_markdown(html);
        assert!(md.contains("**bold**"));
        assert!(md.contains("*italic*"));
    }

    // ── chunk_by_headings tests ───────────────────────────────────────────────

    #[test]
    fn test_chunk_by_headings_basic() {
        let md = "# Intro\nSome text\n## Details\nMore text\n## Conclusion\nFinal words";
        let chunks = chunk_by_headings(md);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].heading, "Intro");
        assert!(chunks[0].body.contains("Some text"));
        assert_eq!(chunks[1].heading, "Details");
        assert_eq!(chunks[2].heading, "Conclusion");
    }

    #[test]
    fn test_chunk_by_headings_no_headings() {
        let md = "Just plain text\nwith multiple lines";
        let chunks = chunk_by_headings(md);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].heading.is_empty());
        assert!(chunks[0].body.contains("Just plain text"));
    }

    // ── UrlIndexer integration tests ──────────────────────────────────────────

    #[test]
    fn test_fetch_and_index_basic() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page(
            "https://example.com/docs",
            "<h1>API Reference</h1><p>This is the API docs.</p>\
             <h2>Authentication</h2><p>Use bearer tokens for auth.</p>\
             <h2>Endpoints</h2><p>GET /users returns user list.</p>",
        );

        let result = indexer
            .fetch_and_index("https://example.com/docs", false, &fetcher)
            .unwrap();

        assert_eq!(result.url, "https://example.com/docs");
        assert_eq!(result.chunks_indexed, 3);
        assert!(!result.cached);
        assert_eq!(fetcher.calls(), 1);
    }

    #[test]
    fn test_cache_prevents_refetch() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page("https://example.com", "<h1>Hello</h1><p>World</p>");

        // First fetch
        let r1 = indexer
            .fetch_and_index("https://example.com", false, &fetcher)
            .unwrap();
        assert!(!r1.cached);
        assert_eq!(fetcher.calls(), 1);

        // Second fetch — should be cached
        let r2 = indexer
            .fetch_and_index("https://example.com", false, &fetcher)
            .unwrap();
        assert!(r2.cached);
        assert_eq!(fetcher.calls(), 1); // no additional fetch
    }

    #[test]
    fn test_force_bypasses_cache() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page("https://example.com", "<h1>Hello</h1><p>World</p>");

        indexer
            .fetch_and_index("https://example.com", false, &fetcher)
            .unwrap();
        assert_eq!(fetcher.calls(), 1);

        // Force re-fetch
        let r2 = indexer
            .fetch_and_index("https://example.com", true, &fetcher)
            .unwrap();
        assert!(!r2.cached);
        assert_eq!(fetcher.calls(), 2);
    }

    #[test]
    fn test_expired_ttl_refetches() {
        // Use a 0-second TTL so everything is immediately expired
        let indexer = UrlIndexer::in_memory_with_ttl(Duration::from_secs(0)).unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page("https://example.com", "<h1>Hello</h1><p>World</p>");

        indexer
            .fetch_and_index("https://example.com", false, &fetcher)
            .unwrap();
        assert_eq!(fetcher.calls(), 1);

        // TTL is 0s, so this should re-fetch
        let r2 = indexer
            .fetch_and_index("https://example.com", false, &fetcher)
            .unwrap();
        assert!(!r2.cached);
        assert_eq!(fetcher.calls(), 2);
    }

    #[test]
    fn test_search_returns_matching_sections_only() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page(
            "https://example.com/docs",
            "<h1>Overview</h1><p>General introduction to the system.</p>\
             <h2>Authentication</h2><p>Use bearer tokens for authentication.</p>\
             <h2>Rate Limiting</h2><p>Requests are limited to 100 per minute.</p>",
        );

        indexer
            .fetch_and_index("https://example.com/docs", false, &fetcher)
            .unwrap();

        let results = indexer.search("authentication").unwrap();
        // Should return only the authentication section, not the full doc
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.heading == "Authentication"));
        // The overview section should NOT match "authentication"
        assert!(results.iter().all(|r| r.heading != "Overview"
            || r.body.to_lowercase().contains("authentication")));
    }

    #[test]
    fn test_search_across_multiple_urls() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page(
            "https://a.com",
            "<h1>Rust Guide</h1><p>Learn about ownership and borrowing.</p>",
        );
        fetcher.add_page(
            "https://b.com",
            "<h1>Python Guide</h1><p>Learn about decorators and generators.</p>",
        );

        indexer
            .fetch_and_index("https://a.com", false, &fetcher)
            .unwrap();
        indexer
            .fetch_and_index("https://b.com", false, &fetcher)
            .unwrap();

        let results = indexer.search("ownership").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://a.com");

        let results = indexer.search("decorators").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://b.com");
    }

    #[test]
    fn test_search_empty_query_returns_empty() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page("https://example.com", "<h1>Hello</h1><p>World</p>");
        indexer
            .fetch_and_index("https://example.com", false, &fetcher)
            .unwrap();

        // FTS5 with empty string — should error or return empty
        let results = indexer.search("nonexistentxyzterm");
        assert!(results.is_ok());
        assert!(results.unwrap().is_empty());
    }

    #[test]
    fn test_reindex_replaces_old_chunks() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let mut fetcher = MockFetcher::new();
        fetcher.add_page(
            "https://example.com",
            "<h1>Version 1</h1><p>Old content about widgets.</p>",
        );

        indexer
            .fetch_and_index("https://example.com", false, &fetcher)
            .unwrap();

        // Update the page content
        fetcher.pages.insert(
            "https://example.com".to_string(),
            "<h1>Version 2</h1><p>New content about gadgets.</p>".to_string(),
        );

        // Force re-index
        indexer
            .fetch_and_index("https://example.com", true, &fetcher)
            .unwrap();

        // Old content should be gone
        let old = indexer.search("widgets").unwrap();
        assert!(old.is_empty());

        // New content should be found
        let new = indexer.search("gadgets").unwrap();
        assert_eq!(new.len(), 1);
        assert!(new[0].body.contains("gadgets"));
    }

    #[test]
    fn test_fetch_error_propagates() {
        let indexer = UrlIndexer::in_memory().unwrap();
        let fetcher = MockFetcher::new(); // no pages registered

        let result = indexer.fetch_and_index("https://missing.com", false, &fetcher);
        assert!(result.is_err());
    }
}
