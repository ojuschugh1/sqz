//! Claude Code `CLAUDE.md` prompt-level guidance installer.
//!
//! The PreToolUse hook (`.claude/settings.local.json`) only intercepts
//! `Bash`-tool commands. Claude Code's built-in `Read`/`Grep`/`Glob`
//! tools bypass shell hooks entirely — confirmed in
//! github.com/anthropics/claude-code/issues/4544. That means a heavy
//! session (file reads, searches, directory listings) can pass through
//! Claude Code without sqz ever seeing a single byte.
//!
//! Reported in issue #12 by @JCKodel:
//!
//! > Running claude on a long session don't show anything in the
//! > dashboard […] In practice, both Cursor and claude cli shows 0.
//!
//! This module installs prompt-level guidance into the project's
//! `CLAUDE.md` file. Claude Code reads `CLAUDE.md` at session start,
//! so the agent sees the instructions on every turn.
//!
//! The guidance block:
//!   * recommends `sqz_read_file` / `sqz_grep` / `sqz_list_dir` MCP
//!     tools (registered via `sqz init`'s MCP config merge) for any
//!     file I/O larger than ~2KB or that might be repeated;
//!   * keeps the built-in `Read` available for tiny configs and
//!     byte-exact needs (lockfiles, signatures);
//!   * documents the `§ref:HASH§` escape hatch so the agent doesn't
//!     thrash on a token it can't parse.
//!
//! Same install/uninstall semantics as
//! `codex_integration::install_agents_md_guidance`:
//!   * If `CLAUDE.md` doesn't exist → create it with sqz's block.
//!   * If it exists without the sqz block → append with a blank-line
//!     separator.
//!   * If it exists with the sqz block → no-op (idempotent).
//!
//! The block is wrapped in HTML comment sentinels so `uninstall` can
//! excise it byte-exact.

use std::path::{Path, PathBuf};

use crate::error::{Result, SqzError};

// ── Sentinels (must match `remove_claude_md_guidance` byte-exact) ────────

const CLAUDE_MD_BEGIN: &str =
    "<!-- BEGIN sqz-claude-guidance (auto-installed by sqz init; remove this block to disable) -->";
const CLAUDE_MD_END: &str = "<!-- END sqz-claude-guidance -->";

/// Compute the path where `CLAUDE.md` lives for the given project.
///
/// Claude Code reads `CLAUDE.md` from the project root (not from
/// `.claude/`). This matches Anthropic's documented convention
/// (docs.anthropic.com/en/docs/claude-code/memory).
pub fn claude_md_path(project_dir: &Path) -> PathBuf {
    project_dir.join("CLAUDE.md")
}

/// Compute the path of the user-level `~/.claude.json` config.
///
/// This is where Claude Code stores its `mcpServers` map. Registering
/// sqz-mcp here makes it available to every Claude Code session on
/// this machine. Matches what @JCKodel had to configure manually in
/// issue #12 — sqz init should do this for them.
///
/// Returns `None` if the home directory cannot be resolved (very
/// unusual — typically only in sandboxes or broken environments).
pub fn claude_user_json_path() -> Option<PathBuf> {
    // Honour $HOME (standard on Unix, set explicitly on Windows when
    // Claude Code is installed in the user-scope convention).
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".claude.json"))
}

/// Build the markdown block that sqz appends to `CLAUDE.md`.
///
/// `sqz_path` is the absolute path of the sqz binary; we use it
/// verbatim in the markdown fence blocks so Windows backslash paths
/// render correctly and the user can copy-paste commands unchanged.
pub fn claude_md_guidance_block(sqz_path: &str) -> String {
    format!(
        "\
{begin}

## sqz — Context Compression (READ FIRST)

sqz is installed in this project. It compresses tool output so large
files, long logs, and verbose command output cost far fewer tokens.
There are **two ways** sqz is wired in, and you should prefer each
one in the situations below.

### Preferred tools (MCP)

The `sqz-mcp` server is registered in this project's MCP config. It
exposes three read-only tools that compress their output through the
sqz pipeline:

- **`sqz_read_file`** — read a file from disk and return a compressed
  view. **PREFER this over the built-in `Read` tool** for any file
  larger than ~2KB or any file you might read more than once in the
  same session. Repeat reads return a 13-token `§ref:HASH§` reference
  instead of the full content.

- **`sqz_grep`** — search files for a literal string or regex.
  **PREFER this over the built-in `Grep`** for anything that might
  match more than a handful of lines. Caps at 200 matches by default;
  raise with `max_matches` if needed.

- **`sqz_list_dir`** — list a directory. Skips `.git`, `node_modules`,
  `target`, `dist`, `build`, `vendor`, `__pycache__` so the output
  stays focused. **PREFER this over `ls -la` via Bash** when you want
  to see a project layout.

The built-in `Read`, `Grep`, `Glob` tools remain available. Use them for:
- Tiny config files (<1KB) where compression can't help.
- Byte-exact reads you'll hash or diff (lockfiles, signatures).
- Globbing (sqz has no glob tool; `Glob` is still the right choice).

### Bash commands (hooked automatically)

When you run a shell command through the `Bash` tool, a PreToolUse hook
rewrites it to pipe output through `sqz compress`. This is transparent:
you don't need to remember to add anything, but it's useful to know
that these commands get compressed automatically:

```bash
git status           # → git status 2>&1 | sqz compress --cmd git
cargo test           # → cargo test 2>&1 | sqz compress --cmd cargo
docker ps            # → docker ps 2>&1 | sqz compress --cmd docker
kubectl get pods     # → kubectl get pods 2>&1 | sqz compress --cmd kubectl
```

The rewrite is skipped for interactive commands (`vim`, `ssh`,
`python`), compound commands (`a && b`, `a > file.txt`), and anything
already going through sqz.

### Escape hatch — when you see a `§ref:HASH§` token

If tool output contains a `§ref:a1b2c3d4§` token and you need the full
content it points at, resolve it. Three equivalent ways:

- Shell: `{sqz_path} expand a1b2c3d4` (or paste the whole token
  `{sqz_path} expand §ref:a1b2c3d4§`).
- MCP tool: call `expand` with `{{ \"prefix\": \"a1b2c3d4\" }}`.
- To get uncompressed output for one command: prefix it with
  `SQZ_NO_DEDUP=1` (e.g. `SQZ_NO_DEDUP=1 git log | sqz compress`).

If the compressed output is actively making the task harder (looping
on refs, small retries replacing one big read), call the `passthrough`
MCP tool to get raw text.

### When NOT to use sqz tools

- Writing or editing files — use the built-in `Write`/`Edit` tools.
  sqz has no write tools (by design; see issue #5 follow-up).
- Running commands interactively or in watch mode.
- Reading very small files (<1KB) where compression can't help.

{end}
",
        begin = CLAUDE_MD_BEGIN,
        end = CLAUDE_MD_END,
    )
}

/// Return `true` if the given `CLAUDE.md` content already contains sqz's
/// guidance block (matched by the BEGIN sentinel).
fn claude_md_has_sqz_block(content: &str) -> bool {
    content.contains(CLAUDE_MD_BEGIN)
}

/// Install sqz's guidance block into `CLAUDE.md` at `project_dir`.
///
/// If `CLAUDE.md` doesn't exist yet, create it with a minimal preamble
/// and sqz's block. If it exists, append sqz's block (separated by a
/// blank line so it renders as a new markdown section). If the block
/// is already present (detected by the BEGIN sentinel), return
/// `Ok(false)` without touching the file — `sqz init` stays
/// idempotent.
///
/// Returns `true` when the file was created or modified, `false` when
/// sqz's block was already present.
pub fn install_claude_md_guidance(project_dir: &Path, sqz_path: &str) -> Result<bool> {
    let path = claude_md_path(project_dir);
    let block = claude_md_guidance_block(sqz_path);

    if path.exists() {
        let existing = std::fs::read_to_string(&path).map_err(|e| {
            SqzError::Other(format!("failed to read {}: {e}", path.display()))
        })?;
        if claude_md_has_sqz_block(&existing) {
            return Ok(false);
        }
        // Append with a guaranteed blank-line separator so sqz's
        // section doesn't accidentally fuse with a trailing user
        // section.
        let mut new_content = existing;
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        if !new_content.ends_with("\n\n") {
            new_content.push('\n');
        }
        new_content.push_str(&block);
        std::fs::write(&path, new_content).map_err(|e| {
            SqzError::Other(format!("failed to write {}: {e}", path.display()))
        })?;
        return Ok(true);
    }

    // Fresh CLAUDE.md — add a tiny preamble so the file is
    // self-explanatory to users who encounter it for the first time.
    let mut content = String::from(
        "# CLAUDE.md\n\
         \n\
         Project-level instructions for [Claude Code](https://docs.anthropic.com/en/docs/claude-code).\n\
         \n",
    );
    content.push_str(&block);
    std::fs::write(&path, content).map_err(|e| {
        SqzError::Other(format!("failed to write {}: {e}", path.display()))
    })?;
    Ok(true)
}

/// Remove sqz's guidance block from `CLAUDE.md` if present.
///
/// Locates the block by its BEGIN/END sentinels and excises the
/// entire range including one preceding blank line (if any) so the
/// remaining file reads cleanly. If `CLAUDE.md` becomes empty (or
/// contains only the sqz preamble) after removal, deletes the file.
///
/// Returns the path plus a flag indicating whether anything changed.
/// Missing file or missing block → `(path, false)`.
pub fn remove_claude_md_guidance(
    project_dir: &Path,
) -> Result<Option<(PathBuf, bool)>> {
    let path = claude_md_path(project_dir);
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path).map_err(|e| {
        SqzError::Other(format!("failed to read {}: {e}", path.display()))
    })?;

    let begin_idx = match content.find(CLAUDE_MD_BEGIN) {
        Some(i) => i,
        None => return Ok(Some((path, false))),
    };
    let after_end_idx = match content.find(CLAUDE_MD_END) {
        Some(i) => i + CLAUDE_MD_END.len(),
        None => {
            // BEGIN without END — file was edited by hand. Leave it
            // alone rather than risk mutilating user content.
            return Ok(Some((path, false)));
        }
    };

    // Extend the removed range backwards to include one preceding
    // blank line if present, so we don't leave a trailing `\n\n`
    // that looks accidental.
    let mut start = begin_idx;
    let bytes = content.as_bytes();
    while start > 0 && (bytes[start - 1] == b'\n') {
        start -= 1;
        if start > 0 && bytes[start - 1] != b'\n' {
            break;
        }
    }
    // And forwards to include the trailing newline on the END line.
    let mut end = after_end_idx;
    while end < content.len() && content.as_bytes()[end] == b'\n' {
        end += 1;
        break; // Exactly one trailing newline.
    }

    let mut new_content = String::with_capacity(content.len() - (end - start));
    new_content.push_str(&content[..start]);
    new_content.push_str(&content[end..]);

    // If nothing meaningful remains (just our preamble or whitespace),
    // delete the file — don't leave an empty marker behind.
    let remaining_trimmed = new_content.trim();
    let is_only_preamble = remaining_trimmed
        == "# CLAUDE.md\n\n\
            Project-level instructions for [Claude Code](https://docs.anthropic.com/en/docs/claude-code)."
            .trim()
        || remaining_trimmed.is_empty();

    if is_only_preamble {
        std::fs::remove_file(&path).map_err(|e| {
            SqzError::Other(format!("failed to delete {}: {e}", path.display()))
        })?;
    } else {
        std::fs::write(&path, new_content).map_err(|e| {
            SqzError::Other(format!("failed to write {}: {e}", path.display()))
        })?;
    }

    Ok(Some((path, true)))
}

// ── MCP server registration in ~/.claude.json ────────────────────────────
//
// Claude Code reads its `mcpServers` map from `~/.claude.json`. Adding
// sqz-mcp here makes all three new tools (`sqz_read_file`, `sqz_grep`,
// `sqz_list_dir`) plus `compress`/`passthrough`/`expand` available in
// every Claude Code session on this machine.
//
// Reported in issue #12: @JCKodel had to add this entry by hand. `sqz
// init` should do it automatically so the average user doesn't have to
// know the config path or the exact JSON shape.

/// JSON sentinel we stamp into our `mcpServers.sqz` entry so we can
/// distinguish sqz-installed entries from user-edited ones on upgrade
/// or uninstall.
const SQZ_MCP_SENTINEL_KEY: &str = "_sqz_managed";

/// Register sqz-mcp as an MCP server in `~/.claude.json`.
///
/// Idempotent: if an `sqz` entry already exists and points at
/// `sqz-mcp --transport stdio`, we leave it alone and return
/// `Ok(false)`. If the entry exists but diverges (user edited the
/// command args), we also leave it alone — the user's customisation
/// wins. Only fresh installs write a new entry.
///
/// Returns:
///   * `Ok(true)` when a new entry was added.
///   * `Ok(false)` when an entry already existed (either sqz-managed
///     or user-customised).
///   * `Err(_)` on JSON parse error or write failure.
pub fn install_claude_mcp_config() -> Result<bool> {
    install_claude_mcp_config_at(None)
}

/// Internal: accept a home-dir override so tests can point at a tempdir
/// without touching `env::set_var("HOME")` — that mutates process-wide
/// state and races with parallel tests that also read HOME (e.g. the
/// api_proxy property tests that open `~/.sqz/sessions.db`).
pub(crate) fn install_claude_mcp_config_at(home_override: Option<&Path>) -> Result<bool> {
    let path = match home_override {
        Some(h) => h.join(".claude.json"),
        None => claude_user_json_path().ok_or_else(|| {
            SqzError::Other(
                "cannot resolve $HOME — ~/.claude.json location unknown".to_string(),
            )
        })?,
    };

    // Parent directory for new installs. On most systems $HOME already
    // exists; this is a safety net for sandboxed test environments.
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let mut root: serde_json::Value = if path.exists() {
        let text = std::fs::read_to_string(&path).map_err(|e| {
            SqzError::Other(format!("failed to read {}: {e}", path.display()))
        })?;
        if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&text).map_err(|e| {
                SqzError::Other(format!(
                    "~/.claude.json exists but is not valid JSON: {e}"
                ))
            })?
        }
    } else {
        serde_json::json!({})
    };

    // Ensure root is an object (Claude Code's config is always an
    // object; anything else is user corruption we shouldn't rewrite).
    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| SqzError::Other(
            "~/.claude.json root must be a JSON object".to_string(),
        ))?;

    // Ensure mcpServers is an object.
    let mcp = root_obj
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !mcp.is_object() {
        *mcp = serde_json::json!({});
    }
    let mcp_obj = mcp
        .as_object_mut()
        .expect("just ensured mcp is an object");

    // Already present? Don't overwrite — the user may have customised
    // the command or args. Respect that by leaving the entry alone
    // regardless of whether it matches our defaults.
    if mcp_obj.contains_key("sqz") {
        return Ok(false);
    }

    mcp_obj.insert(
        "sqz".to_string(),
        serde_json::json!({
            "command": "sqz-mcp",
            "args": ["--transport", "stdio"],
            SQZ_MCP_SENTINEL_KEY: true
        }),
    );

    // Write back with a two-space indent (matches Claude Code's own
    // formatting so diffs stay clean).
    let out = serde_json::to_string_pretty(&root).map_err(|e| {
        SqzError::Other(format!("failed to serialize ~/.claude.json: {e}"))
    })?;
    std::fs::write(&path, out).map_err(|e| {
        SqzError::Other(format!("failed to write {}: {e}", path.display()))
    })?;
    Ok(true)
}

/// Remove sqz's `mcpServers.sqz` entry from `~/.claude.json` if we
/// installed it (detected by the `_sqz_managed` sentinel). Leaves
/// user-customised entries alone.
///
/// Returns `Ok(Some((path, true)))` when the entry was removed,
/// `Ok(Some((path, false)))` when present but not managed by sqz,
/// `Ok(None)` when the file doesn't exist, `Err(_)` on read/write
/// failure.
pub fn remove_claude_mcp_config() -> Result<Option<(PathBuf, bool)>> {
    remove_claude_mcp_config_at(None)
}

/// Internal: home-dir-injectable counterpart used by tests. See
/// `install_claude_mcp_config_at` for rationale.
pub(crate) fn remove_claude_mcp_config_at(
    home_override: Option<&Path>,
) -> Result<Option<(PathBuf, bool)>> {
    let path = match home_override {
        Some(h) => h.join(".claude.json"),
        None => match claude_user_json_path() {
            Some(p) => p,
            None => return Ok(None),
        },
    };
    if !path.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&path).map_err(|e| {
        SqzError::Other(format!("failed to read {}: {e}", path.display()))
    })?;
    let mut root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            // Corrupted/non-JSON file — don't touch it.
            return Ok(Some((path, false)));
        }
    };

    let changed = {
        let Some(root_obj) = root.as_object_mut() else {
            return Ok(Some((path, false)));
        };
        let Some(mcp) = root_obj.get_mut("mcpServers").and_then(|v| v.as_object_mut())
        else {
            return Ok(Some((path, false)));
        };
        let is_managed = mcp
            .get("sqz")
            .and_then(|v| v.get(SQZ_MCP_SENTINEL_KEY))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !is_managed {
            return Ok(Some((path, false)));
        }
        mcp.remove("sqz").is_some()
    };

    if changed {
        let out = serde_json::to_string_pretty(&root).map_err(|e| {
            SqzError::Other(format!("failed to serialize ~/.claude.json: {e}"))
        })?;
        std::fs::write(&path, out).map_err(|e| {
            SqzError::Other(format!("failed to write {}: {e}", path.display()))
        })?;
    }

    Ok(Some((path, changed)))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn guidance_block_contains_tool_recommendations() {
        let block = claude_md_guidance_block("/usr/local/bin/sqz");
        assert!(block.contains(CLAUDE_MD_BEGIN));
        assert!(block.contains(CLAUDE_MD_END));
        // The agent must see explicit "PREFER this over Read" instructions
        // for each of the three new MCP tools.
        assert!(
            block.contains("PREFER this over the built-in `Read` tool"),
            "guidance must tell the agent when to prefer sqz_read_file"
        );
        assert!(block.contains("sqz_read_file"));
        assert!(block.contains("sqz_grep"));
        assert!(block.contains("sqz_list_dir"));
        // Escape hatch must be documented so the agent doesn't thrash
        // on §ref§ tokens.
        assert!(block.contains("§ref:"));
        assert!(block.contains("/usr/local/bin/sqz expand"));
    }

    #[test]
    fn install_creates_new_claude_md() {
        let dir = TempDir::new().unwrap();
        let changed =
            install_claude_md_guidance(dir.path(), "/usr/local/bin/sqz").unwrap();
        assert!(changed);
        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.starts_with("# CLAUDE.md"));
        assert!(content.contains(CLAUDE_MD_BEGIN));
        assert!(content.contains(CLAUDE_MD_END));
    }

    #[test]
    fn install_appends_to_existing_claude_md() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(
            &path,
            "# My project rules\n\n- Always use 2-space indent\n- Be polite\n",
        )
        .unwrap();

        let changed =
            install_claude_md_guidance(dir.path(), "/usr/local/bin/sqz").unwrap();
        assert!(changed);

        let content = std::fs::read_to_string(&path).unwrap();
        // User's original content must survive unchanged.
        assert!(content.contains("My project rules"));
        assert!(content.contains("Be polite"));
        // sqz's block must come AFTER the user's content.
        let user_idx = content.find("Be polite").unwrap();
        let sqz_idx = content.find(CLAUDE_MD_BEGIN).unwrap();
        assert!(
            sqz_idx > user_idx,
            "sqz block must append after existing content, not prepend"
        );
    }

    #[test]
    fn install_is_idempotent() {
        let dir = TempDir::new().unwrap();
        install_claude_md_guidance(dir.path(), "/usr/local/bin/sqz").unwrap();
        let second = install_claude_md_guidance(dir.path(), "/usr/local/bin/sqz")
            .unwrap();
        assert!(!second, "second install must be a no-op");

        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        let occurrences = content.matches(CLAUDE_MD_BEGIN).count();
        assert_eq!(
            occurrences, 1,
            "re-running sqz init must not duplicate the block"
        );
    }

    #[test]
    fn remove_returns_none_when_file_missing() {
        let dir = TempDir::new().unwrap();
        assert!(remove_claude_md_guidance(dir.path()).unwrap().is_none());
    }

    #[test]
    fn remove_excises_block_and_preserves_user_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(
            &path,
            "# My project rules\n\n- Always use 2-space indent\n- Be polite\n",
        )
        .unwrap();

        install_claude_md_guidance(dir.path(), "/usr/local/bin/sqz").unwrap();
        let (_returned_path, changed) =
            remove_claude_md_guidance(dir.path()).unwrap().unwrap();
        assert!(changed);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(CLAUDE_MD_BEGIN));
        assert!(!content.contains(CLAUDE_MD_END));
        assert!(
            content.contains("My project rules"),
            "user's rules must survive uninstall"
        );
        assert!(content.contains("Be polite"));
    }

    #[test]
    fn remove_deletes_file_if_only_preamble_remains() {
        // Pure sqz install (no user content): uninstall should delete
        // the file rather than leave an empty preamble behind.
        let dir = TempDir::new().unwrap();
        install_claude_md_guidance(dir.path(), "/usr/local/bin/sqz").unwrap();
        let path = dir.path().join("CLAUDE.md");
        assert!(path.exists());

        remove_claude_md_guidance(dir.path()).unwrap();
        assert!(!path.exists(), "pure-sqz CLAUDE.md should be deleted on uninstall");
    }

    #[test]
    fn remove_is_noop_when_block_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(&path, "# User-only file\n").unwrap();

        let (_, changed) = remove_claude_md_guidance(dir.path()).unwrap().unwrap();
        assert!(!changed);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "# User-only file\n", "file must be untouched");
    }

    // ── MCP registration tests ─────────────────────────────────────────
    //
    // These tests use the `_at` variants to inject a tempdir without
    // touching $HOME. The public functions read $HOME directly, but we
    // avoid that in tests because env::set_var is process-wide and
    // races with parallel tests that also read HOME (e.g. the api_proxy
    // property tests open `~/.sqz/sessions.db`).

    #[test]
    fn install_mcp_creates_new_config() {
        let dir = TempDir::new().unwrap();
        let changed = install_claude_mcp_config_at(Some(dir.path())).unwrap();
        assert!(changed);

        let path = dir.path().join(".claude.json");
        let content = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(json["mcpServers"]["sqz"]["command"], "sqz-mcp");
        assert_eq!(
            json["mcpServers"]["sqz"]["args"],
            serde_json::json!(["--transport", "stdio"])
        );
        // Sentinel so uninstall can distinguish sqz-managed from
        // user-edited entries.
        assert_eq!(json["mcpServers"]["sqz"][SQZ_MCP_SENTINEL_KEY], true);
    }

    #[test]
    fn install_mcp_preserves_existing_servers() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        let existing = serde_json::json!({
            "someOtherKey": "preserved",
            "mcpServers": {
                "dart-mcp-server": {
                    "command": "dart",
                    "args": ["mcp-server"],
                    "env": {}
                }
            }
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        install_claude_mcp_config_at(Some(dir.path())).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        // User's existing MCP server survives.
        assert_eq!(json["mcpServers"]["dart-mcp-server"]["command"], "dart");
        // sqz was added alongside it.
        assert_eq!(json["mcpServers"]["sqz"]["command"], "sqz-mcp");
        // Top-level keys unrelated to MCP also survive.
        assert_eq!(json["someOtherKey"], "preserved");
    }

    #[test]
    fn install_mcp_is_idempotent() {
        let dir = TempDir::new().unwrap();
        install_claude_mcp_config_at(Some(dir.path())).unwrap();
        let second = install_claude_mcp_config_at(Some(dir.path())).unwrap();
        assert!(!second, "second install must be a no-op");
    }

    #[test]
    fn install_mcp_preserves_user_customised_entry() {
        // If the user has already configured sqz with their own
        // command/args, we MUST NOT overwrite them. Respecting that
        // is the difference between a helpful install and one that
        // silently breaks user config.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        let user_config = serde_json::json!({
            "mcpServers": {
                "sqz": {
                    "command": "/custom/path/sqz-mcp",
                    "args": ["--verbose"],
                    "env": { "SQZ_PRESET": "aggressive" }
                }
            }
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&user_config).unwrap(),
        )
        .unwrap();

        let changed = install_claude_mcp_config_at(Some(dir.path())).unwrap();
        assert!(!changed, "must not overwrite user-customised entry");

        let content = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        // User's custom command survives.
        assert_eq!(
            json["mcpServers"]["sqz"]["command"],
            "/custom/path/sqz-mcp"
        );
        assert_eq!(
            json["mcpServers"]["sqz"]["args"],
            serde_json::json!(["--verbose"])
        );
    }

    #[test]
    fn remove_mcp_only_removes_sqz_managed_entry() {
        // Complement to the test above: uninstall must leave
        // user-customised entries alone too.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        let user_config = serde_json::json!({
            "mcpServers": {
                "sqz": {
                    "command": "/custom/path/sqz-mcp",
                    "args": ["--verbose"]
                }
            }
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&user_config).unwrap(),
        )
        .unwrap();

        let (_, changed) =
            remove_claude_mcp_config_at(Some(dir.path())).unwrap().unwrap();
        assert!(!changed, "must not remove user-customised entry");

        let content = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            json["mcpServers"]["sqz"]["command"],
            "/custom/path/sqz-mcp",
            "user's custom entry must survive uninstall"
        );
    }

    #[test]
    fn remove_mcp_removes_managed_entry_and_preserves_others() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        let existing = serde_json::json!({
            "mcpServers": {
                "dart-mcp-server": {
                    "command": "dart",
                    "args": ["mcp-server"]
                }
            }
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        install_claude_mcp_config_at(Some(dir.path())).unwrap();
        let (_, changed) =
            remove_claude_mcp_config_at(Some(dir.path())).unwrap().unwrap();
        assert!(changed);

        let content = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(
            json["mcpServers"].get("sqz").is_none(),
            "sqz entry must be removed"
        );
        assert_eq!(
            json["mcpServers"]["dart-mcp-server"]["command"], "dart",
            "other MCP servers must survive"
        );
    }

    #[test]
    fn install_mcp_handles_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(&path, "").unwrap();

        let changed = install_claude_mcp_config_at(Some(dir.path())).unwrap();
        assert!(changed);

        let content = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(json["mcpServers"]["sqz"]["command"], "sqz-mcp");
    }

    #[test]
    fn install_mcp_rejects_non_object_root() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(&path, r#"["not", "an", "object"]"#).unwrap();

        let result = install_claude_mcp_config_at(Some(dir.path()));
        assert!(result.is_err(), "array root must be rejected — corrupted config");
    }
}
