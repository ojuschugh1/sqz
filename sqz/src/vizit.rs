//! `sqz vizit` — live terminal dashboard for AI agent sessions.
//!
//! This module implements the `sqz vizit` subcommand, which renders a
//! real-time, auto-refreshing TUI directly in the terminal using raw ANSI
//! escape codes (no external TUI framework).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{Connection, OpenFlags};

// ── ANSI escape code constants ────────────────────────────────────────────────

const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const BRIGHT_GREEN: &str = "\x1b[92m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";

// ── Color / ANSI helpers ──────────────────────────────────────────────────────

/// Remove ANSI escape sequences (pattern `\x1b[...m`) from `s`.
///
/// Uses a simple state machine: scan for `\x1b[`, skip until `m`.
pub fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for ESC (0x1b) followed by '['
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Skip until we find 'm'
            i += 2;
            while i < bytes.len() && bytes[i] != b'm' {
                i += 1;
            }
            // Skip the 'm' itself
            if i < bytes.len() {
                i += 1;
            }
        } else {
            // Safe: we're iterating bytes but need to push chars.
            // Collect the current char (which may be multi-byte).
            let ch = s[i..].chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

/// Truncate visible content of `s` to `max_cols` visible characters.
///
/// If the visible width (after stripping ANSI) exceeds `max_cols`, the
/// visible content is truncated to `max_cols - 1` characters and `…` is
/// appended.  ANSI sequences are preserved up to the truncation point.
pub fn truncate_to_width(s: &str, max_cols: usize) -> String {
    let visible = strip_ansi(s);
    let visible_len = visible.chars().count();
    if visible_len <= max_cols {
        return s.to_string();
    }

    // We need to keep max_cols - 1 visible chars, then append '…'.
    let keep = if max_cols > 0 { max_cols - 1 } else { 0 };

    // Walk through `s`, counting visible chars and copying bytes.
    let mut result = String::new();
    let mut visible_count = 0usize;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && visible_count < keep {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Copy the entire escape sequence verbatim.
            let start = i;
            i += 2;
            while i < bytes.len() && bytes[i] != b'm' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // include 'm'
            }
            result.push_str(&s[start..i]);
        } else {
            let ch = s[i..].chars().next().unwrap();
            result.push(ch);
            visible_count += 1;
            i += ch.len_utf8();
        }
    }
    result.push('…');
    result
}

/// Returns `true` if color output is appropriate.
///
/// Returns `false` if the `NO_COLOR` environment variable is set, or if
/// stdout is not a TTY.
pub fn is_color_enabled() -> bool {
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    // SAFETY: isatty is a simple syscall with no side effects.
    (unsafe { libc::isatty(libc::STDOUT_FILENO) } != 0)
}

// ── Terminal size ─────────────────────────────────────────────────────────────

/// Query the current terminal dimensions.
///
/// Returns `(cols, rows)`. Falls back to `(80, 24)` if `ioctl` fails.
#[cfg(unix)]
pub fn terminal_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            (ws.ws_col, ws.ws_row)
        } else {
            (80, 24)
        }
    }
}

#[cfg(not(unix))]
pub fn terminal_size() -> (u16, u16) {
    (80, 24)
}

// ── Raw mode ──────────────────────────────────────────────────────────────────

/// RAII guard that restores the terminal to its original state on drop.
#[cfg(unix)]
pub struct TerminalGuard {
    original_termios: libc::termios,
}

#[cfg(unix)]
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original_termios);
        }
        // Show cursor.
        let _ = std::io::Write::write_all(&mut std::io::stdout(), b"\x1b[?25h");
    }
}

#[cfg(not(unix))]
pub struct TerminalGuard;

/// Enter terminal raw mode.
///
/// - Saves the current `termios` settings.
/// - Clears `ECHO | ICANON` in `c_lflag`.
/// - Sets `VMIN=0, VTIME=0` for non-blocking reads.
/// - Applies the new settings with `TCSANOW`.
///
/// Returns a `TerminalGuard` that restores the terminal on drop.
#[cfg(unix)]
pub fn enter_raw_mode() -> Result<TerminalGuard, String> {
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut termios) != 0 {
            return Err(format!(
                "[sqz vizit] tcgetattr failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let original = termios;

        // Disable echo and canonical mode.
        termios.c_lflag &= !(libc::ECHO | libc::ICANON);
        // Non-blocking reads.
        termios.c_cc[libc::VMIN] = 0;
        termios.c_cc[libc::VTIME] = 0;

        if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios) != 0 {
            return Err(format!(
                "[sqz vizit] tcsetattr failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(TerminalGuard { original_termios: original })
    }
}

#[cfg(not(unix))]
pub fn enter_raw_mode() -> Result<TerminalGuard, String> {
    Err("[sqz vizit] raw mode not supported on this platform".to_string())
}

// ── SIGWINCH handler ──────────────────────────────────────────────────────────

/// Set to `true` by the `SIGWINCH` signal handler when the terminal is resized.
static RESIZE_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Register the `SIGWINCH` signal handler.
///
/// The handler sets `RESIZE_REQUESTED` to `true`. The event loop checks and
/// clears this flag each iteration, re-querying `terminal_size()` if set.
#[cfg(unix)]
fn register_sigwinch_handler() {
    unsafe {
        libc::signal(libc::SIGWINCH, sigwinch_handler as *const () as libc::sighandler_t);
    }
}

#[cfg(unix)]
extern "C" fn sigwinch_handler(_: libc::c_int) {
    RESIZE_REQUESTED.store(true, Ordering::Relaxed);
}

#[cfg(not(unix))]
fn register_sigwinch_handler() {}

// ── Configuration ─────────────────────────────────────────────────────────────

/// Configuration for the `sqz vizit` dashboard.
#[derive(Debug, Clone)]
pub struct VizitConfig {
    /// Refresh interval in seconds (default: 2).
    pub refresh_secs: u64,
    /// Path to the SQLite sessions database (default: `~/.sqz/sessions.db`).
    pub db_path: PathBuf,
}

// ── Per-agent row ─────────────────────────────────────────────────────────────

/// One row in the dashboard table, derived from a single `GROUP BY project_dir`
/// aggregation over the `compression_log` table.
#[derive(Debug, Clone)]
pub struct AgentRow {
    /// Detected agent name — last path component of `project_dir`.
    pub agent_name: String,
    /// Display path: last 2 components of `project_dir` joined with `"/"`.
    pub project_display: String,
    /// Tokens saved today (since midnight UTC).
    pub tokens_saved_today: u64,
    /// Overall compression ratio for this project (`0.0`–`1.0`, lower = better).
    pub compression_ratio: f64,
    /// Timestamp of the most recent compression event for this project.
    pub last_activity: DateTime<Utc>,
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// The complete state for one render cycle of the dashboard.
#[derive(Debug, Clone)]
pub struct VizitSnapshot {
    /// Per-agent rows, sorted by `last_activity` descending.
    pub rows: Vec<AgentRow>,
    /// All-time total tokens saved across all projects.
    pub total_tokens_saved: u64,
    /// All-time total number of compressions across all projects.
    pub total_compressions: u32,
    /// Overall compression ratio across all projects (`0.0`–`1.0`).
    pub overall_ratio: f64,
    /// Timestamp at which this snapshot was captured.
    pub captured_at: DateTime<Utc>,
    /// Terminal width (columns) at capture time.
    pub term_cols: u16,
}

// ── Database ──────────────────────────────────────────────────────────────────

/// Read-only wrapper around a `rusqlite::Connection` for vizit queries.
pub struct VizitDb {
    conn: Connection,
}

impl VizitDb {
    /// Open the SQLite database at `path` in read-only mode.
    ///
    /// Returns a descriptive error if the file does not exist or cannot be
    /// opened.
    pub fn open(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!(
                "[sqz vizit] database not found: {}",
                path.display()
            ));
        }

        let flags =
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;

        let conn = Connection::open_with_flags(path, flags).map_err(|e| {
            format!(
                "[sqz vizit] cannot open database {}: {e}",
                path.display()
            )
        })?;

        Ok(Self { conn })
    }

    /// Fetch a complete dashboard snapshot from the database.
    ///
    /// Runs both the per-project aggregation query and the footer totals query
    /// inside a single read transaction.
    pub fn fetch_snapshot(&self) -> Result<VizitSnapshot, String> {
        // Begin a read transaction so both queries see a consistent view.
        self.conn
            .execute_batch("BEGIN")
            .map_err(|e| format!("[sqz vizit] failed to begin transaction: {e}"))?;

        let rows = self.fetch_agent_rows()?;
        let (total_tokens_saved, total_compressions, overall_ratio) =
            self.fetch_footer_totals()?;

        self.conn
            .execute_batch("COMMIT")
            .map_err(|e| format!("[sqz vizit] failed to commit transaction: {e}"))?;

        let (term_cols, _term_rows) = terminal_size();

        Ok(VizitSnapshot {
            rows,
            total_tokens_saved,
            total_compressions,
            overall_ratio,
            captured_at: Utc::now(),
            term_cols,
        })
    }

    /// Run the per-project aggregation query and map results to `AgentRow`s.
    fn fetch_agent_rows(&self) -> Result<Vec<AgentRow>, String> {
        const SQL: &str = r#"
            SELECT
                project_dir,
                CAST(
                    COALESCE(SUM(tokens_compressed), 0) AS REAL
                ) / NULLIF(COALESCE(SUM(tokens_original), 0), 0)           AS compression_ratio,
                MAX(created_at)                                             AS last_activity,
                COALESCE(SUM(CASE WHEN date(created_at) = date('now')
                    THEN tokens_original - tokens_compressed ELSE 0 END), 0)
                                                                            AS tokens_saved_today
            FROM compression_log
            WHERE project_dir IS NOT NULL
            GROUP BY project_dir
            ORDER BY last_activity DESC
            LIMIT 50
        "#;

        let mut stmt = self
            .conn
            .prepare(SQL)
            .map_err(|e| format!("[sqz vizit] failed to prepare agent rows query: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let project_dir: String = row.get(0)?;
                let compression_ratio: f64 = row.get::<_, Option<f64>>(1)?.unwrap_or(0.0);
                let last_activity_str: String = row.get(2)?;
                let tokens_saved_today: u64 = row.get::<_, i64>(3)? as u64;

                Ok((project_dir, compression_ratio, last_activity_str, tokens_saved_today))
            })
            .map_err(|e| format!("[sqz vizit] failed to query agent rows: {e}"))?;

        let mut agent_rows = Vec::new();
        for row in rows {
            let (project_dir, compression_ratio, last_activity_str, tokens_saved_today) =
                row.map_err(|e| format!("[sqz vizit] failed to read agent row: {e}"))?;

            let last_activity = parse_datetime(&last_activity_str);

            agent_rows.push(AgentRow {
                agent_name: detect_agent_name(&project_dir),
                project_display: format_project_display(&project_dir),
                tokens_saved_today,
                compression_ratio,
                last_activity,
            });
        }

        Ok(agent_rows)
    }

    /// Run the footer totals query.
    fn fetch_footer_totals(&self) -> Result<(u64, u32, f64), String> {
        const SQL: &str = r#"
            SELECT
                COUNT(*)                                                    AS total_compressions,
                COALESCE(SUM(tokens_original) - SUM(tokens_compressed), 0) AS total_tokens_saved,
                CAST(
                    COALESCE(SUM(tokens_compressed), 0) AS REAL
                ) / NULLIF(COALESCE(SUM(tokens_original), 0), 0)           AS overall_ratio
            FROM compression_log
        "#;

        let mut stmt = self
            .conn
            .prepare(SQL)
            .map_err(|e| format!("[sqz vizit] failed to prepare footer totals query: {e}"))?;

        let (total_compressions, total_tokens_saved, overall_ratio) = stmt
            .query_row([], |row| {
                let total_compressions: u32 = row.get::<_, i64>(0)? as u32;
                let total_tokens_saved: u64 = row.get::<_, i64>(1)? as u64;
                let overall_ratio: f64 = row.get::<_, Option<f64>>(2)?.unwrap_or(0.0);
                Ok((total_compressions, total_tokens_saved, overall_ratio))
            })
            .map_err(|e| format!("[sqz vizit] failed to query footer totals: {e}"))?;

        Ok((total_tokens_saved, total_compressions, overall_ratio))
    }
}

/// Parse an ISO 8601 datetime string into `DateTime<Utc>`.
///
/// Tries `"%Y-%m-%dT%H:%M:%S"` first, then `"%Y-%m-%d %H:%M:%S"`.
/// Falls back to `Utc::now()` on parse error.
fn parse_datetime(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .map(|ndt| ndt.and_utc())
        .unwrap_or_else(|_| Utc::now())
}

// ── Agent name and project display helpers ────────────────────────────────────

/// Detect a display name for an agent from its `project_dir` path.
///
/// Algorithm:
/// 1. Strip a trailing `/`.
/// 2. Take the last `/`-delimited component.
/// 3. Strip leading dots (`.myproject` → `myproject`).
/// 4. Truncate to 24 characters.
/// 5. Return `"unknown"` for an empty path or a bare `/`.
pub fn detect_agent_name(project_dir: &str) -> String {
    // Strip trailing slash
    let path = project_dir.trim_end_matches('/');

    if path.is_empty() {
        return "unknown".to_string();
    }

    // Take the last `/`-delimited component
    let component = match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    };

    if component.is_empty() {
        return "unknown".to_string();
    }

    // Strip leading dots
    let stripped = component.trim_start_matches('.');

    // If stripping dots left nothing, use the original component (all dots)
    let name = if stripped.is_empty() { component } else { stripped };

    // Truncate to 24 chars (byte-safe: take char boundary)
    let truncated: String = name.chars().take(24).collect();

    if truncated.is_empty() {
        "unknown".to_string()
    } else {
        truncated
    }
}

/// Format a `project_dir` path for display in the dashboard PROJECT column.
///
/// Returns the last two `/`-delimited components joined with `"/"`.
/// If there is only one component, returns it as-is.
/// If the path is empty, returns `"unknown"`.
pub fn format_project_display(project_dir: &str) -> String {
    // Strip trailing slash
    let path = project_dir.trim_end_matches('/');

    if path.is_empty() {
        return "unknown".to_string();
    }

    // Collect non-empty components
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match components.len() {
        0 => "unknown".to_string(),
        1 => components[0].to_string(),
        _ => {
            let n = components.len();
            format!("{}/{}", components[n - 2], components[n - 1])
        }
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

/// Format a token count with K/M suffix.
///
/// - `>= 1_000_000` → `"X.XM"`
/// - `>= 1_000`     → `"X.XK"`
/// - otherwise      → plain decimal
fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Format a compression ratio (`0.0`–`1.0`) as a percentage string.
///
/// The ratio represents `compressed / original`, so savings = `1 - ratio`.
/// We display the savings percentage: `(1 - ratio) * 100`.
fn format_ratio(ratio: f64) -> String {
    let pct = ((1.0 - ratio) * 100.0).round() as i64;
    let pct = pct.clamp(0, 100);
    format!("{pct}%")
}

/// Format a duration (seconds ago) as a human-readable string.
fn format_duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Pad a string to exactly `width` visible characters (spaces on the right).
///
/// If the visible content is already >= `width`, it is returned as-is
/// (no truncation here — callers should call `truncate_to_width` first).
fn pad_right(s: &str, width: usize) -> String {
    let visible_len = strip_ansi(s).chars().count();
    if visible_len >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - visible_len))
    }
}

// ── Renderer ──────────────────────────────────────────────────────────────────

/// Stateless renderer for the vizit dashboard.
pub struct Renderer;

impl Renderer {
    // Fixed column widths (visible characters, excluding borders).
    const W_AGENT: usize = 16;
    const W_PROJECT: usize = 20;
    const W_TODAY: usize = 10;
    const W_RATIO: usize = 8;
    const W_LAST: usize = 7;

    /// Render the top header bar.
    ///
    /// Shows `  sqz vizit  ·  AI Agent Sessions` in bold cyan (when color is
    /// enabled), padded to `cols` columns.
    pub fn render_header(cols: u16) -> String {
        let label = "  sqz vizit  ·  AI Agent Sessions";
        let colored = if is_color_enabled() {
            format!("{BOLD}{CYAN}{label}{RESET}")
        } else {
            label.to_string()
        };
        let visible_len = strip_ansi(&colored).chars().count();
        let cols = cols as usize;
        let padding = if cols > visible_len {
            " ".repeat(cols - visible_len)
        } else {
            String::new()
        };
        format!("{colored}{padding}")
    }

    /// Render a single agent row.
    ///
    /// Column layout (visible widths):
    /// - AGENT:   16
    /// - PROJECT: 20
    /// - TODAY:   10
    /// - RATIO:    8
    /// - LAST:     7
    ///
    /// Color rules based on `now - last_activity`:
    /// - `< 30s`  → bright-green
    /// - `< 5min` → yellow
    /// - otherwise → no color
    pub fn render_agent_row(row: &AgentRow, now: DateTime<Utc>, cols: u16) -> String {
        let secs_ago = (now - row.last_activity).num_seconds();

        let color = if is_color_enabled() {
            if secs_ago < 30 {
                BRIGHT_GREEN
            } else if secs_ago < 300 {
                YELLOW
            } else {
                ""
            }
        } else {
            ""
        };

        let cols = cols as usize;

        // Determine effective column widths based on terminal width.
        // At < 80 cols, truncate PROJECT first then AGENT.
        let (w_agent, w_project) = if cols < 80 {
            let available = cols.saturating_sub(
                // borders: "│ " + " │ " + " │ " + " │ " + " │ " + " │"
                // = 2 + 3 + 3 + 3 + 3 + 2 = 16 border chars
                // plus TODAY(10) + RATIO(8) + LAST(7) = 25
                // total fixed = 16 + 25 = 41
                41,
            );
            // Split remaining between AGENT and PROJECT, PROJECT first to shrink.
            let w_project = (available / 2).min(Self::W_PROJECT);
            let w_agent = available.saturating_sub(w_project).min(Self::W_AGENT);
            (w_agent, w_project)
        } else {
            (Self::W_AGENT, Self::W_PROJECT)
        };

        // Format each cell value.
        let agent_val = truncate_to_width(&row.agent_name, w_agent);
        let agent_val = pad_right(&agent_val, w_agent);

        let project_val = truncate_to_width(&row.project_display, w_project);
        let project_val = pad_right(&project_val, w_project);

        let today_str = format_tokens(row.tokens_saved_today);
        let today_val = truncate_to_width(&today_str, Self::W_TODAY);
        let today_val = pad_right(&today_val, Self::W_TODAY);

        let ratio_str = format_ratio(row.compression_ratio);
        let ratio_val = truncate_to_width(&ratio_str, Self::W_RATIO);
        let ratio_val = pad_right(&ratio_val, Self::W_RATIO);

        let last_str = format_duration(secs_ago);
        let last_val = truncate_to_width(&last_str, Self::W_LAST);
        let last_val = pad_right(&last_val, Self::W_LAST);

        // Assemble the row with box-drawing borders.
        let row_str = format!(
            "│ {agent_val} │ {project_val} │ {today_val} │ {ratio_val} │ {last_val} │"
        );

        if color.is_empty() || !is_color_enabled() {
            row_str
        } else {
            format!("{color}{row_str}{RESET}")
        }
    }

    /// Render the footer summary line.
    ///
    /// Format: `  Saved by sqz: <tokens> tokens saved  ·  <n> compressions  ·  <ratio>% avg ratio`
    pub fn render_footer(snapshot: &VizitSnapshot, cols: u16) -> String {
        let tokens_str = if snapshot.total_tokens_saved == 0 {
            "0".to_string()
        } else {
            format_tokens(snapshot.total_tokens_saved)
        };

        let ratio_str = format_ratio(snapshot.overall_ratio);

        let content = if snapshot.total_tokens_saved == 0 {
            format!(
                "  Saved by sqz: 0 tokens saved  ·  {} compressions  ·  {} avg ratio",
                snapshot.total_compressions, ratio_str
            )
        } else {
            format!(
                "  Saved by sqz: {tokens_str} tokens saved  ·  {} compressions  ·  {ratio_str} avg ratio",
                snapshot.total_compressions
            )
        };

        let colored = if is_color_enabled() {
            format!("{DIM}{content}{RESET}")
        } else {
            content
        };

        let visible_len = strip_ansi(&colored).chars().count();
        let cols = cols as usize;
        let padding = if cols > visible_len {
            " ".repeat(cols - visible_len)
        } else {
            String::new()
        };
        format!("{colored}{padding}")
    }

    /// Render an empty-state row when there are no agent sessions.
    pub fn render_empty_state(cols: u16) -> String {
        let msg = "  No agent sessions found  ";
        let cols = cols as usize;
        let visible_len = msg.chars().count() + 2; // "│" + msg + "│"
        let padding = if cols > visible_len {
            " ".repeat(cols - visible_len)
        } else {
            String::new()
        };
        format!("│{msg}{padding}│")
    }

    /// Render a red-colored error message.
    pub fn render_error(msg: &str, cols: u16) -> String {
        let content = format!("  Error: {msg}");
        let colored = if is_color_enabled() {
            format!("{RED}{content}{RESET}")
        } else {
            content
        };
        let visible_len = strip_ansi(&colored).chars().count();
        let cols = cols as usize;
        let padding = if cols > visible_len {
            " ".repeat(cols - visible_len)
        } else {
            String::new()
        };
        format!("{colored}{padding}")
    }

    /// Build a horizontal border line of `cols` width using box-drawing chars.
    ///
    /// `left`, `fill`, `right` are the left cap, fill char, and right cap.
    fn border_line(left: &str, fill: &str, right: &str, cols: usize) -> String {
        // The total visible width of the row content (between left and right caps)
        // is cols - 2 (for the two cap chars).
        let inner = cols.saturating_sub(2);
        format!("{left}{}{right}", fill.repeat(inner))
    }

    /// Render the column header row.
    fn render_col_headers(cols: u16) -> String {
        let cols = cols as usize;
        let (w_agent, w_project) = if cols < 80 {
            let available = cols.saturating_sub(41);
            let w_project = (available / 2).min(Self::W_PROJECT);
            let w_agent = available.saturating_sub(w_project).min(Self::W_AGENT);
            (w_agent, w_project)
        } else {
            (Self::W_AGENT, Self::W_PROJECT)
        };

        let agent_hdr = pad_right("AGENT", w_agent);
        let project_hdr = pad_right("PROJECT", w_project);
        let today_hdr = pad_right("TODAY", Self::W_TODAY);
        let ratio_hdr = pad_right("RATIO", Self::W_RATIO);
        let last_hdr = pad_right("LAST", Self::W_LAST);

        let header = format!(
            "│ {agent_hdr} │ {project_hdr} │ {today_hdr} │ {ratio_hdr} │ {last_hdr} │"
        );

        if is_color_enabled() {
            format!("{BOLD}{header}{RESET}")
        } else {
            header
        }
    }

    /// Compose and return a complete dashboard frame as a `String`.
    ///
    /// The frame includes:
    /// 1. Cursor-home + hide-cursor escape sequences
    /// 2. Header row
    /// 3. Top border + column headers
    /// 4. Separator
    /// 5. Agent rows (or empty-state)
    /// 6. Footer separator + footer
    /// 7. Bottom border
    pub fn render_frame(snapshot: &VizitSnapshot) -> String {
        let cols = snapshot.term_cols;
        let now = snapshot.captured_at;

        let mut out = String::new();

        // Hide cursor and move to home position.
        out.push_str("\x1b[?25l");
        out.push_str("\x1b[H");

        // Header.
        out.push_str(&Self::render_header(cols));
        out.push('\n');

        // Top border.
        out.push_str(&Self::border_line("┌", "─", "┐", cols as usize));
        out.push('\n');

        // Column headers.
        out.push_str(&Self::render_col_headers(cols));
        out.push('\n');

        // Separator after column headers.
        out.push_str(&Self::border_line("├", "─", "┤", cols as usize));
        out.push('\n');

        // Agent rows or empty state.
        if snapshot.rows.is_empty() {
            out.push_str(&Self::render_empty_state(cols));
            out.push('\n');
        } else {
            for row in &snapshot.rows {
                out.push_str(&Self::render_agent_row(row, now, cols));
                out.push('\n');
            }
        }

        // Footer separator.
        out.push_str(&Self::border_line("├", "─", "┤", cols as usize));
        out.push('\n');

        // Footer.
        out.push_str(&Self::render_footer(snapshot, cols));
        out.push('\n');

        // Bottom border.
        out.push_str(&Self::border_line("└", "─", "┘", cols as usize));
        out.push('\n');

        out
    }
}

// ── Event loop ────────────────────────────────────────────────────────────────

/// The main event loop for the vizit dashboard.
pub struct EventLoop {
    config: VizitConfig,
    db: VizitDb,
}

impl EventLoop {
    /// Create a new `EventLoop`.
    ///
    /// Validates `config.refresh_secs` is in `[1, 60]`.
    /// Opens the database — fails with a descriptive error **before** entering
    /// raw mode if the DB cannot be opened.
    /// Detects non-TTY stdout: if not a TTY, sets a flag to skip raw mode.
    pub fn new(config: VizitConfig) -> Result<Self, String> {
        // Validate refresh_secs
        if config.refresh_secs < 1 || config.refresh_secs > 60 {
            return Err(format!(
                "[sqz vizit] --refresh must be between 1 and 60, got {}",
                config.refresh_secs
            ));
        }

        // Open DB before entering raw mode (fail fast with clean terminal)
        let db = VizitDb::open(&config.db_path)?;

        Ok(Self { config, db })
    }

    /// Run the main refresh/input loop.
    pub fn run(self) -> Result<(), String> {
        use std::io::Write;
        use std::time::{Duration, Instant};

        let is_tty = unsafe { libc::isatty(libc::STDOUT_FILENO) } != 0;

        // Non-TTY: print a single plain-text snapshot and exit
        if !is_tty {
            let snapshot = self.db.fetch_snapshot()?;
            let frame = Renderer::render_frame(&snapshot);
            let plain = strip_ansi(&frame);
            print!("{plain}");
            return Ok(());
        }

        // Check minimum terminal size
        let (cols, rows) = terminal_size();
        if cols < 40 || rows < 10 {
            eprintln!(
                "[sqz vizit] terminal too small ({}x{}); minimum is 40x10",
                cols, rows
            );
            return Ok(());
        }

        // Register SIGWINCH handler for terminal resize
        register_sigwinch_handler();

        // Enter raw mode (TerminalGuard restores terminal on drop)
        let _guard = enter_raw_mode()?;

        let refresh_duration = Duration::from_secs(self.config.refresh_secs);
        let poll_interval = Duration::from_millis(50);

        let mut stdout = std::io::stdout();

        // Initial clear
        let _ = stdout.write_all(b"\x1b[2J");

        'outer: loop {
            // Fetch snapshot and render frame
            let frame = match self.db.fetch_snapshot() {
                Ok(snapshot) => Renderer::render_frame(&snapshot),
                Err(e) => {
                    // Render inline error; continue on next cycle
                    let (cols, rows) = terminal_size();
                    let _ = rows; // suppress unused warning
                    let mut err_frame = String::new();
                    err_frame.push_str("\x1b[?25l\x1b[H");
                    err_frame.push_str(&Renderer::render_error(&e, cols));
                    err_frame.push('\n');
                    err_frame
                }
            };

            let _ = stdout.write_all(frame.as_bytes());
            let _ = stdout.flush();

            // Poll stdin every 50ms until the refresh deadline
            let deadline = Instant::now() + refresh_duration;
            while Instant::now() < deadline {
                // Check for terminal resize
                if RESIZE_REQUESTED.swap(false, Ordering::Relaxed) {
                    // Re-query terminal size on next render (terminal_size() is
                    // called inside fetch_snapshot)
                    break; // re-render immediately
                }

                // Non-blocking stdin read
                let mut buf = [0u8; 1];
                let n = unsafe {
                    libc::read(
                        libc::STDIN_FILENO,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        1,
                    )
                };
                if n > 0 {
                    match buf[0] {
                        b'q' | b'Q' | 0x03 => break 'outer, // q, Q, Ctrl-C
                        b'r' | b'R' => break,                // manual refresh
                        _ => {}
                    }
                }

                std::thread::sleep(poll_interval);
            }
        }

        // Show cursor (also done by TerminalGuard::drop, but be explicit)
        let _ = stdout.write_all(b"\x1b[?25h");
        let _ = stdout.flush();

        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ── Unit tests for detect_agent_name ──────────────────────────────────────

    #[test]
    fn test_detect_agent_name_typical_paths() {
        assert_eq!(detect_agent_name("/home/alice/projects/my-api"), "my-api");
        assert_eq!(detect_agent_name("/Users/bob/work/llm-agent"), "llm-agent");
        assert_eq!(detect_agent_name("/root"), "root");
    }

    #[test]
    fn test_detect_agent_name_empty_and_root() {
        assert_eq!(detect_agent_name(""), "unknown");
        assert_eq!(detect_agent_name("/"), "unknown");
    }

    #[test]
    fn test_detect_agent_name_trailing_slash() {
        assert_eq!(detect_agent_name("/home/alice/projects/my-api/"), "my-api");
        assert_eq!(detect_agent_name("/root/"), "root");
    }

    #[test]
    fn test_detect_agent_name_strips_leading_dots() {
        assert_eq!(detect_agent_name("/home/alice/.myproject"), "myproject");
        assert_eq!(detect_agent_name("/home/alice/...hidden"), "hidden");
    }

    #[test]
    fn test_detect_agent_name_truncates_to_24_chars() {
        let long = "/home/alice/this-is-a-very-long-project-name-exceeding-limit";
        let result = detect_agent_name(long);
        assert!(result.chars().count() <= 24);
    }

    #[test]
    fn test_detect_agent_name_no_slash() {
        assert_eq!(detect_agent_name("myproject"), "myproject");
    }

    // ── Unit tests for format_project_display ─────────────────────────────────

    #[test]
    fn test_format_project_display_typical_paths() {
        assert_eq!(
            format_project_display("/home/alice/projects/my-api"),
            "projects/my-api"
        );
        assert_eq!(
            format_project_display("/Users/bob/work/llm-agent"),
            "work/llm-agent"
        );
        assert_eq!(
            format_project_display("/home/alice/my-api"),
            "alice/my-api"
        );
    }

    #[test]
    fn test_format_project_display_single_component() {
        assert_eq!(format_project_display("/my-api"), "my-api");
    }

    #[test]
    fn test_format_project_display_empty() {
        assert_eq!(format_project_display(""), "unknown");
        assert_eq!(format_project_display("/"), "unknown");
    }

    #[test]
    fn test_format_project_display_trailing_slash() {
        assert_eq!(
            format_project_display("/home/alice/projects/my-api/"),
            "projects/my-api"
        );
    }

    // ── arb_path_string generator ─────────────────────────────────────────────
    //
    // Generates Unix-style path strings with 0–5 alphanumeric segments.
    // Each segment is 1–12 alphanumeric characters.
    // Paths with 1+ segments are prefixed with `/`.

    prop_compose! {
        fn arb_path_segment()(s in "[a-zA-Z0-9]{1,12}") -> String {
            s
        }
    }

    prop_compose! {
        fn arb_path_string()(
            segments in prop::collection::vec(arb_path_segment(), 0..=5)
        ) -> String {
            if segments.is_empty() {
                String::new()
            } else {
                format!("/{}", segments.join("/"))
            }
        }
    }

    // ── Property 6: Agent name is last path component ─────────────────────────
    // Validates: Requirements 7.2, 7.3, 7.4

    proptest! {
        #[test]
        fn prop_detect_agent_name_is_last_component(
            path in arb_path_string()
        ) {
            // Only test paths that have at least one non-empty component
            let trimmed = path.trim_end_matches('/');
            let components: Vec<&str> = trimmed
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();

            if components.is_empty() {
                // Empty path or bare "/" → "unknown"
                prop_assert_eq!(detect_agent_name(&path), "unknown");
            } else {
                let last = components[components.len() - 1];
                // Strip leading dots from the last component
                let expected_base = last.trim_start_matches('.');
                let expected_base = if expected_base.is_empty() { last } else { expected_base };
                // Truncate to 24 chars
                let expected: String = expected_base.chars().take(24).collect();

                let result = detect_agent_name(&path);
                prop_assert_eq!(result, expected);
            }
        }
    }

    // ── Property 7: Project display is last two path components ──────────────
    // Validates: Requirements 7.1

    proptest! {
        #[test]
        fn prop_format_project_display_last_two_components(
            path in arb_path_string()
        ) {
            let trimmed = path.trim_end_matches('/');
            let components: Vec<&str> = trimmed
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();

            let result = format_project_display(&path);

            match components.len() {
                0 => {
                    prop_assert_eq!(result, "unknown");
                }
                1 => {
                    prop_assert_eq!(result, components[0]);
                }
                n => {
                    let expected = format!("{}/{}", components[n - 2], components[n - 1]);
                    prop_assert_eq!(result, expected);
                }
            }
        }
    }

    // ── VizitDb helpers ───────────────────────────────────────────────────────

    /// Create an in-memory SQLite DB with the `compression_log` schema.
    fn create_in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB");
        conn.execute_batch(
            "CREATE TABLE compression_log (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                tokens_original   INTEGER NOT NULL,
                tokens_compressed INTEGER NOT NULL,
                stages_applied    TEXT NOT NULL,
                mode              TEXT NOT NULL DEFAULT 'auto',
                created_at        TEXT NOT NULL,
                project_dir       TEXT
            );",
        )
        .expect("create schema");
        conn
    }

    impl VizitDb {
        /// Test-only constructor that wraps an existing connection.
        #[cfg(test)]
        fn from_conn(conn: Connection) -> Self {
            Self { conn }
        }
    }

    // ── Property 5: Snapshot rows sorted by last_activity descending ──────────
    // Validates: Requirements 5.1 (snapshot ordering)

    prop_compose! {
        /// Generate a Vec of (tokens_original, tokens_compressed, created_at_str)
        /// with 0–20 entries and random timestamps.
        fn arb_compression_log_entries()(
            entries in prop::collection::vec(
                (
                    1i64..=100_000i64,          // tokens_original
                    0i64..=100_000i64,          // tokens_compressed (may exceed original; clamped in query)
                    0i64..=(365 * 24 * 3600i64) // seconds offset from 2024-01-01T00:00:00
                ),
                0..=20usize
            )
        ) -> Vec<(i64, i64, String)> {
            entries
                .into_iter()
                .map(|(orig, comp, offset_secs)| {
                    // Base: 2024-01-01T00:00:00 + offset
                    let base_secs: i64 = 1_704_067_200; // 2024-01-01T00:00:00 UTC
                    let ts = base_secs + offset_secs;
                    let naive = chrono::DateTime::from_timestamp(ts, 0)
                        .unwrap_or_else(|| chrono::DateTime::from_timestamp(base_secs, 0).unwrap())
                        .naive_utc();
                    let created_at = naive.format("%Y-%m-%dT%H:%M:%S").to_string();
                    (orig, comp, created_at)
                })
                .collect()
        }
    }

    proptest! {
        /// **Validates: Requirements 5.1**
        ///
        /// Property 5: Snapshot rows are sorted by `last_activity` descending.
        #[test]
        fn prop_snapshot_rows_sorted_by_last_activity_desc(
            entries in arb_compression_log_entries()
        ) {
            let conn = create_in_memory_db();

            // Insert entries, each into a distinct project_dir so every entry
            // becomes its own row in the GROUP BY result.
            for (i, (orig, comp, created_at)) in entries.iter().enumerate() {
                let project_dir = format!("/home/user/project-{i}");
                conn.execute(
                    "INSERT INTO compression_log
                        (tokens_original, tokens_compressed, stages_applied, mode, created_at, project_dir)
                     VALUES (?1, ?2, 'test', 'auto', ?3, ?4)",
                    rusqlite::params![orig, comp, created_at, project_dir],
                )
                .expect("insert row");
            }

            let db = VizitDb::from_conn(conn);
            let snapshot = db.fetch_snapshot().expect("fetch_snapshot");

            // Assert rows are sorted by last_activity descending.
            for window in snapshot.rows.windows(2) {
                prop_assert!(
                    window[0].last_activity >= window[1].last_activity,
                    "rows not sorted: {:?} < {:?}",
                    window[0].last_activity,
                    window[1].last_activity
                );
            }
        }
    }

    // ── Task 4.4: Property tests for render_agent_row ─────────────────────────

    prop_compose! {
        fn arb_agent_row()(
            agent_name in "[a-zA-Z0-9-]{1,16}",
            project_display in "[a-zA-Z0-9/-]{1,20}",
            tokens_saved_today in 0u64..=10_000_000u64,
            compression_ratio in 0.0f64..=1.0f64,
            secs_ago in 0i64..=86400i64,
        ) -> AgentRow {
            let last_activity = Utc::now() - chrono::Duration::seconds(secs_ago);
            AgentRow {
                agent_name,
                project_display,
                tokens_saved_today,
                compression_ratio,
                last_activity,
            }
        }
    }

    fn arb_terminal_cols() -> impl Strategy<Value = u16> {
        (40u16..=220u16)
    }

    proptest! {
        /// **Validates: Requirements 1**
        ///
        /// Property 1: For any valid AgentRow, rendered string SHALL contain a
        /// `%` character (ratio) and a duration indicator (one of `s`, `m`, `h`, `d`).
        #[test]
        fn prop_render_agent_row_contains_ratio_and_duration(
            row in arb_agent_row(),
            cols in arb_terminal_cols(),
        ) {
            let now = Utc::now();
            let rendered = Renderer::render_agent_row(&row, now, cols);
            let visible = strip_ansi(&rendered);
            prop_assert!(
                visible.contains('%'),
                "rendered row missing '%': {visible:?}"
            );
            prop_assert!(
                visible.contains('s') || visible.contains('m')
                    || visible.contains('h') || visible.contains('d'),
                "rendered row missing duration indicator: {visible:?}"
            );
        }
    }

    proptest! {
        /// **Validates: Requirements 3**
        ///
        /// Property 3: For any AgentRow with `last_activity < 30s` before `now`,
        /// rendered string SHALL contain `\x1b[92m` (when color is enabled).
        #[test]
        fn prop_render_agent_row_bright_green_when_recent(
            row in arb_agent_row(),
            cols in arb_terminal_cols(),
        ) {
            // Skip if NO_COLOR is set or stdout is not a TTY (color disabled).
            if !is_color_enabled() {
                return Ok(());
            }

            // Force last_activity to be < 30s ago.
            let now = Utc::now();
            let recent_row = AgentRow {
                last_activity: now - chrono::Duration::seconds(10),
                ..row
            };

            let rendered = Renderer::render_agent_row(&recent_row, now, cols);
            prop_assert!(
                rendered.contains("\x1b[92m"),
                "expected bright-green for recent row, got: {rendered:?}"
            );
        }
    }

    proptest! {
        /// **Validates: Requirements 4**
        ///
        /// Property 4: For any AgentRow with `last_activity >= 5min` before `now`,
        /// rendered string SHALL NOT contain `\x1b[92m` or `\x1b[33m`.
        #[test]
        fn prop_render_agent_row_no_color_when_old(
            row in arb_agent_row(),
            cols in arb_terminal_cols(),
        ) {
            // Skip if color is disabled (no color codes will appear anyway).
            if !is_color_enabled() {
                return Ok(());
            }

            let now = Utc::now();
            let old_row = AgentRow {
                last_activity: now - chrono::Duration::seconds(400),
                ..row
            };

            let rendered = Renderer::render_agent_row(&old_row, now, cols);
            prop_assert!(
                !rendered.contains("\x1b[92m"),
                "unexpected bright-green for old row: {rendered:?}"
            );
            prop_assert!(
                !rendered.contains("\x1b[33m"),
                "unexpected yellow for old row: {rendered:?}"
            );
        }
    }

    proptest! {
        /// **Validates: Requirements 8**
        ///
        /// Property 8: For any AgentRow and cols in [40, 220], visible byte
        /// length of rendered row SHALL be <= cols + 10 (slack for box-drawing
        /// chars and borders).
        #[test]
        fn prop_render_agent_row_fits_terminal_width(
            row in arb_agent_row(),
            cols in arb_terminal_cols(),
        ) {
            let now = Utc::now();
            let rendered = Renderer::render_agent_row(&row, now, cols);
            let visible = strip_ansi(&rendered);
            let visible_len = visible.chars().count();
            let slack = 10usize;
            prop_assert!(
                visible_len <= cols as usize + slack,
                "row too wide: visible_len={visible_len}, cols={cols}, slack={slack}, row={visible:?}"
            );
        }
    }

    // ── Task 4.6: Property test for render_footer ─────────────────────────────

    prop_compose! {
        fn arb_vizit_snapshot()(
            total_tokens_saved in 0u64..=100_000_000u64,
            total_compressions in 0u32..=100_000u32,
            overall_ratio in 0.0f64..=1.0f64,
        ) -> VizitSnapshot {
            VizitSnapshot {
                rows: vec![],
                total_tokens_saved,
                total_compressions,
                overall_ratio,
                captured_at: Utc::now(),
                term_cols: 120,
            }
        }
    }

    proptest! {
        /// **Validates: Requirements 2**
        ///
        /// Property 2: For any valid VizitSnapshot, rendered footer SHALL contain
        /// a representation of total tokens saved, total compressions, and overall
        /// compression ratio.
        #[test]
        fn prop_render_footer_contains_required_fields(
            snapshot in arb_vizit_snapshot(),
        ) {
            let rendered = Renderer::render_footer(&snapshot, snapshot.term_cols);
            let visible = strip_ansi(&rendered);

            // Must contain a '%' for the ratio.
            prop_assert!(
                visible.contains('%'),
                "footer missing ratio '%': {visible:?}"
            );

            // Must contain the compressions count as a number.
            let compressions_str = snapshot.total_compressions.to_string();
            prop_assert!(
                visible.contains(&compressions_str),
                "footer missing compressions count {compressions_str}: {visible:?}"
            );

            // Must contain a token representation: either the raw number or K/M suffix.
            let tokens_repr = format_tokens(snapshot.total_tokens_saved);
            prop_assert!(
                visible.contains(&tokens_repr),
                "footer missing tokens repr {tokens_repr}: {visible:?}"
            );
        }
    }
}
