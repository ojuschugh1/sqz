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
    original    BLOB
);

CREATE TABLE IF NOT EXISTS compression_log (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    tokens_original  INTEGER NOT NULL,
    tokens_compressed INTEGER NOT NULL,
    stages_applied   TEXT NOT NULL,
    mode             TEXT NOT NULL DEFAULT 'auto',
    created_at       TEXT NOT NULL
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

        Ok(session.id.clone())
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
        Ok(())
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

    /// Delete a cache entry by content hash.
    pub fn delete_cache_entry(&self, hash: &str) -> Result<()> {
        self.db.execute(
            "DELETE FROM cache_entries WHERE hash = ?1",
            params![hash],
        )?;
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
        let now = Utc::now().to_rfc3339();
        let stages_str = stages.join(",");
        self.db.execute(
            "INSERT INTO compression_log (tokens_original, tokens_compressed, stages_applied, mode, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![tokens_original, tokens_compressed, stages_str, mode, now],
        ).map_err(SqzError::SessionStore)?;
        Ok(())
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
        self.db.execute("DELETE FROM known_files", [])
            .map_err(SqzError::SessionStore)?;
        Ok(())
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
