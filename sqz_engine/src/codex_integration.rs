//! OpenAI Codex integration for sqz.
//!
//! Codex is OpenAI's terminal coding agent (openai/codex). It does not
//! expose a stable per-tool-call hook like Claude Code's PreToolUse — the
//! only event hook documented at the time of writing is `notify`, which
//! fires at the END of a turn (once per user message) and so is useless
//! for compressing tool output before the model sees it. An experimental
//! `features.codex_hooks` flag is mentioned in the config reference but
//! described as "under development; off by default" and the public `hooks.json`
//! schema is not documented enough to target safely. See
//! <https://developers.openai.com/codex/config-reference>.
//!
//! What Codex DOES expose reliably today:
//!
//!   1. **MCP servers** via `~/.codex/config.toml` under
//!      `[mcp_servers.<id>]`. The TOML key is `mcp_servers` (snake_case),
//!      NOT `mcpServers` (camelCase) as in JSON-based tools. See
//!      <https://github.com/openai/codex/blob/main/docs/config.md>.
//!
//!   2. **`AGENTS.md`** — project-level markdown instructions Codex reads
//!      at session start. It's the Codex analogue of `CLAUDE.md` and
//!      `.cursor/rules/*.mdc`. A single `AGENTS.md` is the cross-tool
//!      convention (Codex, GitHub Copilot, Cursor, Windsurf, Amp, Devin
//!      all read it). See <https://agentsmd.io>.
//!
//!   3. **The shell hook** installed by `sqz init` — works transparently
//!      because Codex runs bash via its sandboxed exec tool and sees the
//!      compressed stdout automatically. No Codex-specific wiring needed.
//!
//! This module implements (1) and (2). Guidance-file approach for (2)
//! mirrors how RTK handles the same "no programmatic hook" problem with
//! Codex — see <https://github.com/rtk-ai/rtk/blob/master/hooks/codex/README.md>.

use std::path::{Path, PathBuf};

use crate::error::{Result, SqzError};

// ── Paths ─────────────────────────────────────────────────────────────────

/// Return the location of Codex's user-level `config.toml`.
///
/// Honors `$CODEX_HOME` when set (Codex supports it for isolated installs),
/// falling back to `~/.codex/config.toml`. Matches the behaviour of the
/// Codex CLI itself — see the `sqlite_home` discussion in Codex's
/// `docs/config.md`, which describes the same resolution order.
pub fn codex_config_path() -> PathBuf {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(home).join("config.toml");
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".codex").join("config.toml")
}

/// Return the path where Codex reads project-level instructions.
///
/// `AGENTS.md` lives in the project root and is the cross-tool standard
/// (Codex, Copilot, Cursor). We do NOT fall back to the deprecated
/// `codex.md` / `.codexrules` names — they are pre-consolidation legacy
/// and new installs should target `AGENTS.md`.
pub fn agents_md_path(project_dir: &Path) -> PathBuf {
    project_dir.join("AGENTS.md")
}

// ── Sentinel-bracketed guidance block ────────────────────────────────────
//
// `AGENTS.md` is a free-form markdown file the user may edit freely. sqz
// writes its guidance between BEGIN/END sentinels so we can later locate
// and remove exactly what we added, without clobbering unrelated user
// content. Matches the pattern we already use for shell RC hooks in
// sqz/src/shell_hook.rs.

const AGENTS_MD_BEGIN: &str = "<!-- BEGIN sqz-agents-guidance (auto-installed by sqz init; remove this block to disable) -->";
const AGENTS_MD_END: &str = "<!-- END sqz-agents-guidance -->";

/// Build the markdown block that sqz appends to `AGENTS.md`.
///
/// The block is wrapped in HTML comment sentinels so `remove_agents_md_guidance`
/// can locate and excise it byte-exact on uninstall. `sqz_path` is the
/// absolute path of the sqz binary; we use it verbatim (no JSON escaping)
/// because this is a markdown file the user reads — raw backslashes on
/// Windows render correctly as-is, and the user can copy-paste the
/// commands straight into their shell.
pub fn agents_md_guidance_block(sqz_path: &str) -> String {
    format!(
        "\
{begin}

## sqz — Token-Optimized CLI Output

When running shell commands whose output may be long (directory listings,
git log/diff, test runners, build logs, `docker ps`, `kubectl get`, etc.),
pipe the output through `sqz compress` to reduce token consumption.

`sqz` is a stdin-to-stdout compressor, not a command wrapper. The correct
usage is to pipe the command's output into `sqz compress`:

```bash
# Instead of:     Use:
git status        git status 2>&1 | {sqz_path} compress
cargo test        cargo test 2>&1 | {sqz_path} compress
git log -10       git log -10 2>&1 | {sqz_path} compress
docker ps         docker ps 2>&1 | {sqz_path} compress
ls -la            ls -la 2>&1 | {sqz_path} compress
```

The `2>&1` captures stderr too, which is useful for commands like `cargo
test` where diagnostics go to stderr. `sqz compress` filters and compresses
the combined output while preserving filenames, paths, and identifiers.
It typically saves 60-90% tokens on verbose commands.

Do NOT pipe output for:
- Interactive commands (`vim`, `ssh`, `python`, REPLs)
- Compound commands with shell operators (`cmd && other`, `cmd > file.txt`,
  `cmd; other`) — run those directly
- Short commands whose output is already a few lines

If `sqz` is not on PATH, run commands normally.

The `sqz-mcp` MCP server is also available — Codex reads it from
`~/.codex/config.toml` under `[mcp_servers.sqz]`. It exposes three
tools: `compress` (the default pipeline), `passthrough` (return text
unchanged — the escape hatch below), and `expand` (resolve a
`§ref:HASH§` token back to the original bytes).

## Escape hatch — when sqz output confuses you

If you see a `§ref:HASH§` token and can't parse it, or compressed
output is leading you to make lots of small retries instead of one
big request, use one of these:

- **`{sqz_path} expand <prefix>`** — resolve a dedup ref back to the
  original bytes. Accepts bare hex (`sqz expand a1b2c3d4`) or the full
  token pasted verbatim (`sqz expand §ref:a1b2c3d4§`).
- **`SQZ_NO_DEDUP=1`** — set this env var for one command to disable
  dedup: `SQZ_NO_DEDUP=1 git status 2>&1 | sqz compress`. You'll get
  the full compressed output with no `§ref:…§` tokens.
- **`--no-cache`** — same opt-out as a CLI flag:
  `git status 2>&1 | sqz compress --no-cache`.

If you're using the MCP server, the `passthrough` tool returns raw
text and the `expand` tool resolves refs — call them when you need
data sqz hasn't touched.

{end}
",
        begin = AGENTS_MD_BEGIN,
        end = AGENTS_MD_END,
    )
}

// ── AGENTS.md install/uninstall ──────────────────────────────────────────

/// Return `true` if the given `AGENTS.md` content already contains sqz's
/// guidance block (matched by the BEGIN sentinel).
fn agents_md_has_sqz_block(content: &str) -> bool {
    content.contains(AGENTS_MD_BEGIN)
}

/// Install sqz's guidance block into `AGENTS.md` at `project_dir`.
///
/// If `AGENTS.md` doesn't exist yet, create it with sqz's block as the
/// sole content. If it exists, append sqz's block (separated by a blank
/// line so it renders as a new markdown section). If the block is
/// already present (detected by the BEGIN sentinel), return `Ok(false)`
/// without touching the file — `sqz init` stays idempotent.
///
/// Returns `true` when the file was created or modified, `false` when
/// sqz's block was already present.
pub fn install_agents_md_guidance(project_dir: &Path, sqz_path: &str) -> Result<bool> {
    let path = agents_md_path(project_dir);
    let block = agents_md_guidance_block(sqz_path);

    if path.exists() {
        let existing = std::fs::read_to_string(&path).map_err(|e| {
            SqzError::Other(format!("failed to read {}: {e}", path.display()))
        })?;
        if agents_md_has_sqz_block(&existing) {
            return Ok(false);
        }
        // Append with a guaranteed blank-line separator so sqz's section
        // doesn't accidentally fuse with a trailing user section.
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

    // Fresh AGENTS.md — write just sqz's block. A tiny preamble is added
    // so `AGENTS.md` is self-explanatory to readers who encounter it for
    // the first time and don't know the convention.
    let preamble = "\
# AGENTS.md

Instructions for AI coding agents (OpenAI Codex, GitHub Copilot, Cursor,
Windsurf, Amp, Devin) working in this repository. See <https://agentsmd.io>.

";
    let content = format!("{preamble}{block}");
    std::fs::write(&path, content)
        .map_err(|e| SqzError::Other(format!("failed to create {}: {e}", path.display())))?;
    Ok(true)
}

/// Remove sqz's guidance block from `AGENTS.md`.
///
/// Locates the block by its BEGIN/END sentinels and excises it. If the
/// resulting `AGENTS.md` is empty (modulo whitespace) or contains only
/// the stock preamble sqz itself wrote on first install, the file is
/// deleted entirely — leaving an empty `AGENTS.md` would be noise.
/// If sentinel markers are missing (user edited them out or the file
/// was never installed by sqz), this is a no-op.
///
/// Returns `Ok((path, removed_or_changed))`:
///   - `removed_or_changed = true`  — the file was modified or deleted
///   - `removed_or_changed = false` — no sqz block was found; nothing changed
///
/// Returns `Ok(None)` when `AGENTS.md` does not exist at all.
pub fn remove_agents_md_guidance(project_dir: &Path) -> Result<Option<(PathBuf, bool)>> {
    let path = agents_md_path(project_dir);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| {
        SqzError::Other(format!("failed to read {}: {e}", path.display()))
    })?;
    if !agents_md_has_sqz_block(&content) {
        return Ok(Some((path, false)));
    }

    // Find the slice that contains sqz's block (BEGIN..=END sentinel,
    // plus any immediate surrounding blank line we contributed on install
    // so the remaining file reads cleanly).
    let begin_idx = match content.find(AGENTS_MD_BEGIN) {
        Some(i) => i,
        None => return Ok(Some((path, false))),
    };
    let after_end_idx = match content.find(AGENTS_MD_END) {
        Some(i) => i + AGENTS_MD_END.len(),
        None => {
            // BEGIN without END — truncated/edited file. Preserve the
            // user's file and emit a soft no-op rather than destroying
            // content we can't precisely delimit.
            return Ok(Some((path, false)));
        }
    };

    let mut new_content = String::with_capacity(content.len());
    new_content.push_str(&content[..begin_idx]);
    // Trim any trailing blank-line run that was inserted to separate
    // sqz's block from what preceded it — but keep one trailing newline
    // if the file had content before the block.
    while new_content.ends_with("\n\n\n") {
        new_content.pop();
    }
    let tail = &content[after_end_idx..];
    // Skip the leading newline(s) right after our END sentinel so the
    // trailing user content starts cleanly.
    let trimmed_tail = tail.trim_start_matches('\n');
    if !trimmed_tail.is_empty() {
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(trimmed_tail);
    }

    // If the remaining content is empty (or only the stock preamble sqz
    // wrote on first install), remove the file entirely — a near-empty
    // AGENTS.md is worse than no file.
    let essentially_empty = is_essentially_empty_agents_md(&new_content);
    if essentially_empty {
        std::fs::remove_file(&path).map_err(|e| {
            SqzError::Other(format!("failed to remove {}: {e}", path.display()))
        })?;
        return Ok(Some((path, true)));
    }

    std::fs::write(&path, new_content).map_err(|e| {
        SqzError::Other(format!("failed to write {}: {e}", path.display()))
    })?;
    Ok(Some((path, true)))
}

/// Return `true` when the remaining `AGENTS.md` content has no
/// user-authored material — either empty or just the stock preamble sqz
/// itself wrote on first install. Used by `remove_agents_md_guidance`
/// to decide between "write back the trimmed file" and "delete it".
fn is_essentially_empty_agents_md(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return true;
    }
    // Match the exact preamble sqz writes on first install (with its
    // heading, one paragraph of explanation, and no user additions).
    const PREAMBLE_MARKERS: &[&str] = &[
        "# AGENTS.md",
        "Instructions for AI coding agents",
        "See <https://agentsmd.io>.",
    ];
    let has_only_preamble = PREAMBLE_MARKERS.iter().all(|m| trimmed.contains(m))
        && !trimmed
            .lines()
            .any(|l| {
                let l = l.trim();
                !l.is_empty()
                    && !l.starts_with('#')
                    && !PREAMBLE_MARKERS.iter().any(|m| l.contains(m))
            });
    has_only_preamble
}

// ── ~/.codex/config.toml merger ──────────────────────────────────────────

/// Merge sqz's MCP server entry into Codex's user-level `config.toml`.
///
/// Codex reads MCP servers from `~/.codex/config.toml` (or
/// `$CODEX_HOME/config.toml` when set). The relevant TOML shape is:
///
/// ```toml
/// [mcp_servers.sqz]
/// command = "sqz-mcp"
/// args = ["--transport", "stdio"]
/// ```
///
/// Notes:
///   - Key is `mcp_servers` (snake_case). JSON-tool users who write
///     `mcpServers` will be quietly ignored by Codex.
///   - We go through `toml_edit` so the user's existing comments,
///     key order, and formatting are preserved across the round-trip.
///     Plain `toml::from_str → toml::to_string` wipes comments.
///
/// Idempotent: if `[mcp_servers.sqz]` is already present with a `command`
/// field, we do not overwrite — the user may have tuned it. Returns
/// `Ok(false)` in that case.
///
/// If the config file doesn't exist yet, it is created with only sqz's
/// entry (and its parent `~/.codex/` directory is created on demand).
pub fn install_codex_mcp_config() -> Result<bool> {
    install_codex_mcp_config_at(None)
}

/// Internal: home-dir-injectable counterpart used by tests. Avoids
/// `std::env::set_var` which races with parallel tests that also read
/// HOME (e.g. the api_proxy property tests that open `~/.sqz/sessions.db`).
/// See `claude_md_integration::install_claude_mcp_config_at` for the
/// same pattern.
pub(crate) fn install_codex_mcp_config_at(home_override: Option<&Path>) -> Result<bool> {
    let path = match home_override {
        Some(h) => h.join("config.toml"),
        None => codex_config_path(),
    };

    // Read or start from blank. We build on a toml_edit::DocumentMut so
    // any prior comments/whitespace survive the round-trip.
    let existing = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| {
            SqzError::Other(format!("failed to read {}: {e}", path.display()))
        })?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = existing
        .parse()
        .map_err(|e| SqzError::Other(format!(
            "failed to parse {} as TOML: {e}",
            path.display()
        )))?;

    // Idempotency: if [mcp_servers.sqz].command is already set, skip.
    // We only check `command` (not just the table's presence) because a
    // stub `[mcp_servers.sqz]` with no keys would be a misconfigured
    // install worth repairing.
    if let Some(existing_cmd) = doc
        .get("mcp_servers")
        .and_then(|v| v.get("sqz"))
        .and_then(|v| v.get("command"))
    {
        if existing_cmd.is_value() {
            return Ok(false);
        }
    }

    // Build the sqz table in place. Using `get_mut(..).or_insert_with(..)`
    // pattern so we add keys to any existing [mcp_servers] without
    // replacing the whole table. We deliberately leave the parent
    // `mcp_servers` implicit (i.e. don't emit an explicit
    // `[mcp_servers]` header before the sub-tables). The TOML spec
    // treats `[mcp_servers.sqz]` and `[mcp_servers.jira]` as proper
    // subtables that implicitly create their parent, and emitting a
    // bare `[mcp_servers]` header before them is redundant noise in
    // the file.
    let mcp_servers = doc
        .entry("mcp_servers")
        .or_insert_with(|| {
            // Start the new table as implicit so toml_edit doesn't
            // write `[mcp_servers]` before the first subtable header.
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        })
        .as_table_mut()
        .ok_or_else(|| SqzError::Other(format!(
            "{}: `mcp_servers` is not a table — refusing to overwrite",
            path.display()
        )))?;

    let sqz = mcp_servers
        .entry("sqz")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| SqzError::Other(format!(
            "{}: `mcp_servers.sqz` is not a table — refusing to overwrite",
            path.display()
        )))?;

    // Populate the server's required fields. Only write keys the user
    // hasn't already set so a hand-tuned config survives re-runs.
    if !sqz.contains_key("command") {
        sqz["command"] = toml_edit::value("sqz-mcp");
    }
    if !sqz.contains_key("args") {
        let mut args = toml_edit::Array::new();
        args.push("--transport");
        args.push("stdio");
        sqz["args"] = toml_edit::Item::Value(toml_edit::Value::Array(args));
    }

    // Make sure the parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            SqzError::Other(format!(
                "failed to create {}: {e}",
                parent.display()
            ))
        })?;
    }

    std::fs::write(&path, doc.to_string()).map_err(|e| {
        SqzError::Other(format!("failed to write {}: {e}", path.display()))
    })?;
    Ok(true)
}

/// Remove sqz's MCP entry from Codex's user-level `config.toml`.
///
/// Surgical: only removes `[mcp_servers.sqz]`. Other `[mcp_servers.*]`
/// entries and unrelated keys are preserved. Uses `toml_edit` to keep
/// the user's comments, key order, and formatting intact.
///
/// If removing sqz empties out `[mcp_servers]`, the now-empty table is
/// dropped so the config file doesn't end up with dangling headers.
/// If the file would become empty after the surgery (only sqz's entry
/// existed) we delete it entirely — a zero-byte `config.toml` is just
/// noise.
///
/// Returns `Ok((path, changed))`:
///   - `changed = true`  — sqz's entry was removed or the file deleted
///   - `changed = false` — no sqz entry found; nothing changed
///
/// Returns `Ok(None)` if the config file does not exist at all.
pub fn remove_codex_mcp_config() -> Result<Option<(PathBuf, bool)>> {
    remove_codex_mcp_config_at(None)
}

/// Internal: home-dir-injectable counterpart used by tests. See
/// `install_codex_mcp_config_at` for rationale.
pub(crate) fn remove_codex_mcp_config_at(
    home_override: Option<&Path>,
) -> Result<Option<(PathBuf, bool)>> {
    let path = match home_override {
        Some(h) => h.join("config.toml"),
        None => codex_config_path(),
    };
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| SqzError::Other(format!("failed to read {}: {e}", path.display())))?;

    let mut doc: toml_edit::DocumentMut = match raw.parse() {
        Ok(d) => d,
        Err(_) => {
            // Can't parse — leave it alone rather than destroy user data.
            return Ok(Some((path, false)));
        }
    };

    let mcp_table = match doc.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
        Some(t) => t,
        None => return Ok(Some((path, false))),
    };
    if !mcp_table.contains_key("sqz") {
        return Ok(Some((path, false)));
    }
    mcp_table.remove("sqz");

    // If mcp_servers is now empty, drop the whole table so the file
    // doesn't carry a dangling `[mcp_servers]` header.
    let mcp_is_empty = mcp_table.iter().count() == 0;
    if mcp_is_empty {
        doc.remove("mcp_servers");
    }

    // If the whole document is empty now, remove the file.
    let doc_is_empty = doc.iter().count() == 0;
    if doc_is_empty {
        std::fs::remove_file(&path).map_err(|e| {
            SqzError::Other(format!("failed to remove {}: {e}", path.display()))
        })?;
        return Ok(Some((path, true)));
    }

    std::fs::write(&path, doc.to_string()).map_err(|e| {
        SqzError::Other(format!("failed to write {}: {e}", path.display()))
    })?;
    Ok(Some((path, true)))
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // codex_config_path() reads CODEX_HOME / HOME env vars. Testing it
    // with set_var is inherently racy under parallel test execution, so
    // we only verify the structural invariant: the returned path always
    // ends with "config.toml".
    #[test]
    fn codex_config_path_ends_with_config_toml() {
        let got = codex_config_path();
        assert!(
            got.ends_with("config.toml"),
            "codex_config_path() must end with config.toml, got: {}",
            got.display()
        );
    }

    // ── AGENTS.md guidance block ─────────────────────────────────────

    #[test]
    fn agents_md_guidance_block_contains_sqz_invocation() {
        let block = agents_md_guidance_block("/usr/local/bin/sqz");
        assert!(block.contains(AGENTS_MD_BEGIN));
        assert!(block.contains(AGENTS_MD_END));
        assert!(block.contains("| /usr/local/bin/sqz compress"));
        assert!(block.contains("sqz-mcp"));
    }

    #[test]
    fn install_agents_md_creates_file_with_preamble() {
        let dir = tempfile::tempdir().unwrap();
        let created = install_agents_md_guidance(dir.path(), "sqz").unwrap();
        assert!(created);
        let content = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.starts_with("# AGENTS.md"));
        assert!(content.contains(AGENTS_MD_BEGIN));
        assert!(content.contains(AGENTS_MD_END));
    }

    #[test]
    fn install_agents_md_appends_without_clobbering_user_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(
            &path,
            "# My project rules\n\nBe polite. Run tests before committing.\n",
        ).unwrap();

        let created = install_agents_md_guidance(dir.path(), "sqz").unwrap();
        assert!(created);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# My project rules"),
            "original heading must survive");
        assert!(content.contains("Be polite. Run tests before committing."),
            "original body must survive");
        let user_idx = content.find("Be polite").unwrap();
        let sqz_idx = content.find(AGENTS_MD_BEGIN).unwrap();
        assert!(sqz_idx > user_idx,
            "sqz's block must append after user content, not prepend");
    }

    #[test]
    fn install_agents_md_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let first = install_agents_md_guidance(dir.path(), "sqz").unwrap();
        assert!(first);
        let second = install_agents_md_guidance(dir.path(), "sqz").unwrap();
        assert!(!second, "second install must be a no-op");

        let content = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        let occurrences = content.matches(AGENTS_MD_BEGIN).count();
        assert_eq!(occurrences, 1, "must not duplicate the block on re-install");
    }

    #[test]
    fn remove_agents_md_preserves_user_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(
            &path,
            "# My project rules\n\nBe polite. Run tests before committing.\n",
        ).unwrap();
        install_agents_md_guidance(dir.path(), "sqz").unwrap();

        let (returned_path, changed) =
            remove_agents_md_guidance(dir.path()).unwrap().unwrap();
        assert_eq!(returned_path, path);
        assert!(changed);
        assert!(path.exists(),
            "file must NOT be deleted — it has user content");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(AGENTS_MD_BEGIN));
        assert!(!content.contains(AGENTS_MD_END));
        assert!(content.contains("# My project rules"),
            "user heading must survive the uninstall");
        assert!(content.contains("Be polite. Run tests before committing."),
            "user body must survive the uninstall");
    }

    #[test]
    fn remove_agents_md_deletes_file_when_only_sqz_preamble_remains() {
        let dir = tempfile::tempdir().unwrap();
        install_agents_md_guidance(dir.path(), "sqz").unwrap();
        let path = dir.path().join("AGENTS.md");
        assert!(path.exists());

        let (_returned, changed) =
            remove_agents_md_guidance(dir.path()).unwrap().unwrap();
        assert!(changed);
        assert!(
            !path.exists(),
            "fresh-install AGENTS.md must be removed when sqz block is stripped"
        );
    }

    #[test]
    fn remove_agents_md_noop_when_block_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "# User-authored, sqz never touched this.\n").unwrap();

        let (returned_path, changed) =
            remove_agents_md_guidance(dir.path()).unwrap().unwrap();
        assert_eq!(returned_path, path);
        assert!(!changed, "no sqz block means no change");
        assert!(path.exists(), "user file must be untouched");
    }

    #[test]
    fn remove_agents_md_returns_none_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = remove_agents_md_guidance(dir.path()).unwrap();
        assert!(result.is_none());
    }

    // ── ~/.codex/config.toml merger ──────────────────────────────────

    #[test]
    fn install_codex_mcp_config_creates_file_with_sqz_entry() {
        let dir = tempfile::tempdir().unwrap();
        let created = install_codex_mcp_config_at(Some(dir.path())).unwrap();
        assert!(created);
        let content = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(
            content.contains("[mcp_servers.sqz]"),
            "config.toml must contain [mcp_servers.sqz] header; got:\n{content}"
        );
        assert!(content.contains("command = \"sqz-mcp\""),
            "command must be sqz-mcp");
        assert!(content.contains("--transport"));
        assert!(content.contains("stdio"));
    }

    #[test]
    fn install_codex_mcp_config_preserves_existing_other_servers() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "# User's existing Codex config, with a comment.\n\
             model = \"gpt-5\"\n\
             \n\
             [mcp_servers.other]\n\
             command = \"other-server\"\n\
             args = [\"--flag\"]\n",
        ).unwrap();

        let created = install_codex_mcp_config_at(Some(dir.path())).unwrap();
        assert!(created);

        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains("# User's existing Codex config"),
            "comment must survive: {after}");
        assert!(after.contains("model = \"gpt-5\""),
            "top-level key must survive: {after}");
        assert!(after.contains("[mcp_servers.other]"),
            "existing server entry must survive: {after}");
        assert!(after.contains("command = \"other-server\""),
            "existing server command must survive: {after}");
        assert!(after.contains("[mcp_servers.sqz]"),
            "sqz entry must be added: {after}");
    }

    #[test]
    fn install_codex_mcp_config_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_codex_mcp_config_at(Some(dir.path())).unwrap());
        assert!(
            !install_codex_mcp_config_at(Some(dir.path())).unwrap(),
            "second install with complete [mcp_servers.sqz] must be a no-op"
        );
    }

    #[test]
    fn install_codex_mcp_config_does_not_overwrite_user_tuned_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[mcp_servers.sqz]\n\
             command = \"/custom/path/sqz-mcp\"\n\
             args = [\"--transport\", \"sse\", \"--port\", \"3999\"]\n",
        ).unwrap();

        let changed = install_codex_mcp_config_at(Some(dir.path())).unwrap();
        assert!(!changed, "existing complete entry must be idempotent-skipped");
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains("/custom/path/sqz-mcp"),
            "user's custom command must survive re-init");
        assert!(after.contains("\"sse\""),
            "user's custom transport must survive");
    }

    #[test]
    fn remove_codex_mcp_config_removes_only_sqz_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "# keep this comment\n\
             model = \"gpt-5\"\n\
             \n\
             [mcp_servers.other]\n\
             command = \"other-server\"\n\
             \n\
             [mcp_servers.sqz]\n\
             command = \"sqz-mcp\"\n\
             args = [\"--transport\", \"stdio\"]\n",
        ).unwrap();

        let (path, changed) = remove_codex_mcp_config_at(Some(dir.path())).unwrap().unwrap();
        assert_eq!(path, cfg);
        assert!(changed);

        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains("# keep this comment"),
            "comment must survive: {after}");
        assert!(after.contains("model = \"gpt-5\""),
            "top-level key must survive: {after}");
        assert!(after.contains("[mcp_servers.other]"),
            "other server entry must survive: {after}");
        assert!(!after.contains("[mcp_servers.sqz]"),
            "sqz entry must be gone: {after}");
    }

    #[test]
    fn remove_codex_mcp_config_deletes_file_when_sqz_was_the_only_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        install_codex_mcp_config_at(Some(dir.path())).unwrap();
        let (path, changed) = remove_codex_mcp_config_at(Some(dir.path())).unwrap().unwrap();
        assert_eq!(path, cfg);
        assert!(changed);
        assert!(!cfg.exists(),
            "config.toml with only sqz must be deleted on uninstall");
    }

    #[test]
    fn remove_codex_mcp_config_returns_none_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = remove_codex_mcp_config_at(Some(dir.path())).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn remove_codex_mcp_config_noop_when_sqz_entry_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "[mcp_servers.other]\ncommand = \"x\"\n").unwrap();
        let (path, changed) = remove_codex_mcp_config_at(Some(dir.path())).unwrap().unwrap();
        assert_eq!(path, cfg);
        assert!(!changed);
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains("[mcp_servers.other]"));
    }
}
