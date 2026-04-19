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
/// Config path: `opencode.json` (project root) — adds `"plugin": ["sqz"]`

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Generate the OpenCode TypeScript plugin content.
///
/// The plugin intercepts shell tool calls and rewrites them to pipe
/// output through `sqz hook opencode`, which compresses the output.
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
 * Config:  add "plugin": ["sqz"] to opencode.json
 */

export const SqzPlugin = async (ctx: any) => {{
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
  //     (covers the tail of prior wraps regardless of case)
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

  return {{
    "tool.execute.before": async (input: any, output: any) => {{
      const tool = input.tool ?? "";
      if (!shouldIntercept(tool)) return;

      const cmd = output.args?.command ?? "";
      if (!cmd || isAlreadyWrapped(cmd) || isInteractive(cmd)) return;

      // Rewrite: pipe through sqz compress
      const base = extractBaseCmd(cmd);
      output.args.command = `SQZ_CMD=${{base}} ${{cmd}} 2>&1 | ${{SQZ_PATH}} compress`;
    }},
  }};
}};
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
/// Returns `true` if the plugin was installed, `false` if it already exists.
pub fn install_opencode_plugin(sqz_path: &str) -> Result<bool> {
    let plugin_path = opencode_plugin_path();

    if plugin_path.exists() {
        return Ok(false);
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

/// Update the project's `opencode.json` to reference the sqz plugin.
///
/// If `opencode.json` exists, adds `"sqz"` to the `"plugin"` array.
/// If it doesn't exist, creates a minimal config with the plugin reference.
///
/// Returns `true` if the config was created/updated, `false` if sqz was
/// already listed.
pub fn update_opencode_config(project_dir: &Path) -> Result<bool> {
    let config_path = project_dir.join("opencode.json");

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path).map_err(|e| {
            crate::error::SqzError::Other(format!("failed to read opencode.json: {e}"))
        })?;

        // Parse existing config
        let mut config: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
            crate::error::SqzError::Other(format!("failed to parse opencode.json: {e}"))
        })?;

        // Check if sqz is already in the plugin array
        if let Some(plugins) = config.get("plugin").and_then(|v| v.as_array()) {
            if plugins.iter().any(|v| v.as_str() == Some("sqz")) {
                return Ok(false); // Already configured
            }
        }

        // Add sqz to the plugin array
        let plugins = config
            .as_object_mut()
            .ok_or_else(|| crate::error::SqzError::Other("opencode.json is not an object".into()))?
            .entry("plugin")
            .or_insert_with(|| serde_json::json!([]));

        if let Some(arr) = plugins.as_array_mut() {
            arr.push(serde_json::json!("sqz"));
        }

        let updated = serde_json::to_string_pretty(&config).map_err(|e| {
            crate::error::SqzError::Other(format!("failed to serialize opencode.json: {e}"))
        })?;

        std::fs::write(&config_path, format!("{updated}\n")).map_err(|e| {
            crate::error::SqzError::Other(format!("failed to write opencode.json: {e}"))
        })?;

        Ok(true)
    } else {
        // Create a minimal opencode.json with sqz plugin + MCP
        let config = serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "mcp": {
                "sqz": {
                    "type": "local",
                    "command": ["sqz-mcp", "--transport", "stdio"]
                }
            },
            "plugin": ["sqz"]
        });

        let content = serde_json::to_string_pretty(&config).map_err(|e| {
            crate::error::SqzError::Other(format!("failed to serialize opencode.json: {e}"))
        })?;

        std::fs::write(&config_path, format!("{content}\n")).map_err(|e| {
            crate::error::SqzError::Other(format!("failed to write opencode.json: {e}"))
        })?;

        Ok(true)
    }
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

    let rewritten = format!(
        "SQZ_CMD={} {} 2>&1 | sqz compress",
        escaped_base, command
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
        assert!(cmd.contains("SQZ_CMD=git"), "should set SQZ_CMD: {cmd}");
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
        let plugins = parsed["plugin"].as_array().unwrap();
        assert!(plugins.iter().any(|v| v.as_str() == Some("sqz")));
        assert!(plugins.iter().any(|v| v.as_str() == Some("other")));
    }

    #[test]
    fn test_update_opencode_config_skips_if_present() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("opencode.json");
        std::fs::write(
            &config_path,
            r#"{"plugin":["sqz"]}"#,
        )
        .unwrap();

        let result = update_opencode_config(dir.path()).unwrap();
        assert!(!result, "should skip if sqz already present");
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
    /// produced `SQZ_CMD=FOO=bar` wraps.
    #[test]
    fn test_process_opencode_hook_skips_leading_env_assignments_for_base() {
        let input = r#"{"tool":"bash","args":{"command":"FOO=bar BAZ=qux make test"}}"#;
        let result = process_opencode_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["args"]["command"].as_str().unwrap();
        assert!(
            cmd.contains("SQZ_CMD=make"),
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
}
