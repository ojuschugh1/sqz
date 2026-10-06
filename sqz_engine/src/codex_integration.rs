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
//! This module implements (1) and (2). The guidance-file approach for (2)
//! is the standard answer to Codex's "no programmatic hook" limitation.

use std::path::{Path, PathBuf};

use crate::error::{Result, SqzError};
use crate::kiro_integration::{embedded_sqz_path, SteeringState};

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

const AGENTS_MD_BEGIN: &str = "<!-- BEGIN sqz-guidance (auto-installed by sqz init; remove this block to disable) -->";
const AGENTS_MD_END: &str = "<!-- END sqz-guidance -->";
const CODEX_LEGACY_BEGIN: &str = "<!-- BEGIN sqz-agents-guidance (auto-installed by sqz init; remove this block to disable) -->";
const CODEX_LEGACY_END: &str = "<!-- END sqz-agents-guidance -->";
const ZED_LEGACY_BEGIN: &str = "<!-- BEGIN sqz-zed-guidance (auto-installed by sqz init; remove this block to disable) -->";
const ZED_LEGACY_END: &str = "<!-- END sqz-zed-guidance -->";

pub fn sqz_guidance(sqz_path: &str) -> String {
    format!(
        "\
## sqz

Pipe shell commands with long output (git log or diff, tests, builds,
`ls -la`) through sqz:

```bash
git status 2>&1 | {sqz_path} compress
```

Don't pipe interactive commands (REPLs, `ssh`), chains with `&&`, `||`
or `;`, output redirected to a file, or short output. If that binary is
missing, run commands without it.

When the sqz MCP tools are available, prefer `sqz_read_file`, `sqz_grep`
and `sqz_list_dir` over the built-in read, grep and `ls` for long or
repeated reads, and use `sqz_recall` to find output seen earlier.

A `§ref:HASH§` stands in for output you have already seen, and
`§ref:HASH:L40-80§` for lines 40-80 of it. Get the text back with the
`expand` tool or `{sqz_path} expand <token>`. `«A1»` stands for the
phrase listed for it under `[Abbreviations]` at the top of the output.
For output without refs or abbreviations, put `SQZ_NO_DEDUP=1` or
`SQZ_NO_ABBREV=1` right before `{sqz_path} compress`.
"
    )
}

/// Build the markdown block that sqz appends to `AGENTS.md`.
///
/// The block is wrapped in HTML comment sentinels so `remove_agents_md_guidance`
/// can locate and excise it byte-exact on uninstall. `sqz_path` is the
/// absolute path of the sqz binary; we use it verbatim (no JSON escaping)
/// because this is a markdown file the user reads — raw backslashes on
/// Windows render correctly as-is, and the user can copy-paste the
/// commands straight into their shell.
pub fn agents_md_guidance_block(sqz_path: &str) -> String {
    format!("{AGENTS_MD_BEGIN}\n\n{}\n{AGENTS_MD_END}\n", sqz_guidance(sqz_path))
}

// ── AGENTS.md install/uninstall ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Shared,
    Codex,
    Zed,
}

impl BlockKind {
    pub(crate) const ALL: [BlockKind; 3] = [BlockKind::Shared, BlockKind::Codex, BlockKind::Zed];

    fn markers(self) -> (&'static str, &'static str) {
        match self {
            BlockKind::Shared => (AGENTS_MD_BEGIN, AGENTS_MD_END),
            BlockKind::Codex => (CODEX_LEGACY_BEGIN, CODEX_LEGACY_END),
            BlockKind::Zed => (ZED_LEGACY_BEGIN, ZED_LEGACY_END),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Block {
    start: usize,
    end: usize,
    kind: BlockKind,
}

fn find_blocks(content: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    for kind in BlockKind::ALL {
        let (begin, end) = kind.markers();
        let mut from = 0;
        while let Some(i) = content[from..].find(begin) {
            let start = from + i;
            let Some(j) = content[start..].find(end) else { break };
            from = start + j + end.len();
            blocks.push(Block { start, end: from, kind });
        }
    }
    blocks.sort_by_key(|b| b.start);
    let mut kept: Vec<Block> = Vec::new();
    for block in blocks {
        if kept.last().is_some_and(|last| block.start < last.end) {
            continue;
        }
        kept.push(block);
    }
    kept
}

fn has_sqz_marker(content: &str) -> bool {
    BlockKind::ALL.iter().any(|kind| content.contains(kind.markers().0))
}

fn excise(content: &str, block: Block) -> String {
    excise_range(content, block.start, block.end)
}

pub(crate) fn excise_range(content: &str, start: usize, end: usize) -> String {
    let head = &content[..start];
    let after = &content[end..];
    let tail = after.strip_prefix('\n').unwrap_or(after);
    let kept_head = head.trim_end_matches('\n');
    let kept_tail = tail.trim_start_matches('\n');
    if kept_tail.is_empty() {
        return if kept_head.is_empty() { String::new() } else { format!("{kept_head}\n") };
    }
    if kept_head.is_empty() {
        return kept_tail.to_string();
    }
    let newlines = (head.len() - kept_head.len() + tail.len() - kept_tail.len()).clamp(1, 2);
    format!("{kept_head}{}{kept_tail}", "\n".repeat(newlines))
}

fn replace_blocks(content: &str, block: &str) -> Option<String> {
    let first = *find_blocks(content).first()?;
    let mut out = format!("{}{block}{}", &content[..first.start], &content[first.end..]);
    loop {
        let blocks = find_blocks(&out);
        let [prev, next, ..] = blocks[..] else { break };
        out = if out[prev.end..next.start].trim().is_empty() {
            format!("{}{}", &out[..prev.end], &out[next.end..])
        } else {
            excise(&out, next)
        };
    }
    Some(out)
}

fn remove_blocks(content: &str, drop: &[BlockKind]) -> Option<String> {
    let mut out = content.to_string();
    let mut changed = false;
    while let Some(block) = find_blocks(&out).into_iter().find(|b| drop.contains(&b.kind)) {
        out = excise(&out, block);
        changed = true;
    }
    changed.then_some(out)
}

fn append_block(content: &str, block: &str) -> String {
    if content.trim().is_empty() {
        return block.to_string();
    }
    let mut out = content.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(block);
    out
}

pub(crate) fn replaced_guidance(content: &str, sqz_path: &str) -> Option<String> {
    replace_blocks(content, agents_md_guidance_block(sqz_path).trim_end_matches('\n'))
}

pub(crate) fn without_guidance(content: &str) -> Option<String> {
    remove_blocks(content, &BlockKind::ALL)
}

fn with_guidance(content: &str, sqz_path: &str) -> Option<String> {
    let updated = if find_blocks(content).is_empty() {
        if has_sqz_marker(content) {
            return None;
        }
        append_block(content, &agents_md_guidance_block(sqz_path))
    } else {
        replaced_guidance(content, sqz_path)?
    };
    (updated != content).then_some(updated)
}

pub(crate) fn write_guidance(path: &Path, sqz_path: &str) -> Result<bool> {
    let existing = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("failed to read {}: {e}", path.display())))?;
    let Some(updated) = with_guidance(&existing, sqz_path) else {
        return Ok(false);
    };
    std::fs::write(path, updated)
        .map_err(|e| SqzError::Other(format!("failed to write {}: {e}", path.display())))?;
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidanceChange {
    Unchanged,
    Create,
    Append,
    Replace,
}

pub fn guidance_change(path: &Path, sqz_path: &str) -> GuidanceChange {
    if !path.exists() {
        return GuidanceChange::Create;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return GuidanceChange::Unchanged;
    };
    match with_guidance(&content, sqz_path) {
        None => GuidanceChange::Unchanged,
        Some(_) if find_blocks(&content).is_empty() => GuidanceChange::Append,
        Some(_) => GuidanceChange::Replace,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuidanceBlock {
    pub bytes: usize,
    pub legacy: bool,
}

pub fn guidance_blocks(content: &str) -> Vec<GuidanceBlock> {
    find_blocks(content)
        .into_iter()
        .map(|b| GuidanceBlock { bytes: b.end - b.start, legacy: b.kind != BlockKind::Shared })
        .collect()
}

pub fn agents_md_guidance_state(path: &Path) -> SteeringState {
    match std::fs::read_to_string(path) {
        Ok(content) => guidance_state(&content),
        Err(_) => SteeringState::Missing,
    }
}

pub(crate) fn guidance_state(content: &str) -> SteeringState {
    let blocks = find_blocks(content);
    let [block] = blocks[..] else {
        return if blocks.is_empty() { SteeringState::UserAuthored } else { SteeringState::Stale };
    };
    let text = &content[block.start..block.end];
    let current = block.kind == BlockKind::Shared
        && embedded_sqz_path(text).is_some_and(|p| {
            text.replace("\r\n", "\n") == agents_md_guidance_block(p).trim_end_matches('\n')
        });
    if current {
        SteeringState::Current
    } else {
        SteeringState::Stale
    }
}

pub fn refresh_agents_md_guidance(path: &Path, fallback_sqz_path: &str) -> Result<bool> {
    if agents_md_guidance_state(path) != SteeringState::Stale {
        return Ok(false);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("failed to read {}: {e}", path.display())))?;
    let sqz_path = find_blocks(&content)
        .iter()
        .filter_map(|b| embedded_sqz_path(&content[b.start..b.end]))
        .find(|p| Path::new(p).is_file())
        .unwrap_or(fallback_sqz_path)
        .to_string();
    write_guidance(path, &sqz_path)
}

pub fn codex_mcp_registered() -> bool {
    std::fs::read_to_string(codex_config_path())
        .ok()
        .and_then(|raw| raw.parse::<toml_edit::DocumentMut>().ok())
        .is_some_and(|doc| {
            doc.get("mcp_servers")
                .and_then(|v| v.get("sqz"))
                .and_then(|v| v.get("command"))
                .is_some()
        })
}

pub(crate) fn remove_guidance_from(
    path: &Path,
    drop: &[BlockKind],
) -> Result<Option<(PathBuf, bool)>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("failed to read {}: {e}", path.display())))?;
    let Some(updated) = remove_blocks(&content, drop) else {
        return Ok(Some((path.to_path_buf(), false)));
    };
    if is_essentially_empty_agents_md(&updated) {
        std::fs::remove_file(path)
            .map_err(|e| SqzError::Other(format!("failed to remove {}: {e}", path.display())))?;
    } else {
        std::fs::write(path, updated)
            .map_err(|e| SqzError::Other(format!("failed to write {}: {e}", path.display())))?;
    }
    Ok(Some((path.to_path_buf(), true)))
}

pub fn remove_all_sqz_guidance(project_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for path in [project_dir.join(".rules"), agents_md_path(project_dir)] {
        if let Some((path, true)) = remove_guidance_from(&path, &BlockKind::ALL)? {
            changed.push(path);
        }
    }
    Ok(changed)
}

/// Install sqz's guidance block into `AGENTS.md` at `project_dir`.
///
/// If `AGENTS.md` doesn't exist yet, create it with sqz's block as the
/// sole content. If it exists, append sqz's block (separated by a blank
/// line so it renders as a new markdown section). If sqz blocks are
/// already present, the first becomes the current block and the rest
/// are removed, so `sqz init` stays idempotent.
///
/// Returns `true` when the file was created or modified, `false` when
/// it was left as it was.
pub fn install_agents_md_guidance(project_dir: &Path, sqz_path: &str) -> Result<bool> {
    let path = agents_md_path(project_dir);
    let block = agents_md_guidance_block(sqz_path);

    if path.exists() {
        return write_guidance(&path, sqz_path);
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
/// Locates the block by its BEGIN/END sentinels and excises it, keeping
/// the shared block while Zed is registered and reads this file. If the
/// resulting `AGENTS.md` is empty (modulo whitespace) or contains only
/// the stock preamble sqz itself wrote on first install, the file is
/// deleted entirely — leaving an empty `AGENTS.md` would be noise.
/// If sentinel markers are missing (user edited them out or the file
/// was never installed by sqz), this is a no-op.
///
/// Returns `Ok((path, removed_or_changed))`:
///   - `removed_or_changed = true`: the file was modified or deleted
///   - `removed_or_changed = false`: nothing was removed
///
/// Returns `Ok(None)` when `AGENTS.md` does not exist at all.
pub fn remove_agents_md_guidance(project_dir: &Path) -> Result<Option<(PathBuf, bool)>> {
    let zed_uses_it = crate::zed_integration::zed_mcp_registered()
        && crate::zed_integration::zed_guidance_target(project_dir) == agents_md_path(project_dir);
    remove_agents_md_guidance_at(project_dir, zed_uses_it)
}

pub(crate) fn remove_agents_md_guidance_at(
    project_dir: &Path,
    shared_in_use: bool,
) -> Result<Option<(PathBuf, bool)>> {
    let drop: &[BlockKind] = if shared_in_use {
        &[BlockKind::Codex]
    } else {
        &[BlockKind::Codex, BlockKind::Shared]
    };
    remove_guidance_from(&agents_md_path(project_dir), drop)
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
        assert!(block.contains("sqz_read_file"));
    }

    #[test]
    fn shared_guidance_is_short_plain_and_complete() {
        let text = sqz_guidance("/usr/local/bin/sqz");
        assert!(text.len() < 1000, "{} bytes", text.len());
        assert!(!text.contains('%'));
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
        assert!(text.contains("\ngit status 2>&1 | /usr/local/bin/sqz compress\n"));
        for needle in [
            "run commands without it",
            "sqz_read_file",
            "sqz_grep",
            "sqz_list_dir",
            "sqz_recall",
            "`expand`",
            "/usr/local/bin/sqz expand",
            "§ref:HASH§",
            "§ref:HASH:L40-80§",
            "«A1»",
            "[Abbreviations]",
            "SQZ_NO_DEDUP=1",
            "SQZ_NO_ABBREV=1",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
        assert_eq!(embedded_sqz_path(&text), Some("/usr/local/bin/sqz"));
    }

    fn legacy(kind: &str, body: &str) -> String {
        format!(
            "<!-- BEGIN sqz-{kind}-guidance (auto-installed by sqz init; remove this block to disable) -->\n\n{body}\n\n<!-- END sqz-{kind}-guidance -->"
        )
    }

    #[test]
    fn old_blocks_are_replaced_in_place_by_one_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        let head = "# Rules\n\nBe polite.\n\n";
        let tail = "\n\n## Notes\n\nKeep this.\n";
        std::fs::write(
            &path,
            format!(
                "{head}{}\n\n{}{tail}",
                legacy("agents", "git status        git status 2>&1 | /old/release/sqz compress"),
                legacy("zed", "git status 2>&1 | /old/debug/sqz compress"),
            ),
        )
        .unwrap();

        assert!(install_agents_md_guidance(dir.path(), "/new/sqz").unwrap());
        let block = agents_md_guidance_block("/new/sqz");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{head}{}{tail}", block.trim_end_matches('\n'))
        );
        assert!(!install_agents_md_guidance(dir.path(), "/new/sqz").unwrap());
    }

    #[test]
    fn a_block_between_user_paragraphs_is_removed_without_touching_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(
            &path,
            format!(
                "{}\n\nMine.\n\n{}\n\nAlso mine.\n",
                legacy("zed", "git status 2>&1 | /old/sqz compress"),
                legacy("agents", "git status        git status 2>&1 | /old/sqz compress"),
            ),
        )
        .unwrap();

        assert!(install_agents_md_guidance(dir.path(), "/new/sqz").unwrap());
        let block = agents_md_guidance_block("/new/sqz");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}\n\nMine.\n\nAlso mine.\n", block.trim_end_matches('\n'))
        );
    }

    #[test]
    fn refresh_keeps_the_first_binary_that_still_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("sqz");
        std::fs::write(&bin, "").unwrap();
        let bin = bin.to_string_lossy().to_string();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(
            &path,
            format!(
                "{}\n\n{}\n",
                legacy("agents", "git status        git status 2>&1 | /gone/sqz compress"),
                legacy("zed", &format!("git status 2>&1 | {bin} compress")),
            ),
        )
        .unwrap();

        assert_eq!(agents_md_guidance_state(&path), SteeringState::Stale);
        assert!(refresh_agents_md_guidance(&path, "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), agents_md_guidance_block(&bin));
        assert_eq!(agents_md_guidance_state(&path), SteeringState::Current);
        assert!(!refresh_agents_md_guidance(&path, "/fallback/sqz").unwrap());
    }

    #[test]
    fn refresh_falls_back_when_the_old_binary_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(
            &path,
            format!("{}\n", legacy("agents", "git status        git status 2>&1 | /gone/sqz compress")),
        )
        .unwrap();

        assert!(refresh_agents_md_guidance(&path, "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), agents_md_guidance_block("/fallback/sqz"));
    }

    #[test]
    fn outdated_text_in_the_new_block_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, agents_md_guidance_block("/bin/sqz").replace("sqz_recall", "recall")).unwrap();
        assert_eq!(agents_md_guidance_state(&path), SteeringState::Stale);
        std::fs::write(&path, format!("{0}\n{0}", agents_md_guidance_block("/bin/sqz"))).unwrap();
        assert_eq!(agents_md_guidance_state(&path), SteeringState::Stale);
    }

    #[test]
    fn files_without_a_complete_sqz_block_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "# Mine\n").unwrap();
        assert_eq!(agents_md_guidance_state(&path), SteeringState::UserAuthored);
        assert!(!refresh_agents_md_guidance(&path, "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Mine\n");

        let broken = "# Mine\n\n<!-- BEGIN sqz-agents-guidance (auto-installed by sqz init; remove this block to disable) -->\nhalf\n";
        std::fs::write(&path, broken).unwrap();
        assert!(!install_agents_md_guidance(dir.path(), "/new/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn guidance_change_says_what_init_would_do() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        assert_eq!(guidance_change(&path, "/bin/sqz"), GuidanceChange::Create);
        std::fs::write(&path, "# Mine\n").unwrap();
        assert_eq!(guidance_change(&path, "/bin/sqz"), GuidanceChange::Append);
        std::fs::write(&path, format!("# Mine\n\n{}\n", legacy("zed", "git status 2>&1 | /bin/sqz compress"))).unwrap();
        assert_eq!(guidance_change(&path, "/bin/sqz"), GuidanceChange::Replace);
        install_agents_md_guidance(dir.path(), "/bin/sqz").unwrap();
        assert_eq!(guidance_change(&path, "/bin/sqz"), GuidanceChange::Unchanged);
    }

    #[test]
    fn codex_removal_keeps_the_shared_block_while_zed_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        let block = agents_md_guidance_block("sqz");
        std::fs::write(&path, format!("# Mine\n\n{}\n\n{block}", legacy("agents", "a"))).unwrap();

        let (_, changed) = remove_agents_md_guidance_at(dir.path(), true).unwrap().unwrap();
        assert!(changed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("# Mine\n\n{block}"));
        let (_, changed) = remove_agents_md_guidance_at(dir.path(), true).unwrap().unwrap();
        assert!(!changed);
        let (_, changed) = remove_agents_md_guidance_at(dir.path(), false).unwrap().unwrap();
        assert!(changed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Mine\n");
    }

    #[test]
    fn full_uninstall_removes_old_and_new_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("AGENTS.md");
        let rules = dir.path().join(".rules");
        std::fs::write(
            &agents,
            format!("# Mine\n\n{}\n\n{}", legacy("agents", "a"), agents_md_guidance_block("sqz")),
        )
        .unwrap();
        std::fs::write(&rules, format!("Tabs.\n\n{}\n", legacy("zed", "z"))).unwrap();

        let changed = remove_all_sqz_guidance(dir.path()).unwrap();
        assert_eq!(changed, vec![rules.clone(), agents.clone()]);
        assert_eq!(std::fs::read_to_string(&agents).unwrap(), "# Mine\n");
        assert_eq!(std::fs::read_to_string(&rules).unwrap(), "Tabs.\n");
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
            remove_agents_md_guidance_at(dir.path(), false).unwrap().unwrap();
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
            remove_agents_md_guidance_at(dir.path(), false).unwrap().unwrap();
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
            remove_agents_md_guidance_at(dir.path(), false).unwrap().unwrap();
        assert_eq!(returned_path, path);
        assert!(!changed, "no sqz block means no change");
        assert!(path.exists(), "user file must be untouched");
    }

    #[test]
    fn remove_agents_md_returns_none_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = remove_agents_md_guidance_at(dir.path(), false).unwrap();
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
