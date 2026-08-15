//! Zed editor (Zed Agent) integration.
//!
//! Requested in issue #38. Zed has no PreToolUse-style hook that could
//! rewrite terminal commands, so the integration has two surfaces,
//! mirroring the Codex integration's shape:
//!
//! 1. **Project instructions** — Zed's Agent reads the first matching
//!    file of `.rules`, `.cursorrules`, `.windsurfrules`, `.clinerules`,
//!    `.github/copilot-instructions.md`, `AGENT.md`, `AGENTS.md`,
//!    `CLAUDE.md`, `GEMINI.md` (per zed.dev/docs/ai/instructions). sqz
//!    appends a sentinel-bracketed guidance block to `.rules` when that
//!    file exists (it shadows everything else), otherwise to `AGENTS.md`
//!    (shared with the Codex integration's file, separate block).
//!
//! 2. **MCP registration** — Zed reads MCP servers from the
//!    `context_servers` map in its `settings.json`
//!    (`~/.config/zed/settings.json` on macOS/Linux, `%APPDATA%\Zed\`
//!    on Windows). Registering `sqz-mcp` gives the Zed Agent the
//!    `compress`/`passthrough`/`expand` tools plus the lossless
//!    `sqz_read_file`/`sqz_grep`/`sqz_list_dir` file tools — which is
//!    the part that matters for local models with small context windows.
//!
//! Zed's `settings.json` is JSONC (Zed's own template ships comments).
//! Lesson learned on issue #6 (OpenCode): never round-trip a user's
//! commented config through serde_json — it silently strips their
//! comments. When the settings file doesn't parse as strict JSON we
//! REFUSE to edit it and return [`ZedMcpInstall::SkippedJsonc`] so the
//! CLI can print copy-paste instructions instead.

use std::path::{Path, PathBuf};

use crate::error::{Result, SqzError};

// ── Paths ─────────────────────────────────────────────────────────────────

/// Path of Zed's user settings file.
///
/// * macOS / Linux: `$XDG_CONFIG_HOME/zed/settings.json`, falling back
///   to `~/.config/zed/settings.json`. Zed uses the XDG location on
///   macOS too (unlike most Mac apps) — see zed.dev/docs/ai/instructions
///   which documents `~/.config/zed/AGENTS.md` for macOS.
/// * Windows: `%APPDATA%\Zed\settings.json`.
pub fn zed_settings_path() -> PathBuf {
    zed_config_dir().join("settings.json")
}

fn zed_config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("Zed");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg).join("zed");
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".config").join("zed")
}

/// The project file Zed will actually read sqz's guidance from.
///
/// Zed uses the FIRST match in its instruction-file list, and `.rules`
/// outranks `AGENTS.md`. So when a `.rules` file exists we must write
/// there — a block appended to `AGENTS.md` would be invisible to Zed.
pub fn zed_guidance_target(project_dir: &Path) -> PathBuf {
    let rules = project_dir.join(".rules");
    if rules.exists() {
        rules
    } else {
        project_dir.join("AGENTS.md")
    }
}

// ── Sentinel-bracketed guidance block ─────────────────────────────────────

const ZED_GUIDANCE_BEGIN: &str =
    "<!-- BEGIN sqz-zed-guidance (auto-installed by sqz init; remove this block to disable) -->";
const ZED_GUIDANCE_END: &str = "<!-- END sqz-zed-guidance -->";

/// Build the markdown block sqz appends to the Zed instruction file.
pub fn zed_guidance_block(sqz_path: &str) -> String {
    format!(
        "\
{begin}

## sqz — Token-Optimized Output (Zed Agent)

When running terminal commands whose output may be long (git log/diff,
test runners, build logs, `ls -la`, `docker ps`), pipe the output
through `sqz compress` to cut token consumption:

```bash
git status 2>&1 | {sqz_path} compress
cargo test 2>&1 | {sqz_path} compress
```

Do NOT pipe interactive commands (REPLs, `ssh`), compound commands with
shell operators (`&&`, `>`, `;`), or output that is already short.

The `sqz` MCP server is registered in Zed's `settings.json` under
`context_servers`. Prefer its tools when they fit:

- `sqz_read_file` / `sqz_grep` / `sqz_list_dir` — file access with
  caching; repeat reads of unchanged content cost ~13 tokens.
- `compress` — compress a blob of text you already have.
- `passthrough` — return text unchanged (escape hatch).
- `expand` — resolve a `§ref:HASH§` token back to the original bytes.

If you see a `§ref:HASH§` token you can't parse, run
`{sqz_path} expand <prefix>` or set `SQZ_NO_DEDUP=1` on the command.
If a `«A1»` symbol replaced a value you need verbatim, re-run with
`SQZ_NO_ABBREV=1` (or `--no-abbrev`).

{end}
",
        begin = ZED_GUIDANCE_BEGIN,
        end = ZED_GUIDANCE_END,
    )
}

// ── Guidance install / uninstall ──────────────────────────────────────────

/// Install sqz's Zed guidance block into the project's effective Zed
/// instruction file (`.rules` when present, else `AGENTS.md`).
///
/// Idempotent: returns `Ok(false)` when the target already contains the
/// block. Appends to existing files rather than clobbering them.
pub fn install_zed_guidance(project_dir: &Path, sqz_path: &str) -> Result<bool> {
    let path = zed_guidance_target(project_dir);
    let block = zed_guidance_block(sqz_path);

    if path.exists() {
        let existing = std::fs::read_to_string(&path).map_err(|e| {
            SqzError::Other(format!("failed to read {}: {e}", path.display()))
        })?;
        if existing.contains(ZED_GUIDANCE_BEGIN) {
            return Ok(false);
        }
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        if !updated.is_empty() {
            updated.push('\n');
        }
        updated.push_str(&block);
        std::fs::write(&path, updated).map_err(|e| {
            SqzError::Other(format!("failed to write {}: {e}", path.display()))
        })?;
    } else {
        std::fs::write(&path, &block).map_err(|e| {
            SqzError::Other(format!("failed to write {}: {e}", path.display()))
        })?;
    }
    Ok(true)
}

/// Remove sqz's Zed guidance block from BOTH `.rules` and `AGENTS.md`
/// (whichever contain it) — the target can have moved between installs
/// if the user created `.rules` later.
///
/// Returns the list of files that were modified. A file left empty
/// (or whitespace-only) after stripping is deleted, matching the
/// AGENTS.md uninstall behaviour of the Codex integration.
pub fn remove_zed_guidance(project_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for candidate in [project_dir.join(".rules"), project_dir.join("AGENTS.md")] {
        if !candidate.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&candidate).map_err(|e| {
            SqzError::Other(format!("failed to read {}: {e}", candidate.display()))
        })?;
        let Some(stripped) = strip_sentinel_block(&content) else {
            continue;
        };
        if stripped.trim().is_empty() {
            std::fs::remove_file(&candidate).map_err(|e| {
                SqzError::Other(format!("failed to remove {}: {e}", candidate.display()))
            })?;
        } else {
            std::fs::write(&candidate, stripped).map_err(|e| {
                SqzError::Other(format!("failed to write {}: {e}", candidate.display()))
            })?;
        }
        changed.push(candidate);
    }
    Ok(changed)
}

/// Excise the sentinel-bracketed block (plus surrounding blank padding).
/// Returns `None` when the content has no block.
fn strip_sentinel_block(content: &str) -> Option<String> {
    let start = content.find(ZED_GUIDANCE_BEGIN)?;
    let end_marker = content[start..].find(ZED_GUIDANCE_END)?;
    let mut end = start + end_marker + ZED_GUIDANCE_END.len();
    // Swallow the trailing newline(s) the block brought with it.
    while content[end..].starts_with('\n') {
        end += 1;
    }
    // And the blank separator line we inserted before the block.
    let mut trimmed_start = start;
    while trimmed_start > 0 && content[..trimmed_start].ends_with('\n') {
        trimmed_start -= 1;
    }
    if trimmed_start < content.len() && content[trimmed_start..].starts_with('\n') {
        trimmed_start += 1; // keep exactly one newline before the cut
    }
    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..trimmed_start]);
    out.push_str(&content[end..]);
    Some(out)
}

// ── MCP registration in Zed's settings.json ──────────────────────────────

/// Outcome of [`install_zed_mcp_config`], surfaced so the CLI can tell
/// the user what actually happened — especially the JSONC case where we
/// deliberately don't touch their file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZedMcpInstall {
    /// `settings.json` did not exist; created it with just our entry.
    Created,
    /// Existing strict-JSON settings; our entry was merged in.
    Merged,
    /// A `context_servers.sqz` entry already exists (ours or the
    /// user's customisation). Left alone.
    AlreadyPresent,
    /// The file exists but is JSONC (comments / trailing commas). We
    /// refuse to rewrite it — that would strip the user's comments
    /// (issue #6 lesson). The CLI should print [`zed_mcp_snippet`]
    /// for manual copy-paste.
    SkippedJsonc,
}

/// The `context_servers` snippet for manual installation, printed when
/// sqz can't safely edit a JSONC settings file.
pub fn zed_mcp_snippet() -> String {
    r#"  "context_servers": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {}
    }
  }"#
    .to_string()
}

/// Register sqz-mcp under `context_servers.sqz` in Zed's settings.json.
///
/// * Missing file → created with just our entry.
/// * Strict-JSON file → surgically merged, all user keys preserved.
/// * Existing `sqz` entry → left alone (user customisation wins),
///   [`ZedMcpInstall::AlreadyPresent`].
/// * JSONC file → untouched, [`ZedMcpInstall::SkippedJsonc`].
pub fn install_zed_mcp_config() -> Result<ZedMcpInstall> {
    install_zed_mcp_config_at(&zed_settings_path())
}

/// Path-injectable counterpart for tests.
pub(crate) fn install_zed_mcp_config_at(path: &Path) -> Result<ZedMcpInstall> {
    let sqz_entry = serde_json::json!({
        "command": "sqz-mcp",
        "args": ["--transport", "stdio"],
        "env": {}
    });

    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                SqzError::Other(format!("create {}: {e}", parent.display()))
            })?;
        }
        let root = serde_json::json!({ "context_servers": { "sqz": sqz_entry } });
        std::fs::write(path, serde_json::to_string_pretty(&root).unwrap_or_default())
            .map_err(|e| SqzError::Other(format!("write {}: {e}", path.display())))?;
        return Ok(ZedMcpInstall::Created);
    }

    let text = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("read {}: {e}", path.display())))?;
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(&text) {
            Ok(v) => v,
            // Zed settings are JSONC more often than not (Zed's own
            // template has comments). Don't strip them — bail out and
            // let the CLI print manual instructions.
            Err(_) => return Ok(ZedMcpInstall::SkippedJsonc),
        }
    };

    let root_obj = root.as_object_mut().ok_or_else(|| {
        SqzError::Other(format!(
            "{} root must be a JSON object",
            path.display()
        ))
    })?;

    let servers = root_obj
        .entry("context_servers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    let servers_obj = servers.as_object_mut().expect("just ensured object");

    if servers_obj.contains_key("sqz") {
        return Ok(ZedMcpInstall::AlreadyPresent);
    }
    servers_obj.insert("sqz".to_string(), sqz_entry);

    std::fs::write(
        path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
    )
    .map_err(|e| SqzError::Other(format!("write {}: {e}", path.display())))?;
    Ok(ZedMcpInstall::Merged)
}

/// Outcome of [`remove_zed_mcp_config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZedMcpRemove {
    /// Entry found and removed.
    Removed,
    /// File missing or no `sqz` entry — nothing to do.
    NotPresent,
    /// JSONC file — refused to rewrite; user must remove the entry by
    /// hand.
    SkippedJsonc,
}

/// Remove `context_servers.sqz` from Zed's settings.json. Never deletes
/// the file itself — it's the user's editor config, and an empty
/// `context_servers` key is simply dropped.
pub fn remove_zed_mcp_config() -> Result<ZedMcpRemove> {
    remove_zed_mcp_config_at(&zed_settings_path())
}

/// Path-injectable counterpart for tests.
pub(crate) fn remove_zed_mcp_config_at(path: &Path) -> Result<ZedMcpRemove> {
    if !path.exists() {
        return Ok(ZedMcpRemove::NotPresent);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("read {}: {e}", path.display())))?;
    let mut root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            // Only report SkippedJsonc when there is actually an sqz
            // entry to remove — otherwise this is a no-op either way.
            if text.contains("\"sqz\"") {
                return Ok(ZedMcpRemove::SkippedJsonc);
            }
            return Ok(ZedMcpRemove::NotPresent);
        }
    };

    let Some(root_obj) = root.as_object_mut() else {
        return Ok(ZedMcpRemove::NotPresent);
    };
    let Some(servers) = root_obj
        .get_mut("context_servers")
        .and_then(|s| s.as_object_mut())
    else {
        return Ok(ZedMcpRemove::NotPresent);
    };
    if servers.remove("sqz").is_none() {
        return Ok(ZedMcpRemove::NotPresent);
    }
    let drop_key = servers.is_empty();
    if drop_key {
        root_obj.remove("context_servers");
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
    )
    .map_err(|e| SqzError::Other(format!("write {}: {e}", path.display())))?;
    Ok(ZedMcpRemove::Removed)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Guidance target selection ────────────────────────────────────

    #[test]
    fn guidance_targets_agents_md_by_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(zed_guidance_target(dir.path()), dir.path().join("AGENTS.md"));
    }

    #[test]
    fn guidance_targets_rules_when_present() {
        // Zed reads `.rules` FIRST — a block in AGENTS.md would be
        // invisible when `.rules` exists.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".rules"), "my rules\n").unwrap();
        assert_eq!(zed_guidance_target(dir.path()), dir.path().join(".rules"));
    }

    // ── Guidance install / remove ────────────────────────────────────

    #[test]
    fn install_creates_agents_md_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_zed_guidance(dir.path(), "/usr/bin/sqz").unwrap());
        let content = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains(ZED_GUIDANCE_BEGIN));
        assert!(content.contains("/usr/bin/sqz compress"));
        assert!(content.contains("context_servers"));
    }

    #[test]
    fn install_appends_to_existing_rules_preserving_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".rules"), "Always use tabs.\n").unwrap();
        assert!(install_zed_guidance(dir.path(), "sqz").unwrap());
        let content = std::fs::read_to_string(dir.path().join(".rules")).unwrap();
        assert!(content.starts_with("Always use tabs.\n"));
        assert!(content.contains(ZED_GUIDANCE_BEGIN));
        // AGENTS.md must NOT be created — .rules shadows it.
        assert!(!dir.path().join("AGENTS.md").exists());
    }

    #[test]
    fn install_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_zed_guidance(dir.path(), "sqz").unwrap());
        assert!(!install_zed_guidance(dir.path(), "sqz").unwrap());
        let content = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert_eq!(content.matches(ZED_GUIDANCE_BEGIN).count(), 1);
    }

    #[test]
    fn remove_strips_block_and_keeps_user_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".rules"), "Always use tabs.\n").unwrap();
        install_zed_guidance(dir.path(), "sqz").unwrap();

        let changed = remove_zed_guidance(dir.path()).unwrap();
        assert_eq!(changed.len(), 1);
        let content = std::fs::read_to_string(dir.path().join(".rules")).unwrap();
        assert_eq!(content, "Always use tabs.\n");
    }

    #[test]
    fn remove_deletes_file_that_was_only_our_block() {
        let dir = tempfile::tempdir().unwrap();
        install_zed_guidance(dir.path(), "sqz").unwrap();
        assert!(dir.path().join("AGENTS.md").exists());

        remove_zed_guidance(dir.path()).unwrap();
        assert!(!dir.path().join("AGENTS.md").exists());
    }

    #[test]
    fn remove_covers_stale_agents_md_after_rules_appears() {
        // Install went to AGENTS.md; user later created .rules and
        // re-init installed there too. Uninstall must strip both.
        let dir = tempfile::tempdir().unwrap();
        install_zed_guidance(dir.path(), "sqz").unwrap();
        std::fs::write(dir.path().join(".rules"), "rules here\n").unwrap();
        install_zed_guidance(dir.path(), "sqz").unwrap();

        let changed = remove_zed_guidance(dir.path()).unwrap();
        assert_eq!(changed.len(), 2, "both .rules and AGENTS.md should be cleaned");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".rules")).unwrap(),
            "rules here\n"
        );
        assert!(!dir.path().join("AGENTS.md").exists());
    }

    // ── MCP config ───────────────────────────────────────────────────

    #[test]
    fn mcp_creates_fresh_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zed").join("settings.json");
        assert_eq!(
            install_zed_mcp_config_at(&path).unwrap(),
            ZedMcpInstall::Created
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["context_servers"]["sqz"]["command"], "sqz-mcp");
        assert_eq!(parsed["context_servers"]["sqz"]["args"][1], "stdio");
    }

    #[test]
    fn mcp_merges_into_strict_json_preserving_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"theme":"One Dark","context_servers":{"github":{"command":"gh-mcp"}}}"#,
        )
        .unwrap();
        assert_eq!(
            install_zed_mcp_config_at(&path).unwrap(),
            ZedMcpInstall::Merged
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["theme"], "One Dark", "user keys preserved");
        assert_eq!(parsed["context_servers"]["github"]["command"], "gh-mcp");
        assert_eq!(parsed["context_servers"]["sqz"]["command"], "sqz-mcp");
    }

    #[test]
    fn mcp_leaves_existing_sqz_entry_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"context_servers":{"sqz":{"command":"/custom/sqz-mcp"}}}"#,
        )
        .unwrap();
        assert_eq!(
            install_zed_mcp_config_at(&path).unwrap(),
            ZedMcpInstall::AlreadyPresent
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed["context_servers"]["sqz"]["command"], "/custom/sqz-mcp",
            "user customisation must win"
        );
    }

    #[test]
    fn mcp_refuses_to_touch_jsonc() {
        // Zed's own settings template ships with comments. Rewriting it
        // through serde_json would strip them — issue #6 taught us not
        // to do that to people.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let jsonc = "{\n  // The user's carefully commented theme choice\n  \"theme\": \"One Dark\"\n}\n";
        std::fs::write(&path, jsonc).unwrap();
        assert_eq!(
            install_zed_mcp_config_at(&path).unwrap(),
            ZedMcpInstall::SkippedJsonc
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            jsonc,
            "JSONC file must be byte-identical after the skipped install"
        );
    }

    #[test]
    fn mcp_remove_is_surgical() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"theme":"One Dark","context_servers":{"sqz":{"command":"sqz-mcp"},"github":{"command":"gh-mcp"}}}"#,
        )
        .unwrap();
        assert_eq!(remove_zed_mcp_config_at(&path).unwrap(), ZedMcpRemove::Removed);
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["theme"], "One Dark");
        assert_eq!(parsed["context_servers"]["github"]["command"], "gh-mcp");
        assert!(parsed["context_servers"].get("sqz").is_none());
    }

    #[test]
    fn mcp_remove_drops_empty_context_servers_but_keeps_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"theme":"One Dark","context_servers":{"sqz":{"command":"sqz-mcp"}}}"#,
        )
        .unwrap();
        assert_eq!(remove_zed_mcp_config_at(&path).unwrap(), ZedMcpRemove::Removed);
        assert!(path.exists(), "never delete the user's settings.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(parsed.get("context_servers").is_none());
        assert_eq!(parsed["theme"], "One Dark");
    }

    #[test]
    fn mcp_remove_jsonc_with_sqz_entry_reports_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let jsonc = "{\n  // comment\n  \"context_servers\": { \"sqz\": { \"command\": \"sqz-mcp\" } }\n}\n";
        std::fs::write(&path, jsonc).unwrap();
        assert_eq!(
            remove_zed_mcp_config_at(&path).unwrap(),
            ZedMcpRemove::SkippedJsonc
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), jsonc);
    }

    #[test]
    fn mcp_remove_missing_file_is_not_present() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            remove_zed_mcp_config_at(&dir.path().join("settings.json")).unwrap(),
            ZedMcpRemove::NotPresent
        );
    }
}
