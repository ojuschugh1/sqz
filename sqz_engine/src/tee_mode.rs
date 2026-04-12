/// Tee Mode — saves full uncompressed command output for later recovery.
///
/// Configurable as `always`, `failures` (non-zero exit), or `never` (default).
/// Saved outputs are timestamped files in a configurable directory.
///
/// Requirements: 38.1, 38.2, 38.3, 38.4

use crate::{Result, SqzError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── TeeMode enum ─────────────────────────────────────────────────────────

/// Controls when uncompressed output is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeeMode {
    /// Save every command's uncompressed output.
    Always,
    /// Save only when the command exits with a non-zero code.
    Failures,
    /// Disabled (default).
    Never,
}

impl Default for TeeMode {
    fn default() -> Self {
        Self::Never
    }
}

impl std::fmt::Display for TeeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Always => write!(f, "always"),
            Self::Failures => write!(f, "failures"),
            Self::Never => write!(f, "never"),
        }
    }
}

impl std::str::FromStr for TeeMode {
    type Err = SqzError;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "always" => Ok(Self::Always),
            "failures" => Ok(Self::Failures),
            "never" => Ok(Self::Never),
            other => Err(SqzError::Other(format!(
                "invalid tee mode '{other}': expected always, failures, or never"
            ))),
        }
    }
}

// ── TeeEntry ─────────────────────────────────────────────────────────────

/// Metadata for a single saved tee output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeeEntry {
    /// Unique identifier (the filename stem, e.g. "20250715T103045Z_git-status").
    pub id: String,
    /// The command that produced the output.
    pub command: String,
    /// Process exit code.
    pub exit_code: i32,
    /// Timestamp when the output was saved (RFC 3339).
    pub timestamp: String,
    /// Byte length of the saved output.
    pub size_bytes: u64,
}

// ── TeeManager ───────────────────────────────────────────────────────────

/// Manages saving and retrieving uncompressed command outputs.
pub struct TeeManager {
    mode: TeeMode,
    dir: PathBuf,
}

impl TeeManager {
    /// Create a new `TeeManager`.
    ///
    /// * `mode` — when to save (`Always`, `Failures`, `Never`).
    /// * `dir`  — directory for saved files (created on first write).
    pub fn new(mode: TeeMode, dir: PathBuf) -> Self {
        Self { mode, dir }
    }

    /// Create a `TeeManager` with the default directory (`~/.sqz/tee/`).
    pub fn with_default_dir(mode: TeeMode) -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Self::new(mode, home.join(".sqz").join("tee"))
    }

    /// Return the active tee mode.
    pub fn mode(&self) -> TeeMode {
        self.mode
    }

    /// Return the output directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Conditionally save uncompressed output based on the configured mode.
    ///
    /// Returns `Some(id)` when the output was saved, `None` when skipped.
    pub fn save(&self, command: &str, output: &str, exit_code: i32) -> Result<Option<String>> {
        let should_save = match self.mode {
            TeeMode::Always => true,
            TeeMode::Failures => exit_code != 0,
            TeeMode::Never => false,
        };

        if !should_save {
            return Ok(None);
        }

        // Ensure directory exists.
        std::fs::create_dir_all(&self.dir)?;

        let now = Utc::now();
        let ts = now.format("%Y%m%dT%H%M%SZ").to_string();
        let sanitised_cmd = sanitise_command(command);
        let id = format!("{ts}_{sanitised_cmd}");

        // Write the output file.
        let output_path = self.dir.join(format!("{id}.txt"));
        std::fs::write(&output_path, output)?;

        // Write a small metadata sidecar.
        let meta = TeeEntry {
            id: id.clone(),
            command: command.to_owned(),
            exit_code,
            timestamp: now.to_rfc3339(),
            size_bytes: output.len() as u64,
        };
        let meta_path = self.dir.join(format!("{id}.json"));
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(&meta_path, meta_json)?;

        Ok(Some(id))
    }

    /// List all saved tee entries, most recent first.
    pub fn list(&self) -> Result<Vec<TeeEntry>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                match std::fs::read_to_string(&path) {
                    Ok(json) => {
                        if let Ok(te) = serde_json::from_str::<TeeEntry>(&json) {
                            entries.push(te);
                        }
                    }
                    Err(_) => continue, // skip unreadable files
                }
            }
        }

        // Sort newest first (lexicographic on id works because of the timestamp prefix).
        entries.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(entries)
    }

    /// Retrieve the full saved output for a given id.
    pub fn get(&self, id: &str) -> Result<String> {
        let path = self.dir.join(format!("{id}.txt"));
        if !path.exists() {
            return Err(SqzError::Other(format!("tee entry '{id}' not found")));
        }
        Ok(std::fs::read_to_string(&path)?)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Turn a command string into a short, filesystem-safe slug.
fn sanitise_command(cmd: &str) -> String {
    let base = cmd
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join("-");
    base.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .take(60)
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_manager(mode: TeeMode) -> (TeeManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let mgr = TeeManager::new(mode, tmp.path().to_path_buf());
        (mgr, tmp)
    }

    // ── TeeMode basics ──────────────────────────────────────────────────

    #[test]
    fn default_mode_is_never() {
        assert_eq!(TeeMode::default(), TeeMode::Never);
    }

    #[test]
    fn parse_mode_round_trip() {
        for mode in [TeeMode::Always, TeeMode::Failures, TeeMode::Never] {
            let s = mode.to_string();
            let parsed: TeeMode = s.parse().unwrap();
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn parse_mode_case_insensitive() {
        assert_eq!("ALWAYS".parse::<TeeMode>().unwrap(), TeeMode::Always);
        assert_eq!("Failures".parse::<TeeMode>().unwrap(), TeeMode::Failures);
    }

    #[test]
    fn parse_mode_invalid() {
        assert!("bogus".parse::<TeeMode>().is_err());
    }

    // ── TeeManager::save ────────────────────────────────────────────────

    #[test]
    fn save_never_skips() {
        let (mgr, _tmp) = make_manager(TeeMode::Never);
        let result = mgr.save("git status", "output", 0).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn save_always_saves_on_success() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        let id = mgr.save("git status", "hello", 0).unwrap();
        assert!(id.is_some());
        let content = mgr.get(id.as_ref().unwrap()).unwrap();
        assert_eq!(content, "hello");
    }

    #[test]
    fn save_always_saves_on_failure() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        let id = mgr.save("cargo build", "error!", 1).unwrap();
        assert!(id.is_some());
    }

    #[test]
    fn save_failures_skips_success() {
        let (mgr, _tmp) = make_manager(TeeMode::Failures);
        let result = mgr.save("ls", "files", 0).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn save_failures_saves_on_nonzero() {
        let (mgr, _tmp) = make_manager(TeeMode::Failures);
        let id = mgr.save("cargo test", "FAILED", 101).unwrap();
        assert!(id.is_some());
        let content = mgr.get(id.as_ref().unwrap()).unwrap();
        assert_eq!(content, "FAILED");
    }

    // ── TeeManager::list ────────────────────────────────────────────────

    #[test]
    fn list_empty_dir() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        let entries = mgr.list().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn list_returns_saved_entries() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        mgr.save("cmd1", "out1", 0).unwrap();
        mgr.save("cmd2", "out2", 1).unwrap();
        let entries = mgr.list().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn list_sorted_newest_first() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        let id1 = mgr.save("first", "a", 0).unwrap().unwrap();
        // Tiny sleep to ensure different timestamps (filenames).
        std::thread::sleep(std::time::Duration::from_millis(10));
        let id2 = mgr.save("second", "b", 0).unwrap().unwrap();
        let entries = mgr.list().unwrap();
        assert_eq!(entries[0].id, id2);
        assert_eq!(entries[1].id, id1);
    }

    // ── TeeManager::get ─────────────────────────────────────────────────

    #[test]
    fn get_nonexistent_returns_error() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        assert!(mgr.get("no-such-id").is_err());
    }

    #[test]
    fn get_returns_exact_content() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        let body = "line1\nline2\nline3\n";
        let id = mgr.save("echo", body, 0).unwrap().unwrap();
        assert_eq!(mgr.get(&id).unwrap(), body);
    }

    // ── TeeEntry metadata ───────────────────────────────────────────────

    #[test]
    fn entry_metadata_correct() {
        let (mgr, _tmp) = make_manager(TeeMode::Always);
        let output = "some output";
        mgr.save("git diff", output, 42).unwrap();
        let entries = mgr.list().unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.command, "git diff");
        assert_eq!(e.exit_code, 42);
        assert_eq!(e.size_bytes, output.len() as u64);
        assert!(e.id.contains("git-diff"));
    }

    // ── sanitise_command ────────────────────────────────────────────────

    #[test]
    fn sanitise_strips_special_chars() {
        let slug = sanitise_command("git log --oneline | head");
        assert!(slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn sanitise_truncates_long_commands() {
        let long_cmd = "a".repeat(200);
        let slug = sanitise_command(&long_cmd);
        assert!(slug.len() <= 60);
    }
}
