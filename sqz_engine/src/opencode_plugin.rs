/// OpenCode plugin support for sqz.
///
/// OpenCode uses TypeScript plugins loaded from `~/.config/opencode/plugins/`.
/// The plugin hooks into `tool.execute.before` to rewrite bash commands,
/// piping output through `sqz compress` for token savings.
///
/// Unlike Claude Code / Cursor / Gemini (which use JSON hook configs),
/// OpenCode requires a TypeScript file that exports a factory function.
///
/// Plugin path: `~/.config/opencode/plugins/sqz.ts`
/// Config path: `opencode.json` OR `opencode.jsonc` in the project root.
/// The installer (`update_opencode_config`) discovers either variant and
/// merges sqz's entries into whichever exists; a fresh install defaults
/// to `opencode.json`. See issue #6 for the reason the installer must
/// look past the `.json` extension.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Generate the OpenCode TypeScript plugin content.
///
/// The plugin intercepts shell tool calls and rewrites them to pipe
/// output through `sqz hook opencode`, which compresses the output.
///
/// ## Plugin shape (issue #10 comment by @itguy327)
///
/// OpenCode's V1 plugin loader (packages/opencode/src/plugin/shared.ts,
/// function `readV1Plugin` + `resolvePluginId`) requires file-source
/// plugins to default-export `{ id: string, server: Plugin }`. Without
/// an `id`, OpenCode's loader throws "Path plugin ... must export id"
/// — but the loader is lenient and falls through to the "legacy"
/// path (`getLegacyPlugins`), which iterates all exports looking for
/// a factory function. That fallback works but gives the plugin no
/// human-readable name, so OpenCode's UI displays the raw
/// `file:///...` spec instead of "sqz". Reported by @itguy327 on
/// issue #10.
///
/// The fix is a dual-export shape:
///
/// 1. **Default export** — V1 object `{ id: "sqz", server: factory }`
///    so the modern loader identifies the plugin by name.
/// 2. **Named export** `SqzPlugin` — legacy factory fallback. Old
///    OpenCode versions that don't know about V1 walk
///    `Object.values(mod)`; default export dedups against the named
///    export via `Set` identity in `getLegacyPlugins` so the factory
///    fires exactly once either way.
///
/// Concrete verification that the dedup holds: the `seen` Set in
/// `getLegacyPlugins` uses identity, and we assign the same factory
/// reference to both. Also verified end-to-end by loading the
/// generated file under the V1 loader and asserting only one hook
/// registration.
pub fn generate_opencode_plugin(sqz_path: &str) -> String {
    // Escape for embedding in a double-quoted TypeScript string literal.
    // On Windows, sqz_path contains backslashes that must be escaped —
    // same reason we escape hook JSON in generate_hook_configs. See issue #2.
    let sqz_path = crate::tool_hooks::json_escape_string_value(sqz_path);
    format!(
        r#"/**
 * sqz — OpenCode plugin for transparent context compression.
 *
 * Intercepts shell commands and pipes output through sqz for token savings.
 * Install: copy to ~/.config/opencode/plugins/sqz.ts
 * Discovery is automatic — no opencode.json entry needed (and in fact
 * including one causes the plugin to load twice, per issue #10).
 */

const SqzPluginFactory = async (ctx: any) => {{
  const SQZ_PATH = "{sqz_path}";

  // Commands that should not be intercepted.
  const INTERACTIVE = new Set([
    "vim", "vi", "nano", "emacs", "less", "more", "top", "htop",
    "ssh", "python", "python3", "node", "irb", "ghci",
    "psql", "mysql", "sqlite3", "mongo", "redis-cli",
  ]);

  function isInteractive(cmd: string): boolean {{
    const base = cmd.split(/\s+/)[0]?.split("/").pop() ?? "";
    if (INTERACTIVE.has(base)) return true;
    if (cmd.includes("--watch") || cmd.includes("run dev") ||
        cmd.includes("run start") || cmd.includes("run serve")) return true;
    return false;
  }}

  function shouldIntercept(tool: string): boolean {{
    return ["bash", "shell", "terminal", "run_shell_command"].includes(tool.toLowerCase());
  }}

  // Detect that a command has already been wrapped by sqz. Before this
  // guard was in place OpenCode could call the hook twice on the same
  // command (for retried tool calls, or when a previous rewrite was
  // echoed back to the agent and the agent re-submitted it) and each
  // pass would prepend another `SQZ_CMD=$base` prefix, producing monsters
  // like `SQZ_CMD=SQZ_CMD=ddev SQZ_CMD=ddev ddev exec ...` (reported as
  // a follow-up to issue #5). We skip if any of these markers appear:
  //   * the case-insensitive substring "sqz_cmd=" or "sqz compress"
  //     (covers the tail of prior wraps regardless of case; SQZ_CMD= is
  //     legacy pre-issue-#10 but still valid in POSIX shell hooks)
  //   * a leading `VAR=` assignment that starts with SQZ_
  //     (defensive catch-all for exotic wrap variants)
  //   * the base command itself is sqz or sqz-mcp (running sqz directly
  //     — compressing sqz's own output is pointless and causes loops)
  function isAlreadyWrapped(cmd: string): boolean {{
    const lowered = cmd.toLowerCase();
    if (lowered.includes("sqz_cmd=")) return true;
    if (lowered.includes("sqz compress")) return true;
    if (lowered.includes("| sqz ") || lowered.includes("| sqz\t")) return true;
    if (/^\s*SQZ_[A-Z0-9_]+=/.test(cmd)) return true;
    const base = extractBaseCmd(cmd);
    if (base === "sqz" || base === "sqz-mcp" || base === "sqz.exe") return true;
    return false;
  }}

  // Extract the base command name defensively. If the command has
  // leading env-var assignments (VAR=val VAR2=val2 actual_cmd arg1),
  // skip past them so the base is `actual_cmd` — not `VAR=val`.
  function extractBaseCmd(cmd: string): string {{
    const tokens = cmd.split(/\s+/).filter(t => t.length > 0);
    for (const tok of tokens) {{
      // A token is an env assignment if it matches NAME=VALUE where NAME
      // is a valid env var identifier. Skip it and keep looking.
      if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(tok)) continue;
      return tok.split("/").pop() ?? "unknown";
    }}
    return "unknown";
  }}

  // Shell-escape a command-name label so it's safe to inline into the
  // rewritten shell command. Agents occasionally invoke commands via
  // paths with spaces (`"/my tools/foo" --arg`) and in the LLM
  // roundtrip that can survive to `extractBaseCmd`'s output. Quote the
  // label unless it's pure ASCII alphanumeric.
  function shellEscapeLabel(s: string): string {{
    if (/^[A-Za-z0-9_.-]+$/.test(s)) return s;
    return "'" + s.replace(/'/g, "'\\''") + "'";
  }}

  return {{
    "tool.execute.before": async (input: any, output: any) => {{
      const tool = input.tool ?? "";
      if (!shouldIntercept(tool)) return;

      const cmd = output.args?.command ?? "";
      if (!cmd || isAlreadyWrapped(cmd) || isInteractive(cmd)) return;

      // Rewrite: pipe through `sqz compress --cmd <base>`.
      //
      // Issue #10: the previous form was `SQZ_CMD=<base> <cmd> 2>&1 |
      // <sqz> compress`, which uses sh-specific inline env-var syntax.
      // On Windows, OpenCode Desktop routes bash-tool commands through
      // PowerShell (or cmd.exe when $SHELL is unset), and both parse
      // `SQZ_CMD=cmd` as a command name — raising CommandNotFoundException
      // and producing zero compression. `--cmd NAME` is a normal CLI
      // argument, shell-neutral, works in POSIX sh, zsh, fish, PowerShell,
      // and cmd.exe.
      const base = extractBaseCmd(cmd);
      const label = shellEscapeLabel(base);
      output.args.command = `${{cmd}} 2>&1 | ${{SQZ_PATH}} compress --cmd ${{label}}`;
    }},
  }};
}};

// V1 default export — modern OpenCode (post-V1 loader) reads `id` here
// and displays "sqz" in the plugin list. Without this, OpenCode falls
// back to the raw `file:///...` spec as the plugin name (@itguy327 on
// issue #10). `readV1Plugin` in OpenCode's plugin/shared.ts requires
// file-source plugins to declare an id — otherwise `resolvePluginId`
// throws.
export default {{
  id: "sqz",
  server: SqzPluginFactory,
}};

// Legacy named export — pre-V1 OpenCode versions walk Object.values(mod)
// looking for factory functions. Assigning the same reference as the
// default export's `.server` means the legacy `seen` Set dedups via
// identity, so the factory fires exactly once either way. Kept for
// backward compatibility with OpenCode versions that predate the V1
// loader (roughly anything before mid-2025).
export const SqzPlugin = SqzPluginFactory;
"#
    )
}

/// Default path for the OpenCode plugin file.
pub fn opencode_plugin_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".config")
        .join("opencode")
        .join("plugins")
        .join("sqz.ts")
}

/// Install the OpenCode plugin to `~/.config/opencode/plugins/sqz.ts`.
///
/// Always writes the latest generated plugin, overwriting any previous
/// version. The file is machine-generated (not user-edited), so
/// overwriting is safe and ensures fixes like the V1 id export and the
/// --cmd rewrite propagate on re-init. Previously this skipped if the
/// file existed, which left stale plugins in place after upgrades
/// (@itguy327 on issue #10: "that odd display issue is still there").
///
/// Returns `true` if the file was created or updated, `false` if the
/// content was already identical (no disk write needed).
pub fn install_opencode_plugin(sqz_path: &str) -> Result<bool> {
    let plugin_path = opencode_plugin_path();
    let new_content = generate_opencode_plugin(sqz_path);

    // Skip the write if the file already has identical content.
    if plugin_path.exists() {
        if let Ok(existing) = std::fs::read_to_string(&plugin_path) {
            if existing == new_content {
                return Ok(false);
            }
        }
    }

    if let Some(parent) = plugin_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::error::SqzError::Other(format!(
                "failed to create OpenCode plugins dir {}: {e}",
                parent.display()
            ))
        })?;
    }

    let content = generate_opencode_plugin(sqz_path);
    std::fs::write(&plugin_path, &content).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "failed to write OpenCode plugin to {}: {e}",
            plugin_path.display()
        ))
    })?;

    Ok(true)
}

/// Locate an existing OpenCode project config. Returns the path to
/// `opencode.jsonc` if present, else `opencode.json` if present, else
/// `None`. Prefers `.jsonc` because a user who bothered to write a
/// comment-annotated config is more invested in it, and sqz must not
/// silently create a parallel `.json` that would leave the `.jsonc`
/// looking un-updated (reported in issue #6).
pub fn find_opencode_config(project_dir: &Path) -> Option<PathBuf> {
    let jsonc = project_dir.join("opencode.jsonc");
    if jsonc.exists() {
        return Some(jsonc);
    }
    let json = project_dir.join("opencode.json");
    if json.exists() {
        return Some(json);
    }
    None
}

/// Return `true` if the user's OpenCode project config is a `.jsonc`
/// file that contains comments. Callers use this to decide whether to
/// warn the user that sqz's upcoming merge will drop those comments
/// (serde_json round-trips discard them).
pub fn opencode_config_has_comments(project_dir: &Path) -> bool {
    let path = match find_opencode_config(project_dir) {
        Some(p) => p,
        None => return false,
    };
    if path.extension().map(|e| e != "jsonc").unwrap_or(true) {
        return false;
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    strip_jsonc_comments(&content) != content
}

/// Strip JSONC-style comments from `src` while preserving string literals
/// byte-exact. Handles:
/// - `// line comments` through end-of-line
/// - `/* block comments */` (non-nested, which matches standard JSONC)
/// - Escape-aware string parsing so `"//"` inside a string is not stripped
///
/// Returns a string suitable for `serde_json::from_str`. Does not
/// attempt to preserve or round-trip the comments — callers that need
/// to write the file back must be explicit about losing comments.
pub fn strip_jsonc_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let len = bytes.len();

    while i < len {
        let b = bytes[i];

        // Enter a string literal: copy verbatim until the matching close
        // quote, honouring backslash escapes.
        if b == b'"' {
            out.push('"');
            i += 1;
            while i < len {
                let c = bytes[i];
                out.push(c as char);
                if c == b'\\' && i + 1 < len {
                    // Preserve the escape and the escaped char together.
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                i += 1;
                if c == b'"' {
                    break;
                }
            }
            continue;
        }

        // Line comment: skip through newline (but keep the newline so
        // line numbers line up for error messages).
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment: skip through `*/`.
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                // Preserve newlines so line numbers still line up.
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            // Skip the terminating `*/` if we found it; tolerate
            // unterminated comments by exiting the loop.
            if i + 1 < len {
                i += 2;
            }
            continue;
        }

        out.push(b as char);
        i += 1;
    }

    out
}

/// Update an existing `opencode.json`/`opencode.jsonc`, or create a
/// fresh `opencode.json`, so that sqz's plugin and MCP server are
/// registered. Idempotent.
///
/// If a `.jsonc` file exists, it is read with comment-stripping, merged,
/// and written back WITHOUT the comments — we can't losslessly round-trip
/// comments through serde_json. The caller is warned via the return
/// value's second field so `sqz init` can surface the fact.
///
/// If both files exist for some reason (OpenCode merges both), the
/// `.jsonc` is treated as authoritative (per `find_opencode_config`).
///
/// Returns `(updated, comments_lost)` where `updated` is true if any
/// change was written to disk, and `comments_lost` is true if sqz had
/// to drop comments from a `.jsonc` during the merge.
pub fn update_opencode_config(project_dir: &Path) -> Result<bool> {
    let (updated, _) = update_opencode_config_detailed(project_dir)?;
    Ok(updated)
}

/// Like `update_opencode_config` but also reports whether comments had
/// to be dropped from a JSONC file during the merge. Used by the `sqz
/// init` CLI to print a warning.
pub fn update_opencode_config_detailed(project_dir: &Path) -> Result<(bool, bool)> {
    let planned = plan_opencode_config_change(project_dir)?;
    if !planned.will_change {
        return Ok((false, false));
    }
    // Re-run through the same logic, actually writing this time.
    apply_opencode_config_change(project_dir, &planned)
}

/// Dry-run preview of what `update_opencode_config_detailed` would do.
///
/// Returns a `PlannedOpencodeChange` describing whether any write would
/// happen (`will_change`) and whether comments would be lost in the
/// process (`comments_lost`). Callers that only need the boolean answer
/// (e.g. the `sqz init` plan builder deciding whether to list OpenCode
/// in the plan) can check `will_change` directly.
///
/// Added after @Icaruk reported on issue #6 that the plan announced an
/// OpenCode merge on every re-run even when the file was already fully
/// configured, so users saw "no changes" and assumed the tool was
/// broken.
pub fn plan_opencode_config_change(project_dir: &Path) -> Result<PlannedOpencodeChange> {
    compute_opencode_change(project_dir, /*apply=*/ false).map(|r| r.0)
}

/// Result of a dry-run over the OpenCode config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedOpencodeChange {
    /// The file sqz would touch (exists or not).
    pub target_path: PathBuf,
    /// True if writing is needed. False means the config is already in
    /// the desired shape.
    pub will_change: bool,
    /// True if the file is `.jsonc` with comments that would be
    /// stripped during the serde_json round-trip.
    pub comments_lost: bool,
}

fn apply_opencode_config_change(
    project_dir: &Path,
    _planned: &PlannedOpencodeChange,
) -> Result<(bool, bool)> {
    let (planned, _) = compute_opencode_change(project_dir, /*apply=*/ true)?;
    Ok((planned.will_change, planned.comments_lost))
}

/// Shared core: compute-or-apply the OpenCode config change.
///
/// Factored out so `plan_opencode_config_change` (dry-run) and
/// `update_opencode_config_detailed` (write) share the same merge
/// logic. The `apply` flag gates the final write.
fn compute_opencode_change(
    project_dir: &Path,
    apply: bool,
) -> Result<(PlannedOpencodeChange, ())> {
    fn sqz_mcp_value() -> serde_json::Value {
        serde_json::json!({
            "type": "local",
            "command": ["sqz-mcp", "--transport", "stdio"],
            "enabled": true
        })
    }

    if let Some(existing_path) = find_opencode_config(project_dir) {
        let is_jsonc = existing_path
            .extension()
            .map(|e| e == "jsonc")
            .unwrap_or(false);
        let content = std::fs::read_to_string(&existing_path).map_err(|e| {
            crate::error::SqzError::Other(format!(
                "failed to read {}: {e}",
                existing_path.display()
            ))
        })?;
        let parseable = if is_jsonc {
            strip_jsonc_comments(&content)
        } else {
            content.clone()
        };
        let had_comments = is_jsonc && parseable != content;

        let mut config: serde_json::Value = serde_json::from_str(&parseable).map_err(|e| {
            crate::error::SqzError::Other(format!(
                "failed to parse {}: {e}",
                existing_path.display()
            ))
        })?;
        let obj = config.as_object_mut().ok_or_else(|| {
            crate::error::SqzError::Other(format!(
                "{} root is not a JSON object",
                existing_path.display()
            ))
        })?;

        let mut changed = false;

        if let Some(arr) = obj.get_mut("plugin").and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|v| v.as_str() != Some("sqz"));
            if arr.len() != before {
                changed = true;
            }
            if arr.is_empty() {
                obj.remove("plugin");
                changed = true;
            }
        }

        let mcp_entry = obj.entry("mcp").or_insert_with(|| serde_json::json!({}));
        if let Some(mcp_obj) = mcp_entry.as_object_mut() {
            if !mcp_obj.contains_key("sqz") {
                mcp_obj.insert("sqz".to_string(), sqz_mcp_value());
                changed = true;
            } else if let Some(sqz_entry) = mcp_obj.get_mut("sqz").and_then(|v| v.as_object_mut()) {
                if !sqz_entry.contains_key("enabled") {
                    sqz_entry.insert("enabled".to_string(), serde_json::json!(true));
                    changed = true;
                }
            }
        } else {
            return Err(crate::error::SqzError::Other(format!(
                "{} has an `mcp` field that is not an object; \
                 refusing to modify it automatically",
                existing_path.display()
            )));
        }

        let planned = PlannedOpencodeChange {
            target_path: existing_path.clone(),
            will_change: changed,
            comments_lost: changed && had_comments,
        };

        if apply && changed {
            let updated = serde_json::to_string_pretty(&config).map_err(|e| {
                crate::error::SqzError::Other(format!("failed to serialize config: {e}"))
            })?;
            std::fs::write(&existing_path, format!("{updated}\n")).map_err(|e| {
                crate::error::SqzError::Other(format!(
                    "failed to write {}: {e}",
                    existing_path.display()
                ))
            })?;
        }

        Ok((planned, ()))
    } else {
        // No existing config — a fresh opencode.json would be created.
        let target = project_dir.join("opencode.json");
        let planned = PlannedOpencodeChange {
            target_path: target.clone(),
            will_change: true,
            comments_lost: false,
        };

        if apply {
            let config = serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": {
                    "sqz": sqz_mcp_value()
                }
            });
            let content = serde_json::to_string_pretty(&config).map_err(|e| {
                crate::error::SqzError::Other(format!("failed to serialize opencode.json: {e}"))
            })?;
            std::fs::write(&target, format!("{content}\n")).map_err(|e| {
                crate::error::SqzError::Other(format!("failed to write opencode.json: {e}"))
            })?;
        }

        Ok((planned, ()))
    }
}

/// Remove sqz's entries from an existing `opencode.json`/`opencode.jsonc`
/// without deleting the whole file. Removes `mcp.sqz` and any `"sqz"`
/// entry from `plugin`. If this leaves `mcp` or `plugin` empty the keys
/// are dropped too. Returns `(path, changed)` — `changed` is `false`
/// when neither sqz entry was present.
///
/// Callers are expected to honour a `.jsonc` file's comments losing
/// fidelity on write: we parse with comment-stripping and emit as plain
/// JSON. The file keeps its original extension so OpenCode keeps reading
/// it. If the resulting config is completely empty (or would be the
/// near-empty shape we'd create from scratch), we remove the file
/// entirely since that's the cleaner uninstall state.
pub fn remove_sqz_from_opencode_config(project_dir: &Path) -> Result<Option<(PathBuf, bool)>> {
    let path = match find_opencode_config(project_dir) {
        Some(p) => p,
        None => return Ok(None),
    };
    let is_jsonc = path.extension().map(|e| e == "jsonc").unwrap_or(false);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        crate::error::SqzError::Other(format!("failed to read {}: {e}", path.display()))
    })?;
    let parseable = if is_jsonc {
        strip_jsonc_comments(&raw)
    } else {
        raw.clone()
    };
    let mut config: serde_json::Value = match serde_json::from_str(&parseable) {
        Ok(v) => v,
        Err(_) => {
            // Can't parse — be conservative and leave it alone.
            return Ok(Some((path, false)));
        }
    };

    let mut changed = false;

    if let Some(obj) = config.as_object_mut() {
        // Drop `"sqz"` from `plugin[]`.
        if let Some(plugin) = obj.get_mut("plugin").and_then(|v| v.as_array_mut()) {
            let before = plugin.len();
            plugin.retain(|v| v.as_str() != Some("sqz"));
            if plugin.len() != before {
                changed = true;
            }
            // Drop the whole `plugin` key if it's now empty.
            if plugin.is_empty() {
                obj.remove("plugin");
            }
        }

        // Drop `mcp.sqz`, and drop `mcp` itself if that was the only key.
        if let Some(mcp) = obj.get_mut("mcp").and_then(|v| v.as_object_mut()) {
            if mcp.remove("sqz").is_some() {
                changed = true;
            }
            if mcp.is_empty() {
                obj.remove("mcp");
            }
        }
    }

    if !changed {
        return Ok(Some((path, false)));
    }

    // If the remaining config is empty or nearly-so, just remove the file.
    // (A bare `{}` or `{ "$schema": "..." }` is what sqz's own
    // first-install would leave behind, and the user clearly doesn't
    // want sqz here — so nuking the sqz-authored shell is correct.)
    let essentially_empty = match config.as_object() {
        Some(obj) => {
            obj.is_empty()
                || (obj.len() == 1
                    && obj.get("$schema").and_then(|v| v.as_str())
                        == Some("https://opencode.ai/config.json"))
        }
        None => false,
    };

    if essentially_empty {
        std::fs::remove_file(&path).map_err(|e| {
            crate::error::SqzError::Other(format!(
                "failed to remove {}: {e}",
                path.display()
            ))
        })?;
        return Ok(Some((path, true)));
    }

    // Otherwise write back the pruned config. This loses any comments
    // a `.jsonc` had; the caller should surface that fact to the user.
    let updated = serde_json::to_string_pretty(&config).map_err(|e| {
        crate::error::SqzError::Other(format!("failed to serialize config: {e}"))
    })?;
    std::fs::write(&path, format!("{updated}\n")).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "failed to write {}: {e}",
            path.display()
        ))
    })?;
    Ok(Some((path, true)))
}

/// Return `true` if `command` has already been wrapped by an earlier sqz
/// hook pass (or otherwise contains an sqz invocation we should skip).
/// Used by `process_opencode_hook` and the equivalent TS guard in
/// `generate_opencode_plugin` to prevent double-wrapping.
///
/// Checks for any of:
/// - case-insensitive `sqz_cmd=` (prior-wrap prefix)
/// - case-insensitive `sqz compress` (prior-wrap tail)
/// - case-insensitive `| sqz ` or `| sqz\t` (any sqz subcommand pipe)
/// - a leading `SQZ_*=...` env assignment
/// - the base command itself is `sqz`/`sqz-mcp` (running sqz directly)
fn is_already_wrapped(command: &str) -> bool {
    let lowered = command.to_ascii_lowercase();
    if lowered.contains("sqz_cmd=") {
        return true;
    }
    if lowered.contains("sqz compress") {
        return true;
    }
    if lowered.contains("| sqz ") || lowered.contains("| sqz\t") {
        return true;
    }
    // Leading `SQZ_*=...` assignment.
    let trimmed = command.trim_start();
    if let Some(eq_idx) = trimmed.find('=') {
        let name = &trimmed[..eq_idx];
        if name.starts_with("SQZ_")
            && !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            return true;
        }
    }
    // Running sqz or sqz-mcp directly (e.g. `sqz stats`, `sqz-mcp --help`).
    let base = extract_base_cmd(command);
    if base == "sqz" || base == "sqz-mcp" || base == "sqz.exe" {
        return true;
    }
    false
}

/// Extract the base command name from a shell command string, skipping any
/// leading `VAR=value` env-var assignments. Mirrors `extractBaseCmd` in the
/// TS plugin — without this, a command like
/// `FOO=bar BAZ=qux make test` would pick `FOO=bar` as the base, which is
/// nonsense (and caused the recursive `SQZ_CMD=SQZ_CMD=...` reported as a
/// follow-up to issue #5).
fn extract_base_cmd(command: &str) -> &str {
    for tok in command.split_whitespace() {
        if is_env_assignment(tok) {
            continue;
        }
        return tok.rsplit('/').next().unwrap_or("unknown");
    }
    "unknown"
}

/// Return `true` if `token` has the shape `NAME=VALUE` where `NAME` is a
/// valid env-var identifier (letters/digits/underscores, starting with a
/// letter or underscore). Empty token → `false`.
fn is_env_assignment(token: &str) -> bool {
    let eq = match token.find('=') {
        Some(i) => i,
        None => return false,
    };
    if eq == 0 {
        return false;
    }
    let name = &token[..eq];
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Process an OpenCode `tool.execute.before` hook invocation.
///
/// OpenCode's hook format differs from Claude Code / Cursor:
/// - Input: `{ "tool": "bash", "sessionID": "...", "callID": "..." }`
/// - Args:  `{ "command": "git status" }`
///
/// The hook receives both `input` and `output` (args) as separate objects,
/// but when invoked via CLI (`sqz hook opencode`), we receive a combined
/// JSON with both fields.
pub fn process_opencode_hook(input: &str) -> Result<String> {
    let parsed: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| crate::error::SqzError::Other(format!("opencode hook: invalid JSON: {e}")))?;

    let tool = parsed
        .get("tool")
        .or_else(|| parsed.get("toolName"))
        .or_else(|| parsed.get("tool_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Only intercept shell tool calls
    if !matches!(
        tool.to_lowercase().as_str(),
        "bash" | "shell" | "terminal" | "run_shell_command"
    ) {
        return Ok(input.to_string());
    }

    // OpenCode puts args in a separate "args" field or in "toolCall"
    let command = parsed
        .get("args")
        .or_else(|| parsed.get("toolCall"))
        .or_else(|| parsed.get("tool_input"))
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if command.is_empty() || is_already_wrapped(command) {
        return Ok(input.to_string());
    }

    // Determine the base command name. Skip leading VAR=VALUE assignments
    // so an operator-prefixed command like `FOO=bar make test` still picks
    // `make` as the base instead of `FOO=bar`.
    let base = extract_base_cmd(command);

    if matches!(
        base,
        "vim" | "vi" | "nano" | "emacs" | "less" | "more" | "top" | "htop"
            | "ssh" | "python" | "python3" | "node" | "irb" | "ghci"
            | "psql" | "mysql" | "sqlite3" | "mongo" | "redis-cli"
    ) || command.contains("--watch")
        || command.contains("run dev")
        || command.contains("run start")
        || command.contains("run serve")
    {
        return Ok(input.to_string());
    }

    // Rewrite the command
    let base_cmd = base;

    let escaped_base = if base_cmd
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        base_cmd.to_string()
    } else {
        format!("'{}'", base_cmd.replace('\'', "'\\''"))
    };

    // Issue #10: use `--cmd NAME` instead of a sh-specific `SQZ_CMD=NAME`
    // prefix. Ensures the rewrite works in PowerShell and cmd.exe on
    // Windows (OpenCode Desktop's default bash-tool shell when $SHELL
    // is unset or set to a Windows shell), not just POSIX shells.
    let rewritten = format!(
        "{} 2>&1 | sqz compress --cmd {}",
        command, escaped_base,
    );

    // Output in the format OpenCode expects (same as Claude Code for CLI path)
    let output = serde_json::json!({
        "decision": "approve",
        "reason": "sqz: command output will be compressed for token savings",
        "updatedInput": {
            "command": rewritten
        },
        "args": {
            "command": rewritten
        }
    });

    serde_json::to_string(&output)
        .map_err(|e| crate::error::SqzError::Other(format!("opencode hook: serialize error: {e}")))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_opencode_plugin_contains_sqz_path() {
        let content = generate_opencode_plugin("/usr/local/bin/sqz");
        assert!(content.contains("/usr/local/bin/sqz"));
        assert!(content.contains("SqzPlugin"));
        assert!(content.contains("tool.execute.before"));
    }

    #[test]
    fn test_generate_opencode_plugin_windows_path_escaped() {
        // Issue #2: Windows paths embedded in the TS string literal must
        // have backslashes escaped. Before the fix, raw backslashes were
        // interpreted as JS escape sequences (\U, \S, \b) producing an
        // invalid or silently-wrong SQZ_PATH.
        let windows_path = r"C:\Users\SqzUser\.cargo\bin\sqz.exe";
        let content = generate_opencode_plugin(windows_path);
        // The string literal in the generated TS should contain the
        // path with doubled backslashes so that the runtime JS string
        // value equals the original path.
        assert!(
            content.contains(r#"const SQZ_PATH = "C:\\Users\\SqzUser\\.cargo\\bin\\sqz.exe""#),
            "expected JS-escaped path in plugin — got:\n{content}"
        );
        // And must NOT contain an unescaped backslash-sequence like \U
        // (which JS would interpret as a unicode escape and then fail).
        assert!(
            !content.contains(r#"const SQZ_PATH = "C:\U"#),
            "plugin must not contain unescaped backslashes in the string literal"
        );
    }

    #[test]
    fn test_generate_opencode_plugin_has_interactive_check() {
        let content = generate_opencode_plugin("sqz");
        assert!(content.contains("isInteractive"));
        assert!(content.contains("vim"));
        assert!(content.contains("--watch"));
    }

    /// Issue #10 follow-up (@itguy327 comment): OpenCode's plugin UI
    /// shows the raw `file:///...` spec as the plugin name instead of
    /// "sqz" because our generated plugin lacked the V1 `id` field.
    ///
    /// OpenCode's V1 loader in `packages/opencode/src/plugin/shared.ts`
    /// requires file-source plugins to default-export an object with an
    /// `id` field — `resolvePluginId` literally throws "Path plugin …
    /// must export id" if it's missing. When the default export is
    /// absent, the loader falls through to the legacy path which works
    /// but provides no name, so OpenCode displays the file spec
    /// instead.
    ///
    /// Fix: the plugin default-exports `{ id: "sqz", server: factory }`.
    /// This test locks in that shape — dropping either field would
    /// regress the fix.
    #[test]
    fn test_generate_opencode_plugin_declares_v1_id() {
        let content = generate_opencode_plugin("sqz");
        assert!(
            content.contains("id: \"sqz\""),
            "plugin must default-export `id: \"sqz\"` so OpenCode's \
             V1 loader (shared.ts readV1Plugin/resolvePluginId) \
             displays \"sqz\" in the UI instead of the file path; \
             got:\n{content}"
        );
        assert!(
            content.contains("server: SqzPluginFactory"),
            "plugin must default-export `server: <factory>` for V1 \
             loader compliance; got:\n{content}"
        );
        assert!(
            content.contains("export default {"),
            "plugin must have a default export per OpenCode V1 shape; \
             got:\n{content}"
        );
    }

    /// Companion to the V1-shape test: the legacy named export must
    /// stay in place for backward compat with pre-V1 OpenCode.
    ///
    /// The legacy loader walks `Object.values(mod)` and dedupes via a
    /// `Set`, so if our default export's `.server` is the same function
    /// reference as the `SqzPlugin` named export, the factory fires
    /// exactly once either way. This test asserts both exports are
    /// present AND share the same factory name — if someone later
    /// splits them into different functions they'd double-load on old
    /// OpenCode versions.
    #[test]
    fn test_generate_opencode_plugin_legacy_named_export_preserved() {
        let content = generate_opencode_plugin("sqz");
        assert!(
            content.contains("export const SqzPlugin = SqzPluginFactory"),
            "legacy named export must alias the same factory reference \
             as the V1 default export — otherwise old OpenCode versions \
             would see two distinct factories in `Object.values(mod)` \
             and fire the hook twice; got:\n{content}"
        );
    }

    // Note: the older `test_generate_opencode_plugin_has_sqz_guard` was
    // replaced by `test_generate_opencode_plugin_has_double_wrap_guard`
    // (defined further below). The old assertion codified a too-broad
    // guard (`cmd.includes("sqz")`) that the runaway-prefix fix had to
    // tighten — keeping it would pin the bug in place.

    #[test]
    fn test_process_opencode_hook_rewrites_bash() {
        let input = r#"{"tool":"bash","args":{"command":"git status"}}"#;
        let result = process_opencode_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["decision"].as_str().unwrap(), "approve");
        let cmd = parsed["args"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "should pipe through sqz: {cmd}");
        assert!(cmd.contains("git status"), "should preserve original: {cmd}");
        // Issue #10: label is passed via `--cmd NAME` (shell-neutral),
        // not via the sh-specific `SQZ_CMD=NAME` prefix that breaks
        // PowerShell and cmd.exe.
        assert!(cmd.contains("--cmd git"), "should pass base command via --cmd: {cmd}");
        assert!(
            !cmd.contains("SQZ_CMD="),
            "must not emit legacy sh-style env prefix: {cmd}"
        );
    }

    #[test]
    fn test_process_opencode_hook_passes_non_shell() {
        let input = r#"{"tool":"read_file","args":{"path":"file.txt"}}"#;
        let result = process_opencode_hook(input).unwrap();
        assert_eq!(result, input, "non-shell tools should pass through");
    }

    #[test]
    fn test_process_opencode_hook_skips_sqz_commands() {
        let input = r#"{"tool":"bash","args":{"command":"sqz stats"}}"#;
        let result = process_opencode_hook(input).unwrap();
        assert_eq!(result, input, "sqz commands should not be double-wrapped");
    }

    #[test]
    fn test_process_opencode_hook_skips_interactive() {
        let input = r#"{"tool":"bash","args":{"command":"vim file.txt"}}"#;
        let result = process_opencode_hook(input).unwrap();
        assert_eq!(result, input, "interactive commands should pass through");
    }

    #[test]
    fn test_process_opencode_hook_skips_watch() {
        let input = r#"{"tool":"bash","args":{"command":"npm run dev --watch"}}"#;
        let result = process_opencode_hook(input).unwrap();
        assert_eq!(result, input, "watch mode should pass through");
    }

    #[test]
    fn test_process_opencode_hook_invalid_json() {
        let result = process_opencode_hook("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_process_opencode_hook_empty_command() {
        let input = r#"{"tool":"bash","args":{"command":""}}"#;
        let result = process_opencode_hook(input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_process_opencode_hook_run_shell_command() {
        let input = r#"{"tool":"run_shell_command","args":{"command":"ls -la"}}"#;
        let result = process_opencode_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["args"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"));
    }

    #[test]
    fn test_install_opencode_plugin_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        // Override HOME to use temp dir
        std::env::set_var("HOME", dir.path());
        let result = install_opencode_plugin("sqz");
        assert!(result.is_ok());
        // Plugin should be created at ~/.config/opencode/plugins/sqz.ts
        let plugin_path = dir
            .path()
            .join(".config/opencode/plugins/sqz.ts");
        assert!(plugin_path.exists(), "plugin file should exist");
        let content = std::fs::read_to_string(&plugin_path).unwrap();
        assert!(content.contains("SqzPlugin"));
    }

    #[test]
    fn test_update_opencode_config_creates_new() {
        let dir = tempfile::tempdir().unwrap();
        let result = update_opencode_config(dir.path()).unwrap();
        assert!(result, "should create new config");
        let config_path = dir.path().join("opencode.json");
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("\"sqz\""));
        assert!(content.contains("sqz-mcp"));

        // Issue #10: fresh-install must NOT include `"plugin": ["sqz"]`.
        // The local plugin file at ~/.config/opencode/plugins/sqz.ts is
        // what actually installs the hook. Listing sqz in the config's
        // plugin array would make OpenCode try to also load it as an
        // npm package, producing two live copies of the plugin (reported
        // in issue #10).
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(
            parsed.get("plugin").is_none(),
            "fresh-install opencode.json must not include `plugin`; got: {content}"
        );
        assert_eq!(
            parsed["mcp"]["sqz"]["type"].as_str(),
            Some("local"),
            "mcp.sqz must be present"
        );
    }

    #[test]
    fn test_update_opencode_config_adds_to_existing() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("opencode.json");
        std::fs::write(
            &config_path,
            r#"{"$schema":"https://opencode.ai/config.json","plugin":["other"]}"#,
        )
        .unwrap();

        let result = update_opencode_config(dir.path()).unwrap();
        assert!(result, "should update existing config");
        let content = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        // Issue #10: sqz is NOT added to the `plugin` array any more
        // (double-load fix). But pre-existing plugin entries from
        // OTHER plugins must be preserved. And the MCP entry must
        // be added.
        let plugins = parsed["plugin"].as_array().unwrap();
        assert!(
            !plugins.iter().any(|v| v.as_str() == Some("sqz")),
            "issue #10: sqz must NOT be registered as a config-level plugin \
             (the local plugin file at ~/.config/opencode/plugins/sqz.ts \
             already loads it; double-registering causes double hook firing)"
        );
        assert!(
            plugins.iter().any(|v| v.as_str() == Some("other")),
            "pre-existing plugin entries from OTHER plugins must be preserved"
        );
        // MCP server registration IS still added — that's the separate,
        // non-duplicated path.
        assert_eq!(
            parsed["mcp"]["sqz"]["type"].as_str(),
            Some("local"),
            "mcp.sqz must be added"
        );
    }

    /// Issue #10 upgrade path: a user who ran an older sqz release and
    /// got `"plugin": ["sqz"]` written into their config should have
    /// that entry surgically removed when they re-run `sqz init` on a
    /// newer release. Pre-existing entries from other plugins survive.
    #[test]
    fn test_update_opencode_config_removes_legacy_sqz_plugin_entry() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("opencode.json");
        std::fs::write(
            &config_path,
            r#"{"plugin":["other","sqz"]}"#,
        )
        .unwrap();

        let changed = update_opencode_config(dir.path()).unwrap();
        assert!(changed, "must report that the legacy plugin entry was stripped");

        let after = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        let plugins = parsed["plugin"].as_array().unwrap();
        assert!(
            !plugins.iter().any(|v| v.as_str() == Some("sqz")),
            "legacy sqz plugin entry must be stripped on re-init"
        );
        assert!(
            plugins.iter().any(|v| v.as_str() == Some("other")),
            "other plugin entries must survive the cleanup"
        );
    }

    /// Issue #10: when the legacy `"plugin": ["sqz"]` was the ONLY
    /// entry in the plugin array, the whole `plugin` key should be
    /// dropped rather than left as `"plugin": []`.
    #[test]
    fn test_update_opencode_config_drops_empty_plugin_array_after_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("opencode.json");
        std::fs::write(&config_path, r#"{"plugin":["sqz"]}"#).unwrap();

        update_opencode_config(dir.path()).unwrap();

        let after = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert!(
            parsed.get("plugin").is_none(),
            "empty plugin array should be dropped entirely, got: {after}"
        );
    }

    #[test]
    fn test_update_opencode_config_skips_if_present() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("opencode.json");
        std::fs::write(
            &config_path,
            r#"{
  "mcp": {
    "sqz": {
      "type": "local",
      "command": ["sqz-mcp", "--transport", "stdio"],
      "enabled": true
    }
  }
}"#,
        )
        .unwrap();

        let result = update_opencode_config(dir.path()).unwrap();
        assert!(
            !result,
            "a config with mcp.sqz including enabled:true must be idempotent"
        );
    }

    /// When only `plugin[\"sqz\"]` is present the merger must add the
    /// missing `mcp.sqz` entry AND strip the legacy plugin entry.
    /// Before the issue #6 fix the updater only ever touched the
    /// plugin array, leaving MCP registration to chance.
    #[test]
    fn test_update_opencode_config_adds_missing_mcp_entry() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("opencode.json");
        std::fs::write(&config_path, r#"{"plugin":["sqz"]}"#).unwrap();

        let changed = update_opencode_config(dir.path()).unwrap();
        assert!(changed, "must report that mcp.sqz was added");

        let after = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(
            parsed["mcp"]["sqz"]["type"].as_str(),
            Some("local"),
            "mcp.sqz must be populated with the default server entry"
        );
    }

    // ── Issue #5 follow-up: runaway SQZ_CMD= prefix ───────────────────

    /// Regression for the runaway-prefix report on issue #5.
    ///
    /// The user observed `SQZ_CMD=SQZ_CMD=ddev SQZ_CMD=ddev ddev exec ...`
    /// in OpenCode's output — the plugin/hook wrapped a command that had
    /// already been wrapped by a prior pass. Before the fix,
    /// `process_opencode_hook`'s guard was only `command.contains("sqz")`
    /// which missed the uppercase `SQZ_CMD=` prefix and let the wrap
    /// accumulate.
    #[test]
    fn test_process_opencode_hook_skips_already_wrapped_sqz_cmd_prefix() {
        let input = r#"{"tool":"bash","args":{"command":"SQZ_CMD=ddev ddev exec --dir=/var/www/html php -v 2>&1 | /home/user/.cargo/bin/sqz compress"}}"#;
        let result = process_opencode_hook(input).unwrap();
        assert_eq!(
            result, input,
            "already-wrapped command must pass through unchanged; \
             otherwise each pass accumulates another SQZ_CMD= prefix"
        );
    }

    /// Guard must be case-insensitive: `SQZ_CMD=` contains no lowercase
    /// `sqz` and the old `command.contains("sqz")` check missed it.
    #[test]
    fn test_process_opencode_hook_guard_is_case_insensitive() {
        let input = r#"{"tool":"bash","args":{"command":"SQZ_CMD=git git status"}}"#;
        let result = process_opencode_hook(input).unwrap();
        assert_eq!(
            result, input,
            "uppercase SQZ_CMD= prefix must short-circuit the wrap"
        );
    }

    /// When a user command begins with legitimate env-var assignments
    /// (e.g. `FOO=bar make test`) the base command should be `make`,
    /// not `FOO=bar`. The old implementation picked `FOO=bar` and
    /// produced `SQZ_CMD=FOO=bar` wraps. Now it should produce
    /// `--cmd make` (issue #10).
    #[test]
    fn test_process_opencode_hook_skips_leading_env_assignments_for_base() {
        let input = r#"{"tool":"bash","args":{"command":"FOO=bar BAZ=qux make test"}}"#;
        let result = process_opencode_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["args"]["command"].as_str().unwrap();
        assert!(
            cmd.contains("--cmd make"),
            "base command must be `make`, not `FOO=bar`; got: {cmd}"
        );
        assert!(
            cmd.contains("FOO=bar BAZ=qux make test"),
            "original command must be preserved: {cmd}"
        );
    }

    /// Running sqz directly (e.g. `sqz stats`) must not be wrapped.
    #[test]
    fn test_process_opencode_hook_skips_bare_sqz_invocation() {
        for cmd in ["sqz stats", "sqz gain", "/usr/local/bin/sqz compress"] {
            let input = format!(
                r#"{{"tool":"bash","args":{{"command":"{cmd}"}}}}"#
            );
            let result = process_opencode_hook(&input).unwrap();
            assert_eq!(
                result, input,
                "sqz-invoking command `{cmd}` must not be rewrapped"
            );
        }
    }

    /// The generated TypeScript plugin must carry the same hardened
    /// guard the Rust hook has. We can't run the TS from Rust tests,
    /// but we can assert the generated source contains the key markers.
    #[test]
    fn test_generate_opencode_plugin_has_double_wrap_guard() {
        let content = generate_opencode_plugin("sqz");
        assert!(
            content.contains("function isAlreadyWrapped(cmd: string): boolean"),
            "generated plugin must define isAlreadyWrapped helper"
        );
        assert!(
            content.contains(r#"lowered.includes("sqz_cmd=")"#),
            "plugin must check for the SQZ_CMD= prior-wrap prefix"
        );
        assert!(
            content.contains(r#"lowered.includes("sqz compress")"#),
            "plugin must check for the `sqz compress` prior-wrap tail"
        );
        assert!(
            content.contains("isAlreadyWrapped(cmd)"),
            "plugin hook body must call isAlreadyWrapped on the command"
        );
        assert!(
            content.contains("function extractBaseCmd(cmd: string): string"),
            "plugin must define extractBaseCmd that skips env assignments"
        );
        assert!(
            content.contains("extractBaseCmd(cmd)"),
            "plugin hook body must use extractBaseCmd, not raw split"
        );
    }

    // ── Unit tests for the helper functions ──────────────────────────

    #[test]
    fn test_is_already_wrapped_detects_all_marker_shapes() {
        assert!(is_already_wrapped("SQZ_CMD=git git status"));
        assert!(is_already_wrapped("sqz_cmd=git git status"));
        assert!(is_already_wrapped("git status | sqz compress"));
        assert!(is_already_wrapped("git status 2>&1 | /path/sqz compress"));
        assert!(is_already_wrapped("ls -la | sqz compress-stream"));
        assert!(is_already_wrapped("sqz stats"));
        assert!(is_already_wrapped("/usr/local/bin/sqz gain"));
        assert!(is_already_wrapped("SQZ_FOO=bar cmd"));
        assert!(!is_already_wrapped("git status"));
        assert!(!is_already_wrapped("grep sqz logfile.txt"));
        assert!(!is_already_wrapped("cargo test --package my-sqz-crate"));
    }

    #[test]
    fn test_extract_base_cmd_skips_env_assignments() {
        assert_eq!(extract_base_cmd("make test"), "make");
        assert_eq!(extract_base_cmd("FOO=bar make test"), "make");
        assert_eq!(extract_base_cmd("FOO=bar BAZ=qux make test"), "make");
        assert_eq!(extract_base_cmd("/usr/bin/git status"), "git");
        assert_eq!(extract_base_cmd(""), "unknown");
        assert_eq!(extract_base_cmd("FOO=bar"), "unknown");
    }

    #[test]
    fn test_is_env_assignment() {
        assert!(is_env_assignment("FOO=bar"));
        assert!(is_env_assignment("FOO="));
        assert!(is_env_assignment("_underscore=1"));
        assert!(is_env_assignment("MixedCase_1=x"));
        assert!(!is_env_assignment("=bar"));
        assert!(!is_env_assignment("FOO"));
        assert!(!is_env_assignment("--flag=value"));
        assert!(!is_env_assignment("123=value"));
        assert!(!is_env_assignment("FOO BAR=baz"));
    }

    // ── Issue #6: opencode.jsonc support ─────────────────────────────

    /// Regression for issue #6 (@Icaruk). When a user has
    /// `opencode.jsonc` (OpenCode supports both `.json` and `.jsonc`),
    /// sqz init must MERGE into it rather than creating a parallel
    /// `opencode.json`. Before the fix `find_opencode_config` didn't
    /// exist and `update_opencode_config` was hardcoded to the `.json`
    /// path, so users with `.jsonc` ended up with two configs.
    #[test]
    fn test_update_merges_into_existing_jsonc() {
        let dir = tempfile::tempdir().unwrap();
        let jsonc = dir.path().join("opencode.jsonc");
        std::fs::write(
            &jsonc,
            r#"{
  // user's own config with a comment
  "$schema": "https://opencode.ai/config.json",
  "model": "anthropic/claude-sonnet-4-5",
  /* another comment */
  "plugin": ["other-plugin"]
}
"#,
        )
        .unwrap();

        let changed = update_opencode_config(dir.path()).unwrap();
        assert!(changed, "must merge sqz entries into the existing .jsonc");

        // The .jsonc file is the one we wrote back to — NOT a new .json.
        assert!(jsonc.exists(), "original .jsonc must still exist");
        assert!(
            !dir.path().join("opencode.json").exists(),
            "must not create a parallel opencode.json alongside .jsonc \
             (that's the issue #6 bug)"
        );

        let after = std::fs::read_to_string(&jsonc).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        let plugins = parsed["plugin"].as_array().unwrap();
        // Issue #10: sqz is NOT registered in the plugin array any more
        // (double-load fix). Pre-existing OTHER-plugin entries still
        // survive. The MCP server entry is the one we register now.
        assert!(
            !plugins.iter().any(|v| v.as_str() == Some("sqz")),
            "issue #10: sqz must NOT be added to plugin[]"
        );
        assert!(
            plugins.iter().any(|v| v.as_str() == Some("other-plugin")),
            "pre-existing plugin entries must be preserved"
        );
        assert_eq!(
            parsed["model"].as_str(),
            Some("anthropic/claude-sonnet-4-5"),
            "unrelated user keys must survive the merge"
        );
        assert_eq!(
            parsed["mcp"]["sqz"]["type"].as_str(),
            Some("local"),
            "mcp.sqz must be registered"
        );
    }

    /// Detailed variant: comments_lost must be reported when we
    /// rewrite a `.jsonc` that had comments. Callers (sqz init) use
    /// this to warn the user.
    #[test]
    fn test_update_opencode_config_detailed_reports_comments_lost() {
        let dir = tempfile::tempdir().unwrap();
        let jsonc = dir.path().join("opencode.jsonc");
        std::fs::write(
            &jsonc,
            r#"{
  // comment to be dropped
  "plugin": ["other"]
}
"#,
        )
        .unwrap();

        let (changed, comments_lost) =
            update_opencode_config_detailed(dir.path()).unwrap();
        assert!(changed);
        assert!(
            comments_lost,
            "merger must report that comments were dropped from .jsonc"
        );
    }

    /// Issue #6 follow-up: dry-run must report no change when the
    /// config is already fully configured, so the `sqz init` plan
    /// doesn't announce a merge that won't happen.
    #[test]
    fn plan_opencode_reports_no_change_when_already_configured() {
        let dir = tempfile::tempdir().unwrap();
        // First pass configures it.
        update_opencode_config(dir.path()).unwrap();
        // Second pass should be a no-op.
        let planned = plan_opencode_config_change(dir.path()).unwrap();
        assert!(
            !planned.will_change,
            "re-running against a fully configured file must be a no-op"
        );
        assert!(!planned.comments_lost);
    }

    /// Dry-run must report `will_change=true` when mcp.sqz is missing,
    /// and must NOT actually write the file.
    #[test]
    fn plan_opencode_reports_change_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        std::fs::write(&path, r#"{"plugin":["other"]}"#).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let planned = plan_opencode_config_change(dir.path()).unwrap();
        assert!(planned.will_change);
        assert_eq!(planned.target_path, path);

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after, "dry-run must not modify the file");
    }

    /// When no config exists, dry-run reports a fresh create and
    /// points at the default `opencode.json` path.
    #[test]
    fn plan_opencode_reports_fresh_create() {
        let dir = tempfile::tempdir().unwrap();
        let planned = plan_opencode_config_change(dir.path()).unwrap();
        assert!(planned.will_change);
        assert_eq!(planned.target_path, dir.path().join("opencode.json"));
        assert!(!dir.path().join("opencode.json").exists(),
            "dry-run must not create the file");
    }

    /// When no existing config is present, we still default to
    /// creating `opencode.json` (not `.jsonc`). The `.jsonc` variant
    /// is the user's choice to make; we don't force it.
    #[test]
    fn test_update_creates_plain_json_when_nothing_exists() {
        let dir = tempfile::tempdir().unwrap();
        update_opencode_config(dir.path()).unwrap();
        assert!(dir.path().join("opencode.json").exists());
        assert!(!dir.path().join("opencode.jsonc").exists());
    }

    /// `find_opencode_config` prefers `.jsonc` when both exist.
    #[test]
    fn test_find_opencode_config_prefers_jsonc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("opencode.json"), "{}").unwrap();
        std::fs::write(dir.path().join("opencode.jsonc"), "{}").unwrap();
        let found = find_opencode_config(dir.path()).unwrap();
        assert_eq!(
            found.file_name().unwrap(),
            "opencode.jsonc",
            "must prefer the .jsonc variant when both exist — the user \
             is maintaining .jsonc for its comment support"
        );
    }

    #[test]
    fn test_find_opencode_config_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_opencode_config(dir.path()).is_none());
    }

    #[test]
    fn test_opencode_config_has_comments_detects_jsonc_comments() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("opencode.jsonc"),
            "// a line comment\n{\"plugin\":[]}\n",
        )
        .unwrap();
        assert!(opencode_config_has_comments(dir.path()));
    }

    #[test]
    fn test_opencode_config_has_comments_ignores_plain_json() {
        let dir = tempfile::tempdir().unwrap();
        // The fake `//` is inside a JSON string — NOT a comment.
        std::fs::write(
            dir.path().join("opencode.json"),
            r#"{"url":"http://example.com"}"#,
        )
        .unwrap();
        assert!(!opencode_config_has_comments(dir.path()));
    }

    // ── JSONC comment stripper ───────────────────────────────────────

    #[test]
    fn test_strip_jsonc_comments_removes_line_comments() {
        let src = "{\n  // leading comment\n  \"a\": 1 // trailing\n}";
        let stripped = strip_jsonc_comments(src);
        assert!(!stripped.contains("leading comment"));
        assert!(!stripped.contains("trailing"));
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["a"], 1);
    }

    #[test]
    fn test_strip_jsonc_comments_removes_block_comments() {
        let src = "{\n  /* block\n     comment */\n  \"a\": 1\n}";
        let stripped = strip_jsonc_comments(src);
        assert!(!stripped.contains("block"));
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["a"], 1);
    }

    #[test]
    fn test_strip_jsonc_comments_preserves_strings() {
        // The `//` inside the URL must NOT be treated as a line comment,
        // and the `/* ... */` pattern inside the string must NOT be
        // treated as a block comment. This is the classic JSONC parser
        // bug — we want to prove our stripper is string-aware.
        let src = r#"{"url": "http://example.com", "re": "/* not a comment */"}"#;
        let stripped = strip_jsonc_comments(src);
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["url"], "http://example.com");
        assert_eq!(parsed["re"], "/* not a comment */");
    }

    #[test]
    fn test_strip_jsonc_comments_preserves_escaped_quote_in_string() {
        let src = r#"{"s": "a\"//b"}"#;
        let stripped = strip_jsonc_comments(src);
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["s"], r#"a"//b"#);
    }

    #[test]
    fn test_strip_jsonc_comments_tolerates_unterminated_block() {
        // We don't want to panic or infinite-loop on malformed input.
        let src = "{\"a\":1 /* never ends";
        let _ = strip_jsonc_comments(src); // should return without panic
    }

    // ── Surgical uninstall ───────────────────────────────────────────

    /// Regression for the uninstall-wipes-user-config concern tied to
    /// issue #6. Before this change `sqz uninstall` called
    /// `remove_file` on the entire `opencode.json`, destroying any
    /// user config that had been merged with sqz's entries. The
    /// surgical helper keeps the file, removes only sqz's keys.
    #[test]
    fn test_remove_sqz_preserves_other_user_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("opencode.json");
        std::fs::write(
            &config,
            r#"{
  "$schema": "https://opencode.ai/config.json",
  "model": "anthropic/claude-sonnet-4-5",
  "plugin": ["other-plugin", "sqz"],
  "mcp": {
    "sqz": { "type": "local", "command": ["sqz-mcp"] },
    "jira": { "type": "remote", "url": "https://jira.example.com/mcp" }
  }
}
"#,
        )
        .unwrap();

        let (path, changed) =
            remove_sqz_from_opencode_config(dir.path()).unwrap().unwrap();
        assert_eq!(path, config);
        assert!(changed, "must report that sqz entries were removed");
        assert!(
            config.exists(),
            "file must NOT be deleted — only sqz's entries removed"
        );

        let after = std::fs::read_to_string(&config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        let plugins = parsed["plugin"].as_array().unwrap();
        assert!(!plugins.iter().any(|v| v.as_str() == Some("sqz")));
        assert!(plugins.iter().any(|v| v.as_str() == Some("other-plugin")));
        let mcp = parsed["mcp"].as_object().unwrap();
        assert!(!mcp.contains_key("sqz"), "mcp.sqz must be gone");
        assert!(mcp.contains_key("jira"), "mcp.jira must survive");
        assert_eq!(
            parsed["model"].as_str(),
            Some("anthropic/claude-sonnet-4-5"),
            "unrelated keys must survive"
        );
    }

    /// If the file was CREATED by sqz (just $schema + sqz entries),
    /// removing sqz's entries should delete the whole file since
    /// there's nothing else the user wanted to keep.
    #[test]
    fn test_remove_sqz_deletes_file_when_nothing_else_remains() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("opencode.json");
        // This is exactly the shape sqz writes on fresh install.
        std::fs::write(
            &config,
            r#"{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "sqz": { "type": "local", "command": ["sqz-mcp", "--transport", "stdio"] }
  },
  "plugin": ["sqz"]
}
"#,
        )
        .unwrap();

        let (_, changed) =
            remove_sqz_from_opencode_config(dir.path()).unwrap().unwrap();
        assert!(changed);
        assert!(
            !config.exists(),
            "file with only $schema + sqz entries must be removed"
        );
    }

    /// When there's nothing to uninstall (no config present), the
    /// surgical helper returns None rather than erroring.
    #[test]
    fn test_remove_sqz_returns_none_when_config_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = remove_sqz_from_opencode_config(dir.path()).unwrap();
        assert!(result.is_none());
    }

    /// Surgical uninstall against a .jsonc file: strips comments on
    /// read, writes back as plain JSON (to the same .jsonc path).
    #[test]
    fn test_remove_sqz_from_jsonc_drops_comments() {
        let dir = tempfile::tempdir().unwrap();
        let jsonc = dir.path().join("opencode.jsonc");
        std::fs::write(
            &jsonc,
            r#"{
  // user's comment
  "model": "x",
  "plugin": ["sqz", "other"]
}
"#,
        )
        .unwrap();

        let (path, changed) =
            remove_sqz_from_opencode_config(dir.path()).unwrap().unwrap();
        assert_eq!(path, jsonc);
        assert!(changed);
        assert!(path.exists(), "jsonc file kept because `model` and `other` remain");

        let after = std::fs::read_to_string(&jsonc).unwrap();
        assert!(
            !after.contains("// user's comment"),
            "comments are dropped by the serde_json round-trip; \
             documented in update_opencode_config_detailed"
        );
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        let plugins = parsed["plugin"].as_array().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0], "other");
    }

    // ── Issue #10: Windows shell + duplicate plugin load ──────────────────

    /// End-to-end regression for issue #10. The reporter ran `dotnet
    /// build` via OpenCode Desktop on Windows and got a
    /// CommandNotFoundException from PowerShell because sqz emitted
    /// the sh-specific `SQZ_CMD=cmd cmd /c dotnet build …` form.
    ///
    /// The fix: use `sqz compress --cmd NAME` — a normal CLI argument
    /// every shell accepts.
    #[test]
    fn issue_10_opencode_rewrite_works_in_powershell_syntax() {
        let input = r#"{"tool":"bash","args":{"command":"dotnet build NewNeonCheckers3.sln"}}"#;
        let result = process_opencode_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["args"]["command"].as_str().unwrap();

        // Regression asserts: the rewrite must not contain the
        // sh-specific env-var assignment that breaks in PowerShell and
        // cmd.exe.
        assert!(
            !cmd.contains("SQZ_CMD="),
            "issue #10: rewrite must not emit `SQZ_CMD=` (breaks on \
             PowerShell/cmd.exe); got: {cmd}"
        );
        // And it must use the shell-neutral --cmd form instead.
        assert!(
            cmd.contains("--cmd dotnet"),
            "rewrite must pass label via --cmd; got: {cmd}"
        );
        // PowerShell tokenises on whitespace: a command that begins
        // with a word that is NOT an env assignment must be what
        // PowerShell will execute. "dotnet build …" is valid in every
        // shell; "SQZ_CMD=… dotnet build …" is not.
        let first_token = cmd.split_whitespace().next().unwrap_or("");
        assert_eq!(
            first_token, "dotnet",
            "first token of the rewritten command must be the user's \
             command itself, not an env-var assignment; got: {cmd}"
        );
    }

    /// Companion to the above: the TS plugin (which runs inside
    /// OpenCode's Bun runtime) must emit the same shell-neutral form.
    /// Both the Rust-side hook and the TS plugin exist so we test both.
    #[test]
    fn issue_10_ts_plugin_emits_cmd_flag_not_env_prefix() {
        let content = generate_opencode_plugin("sqz");
        // The plugin builds its rewrite with a template literal. Look
        // for the `--cmd` pattern and make sure the legacy `SQZ_CMD=`
        // prefix is nowhere in the output template.
        assert!(
            content.contains("compress --cmd"),
            "TS plugin must build rewrite with `compress --cmd ${{base}}`"
        );
        // The plugin still CONTAINS the SQZ_CMD= string — in a regex
        // (`/^\\s*SQZ_[A-Z0-9_]+=/`) used by `isAlreadyWrapped` to
        // detect legacy pre-wrapped commands from older sqz versions.
        // So we assert specifically that the EMITTED COMMAND has no
        // `SQZ_CMD=${base} ${cmd}` template.
        assert!(
            !content.contains("SQZ_CMD=${base}"),
            "TS plugin must not emit the legacy `SQZ_CMD=${{base}}` prefix"
        );
    }

    /// Bug #1 from issue #10: plugin loaded twice.
    ///
    /// Before the fix, `sqz init` wrote both:
    ///   1. `"plugin": ["sqz"]` in opencode.json
    ///   2. `~/.config/opencode/plugins/sqz.ts`
    ///
    /// Per OpenCode docs: "a local plugin and an npm plugin with
    /// similar names are both loaded separately." So (1) + (2)
    /// produced two live plugin instances firing on every tool call.
    ///
    /// The fix: don't write (1). Rely on (2) — OpenCode auto-loads
    /// `.ts` files from the plugins directory. Keep the MCP server
    /// registration in opencode.json (that's a separate, non-
    /// duplicating concern).
    #[test]
    fn issue_10_fresh_opencode_config_has_no_plugin_entry() {
        let dir = tempfile::tempdir().unwrap();
        update_opencode_config(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("opencode.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // The deliberate absence of `plugin` is the whole fix.
        assert!(
            parsed.get("plugin").is_none(),
            "issue #10: fresh opencode.json must not include `plugin` key; got: {content}"
        );

        // MCP server registration must still be present — it's the
        // separate, non-duplicating path.
        assert_eq!(
            parsed["mcp"]["sqz"]["type"].as_str(),
            Some("local"),
            "mcp.sqz is the one sqz-authored entry that belongs in \
             opencode.json; must still be registered"
        );
    }

    /// When a user upgrades from an older sqz (which wrote `plugin:
    /// ["sqz"]`), running `sqz init` on the new version must
    /// surgically remove the legacy entry so the double-load bug is
    /// actually resolved — not just prevented for fresh installs.
    #[test]
    fn issue_10_reinit_strips_legacy_plugin_entry() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("opencode.json");
        std::fs::write(
            &config,
            // The exact shape an older sqz install would have produced.
            r#"{"$schema":"https://opencode.ai/config.json","mcp":{"sqz":{"type":"local","command":["sqz-mcp","--transport","stdio"]}},"plugin":["sqz"]}"#,
        )
        .unwrap();

        let changed = update_opencode_config(dir.path()).unwrap();
        assert!(changed, "re-init must report a change (the legacy entry was stripped)");

        let after = std::fs::read_to_string(&config).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert!(
            parsed.get("plugin").is_none(),
            "legacy `plugin: [\"sqz\"]` must be stripped on re-init; got: {after}"
        );
        // MCP entry must survive.
        assert_eq!(
            parsed["mcp"]["sqz"]["type"].as_str(),
            Some("local"),
            "mcp.sqz must survive cleanup of the plugin entry"
        );
    }
}
