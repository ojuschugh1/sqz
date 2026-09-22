use std::path::Path;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SqzError};
use crate::types::{CompressedContent, SessionId, SessionState};

/// A lightweight summary of a session for search results. Doesn't include
/// the full conversation — just enough to identify and filter sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub project_dir: PathBuf,
    pub compressed_summary: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// SQLite-backed persistent session and cache store with FTS5 full-text search.
///
/// Stores sessions, cache entries, compression logs, and known files in a
/// single SQLite database. Uses WAL mode for concurrent read access.
///
/// ```rust,no_run
/// use sqz_engine::SessionStore;
/// use std::path::Path;
///
/// let store = SessionStore::open_or_create(Path::new("~/.sqz/sessions.db")).unwrap();
/// let results = store.search("authentication refactor").unwrap();
/// for session in &results {
///     println!("{}: {}", session.id, session.compressed_summary);
/// }
/// ```
pub struct SessionStore {
    db: Connection,
}

// ── Schema ────────────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS sessions (
    id               TEXT PRIMARY KEY,
    project_dir      TEXT NOT NULL,
    compressed_summary TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    data             BLOB NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS sessions_fts USING fts5(
    id,
    project_dir,
    compressed_summary,
    content='sessions',
    content_rowid='rowid',
    tokenize='porter ascii'
);

CREATE TRIGGER IF NOT EXISTS sessions_ai AFTER INSERT ON sessions BEGIN
    INSERT INTO sessions_fts(rowid, id, project_dir, compressed_summary)
    VALUES (new.rowid, new.id, new.project_dir, new.compressed_summary);
END;

CREATE TRIGGER IF NOT EXISTS sessions_ad AFTER DELETE ON sessions BEGIN
    INSERT INTO sessions_fts(sessions_fts, rowid, id, project_dir, compressed_summary)
    VALUES ('delete', old.rowid, old.id, old.project_dir, old.compressed_summary);
END;

CREATE TRIGGER IF NOT EXISTS sessions_au AFTER UPDATE ON sessions BEGIN
    INSERT INTO sessions_fts(sessions_fts, rowid, id, project_dir, compressed_summary)
    VALUES ('delete', old.rowid, old.id, old.project_dir, old.compressed_summary);
    INSERT INTO sessions_fts(rowid, id, project_dir, compressed_summary)
    VALUES (new.rowid, new.id, new.project_dir, new.compressed_summary);
END;

CREATE TABLE IF NOT EXISTS cache_entries (
    hash        TEXT PRIMARY KEY,
    data        TEXT NOT NULL,
    accessed_at TEXT NOT NULL,
    -- Raw pre-compression bytes so `sqz expand <prefix>` can serve
    -- truly uncompressed content to agents that cannot parse
    -- `§ref:…§` dedup tokens. Nullable because the column was added
    -- in an additive migration; rows written before that migration
    -- (or via callers that don't have the original bytes) have NULL.
    original    BLOB,
    -- Pinned entries bypass the TTL freshness check. Used for stable
    -- knowledge-base content (a wiki, project documentation, system
    -- prompts) that's persistently re-injected into every agent and
    -- therefore never gets compacted out of the LLM's context. See
    -- `sqz pin` for the user-facing CLI.
    pinned      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS compression_log (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    tokens_original  INTEGER NOT NULL,
    tokens_compressed INTEGER NOT NULL,
    stages_applied   TEXT NOT NULL,
    mode             TEXT NOT NULL DEFAULT 'auto',
    created_at       TEXT NOT NULL
);

-- Compression-regret signals. A 'rerun' row means the agent re-produced
-- byte-identical output within a short window of last seeing it (the
-- repeat bought no new information); an 'expand' row means a §ref:…§
-- token was expanded back to its original bytes (a recovery round trip).
-- Both are proxies for "compression dropped something the agent needed",
-- surfaced by `sqz stats` so aggressiveness can be judged per command.
CREATE TABLE IF NOT EXISTS regret_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,
    cmd         TEXT NOT NULL,
    hash        TEXT NOT NULL,
    gap_secs    REAL NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS known_files (
    path        TEXT PRIMARY KEY,
    added_at    TEXT NOT NULL
);

-- Small key/value store for engine-wide state that needs to persist across
-- short-lived sqz processes (each shell-hook invocation is a new process).
-- Initially used only for the last_compaction_at marker: cache entries with
-- `accessed_at < last_compaction_at` are treated as stale even if still
-- within the normal TTL. See cache_manager.rs for the freshness model.
CREATE TABLE IF NOT EXISTS metadata (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL
);

-- Full-text search over everything that flowed through sqz: cached
-- command output (kind='output', ref_hash expandable via `sqz expand`)
-- and session summaries (kind='session', ref_hash = session id). Lets
-- agents re-find content they have already seen instead of re-running
-- commands or re-reading files. BM25-ranked via FTS5.
CREATE VIRTUAL TABLE IF NOT EXISTS recall_index USING fts5(
    content,
    ref_hash UNINDEXED,
    kind UNINDEXED,
    created_at UNINDEXED
);
"#;

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(crate) fn apply_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    // Additive migration: add `original` BLOB column to cache_entries if
    // it does not yet exist. Stores the raw pre-compression bytes so
    // `sqz expand <prefix>` can return truly uncompressed content when
    // an agent cannot parse `§ref:…§` tokens (reported on-list by
    // SquireNed for GLM 5.1). `NULL` for rows written by older sqz
    // versions — `expand` treats those as "original unavailable, fall
    // back to the compressed blob" so users don't get spurious errors
    // on pre-migration data.
    //
    // Using pragma_table_info rather than a version table because the
    // rest of sqz does the same — this is the first additive migration.
    let has_original: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('cache_entries') WHERE name = 'original'")?
        .query_row([], |_| Ok(()))
        .is_ok();
    if !has_original {
        conn.execute("ALTER TABLE cache_entries ADD COLUMN original BLOB", [])?;
    }

    // Additive migration: add `project_dir` TEXT column to compression_log.
    // Enables per-project stats filtering (issue #13). NULL for rows
    // written by older sqz versions — aggregate queries treat those as
    // "unknown project".
    let has_project_dir: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('compression_log') WHERE name = 'project_dir'")?
        .query_row([], |_| Ok(()))
        .is_ok();
    if !has_project_dir {
        conn.execute("ALTER TABLE compression_log ADD COLUMN project_dir TEXT", [])?;
    }

    // Additive migration: add `pinned` flag to cache_entries. Pinned
    // entries bypass the TTL freshness check so stable wiki / knowledge
    // base content stays dedup-able forever.
    let has_pinned: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('cache_entries') WHERE name = 'pinned'")?
        .query_row([], |_| Ok(()))
        .is_ok();
    if !has_pinned {
        conn.execute(
            "ALTER TABLE cache_entries ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    Ok(())
}

fn open_connection(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    apply_schema(&conn)?;
    Ok(conn)
}

fn row_to_summary(
    id: String,
    project_dir: String,
    compressed_summary: String,
    created_at: String,
    updated_at: String,
) -> Result<SessionSummary> {
    let created_at = created_at
        .parse::<DateTime<Utc>>()
        .map_err(|e| SqzError::Other(format!("invalid created_at timestamp: {e}")))?;
    let updated_at = updated_at
        .parse::<DateTime<Utc>>()
        .map_err(|e| SqzError::Other(format!("invalid updated_at timestamp: {e}")))?;
    Ok(SessionSummary {
        id,
        project_dir: PathBuf::from(project_dir),
        compressed_summary,
        created_at,
        updated_at,
    })
}

// ── SessionStore ──────────────────────────────────────────────────────────────

impl SessionStore {
    /// Construct a `SessionStore` from an already-open `Connection`.
    /// Intended for testing (e.g., in-memory databases).
    #[cfg(test)]
    pub(crate) fn from_connection(conn: Connection) -> Self {
        Self { db: conn }
    }

    /// Open an existing database at `path`. Returns an error if the file does
    /// not exist or cannot be opened.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        apply_schema(&conn)?;
        Ok(Self { db: conn })
    }

    /// Open the database at `path`, creating it if it does not exist.
    /// If the database is corrupted, a fresh database is created at the same
    /// path and a warning is logged to stderr.
    pub fn open_or_create(path: &Path) -> Result<Self> {
        match open_connection(path) {
            Ok(conn) => Ok(Self { db: conn }),
            Err(e) => {
                eprintln!(
                    "sqz warning: session store at '{}' is corrupted or inaccessible ({e}). \
                     Creating a new database. Prior session data has been lost.",
                    path.display()
                );
                // Remove the corrupted file so we can start fresh.
                if path.exists() {
                    let _ = std::fs::remove_file(path);
                }
                let conn = open_connection(path)
                    .map_err(|e2| SqzError::Other(format!("failed to create new session store: {e2}")))?;
                Ok(Self { db: conn })
            }
        }
    }

    // ── Session CRUD ──────────────────────────────────────────────────────────

    /// Persist a session. Returns the session id.
    pub fn save_session(&self, session: &SessionState) -> Result<SessionId> {
        let data = serde_json::to_vec(session)?;
        let project_dir = session.project_dir.to_string_lossy().to_string();
        let created_at = session.created_at.to_rfc3339();
        let updated_at = session.updated_at.to_rfc3339();

        self.db.execute(
            r#"INSERT INTO sessions (id, project_dir, compressed_summary, created_at, updated_at, data)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)
               ON CONFLICT(id) DO UPDATE SET
                   project_dir        = excluded.project_dir,
                   compressed_summary = excluded.compressed_summary,
                   created_at         = excluded.created_at,
                   updated_at         = excluded.updated_at,
                   data               = excluded.data"#,
            params![
                session.id,
                project_dir,
                session.compressed_summary,
                created_at,
                updated_at,
                data,
            ],
        )?;
        if !session.compressed_summary.is_empty() {
            self.recall_index_upsert(
                &session.id,
                "session",
                session.compressed_summary.as_bytes(),
            );
        }

        Ok(session.id.clone())
    }

    /// Save a lightweight session from just an id and summary for dashboard use.
    pub fn save_dashboard_session(&self, id: &str, summary: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let project_dir = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .to_string_lossy()
            .to_string();
        let empty_data = serde_json::to_vec(&serde_json::json!({"conversation":[]}))?;

        self.db
            .execute(
                "INSERT INTO sessions (id, project_dir, compressed_summary, created_at, updated_at, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET \
                 compressed_summary = excluded.compressed_summary, \
                 updated_at = excluded.updated_at",
                params![id, project_dir, summary, now, now, empty_data],
            )
            .map_err(SqzError::SessionStore)?;
        Ok(())
    }

    /// Load a session by id.
    pub fn load_session(&self, id: SessionId) -> Result<SessionState> {
        let data: Vec<u8> = self.db.query_row(
            "SELECT data FROM sessions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let session: SessionState = serde_json::from_slice(&data)?;
        Ok(session)
    }

    // ── Search ────────────────────────────────────────────────────────────────

    /// Full-text search using FTS5 (porter stemmer, ASCII tokenizer).
    pub fn search(&self, query: &str) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.db.prepare(
            r#"SELECT s.id, s.project_dir, s.compressed_summary, s.created_at, s.updated_at
               FROM sessions s
               JOIN sessions_fts f ON s.rowid = f.rowid
               WHERE sessions_fts MATCH ?1
               ORDER BY rank"#,
        )?;

        let rows = stmt.query_map(params![query], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (id, project_dir, compressed_summary, created_at, updated_at) = row?;
            results.push(row_to_summary(id, project_dir, compressed_summary, created_at, updated_at)?);
        }
        Ok(results)
    }

    /// Query sessions whose `updated_at` falls within `[from, to]`.
    pub fn search_by_date(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.db.prepare(
            r#"SELECT id, project_dir, compressed_summary, created_at, updated_at
               FROM sessions
               WHERE updated_at >= ?1 AND updated_at <= ?2
               ORDER BY updated_at DESC"#,
        )?;

        let rows = stmt.query_map(params![from.to_rfc3339(), to.to_rfc3339()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (id, project_dir, compressed_summary, created_at, updated_at) = row?;
            results.push(row_to_summary(id, project_dir, compressed_summary, created_at, updated_at)?);
        }
        Ok(results)
    }

    /// Return the most recently updated session, or `None` if no sessions exist.
    pub fn latest_session(&self) -> Result<Option<SessionSummary>> {
        let mut stmt = self.db.prepare(
            r#"SELECT id, project_dir, compressed_summary, created_at, updated_at
               FROM sessions
               ORDER BY updated_at DESC
               LIMIT 1"#,
        ).map_err(SqzError::SessionStore)?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        }).map_err(SqzError::SessionStore)?;

        for row in rows {
            let (id, project_dir, compressed_summary, created_at, updated_at) =
                row.map_err(SqzError::SessionStore)?;
            return Ok(Some(row_to_summary(id, project_dir, compressed_summary, created_at, updated_at)?));
        }
        Ok(None)
    }

    /// Query sessions whose `project_dir` matches `dir` exactly.
    pub fn search_by_project(&self, dir: &Path) -> Result<Vec<SessionSummary>> {
        let dir_str = dir.to_string_lossy().to_string();
        let mut stmt = self.db.prepare(
            r#"SELECT id, project_dir, compressed_summary, created_at, updated_at
               FROM sessions
               WHERE project_dir = ?1
               ORDER BY updated_at DESC"#,
        )?;

        let rows = stmt.query_map(params![dir_str], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (id, project_dir, compressed_summary, created_at, updated_at) = row?;
            results.push(row_to_summary(id, project_dir, compressed_summary, created_at, updated_at)?);
        }
        Ok(results)
    }

    // ── Cache entries ─────────────────────────────────────────────────────────

    /// Persist a cache entry keyed by content hash.
    ///
    /// Stores both the compressed JSON (`data`) used for dedup-hit
    /// responses AND the raw uncompressed bytes (`original`) so that
    /// `sqz expand <prefix>` can serve truly uncompressed content. See
    /// [`save_cache_entry_with_original`] for the original-aware version;
    /// this convenience wrapper exists for callers that do not (yet)
    /// have the pre-compression bytes handy. Rows written through this
    /// path leave `original` as `NULL`, and `expand` will degrade to
    /// returning the compressed blob with a note.
    pub fn save_cache_entry(&self, hash: &str, compressed: &CompressedContent) -> Result<()> {
        self.save_cache_entry_with_original(hash, compressed, None)
    }

    /// Persist a cache entry with both compressed and original content.
    ///
    /// `original` must be the exact bytes that produced `compressed`, so
    /// that `expand` is a true inverse of dedup. We store the raw bytes
    /// (not the UTF-8 string) because command output may include
    /// non-UTF-8 sequences — storing the text would lose them.
    pub fn save_cache_entry_with_original(
        &self,
        hash: &str,
        compressed: &CompressedContent,
        original: Option<&[u8]>,
    ) -> Result<()> {
        let data = serde_json::to_string(compressed)?;
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            r#"INSERT INTO cache_entries (hash, data, accessed_at, original)
               VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(hash) DO UPDATE
                   SET data = excluded.data,
                       accessed_at = excluded.accessed_at,
                       -- Don't overwrite a previously-stored `original`
                       -- with NULL. Older callers (that go through
                       -- save_cache_entry rather than the _with_original
                       -- variant) shouldn't erase the expand-able bytes.
                       original = COALESCE(excluded.original, original)"#,
            params![hash, data, now, original],
        )?;
        if let Some(bytes) = original {
            self.recall_index_upsert(hash, "output", bytes);
        }
        Ok(())
    }

    /// Index content for `sqz recall`. Binary content (NUL bytes) and
    /// empty content are skipped; oversized content is indexed up to a
    /// cap so one giant log can't bloat the index.
    fn recall_index_upsert(&self, ref_hash: &str, kind: &str, content: &[u8]) {
        const MAX_INDEXED_BYTES: usize = 262_144;
        if content.is_empty() || content.contains(&0) {
            return;
        }
        let text = String::from_utf8_lossy(content);
        let capped: &str = if text.len() > MAX_INDEXED_BYTES {
            let mut end = MAX_INDEXED_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        } else {
            &text
        };
        let now = Utc::now().to_rfc3339();
        let _ = self.db.execute(
            "DELETE FROM recall_index WHERE ref_hash = ?1",
            params![ref_hash],
        );
        let _ = self.db.execute(
            "INSERT INTO recall_index (content, ref_hash, kind, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![capped, ref_hash, kind, now],
        );
    }

    /// BM25-ranked full-text search over indexed content. Returns the
    /// best `limit` hits with a highlighted snippet each.
    pub fn recall_search(&self, query: &str, limit: u32) -> Result<Vec<RecallHit>> {
        let match_expr = fts_query(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.db.prepare(
            "SELECT ref_hash, kind, snippet(recall_index, 0, '>>', '<<', ' … ', 24), created_at \
             FROM recall_index WHERE recall_index MATCH ?1 \
             ORDER BY bm25(recall_index) LIMIT ?2",
        ).map_err(SqzError::SessionStore)?;
        let rows = stmt.query_map(params![match_expr, limit], |row| {
            Ok(RecallHit {
                ref_hash: row.get(0)?,
                kind: row.get(1)?,
                snippet: row.get(2)?,
                created_at: row.get(3)?,
            })
        }).map_err(SqzError::SessionStore)?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(hits)
    }

    /// Retrieve the stored original bytes for a cached hash, if the
    /// caller populated them via `save_cache_entry_with_original`.
    ///
    /// Returns `Ok(None)` for missing entries AND for entries that were
    /// saved by an older call site that did not pass `original`. The
    /// caller should fall back to the compressed blob in the latter case
    /// and surface a note to the user so they know this specific entry
    /// wasn't round-trippable.
    pub fn get_cache_entry_original(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let result: rusqlite::Result<Option<Vec<u8>>> = self.db.query_row(
            "SELECT original FROM cache_entries WHERE hash = ?1",
            params![hash],
            |row| row.get(0),
        );
        match result {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SqzError::SessionStore(e)),
        }
    }

    /// Most recently accessed entries that still hold their original bytes
    /// and are at least `min_len` bytes long, newest first. Candidates for
    /// slice matching: a ranged re-read can only be a slice of something
    /// larger than itself.
    pub fn recent_originals(&self, min_len: usize, limit: usize) -> Result<Vec<SliceCandidate>> {
        let mut stmt = self.db.prepare(
            "SELECT hash, original, data FROM cache_entries \
             WHERE original IS NOT NULL AND length(original) > ?1 \
             ORDER BY accessed_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![min_len as i64, limit as i64], |row| {
            Ok(SliceCandidate {
                hash: row.get(0)?,
                original: row.get(1)?,
                data_json: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Delete a cache entry by content hash.
    pub fn delete_cache_entry(&self, hash: &str) -> Result<()> {
        self.db.execute(
            "DELETE FROM cache_entries WHERE hash = ?1",
            params![hash],
        )?;
        let _ = self.db.execute(
            "DELETE FROM recall_index WHERE ref_hash = ?1",
            params![hash],
        );
        Ok(())
    }

    /// Return all cache entries ordered by `accessed_at` ASC (oldest first),
    /// as `(hash, size_bytes)` pairs where `size_bytes` is the byte length of
    /// the stored JSON data.
    pub fn list_cache_entries_lru(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.db.prepare(
            "SELECT hash, length(data) FROM cache_entries ORDER BY accessed_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (hash, size) = row?;
            entries.push((hash, size as u64));
        }
        Ok(entries)
    }

    /// Retrieve a cache entry by content hash, updating `accessed_at`.
    pub fn get_cache_entry(&self, hash: &str) -> Result<Option<CompressedContent>> {
        let result: rusqlite::Result<String> = self.db.query_row(
            "SELECT data FROM cache_entries WHERE hash = ?1",
            params![hash],
            |row| row.get(0),
        );

        match result {
            Ok(data) => {
                // Touch accessed_at for LRU tracking.
                let now = Utc::now().to_rfc3339();
                let _ = self.db.execute(
                    "UPDATE cache_entries SET accessed_at = ?1 WHERE hash = ?2",
                    params![now, hash],
                );
                let entry: CompressedContent = serde_json::from_str(&data)?;
                Ok(Some(entry))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SqzError::SessionStore(e)),
        }
    }

    /// Look up a cache entry by a **prefix** of the content hash.
    ///
    /// The inline dedup refs we hand to the LLM carry only the first 16
    /// hex chars of the SHA-256 (`§ref:<16-hex>§`), so when an agent runs
    /// `sqz expand <prefix>` we need to resolve those 16 chars back to the
    /// full 64-char key. Uses `LIKE 'prefix%'` with an index-friendly
    /// anchored pattern so the query stays O(log n) on the primary key.
    ///
    /// Returns `Ok(Some((full_hash, entry)))` on unique match.
    /// Returns `Ok(None)` if no entries match.
    /// Returns `Err(_)` if the prefix is ambiguous (2+ matches) — the
    /// caller should tell the user to use a longer prefix. 16-hex
    /// collisions are astronomically unlikely (one in 2^64) but we
    /// refuse to guess rather than quietly serve a surprise file.
    ///
    /// The prefix is validated as lowercase hex. Non-hex input returns
    /// `None` without touching the database — this is how we handle the
    /// common user-error case of pasting the ref with the `§` markers
    /// still attached (they get rejected before we query SQLite).
    pub fn get_cache_entry_by_prefix(
        &self,
        prefix: &str,
    ) -> Result<Option<(String, CompressedContent)>> {
        // Reject anything that isn't pure lowercase hex. The inline refs
        // we emit are always lowercase so there's no reason to case-fold
        // here; accidentally matching uppercase input would also match
        // unrelated entries if someone hand-crafted a collision.
        if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
            return Ok(None);
        }
        let pattern = format!("{prefix}%");
        let mut stmt = self
            .db
            .prepare("SELECT hash, data FROM cache_entries WHERE hash LIKE ?1 LIMIT 2")?;
        let mut rows = stmt.query(params![pattern])?;

        let first = match rows.next()? {
            Some(r) => {
                let hash: String = r.get(0)?;
                let data: String = r.get(1)?;
                (hash, data)
            }
            None => return Ok(None),
        };

        // Two or more hits — refuse. Prefix ambiguity is user-recoverable:
        // rerun with a longer prefix. Matching one arbitrarily would be
        // a silent data surprise.
        if rows.next()?.is_some() {
            return Err(SqzError::Other(format!(
                "cache: prefix '{prefix}' matches multiple entries — use a longer prefix"
            )));
        }
        drop(rows);
        drop(stmt);

        // Touch accessed_at for LRU tracking — symmetric with get_cache_entry.
        let now = Utc::now().to_rfc3339();
        let _ = self.db.execute(
            "UPDATE cache_entries SET accessed_at = ?1 WHERE hash = ?2",
            params![now, first.0],
        );
        let entry: CompressedContent = serde_json::from_str(&first.1)?;
        Ok(Some((first.0, entry)))
    }

    /// Read the `accessed_at` timestamp for a cached hash without updating
    /// it. Returns `None` if the hash is not cached.
    ///
    /// Used by the dedup freshness check: if `accessed_at` is recent, the
    /// LLM likely still has the original content in its context window, so
    /// returning a ref is safe. If it's old, re-send the full content.
    pub fn get_cache_entry_accessed_at(&self, hash: &str) -> Result<Option<DateTime<Utc>>> {
        let result: rusqlite::Result<String> = self.db.query_row(
            "SELECT accessed_at FROM cache_entries WHERE hash = ?1",
            params![hash],
            |row| row.get(0),
        );
        match result {
            Ok(s) => {
                let ts = s
                    .parse::<DateTime<Utc>>()
                    .map_err(|e| SqzError::Other(format!("invalid accessed_at: {e}")))?;
                Ok(Some(ts))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SqzError::SessionStore(e)),
        }
    }

    /// Check if a cache entry exists without updating `accessed_at`.
    pub fn cache_entry_exists(&self, hash: &str) -> Result<bool> {
        let result: rusqlite::Result<i64> = self.db.query_row(
            "SELECT 1 FROM cache_entries WHERE hash = ?1",
            params![hash],
            |row| row.get(0),
        );
        match result {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(SqzError::SessionStore(e)),
        }
    }

    /// Update `accessed_at` for a cached hash to the current time. Called by
    /// the cache manager when a ref is served so the next staleness check
    /// sees the recent send.
    pub fn touch_cache_entry(&self, hash: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "UPDATE cache_entries SET accessed_at = ?1 WHERE hash = ?2",
            params![now, hash],
        )?;
        Ok(())
    }

    /// Resolve a hash prefix (or full hash) to the unique full hash of
    /// the matching cache entry. Returns `Ok(None)` for non-existent
    /// entries. Returns an error if the prefix is ambiguous (matches
    /// more than one entry) so the caller can ask for a longer prefix.
    /// Does not touch `accessed_at`.
    pub fn resolve_cache_hash_by_prefix(&self, prefix: &str) -> Result<Option<String>> {
        if prefix.is_empty()
            || !prefix
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Ok(None);
        }
        let pattern = format!("{prefix}%");
        let mut stmt = self
            .db
            .prepare("SELECT hash FROM cache_entries WHERE hash LIKE ?1 LIMIT 2")?;
        let mut rows = stmt.query(params![pattern])?;
        let first: Option<String> = match rows.next()? {
            Some(r) => Some(r.get(0)?),
            None => None,
        };
        if rows.next()?.is_some() {
            return Err(SqzError::Other(format!(
                "cache: prefix '{prefix}' matches multiple entries — use a longer prefix"
            )));
        }
        Ok(first)
    }

    /// Mark a cache entry as pinned so it bypasses the TTL freshness
    /// check. Returns `Ok(true)` if a row was updated, `Ok(false)` if the
    /// hash is not in the cache.
    pub fn pin_cache_entry(&self, hash: &str) -> Result<bool> {
        let n = self.db.execute(
            "UPDATE cache_entries SET pinned = 1 WHERE hash = ?1",
            params![hash],
        )?;
        Ok(n > 0)
    }

    /// Unpin a cache entry so it returns to normal TTL-based freshness.
    /// Returns `Ok(true)` if a row was updated, `Ok(false)` otherwise.
    pub fn unpin_cache_entry(&self, hash: &str) -> Result<bool> {
        let n = self.db.execute(
            "UPDATE cache_entries SET pinned = 0 WHERE hash = ?1",
            params![hash],
        )?;
        Ok(n > 0)
    }

    /// Unpin every cache entry. Returns the count of rows that were
    /// previously pinned.
    pub fn unpin_all_cache_entries(&self) -> Result<usize> {
        let n = self
            .db
            .execute("UPDATE cache_entries SET pinned = 0 WHERE pinned = 1", [])?;
        Ok(n)
    }

    /// Whether the given hash is pinned. Returns `false` for unpinned
    /// entries AND for hashes not in the cache.
    pub fn is_cache_entry_pinned(&self, hash: &str) -> Result<bool> {
        let result: rusqlite::Result<i64> = self.db.query_row(
            "SELECT pinned FROM cache_entries WHERE hash = ?1",
            params![hash],
            |row| row.get(0),
        );
        match result {
            Ok(v) => Ok(v != 0),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(SqzError::SessionStore(e)),
        }
    }

    /// List every pinned cache entry. Returns `(hash, accessed_at, size_bytes)`
    /// tuples so the CLI can render a useful table.
    pub fn list_pinned_cache_entries(
        &self,
    ) -> Result<Vec<(String, DateTime<Utc>, u64)>> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT hash, accessed_at, length(data) FROM cache_entries \
                 WHERE pinned = 1 ORDER BY accessed_at DESC",
            )
            .map_err(SqzError::SessionStore)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .map_err(SqzError::SessionStore)?;
        let mut out = Vec::new();
        for row in rows {
            let (hash, accessed_at_str, size) = row?;
            let accessed_at = accessed_at_str.parse::<DateTime<Utc>>().map_err(|e| {
                SqzError::Other(format!("invalid accessed_at on pinned entry: {e}"))
            })?;
            out.push((hash, accessed_at, size));
        }
        Ok(out)
    }

    /// Set a metadata key/value. Persists across sqz process boundaries
    /// (each shell-hook invocation is a short-lived process).
    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get a metadata value. Returns `None` if the key has never been set.
    pub fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let result: rusqlite::Result<String> = self.db.query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            params![key],
            |row| row.get(0),
        );
        match result {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SqzError::SessionStore(e)),
        }
    }

    /// Log a compression event for cumulative stats tracking.
    pub fn log_compression(
        &self,
        tokens_original: u32,
        tokens_compressed: u32,
        stages: &[String],
        mode: &str,
    ) -> Result<()> {
        self.log_compression_with_project(tokens_original, tokens_compressed, stages, mode, None)
    }

    /// Log a compression event tagged with a project directory.
    pub fn log_compression_with_project(
        &self,
        tokens_original: u32,
        tokens_compressed: u32,
        stages: &[String],
        mode: &str,
        project_dir: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let stages_str = stages.join(",");
        self.db.execute(
            "INSERT INTO compression_log (tokens_original, tokens_compressed, stages_applied, mode, created_at, project_dir) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![tokens_original, tokens_compressed, stages_str, mode, now, project_dir],
        ).map_err(SqzError::SessionStore)?;
        Ok(())
    }

    /// Log a compression-regret signal. `kind` is `"rerun"` (byte-identical
    /// output re-produced within the regret window) or `"expand"` (a
    /// `§ref:…§` token expanded back to original bytes). `gap_secs` is the
    /// time since the content was last seen; 0.0 when unknown.
    pub fn log_regret(&self, kind: &str, cmd: &str, hash: &str, gap_secs: f64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO regret_log (kind, cmd, hash, gap_secs, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![kind, cmd, hash, gap_secs, now],
        ).map_err(SqzError::SessionStore)?;
        Ok(())
    }

    /// Aggregate regret counts for `sqz stats`.
    pub fn regret_stats(&self) -> Result<RegretStats> {
        let mut stmt = self.db.prepare(
            "SELECT \
               COALESCE(SUM(CASE WHEN kind = 'rerun' THEN 1 ELSE 0 END), 0), \
               COALESCE(SUM(CASE WHEN kind = 'expand' THEN 1 ELSE 0 END), 0), \
               COALESCE(AVG(CASE WHEN kind = 'rerun' THEN gap_secs END), 0.0) \
             FROM regret_log",
        ).map_err(SqzError::SessionStore)?;
        let stats = stmt.query_row([], |row| {
            Ok(RegretStats {
                reruns: row.get::<_, u32>(0)?,
                expands: row.get::<_, u32>(1)?,
                avg_rerun_gap_secs: row.get::<_, f64>(2)?,
            })
        }).map_err(SqzError::SessionStore)?;
        Ok(stats)
    }

    /// Command families where compression is barely helping: meaningful
    /// volume, under 25% reduction, ranked by tokens that still reached
    /// the model. This is the formatter backlog, computed from real
    /// history instead of guesses. Dedup rows are excluded (a ref is
    /// already the best possible outcome).
    pub fn formatter_gaps(&self, days: u32, limit: u32) -> Result<Vec<CommandStats>> {
        let offset = format!("-{days} days");
        let mut stmt = self.db.prepare(
            "SELECT mode, COUNT(*), SUM(tokens_original), SUM(tokens_compressed) \
             FROM compression_log \
             WHERE created_at >= date('now', ?1) AND mode NOT IN ('dedup', 'slice') \
             GROUP BY mode \
             HAVING SUM(tokens_original) >= 500 \
                AND (SUM(tokens_original) - SUM(tokens_compressed)) * 100.0 \
                    / SUM(tokens_original) < 25.0 \
             ORDER BY SUM(tokens_compressed) DESC \
             LIMIT ?2",
        ).map_err(SqzError::SessionStore)?;
        let rows = stmt.query_map(params![offset, limit], |row| {
            let tokens_in: u64 = row.get(2)?;
            let tokens_out: u64 = row.get(3)?;
            Ok(CommandStats {
                command: row.get(0)?,
                invocations: row.get(1)?,
                tokens_in,
                tokens_out,
                tokens_saved: tokens_in.saturating_sub(tokens_out),
            })
        }).map_err(SqzError::SessionStore)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(results)
    }

    /// Rerun-regret counts grouped by command, most regretted first.
    pub fn regret_by_command(&self, limit: u32) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.db.prepare(
            "SELECT cmd, COUNT(*) FROM regret_log \
             WHERE kind = 'rerun' AND cmd != '' \
             GROUP BY cmd ORDER BY COUNT(*) DESC LIMIT ?1",
        ).map_err(SqzError::SessionStore)?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        }).map_err(SqzError::SessionStore)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(results)
    }

    /// Get cumulative compression stats from the log.
    pub fn compression_stats(&self) -> Result<CompressionStats> {
        let mut stmt = self.db.prepare(
            "SELECT COUNT(*), COALESCE(SUM(tokens_original), 0), COALESCE(SUM(tokens_compressed), 0) FROM compression_log",
        ).map_err(SqzError::SessionStore)?;

        let stats = stmt.query_row([], |row| {
            Ok(CompressionStats {
                total_compressions: row.get::<_, u32>(0)?,
                total_tokens_in: row.get::<_, u64>(1)?,
                total_tokens_out: row.get::<_, u64>(2)?,
            })
        }).map_err(SqzError::SessionStore)?;

        Ok(stats)
    }

    /// Get daily compression gains for the last N days.
    pub fn daily_gains(&self, days: u32) -> Result<Vec<DailyGain>> {
        let mut stmt = self.db.prepare(
            "SELECT date(created_at) as d, COUNT(*), SUM(tokens_original), SUM(tokens_compressed) \
             FROM compression_log \
             WHERE created_at >= date('now', ?1) \
             GROUP BY d ORDER BY d",
        ).map_err(SqzError::SessionStore)?;

        let offset = format!("-{days} days");
        let rows = stmt.query_map(params![offset], |row| {
            let tokens_in: u64 = row.get(2)?;
            let tokens_out: u64 = row.get(3)?;
            Ok(DailyGain {
                date: row.get(0)?,
                compressions: row.get(1)?,
                tokens_in,
                tokens_saved: tokens_in.saturating_sub(tokens_out),
            })
        }).map_err(SqzError::SessionStore)?;

        let mut gains = Vec::new();
        for row in rows {
            gains.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(gains)
    }

    /// Get cumulative compression stats for a specific project directory.
    pub fn compression_stats_for_project(&self, project_dir: &str) -> Result<CompressionStats> {
        let mut stmt = self.db.prepare(
            "SELECT COUNT(*), COALESCE(SUM(tokens_original), 0), COALESCE(SUM(tokens_compressed), 0) \
             FROM compression_log WHERE project_dir = ?1",
        ).map_err(SqzError::SessionStore)?;

        let stats = stmt.query_row(params![project_dir], |row| {
            Ok(CompressionStats {
                total_compressions: row.get::<_, u32>(0)?,
                total_tokens_in: row.get::<_, u64>(1)?,
                total_tokens_out: row.get::<_, u64>(2)?,
            })
        }).map_err(SqzError::SessionStore)?;

        Ok(stats)
    }

    /// Get daily compression gains for a specific project directory.
    pub fn daily_gains_for_project(&self, days: u32, project_dir: &str) -> Result<Vec<DailyGain>> {
        let mut stmt = self.db.prepare(
            "SELECT date(created_at) as d, COUNT(*), SUM(tokens_original), SUM(tokens_compressed) \
             FROM compression_log \
             WHERE created_at >= date('now', ?1) AND project_dir = ?2 \
             GROUP BY d ORDER BY d",
        ).map_err(SqzError::SessionStore)?;

        let offset = format!("-{days} days");
        let rows = stmt.query_map(params![offset, project_dir], |row| {
            let tokens_in: u64 = row.get(2)?;
            let tokens_out: u64 = row.get(3)?;
            Ok(DailyGain {
                date: row.get(0)?,
                compressions: row.get(1)?,
                tokens_in,
                tokens_saved: tokens_in.saturating_sub(tokens_out),
            })
        }).map_err(SqzError::SessionStore)?;

        let mut gains = Vec::new();
        for row in rows {
            gains.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(gains)
    }

    /// List distinct project directories that have compression data.
    pub fn list_projects(&self) -> Result<Vec<(String, u32, u64)>> {
        let mut stmt = self.db.prepare(
            "SELECT project_dir, COUNT(*), COALESCE(SUM(tokens_original) - SUM(tokens_compressed), 0) \
             FROM compression_log \
             WHERE project_dir IS NOT NULL \
             GROUP BY project_dir \
             ORDER BY COUNT(*) DESC",
        ).map_err(SqzError::SessionStore)?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u64>(2)?,
            ))
        }).map_err(SqzError::SessionStore)?;

        let mut projects = Vec::new();
        for row in rows {
            projects.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(projects)
    }

    /// Per-command breakdown of token usage. Returns top N commands by
    /// total tokens consumed, with compression stats for each.
    /// Used by `sqz stats --breakdown` to show where tokens are going.
    pub fn command_breakdown(&self, limit: u32) -> Result<Vec<CommandStats>> {
        let mut stmt = self.db.prepare(
            "SELECT mode, COUNT(*), SUM(tokens_original), SUM(tokens_compressed) \
             FROM compression_log \
             GROUP BY mode \
             ORDER BY SUM(tokens_original) DESC \
             LIMIT ?1",
        ).map_err(SqzError::SessionStore)?;

        let rows = stmt.query_map(params![limit], |row| {
            let tokens_in: u64 = row.get(2)?;
            let tokens_out: u64 = row.get(3)?;
            Ok(CommandStats {
                command: row.get(0)?,
                invocations: row.get(1)?,
                tokens_in,
                tokens_out,
                tokens_saved: tokens_in.saturating_sub(tokens_out),
            })
        }).map_err(SqzError::SessionStore)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(results)
    }

    /// Per-command breakdown filtered to a specific project.
    pub fn command_breakdown_for_project(&self, limit: u32, project_dir: &str) -> Result<Vec<CommandStats>> {
        let mut stmt = self.db.prepare(
            "SELECT mode, COUNT(*), SUM(tokens_original), SUM(tokens_compressed) \
             FROM compression_log \
             WHERE project_dir = ?1 \
             GROUP BY mode \
             ORDER BY SUM(tokens_original) DESC \
             LIMIT ?2",
        ).map_err(SqzError::SessionStore)?;

        let rows = stmt.query_map(params![project_dir, limit], |row| {
            let tokens_in: u64 = row.get(2)?;
            let tokens_out: u64 = row.get(3)?;
            Ok(CommandStats {
                command: row.get(0)?,
                invocations: row.get(1)?,
                tokens_in,
                tokens_out,
                tokens_saved: tokens_in.saturating_sub(tokens_out),
            })
        }).map_err(SqzError::SessionStore)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(results)
    }

    /// Get the total tokens injected in the current session (since last
    /// compaction or start). Used by the adaptive pressure system to
    /// decide whether to escalate compression intensity.
    pub fn session_pressure(&self, since_minutes: u32) -> Result<u64> {
        let mut stmt = self.db.prepare(
            "SELECT COALESCE(SUM(tokens_compressed), 0) \
             FROM compression_log \
             WHERE created_at >= datetime('now', ?1)",
        ).map_err(SqzError::SessionStore)?;

        let offset = format!("-{since_minutes} minutes");
        let total: u64 = stmt.query_row(params![offset], |row| {
            row.get(0)
        }).map_err(SqzError::SessionStore)?;

        Ok(total)
    }

    // ── Known files (persistent cross-command context tracking) ───────────

    /// Record a file path as "known" (its content is in the dedup cache).
    /// Used by cross-command context refs to annotate error messages.
    pub fn add_known_file(&self, path: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT OR REPLACE INTO known_files (path, added_at) VALUES (?1, ?2)",
            params![path, now],
        ).map_err(SqzError::SessionStore)?;
        Ok(())
    }

    /// Load all known file paths from the persistent store.
    pub fn known_files(&self) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare(
            "SELECT path FROM known_files ORDER BY added_at DESC",
        ).map_err(SqzError::SessionStore)?;

        let rows = stmt.query_map([], |row| {
            row.get::<_, String>(0)
        }).map_err(SqzError::SessionStore)?;

        let mut files = Vec::new();
        for row in rows {
            files.push(row.map_err(SqzError::SessionStore)?);
        }
        Ok(files)
    }
    /// Clear all known files (e.g. on session reset).
    pub fn clear_known_files(&self) -> Result<()> {
        self.db
            .execute("DELETE FROM known_files", [])
            .map_err(SqzError::SessionStore)?;
        Ok(())
    }

    /// Number of entries currently in the dedup cache.
    pub fn cache_entry_count(&self) -> Result<u64> {
        self.db.query_row(
            "SELECT COUNT(*) FROM cache_entries", [],
            |row| row.get(0),
        ).map_err(SqzError::SessionStore)
    }

    /// Increment the cache-hit counter (persisted in metadata).
    pub fn record_cache_hit(&self) -> Result<()> {
        let count: u64 = self.db.query_row(
            "SELECT value FROM metadata WHERE key = 'cache_hits'", [],
            |row| row.get::<_, String>(0).map(|s| s.parse::<u64>().unwrap_or(0)),
        ).unwrap_or(0);
        let new = count + 1;
        self.db.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('cache_hits', ?1)",
            params![new.to_string()],
        )?;
        Ok(())
    }

    /// Increment the cache-miss counter (persisted in metadata).
    pub fn record_cache_miss(&self) -> Result<()> {
        let count: u64 = self.db.query_row(
            "SELECT value FROM metadata WHERE key = 'cache_misses'", [],
            |row| row.get::<_, String>(0).map(|s| s.parse::<u64>().unwrap_or(0)),
        ).unwrap_or(0);
        let new = count + 1;
        self.db.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('cache_misses', ?1)",
            params![new.to_string()],
        )?;
        Ok(())
    }

    /// Return (cache_hits, cache_misses) from metadata counters.
    pub fn cache_hit_miss_counts(&self) -> Result<(u64, u64)> {
        let hits: u64 = self.db.query_row(
            "SELECT value FROM metadata WHERE key = 'cache_hits'", [],
            |row| row.get::<_, String>(0).map(|s| s.parse::<u64>().unwrap_or(0)),
        ).unwrap_or(0);
        let misses: u64 = self.db.query_row(
            "SELECT value FROM metadata WHERE key = 'cache_misses'", [],
            |row| row.get::<_, String>(0).map(|s| s.parse::<u64>().unwrap_or(0)),
        ).unwrap_or(0);
        Ok((hits, misses))
    }

    /// Clear the dedup cache. After this, all future reads will be treated
    /// as cache misses and compressed fresh. Does NOT clear compression
    /// stats or session history.
    pub fn clear_cache(&self) -> Result<u64> {
        let count: u64 = self.db.query_row(
            "SELECT COUNT(*) FROM cache_entries", [],
            |row| row.get(0),
        ).unwrap_or(0);
        self.db.execute("DELETE FROM cache_entries", [])
            .map_err(SqzError::SessionStore)?;
        let _ = self.db.execute(
            "DELETE FROM recall_index WHERE kind = 'output'", [],
        );
        // Also clear the compaction marker so freshness checks don't
        // reference timestamps from the deleted entries.
        let _ = self.db.execute(
            "DELETE FROM metadata WHERE key = 'last_compaction_at'", [],
        );
        Ok(count)
    }

    /// Clear compression stats (the `compression_log` table). After this,
    /// `sqz stats` and `sqz gain` will show zero until new compressions
    /// happen.
    pub fn clear_stats(&self) -> Result<u64> {
        let count: u64 = self.db.query_row(
            "SELECT COUNT(*) FROM compression_log", [],
            |row| row.get(0),
        ).unwrap_or(0);
        self.db.execute("DELETE FROM compression_log", [])
            .map_err(SqzError::SessionStore)?;
        Ok(count)
    }

    /// Clear compression stats for a specific project directory only.
    pub fn clear_stats_for_project(&self, project_dir: &str) -> Result<u64> {
        let count: u64 = self.db.query_row(
            "SELECT COUNT(*) FROM compression_log WHERE project_dir = ?1",
            rusqlite::params![project_dir],
            |row| row.get(0),
        ).unwrap_or(0);
        self.db.execute(
            "DELETE FROM compression_log WHERE project_dir = ?1",
            rusqlite::params![project_dir],
        ).map_err(SqzError::SessionStore)?;
        Ok(count)
    }

    /// Full reset: clear cache, stats, sessions, known files, and metadata.
    /// Equivalent to deleting sessions.db and letting sqz recreate it.
    pub fn reset_all(&self) -> Result<()> {
        self.db.execute("DELETE FROM cache_entries", []).map_err(SqzError::SessionStore)?;
        self.db.execute("DELETE FROM compression_log", []).map_err(SqzError::SessionStore)?;
        self.db.execute("DELETE FROM sessions", []).map_err(SqzError::SessionStore)?;
        self.db.execute("DELETE FROM known_files", []).map_err(SqzError::SessionStore)?;
        self.db.execute("DELETE FROM metadata", []).map_err(SqzError::SessionStore)?;
        let _ = self.db.execute("DELETE FROM recall_index", []);
        // VACUUM to reclaim disk space.
        let _ = self.db.execute("VACUUM", []);
        Ok(())
    }

    /// Return all sessions ordered by most recently updated, limited to `limit`.
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<SessionSummary>> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT id, project_dir, compressed_summary, created_at, updated_at \
             FROM sessions ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(SqzError::SessionStore)?;

        let rows = stmt
            .query_map(params![limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(SqzError::SessionStore)?;

        let mut results = Vec::new();
        for row in rows {
            let (id, project_dir, compressed_summary, created_at, updated_at) = row?;
            results.push(row_to_summary(
                id,
                project_dir,
                compressed_summary,
                created_at,
                updated_at,
            )?);
        }
        Ok(results)
    }

    /// Per-tool breakdown: group compression_log by stages_applied (comma-split).
    pub fn per_tool_breakdown(&self) -> Result<Vec<(String, u64, u64, u32)>> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT stages_applied, SUM(tokens_original), SUM(tokens_compressed), COUNT(*) \
             FROM compression_log GROUP BY stages_applied ORDER BY COUNT(*) DESC LIMIT 20",
            )
            .map_err(SqzError::SessionStore)?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .map_err(SqzError::SessionStore)?;

        let mut tool_map: std::collections::HashMap<String, (u64, u64, u32)> =
            std::collections::HashMap::new();
        for row in rows {
            let (stages, tokens_in, tokens_out, count) = row?;
            for stage in stages.split(',') {
                let stage = stage.trim();
                if stage.is_empty() {
                    continue;
                }
                let entry = tool_map.entry(stage.to_string()).or_default();
                entry.0 += tokens_in;
                entry.1 += tokens_out;
                entry.2 += count;
            }
        }

        let mut result: Vec<_> = tool_map.into_iter().collect();
        result.sort_by(|a, b| b.1 .2.cmp(&a.1 .2));
        Ok(result
            .into_iter()
            .map(|(name, (tin, tout, cnt))| (name, tin, tout, cnt))
            .collect())
    }

    /// Per-command breakdown: group compression_log by mode.
    pub fn per_command_breakdown(&self) -> Result<Vec<(String, u64, u64, u32)>> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT COALESCE(mode, 'auto'), SUM(tokens_original), SUM(tokens_compressed), COUNT(*) \
             FROM compression_log GROUP BY mode ORDER BY COUNT(*) DESC LIMIT 20",
            )
            .map_err(SqzError::SessionStore)?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .map_err(SqzError::SessionStore)?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}

/// Cumulative compression statistics.
#[derive(Debug, Clone, Default)]
pub struct CompressionStats {
    pub total_compressions: u32,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
}

impl CompressionStats {
    pub fn tokens_saved(&self) -> u64 {
        self.total_tokens_in.saturating_sub(self.total_tokens_out)
    }

    pub fn reduction_pct(&self) -> f64 {
        if self.total_tokens_in == 0 {
            0.0
        } else {
            (1.0 - self.total_tokens_out as f64 / self.total_tokens_in as f64) * 100.0
        }
    }
}

/// A single day's compression gain.
#[derive(Debug, Clone)]
pub struct DailyGain {
    pub date: String,
    pub compressions: u32,
    pub tokens_saved: u64,
    pub tokens_in: u64,
}

/// Per-command token usage breakdown.
#[derive(Debug, Clone)]
pub struct CommandStats {
    pub command: String,
    pub invocations: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_saved: u64,
}

/// A cache entry with its original bytes and the served form (JSON
/// `CompressedContent`), as returned by [`SessionStore::recent_originals`].
#[derive(Debug, Clone)]
pub struct SliceCandidate {
    pub hash: String,
    pub original: Vec<u8>,
    pub data_json: String,
}

/// One `sqz recall` search hit.
#[derive(Debug, Clone)]
pub struct RecallHit {
    /// Content hash (kind `output`, expandable via `sqz expand`) or
    /// session id (kind `session`).
    pub ref_hash: String,
    pub kind: String,
    /// Match context with `>>`/`<<` around matched terms.
    pub snippet: String,
    pub created_at: String,
}

/// Build a safe FTS5 MATCH expression from free-form user input: each
/// whitespace-separated term becomes a quoted phrase (implicit AND), so
/// FTS5 operator characters in the input can't break the query syntax.
fn fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .filter_map(|term| {
            let cleaned = term.replace('"', "");
            if cleaned.is_empty() {
                None
            } else {
                Some(format!("\"{cleaned}\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Aggregated compression-regret signals (see `regret_log`).
#[derive(Debug, Clone, Default)]
pub struct RegretStats {
    /// Byte-identical output re-produced within the regret window.
    pub reruns: u32,
    /// `§ref:…§` tokens expanded back to original bytes.
    pub expands: u32,
    /// Mean seconds between last sighting and the rerun.
    pub avg_rerun_gap_secs: f64,
}

impl CommandStats {
    pub fn reduction_pct(&self) -> f64 {
        if self.tokens_in == 0 {
            0.0
        } else {
            (1.0 - self.tokens_out as f64 / self.tokens_in as f64) * 100.0
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BudgetState, CorrectionLog, ModelFamily, SessionState};
    use chrono::Utc;
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn make_session(id: &str, project_dir: &str, summary: &str) -> SessionState {
        let now = Utc::now();
        SessionState {
            id: id.to_string(),
            project_dir: PathBuf::from(project_dir),
            conversation: vec![],
            corrections: CorrectionLog::default(),
            pins: vec![],
            learnings: vec![],
            compressed_summary: summary.to_string(),
            budget: BudgetState {
                window_size: 200_000,
                consumed: 0,
                pinned: 0,
                model_family: ModelFamily::AnthropicClaude,
            },
            tool_usage: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    fn in_memory_store() -> SessionStore {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        SessionStore { db: conn }
    }

    fn test_content(data: &str) -> CompressedContent {
        CompressedContent {
            data: data.to_string(),
            tokens_compressed: 1,
            tokens_original: 2,
            stages_applied: vec![],
            compression_ratio: 0.5,
            provenance: crate::types::Provenance::default(),
            verify: None,
        }
    }

    #[test]
    fn recall_indexes_cache_originals_and_searches() {
        let store = in_memory_store();
        let content = test_content("compressed");
        store
            .save_cache_entry_with_original(
                "hash-auth",
                &content,
                Some(b"error: auth middleware rejected the session token"),
            )
            .unwrap();
        store
            .save_cache_entry_with_original(
                "hash-build",
                &content,
                Some(b"cargo build finished in 32s with zero warnings"),
            )
            .unwrap();

        let hits = store.recall_search("auth middleware", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ref_hash, "hash-auth");
        assert_eq!(hits[0].kind, "output");
        assert!(hits[0].snippet.contains(">>auth<<"), "snippet: {}", hits[0].snippet);

        // Re-saving the same hash must not duplicate index rows.
        store
            .save_cache_entry_with_original(
                "hash-auth",
                &content,
                Some(b"error: auth middleware rejected the session token"),
            )
            .unwrap();
        assert_eq!(store.recall_search("middleware", 5).unwrap().len(), 1);
    }

    #[test]
    fn recall_skips_binary_and_survives_odd_queries() {
        let store = in_memory_store();
        let content = test_content("c");
        store
            .save_cache_entry_with_original("hash-bin", &content, Some(b"binary\x00payload"))
            .unwrap();
        assert!(store.recall_search("binary", 5).unwrap().is_empty());

        // FTS5 operator characters in the query must not error.
        for q in ["NOT AND OR", "\"unbalanced", "col:umn", "wild*", "(paren", "-", ""] {
            let _ = store.recall_search(q, 5).unwrap();
        }
    }

    #[test]
    fn recall_deletion_and_clear_coherence() {
        let store = in_memory_store();
        let content = test_content("c");
        store
            .save_cache_entry_with_original("hash-x", &content, Some(b"kubectl rollout restarted deployment"))
            .unwrap();
        assert_eq!(store.recall_search("rollout", 5).unwrap().len(), 1);

        store.delete_cache_entry("hash-x").unwrap();
        assert!(store.recall_search("rollout", 5).unwrap().is_empty());

        // clear_cache drops output entries but keeps session summaries.
        store
            .save_cache_entry_with_original("hash-y", &content, Some(b"terraform plan shows three changes"))
            .unwrap();
        let session = make_session("sess-recall", "/tmp/p", "refactored the billing retry loop");
        store.save_session(&session).unwrap();
        store.clear_cache().unwrap();
        assert!(store.recall_search("terraform", 5).unwrap().is_empty());
        let session_hits = store.recall_search("billing retry", 5).unwrap();
        assert_eq!(session_hits.len(), 1);
        assert_eq!(session_hits[0].kind, "session");
        assert_eq!(session_hits[0].ref_hash, "sess-recall");

        store.reset_all().unwrap();
        assert!(store.recall_search("billing", 5).unwrap().is_empty());
    }

    #[test]
    fn recall_ranks_better_matches_first() {
        let store = in_memory_store();
        let content = test_content("c");
        store
            .save_cache_entry_with_original(
                "hash-dense",
                &content,
                Some(b"panic unwrap panic unwrap panic in worker thread"),
            )
            .unwrap();
        store
            .save_cache_entry_with_original(
                "hash-sparse",
                &content,
                Some(format!("{} panic once at the end", "filler line of ordinary words\n".repeat(50)).as_bytes()),
            )
            .unwrap();
        let hits = store.recall_search("panic", 5).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].ref_hash, "hash-dense", "bm25 should rank the denser match first");
    }

    #[test]
    fn formatter_gaps_ranks_low_reduction_high_volume() {
        let store = in_memory_store();
        // High volume, terrible reduction: a gap.
        for _ in 0..4 {
            store.log_compression(1_000, 950, &[], "xcodebuild").unwrap();
        }
        // High volume, great reduction: not a gap.
        for _ in 0..4 {
            store.log_compression(1_000, 100, &[], "cargo test").unwrap();
        }
        // Low volume, bad reduction: filtered by the 500-token floor.
        store.log_compression(100, 95, &[], "tiny-cmd").unwrap();
        // Dedup rows never count as gaps.
        store.log_compression(5_000, 13, &["dedup".to_string()], "dedup").unwrap();

        let gaps = store.formatter_gaps(7, 10).unwrap();
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].command, "xcodebuild");
        assert_eq!(gaps[0].invocations, 4);
        assert!(gaps[0].reduction_pct() < 25.0);
    }

    #[test]
    fn regret_log_roundtrip() {
        let store = in_memory_store();
        // Empty DB: zero everything.
        let empty = store.regret_stats().unwrap();
        assert_eq!(empty.reruns, 0);
        assert_eq!(empty.expands, 0);
        assert_eq!(empty.avg_rerun_gap_secs, 0.0);

        store.log_regret("rerun", "git log", "aaaa", 10.0).unwrap();
        store.log_regret("rerun", "git log", "bbbb", 30.0).unwrap();
        store.log_regret("rerun", "cargo test", "cccc", 20.0).unwrap();
        store.log_regret("expand", "", "dddd", 0.0).unwrap();

        let stats = store.regret_stats().unwrap();
        assert_eq!(stats.reruns, 3);
        assert_eq!(stats.expands, 1);
        assert!((stats.avg_rerun_gap_secs - 20.0).abs() < 1e-9);

        let top = store.regret_by_command(10).unwrap();
        assert_eq!(top[0], ("git log".to_string(), 2));
        assert_eq!(top[1], ("cargo test".to_string(), 1));
        // Expand rows carry an empty cmd and must not appear.
        assert_eq!(top.len(), 2);
    }

    #[test]
    fn test_save_and_load_session() {
        let store = in_memory_store();
        let session = make_session("sess-1", "/home/user/project", "REST API refactor");

        let id = store.save_session(&session).unwrap();
        assert_eq!(id, "sess-1");

        let loaded = store.load_session("sess-1".to_string()).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.compressed_summary, session.compressed_summary);
        assert_eq!(loaded.project_dir, session.project_dir);
    }

    #[test]
    fn test_save_session_upsert() {
        let store = in_memory_store();
        let mut session = make_session("sess-2", "/proj", "initial summary");
        store.save_session(&session).unwrap();

        session.compressed_summary = "updated summary".to_string();
        store.save_session(&session).unwrap();

        let loaded = store.load_session("sess-2".to_string()).unwrap();
        assert_eq!(loaded.compressed_summary, "updated summary");
    }

    #[test]
    fn test_load_nonexistent_session_errors() {
        let store = in_memory_store();
        let result = store.load_session("does-not-exist".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_search_fts() {
        let store = in_memory_store();
        store.save_session(&make_session("s1", "/proj", "REST API refactor with authentication")).unwrap();
        store.save_session(&make_session("s2", "/proj", "database migration postgres")).unwrap();

        let results = store.search("authentication").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "s1");
    }

    #[test]
    fn test_search_by_date() {
        let store = in_memory_store();
        let now = Utc::now();
        let past = now - chrono::Duration::hours(2);
        let future = now + chrono::Duration::hours(2);

        store.save_session(&make_session("s1", "/proj", "recent session")).unwrap();

        let results = store.search_by_date(past, future).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.id == "s1"));
    }

    #[test]
    fn test_search_by_project() {
        let store = in_memory_store();
        store.save_session(&make_session("s1", "/home/user/alpha", "alpha project")).unwrap();
        store.save_session(&make_session("s2", "/home/user/beta", "beta project")).unwrap();

        let results = store.search_by_project(Path::new("/home/user/alpha")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "s1");
    }

    #[test]
    fn test_cache_entry_round_trip() {
        let store = in_memory_store();
        let entry = CompressedContent {
            data: "compressed data".to_string(),
            tokens_compressed: 10,
            tokens_original: 50,
            stages_applied: vec!["strip_nulls".to_string()],
            compression_ratio: 0.2,
            provenance: crate::types::Provenance::default(),
            verify: None,
        };

        store.save_cache_entry("abc123", &entry).unwrap();

        let loaded = store.get_cache_entry("abc123").unwrap().unwrap();
        assert_eq!(loaded.data, entry.data);
        assert_eq!(loaded.tokens_compressed, entry.tokens_compressed);
        assert_eq!(loaded.tokens_original, entry.tokens_original);
    }

    #[test]
    fn test_get_cache_entry_missing_returns_none() {
        let store = in_memory_store();
        let result = store.get_cache_entry("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_open_or_create_corrupted_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");

        // Write garbage bytes to simulate a corrupted database.
        std::fs::write(&path, b"this is not a valid sqlite database").unwrap();

        // Should succeed by creating a fresh database.
        let store = SessionStore::open_or_create(&path).unwrap();
        let session = make_session("s1", "/proj", "after corruption");
        store.save_session(&session).unwrap();
        let loaded = store.load_session("s1".to_string()).unwrap();
        assert_eq!(loaded.id, "s1");
    }

    // ── Property-based tests ──────────────────────────────────────────────────

    /// Build a `SessionState` with a specific `updated_at` timestamp.
    fn make_session_at(id: &str, summary: &str, updated_at: DateTime<Utc>) -> SessionState {
        let now = Utc::now();
        SessionState {
            id: id.to_string(),
            project_dir: PathBuf::from("/proj"),
            conversation: vec![],
            corrections: CorrectionLog::default(),
            pins: vec![],
            learnings: vec![],
            compressed_summary: summary.to_string(),
            budget: BudgetState {
                window_size: 200_000,
                consumed: 0,
                pinned: 0,
                model_family: ModelFamily::AnthropicClaude,
            },
            tool_usage: vec![],
            created_at: now,
            updated_at,
        }
    }

    // ── Property 26: Session store search correctness ─────────────────────────
    // **Validates: Requirements 20.2, 20.3, 20.4**
    //
    // For any set of sessions saved to the store, a keyword search SHALL return
    // all sessions whose compressed_summary contains the keyword, and no
    // sessions that don't contain it.

    proptest! {
        /// **Validates: Requirements 20.2, 20.3, 20.4**
        ///
        /// For any set of sessions saved to the store, a keyword search SHALL
        /// return all sessions whose `compressed_summary` contains the keyword,
        /// and no sessions that don't contain it.
        #[test]
        fn prop_search_correctness(
            // A simple ASCII keyword: 5-8 lowercase letters, no common English
            // words that the porter stemmer might conflate with other terms.
            keyword in "[b-df-hj-np-tv-z]{5,8}",
            // 1-6 summaries that embed the keyword
            matching_suffixes in proptest::collection::vec("[a-z ]{4,20}", 1..=6usize),
            // 1-6 summaries that do NOT contain the keyword
            non_matching in proptest::collection::vec("[a-z ]{8,30}", 1..=6usize),
        ) {
            // Ensure the keyword doesn't accidentally appear in non-matching summaries.
            for s in &non_matching {
                prop_assume!(!s.contains(keyword.as_str()));
            }

            let store = in_memory_store();

            // Save matching sessions (summary = "<suffix> <keyword> <suffix>")
            let mut matching_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (i, suffix) in matching_suffixes.iter().enumerate() {
                let id = format!("match-{i}");
                let summary = format!("{} {} end", suffix, keyword);
                store.save_session(&make_session(&id, "/proj", &summary)).unwrap();
                matching_ids.insert(id);
            }

            // Save non-matching sessions
            let mut non_matching_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (i, summary) in non_matching.iter().enumerate() {
                let id = format!("nomatch-{i}");
                store.save_session(&make_session(&id, "/proj", summary)).unwrap();
                non_matching_ids.insert(id);
            }

            let results = store.search(&keyword).unwrap();
            let result_ids: std::collections::HashSet<String> =
                results.iter().map(|r| r.id.clone()).collect();

            // Every matching session must appear in results.
            for id in &matching_ids {
                prop_assert!(
                    result_ids.contains(id),
                    "matching session '{}' not found in search results for keyword '{}'",
                    id, keyword
                );
            }

            // No non-matching session may appear in results.
            for id in &non_matching_ids {
                prop_assert!(
                    !result_ids.contains(id),
                    "non-matching session '{}' incorrectly appeared in search results for keyword '{}'",
                    id, keyword
                );
            }
        }
    }

    // ── Property: search_by_date correctness ─────────────────────────────────
    // **Validates: Requirements 20.4**
    //
    // For any set of sessions with different timestamps, searching by a date
    // range SHALL return exactly the sessions whose `updated_at` falls within
    // [from, to], and no sessions outside that range.

    proptest! {
        /// **Validates: Requirements 20.4**
        ///
        /// For any set of sessions with distinct timestamps, `search_by_date`
        /// SHALL return exactly the sessions whose `updated_at` is within
        /// `[from, to]`, and no sessions outside that range.
        #[test]
        fn prop_search_by_date_correctness(
            // Generate 2-8 offsets in seconds from epoch (spread over a wide range)
            offsets in proptest::collection::vec(0i64..=86400i64 * 365, 2..=8usize),
            // The search window: start and end offsets (relative to the minimum offset)
            window_start_delta in 0i64..=3600i64,
            window_end_delta   in 3600i64..=7200i64,
        ) {
            use chrono::TimeZone;

            // Deduplicate offsets so each session has a unique timestamp.
            let mut unique_offsets: Vec<i64> = offsets.clone();
            unique_offsets.sort_unstable();
            unique_offsets.dedup();
            prop_assume!(unique_offsets.len() >= 2);

            let base_offset = unique_offsets[0];
            let from_offset = base_offset + window_start_delta;
            let to_offset   = base_offset + window_end_delta;

            let from = Utc.timestamp_opt(from_offset, 0).unwrap();
            let to   = Utc.timestamp_opt(to_offset,   0).unwrap();

            let store = in_memory_store();

            let mut in_range_ids:  std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut out_range_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

            for (i, &offset) in unique_offsets.iter().enumerate() {
                let ts = Utc.timestamp_opt(offset, 0).unwrap();
                let id = format!("sess-{i}");
                let session = make_session_at(&id, "some summary", ts);
                store.save_session(&session).unwrap();

                if ts >= from && ts <= to {
                    in_range_ids.insert(id);
                } else {
                    out_range_ids.insert(id);
                }
            }

            let results = store.search_by_date(from, to).unwrap();
            let result_ids: std::collections::HashSet<String> =
                results.iter().map(|r| r.id.clone()).collect();

            // Every in-range session must appear.
            for id in &in_range_ids {
                prop_assert!(
                    result_ids.contains(id),
                    "in-range session '{}' missing from search_by_date results",
                    id
                );
            }

            // No out-of-range session may appear.
            for id in &out_range_ids {
                prop_assert!(
                    !result_ids.contains(id),
                    "out-of-range session '{}' incorrectly appeared in search_by_date results",
                    id
                );
            }
        }
    }
}
