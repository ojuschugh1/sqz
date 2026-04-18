/// PreToolUse hook integration for AI coding tools.
///
/// Provides transparent command interception: when an AI tool (Claude Code,
/// Cursor, Copilot, etc.) executes a bash command, the hook rewrites it to
/// pipe output through sqz for compression. The AI tool never knows it
/// happened — it just sees smaller output.
///
/// Supported hook formats (tools that support command rewriting via hooks):
/// - Claude Code: .claude/settings.local.json (nested PreToolUse, matcher: "Bash")
/// - Gemini CLI: .gemini/settings.json (nested BeforeTool, matcher: "run_shell_command")
/// - OpenCode: ~/.config/opencode/plugins/sqz.ts (TypeScript plugin, tool.execute.before)
///
/// Tools that do NOT support command rewriting via hooks (use prompt-level
/// guidance via rules files instead):
/// - Codex: only supports deny in PreToolUse; updatedInput is parsed but ignored
/// - Windsurf: no documented hook API; uses .windsurfrules prompt-level guidance
/// - Cline: PreToolUse cannot rewrite commands; uses .clinerules prompt-level guidance
/// - Cursor: beforeShellExecution hook can allow/deny/ask only; the response
///   has no documented field for rewriting the command. Uses .cursor/rules/sqz.mdc
///   prompt-level guidance instead. The `sqz hook cursor` subcommand remains
///   available and well-formed for users who configure hooks manually, but
///   Cursor's documented hook schema (per GitButler deep-dive and Cupcake
///   reference docs) confirms the response is `{permission, continue,
///   userMessage, agentMessage}` only — no `updated_input`.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// A tool hook configuration for a specific AI coding tool.
#[derive(Debug, Clone)]
pub struct ToolHookConfig {
    /// Name of the AI tool.
    pub tool_name: String,
    /// Path to the hook config file (relative to project root or home).
    pub config_path: PathBuf,
    /// The JSON/TOML content to write.
    pub config_content: String,
    /// Whether this is a project-level or user-level config.
    pub scope: HookScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookScope {
    /// Installed per-project (e.g., .claude/hooks/)
    Project,
    /// Installed globally for the user (e.g., ~/.claude/hooks/)
    User,
}

/// Which AI tool platform is invoking the hook.
/// Each platform has a different JSON output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPlatform {
    /// Claude Code: hookSpecificOutput with updatedInput (camelCase)
    ClaudeCode,
    /// Cursor: flat { permission, updated_input } (snake_case)
    Cursor,
    /// Gemini CLI: decision + hookSpecificOutput.tool_input
    GeminiCli,
    /// Windsurf: exit-code based (no JSON rewriting support confirmed)
    Windsurf,
}

/// Process a PreToolUse hook invocation from an AI tool.
///
/// Reads a JSON payload from `input` describing the tool call, rewrites
/// bash commands to pipe through sqz, and returns the modified payload.
///
/// Input format (Claude Code):
/// ```json
/// {
///   "tool_name": "Bash",
///   "tool_input": {
///     "command": "git status"
///   }
/// }
/// ```
///
/// Output: same structure with command rewritten to pipe through sqz.
/// Exit code 0 = proceed with modified command.
/// Exit code 1 = block the tool call (not used here).
pub fn process_hook(input: &str) -> Result<String> {
    process_hook_for_platform(input, HookPlatform::ClaudeCode)
}

/// Process a hook invocation for Cursor (different output format).
///
/// Cursor uses flat JSON: `{ "permission": "allow", "updated_input": { "command": "..." } }`
/// Returns `{}` when no rewrite (Cursor requires JSON on all code paths).
pub fn process_hook_cursor(input: &str) -> Result<String> {
    process_hook_for_platform(input, HookPlatform::Cursor)
}

/// Process a hook invocation for Gemini CLI.
///
/// Gemini uses: `{ "decision": "allow", "hookSpecificOutput": { "tool_input": { "command": "..." } } }`
pub fn process_hook_gemini(input: &str) -> Result<String> {
    process_hook_for_platform(input, HookPlatform::GeminiCli)
}

/// Process a hook invocation for Windsurf.
///
/// Windsurf hook support is limited. We attempt the same rewrite as Claude Code
/// but the output format may not be honored. Falls back to exit-code semantics.
pub fn process_hook_windsurf(input: &str) -> Result<String> {
    process_hook_for_platform(input, HookPlatform::Windsurf)
}

/// Platform-aware hook processing. Extracts the command from the tool-specific
/// input format, rewrites it, and returns the response in the correct format
/// for the target platform.
fn process_hook_for_platform(input: &str, platform: HookPlatform) -> Result<String> {
    let parsed: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| crate::error::SqzError::Other(format!("hook: invalid JSON input: {e}")))?;

    // Claude Code uses "tool_name" + "tool_input" (official docs).
    // Cursor uses "hook_event_name": "beforeShellExecution" with "command" at top level.
    // Some older references show "toolName" + "toolCall" — accept all.
    let tool_name = parsed
        .get("tool_name")
        .or_else(|| parsed.get("toolName"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let hook_event = parsed
        .get("hook_event_name")
        .or_else(|| parsed.get("agent_action_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Only intercept Bash/shell tool calls.
    //
    // Claude Code's built-in tools (Read, Grep, Glob, Write) bypass shell
    // hooks entirely. PostToolUse hooks can view but NOT modify their output
    // (confirmed: github.com/anthropics/claude-code/issues/4544). The tool
    // output enters the context unchanged. We can only compress Bash command
    // output by rewriting the command via PreToolUse. The MCP server
    // (sqz-mcp) provides compressed alternatives to these built-in tools.
    let is_shell = matches!(tool_name, "Bash" | "bash" | "Shell" | "shell" | "terminal"
        | "run_terminal_command" | "run_shell_command")
        || matches!(hook_event, "beforeShellExecution" | "pre_run_command");

    if !is_shell {
        // Pass through non-bash tools unchanged.
        // Cursor requires valid JSON on all code paths (empty object = passthrough).
        return Ok(match platform {
            HookPlatform::Cursor => "{}".to_string(),
            _ => input.to_string(),
        });
    }

    // Claude Code puts command in tool_input.command (official docs).
    // Cursor puts command at top level: { "command": "git status" }.
    // Windsurf puts command in tool_info.command_line.
    // Some older references show toolCall.command — accept all.
    let command = parsed
        .get("tool_input")
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .or_else(|| parsed.get("command").and_then(|v| v.as_str()))
        .or_else(|| {
            parsed
                .get("tool_info")
                .and_then(|v| v.get("command_line"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            parsed
                .get("toolCall")
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");

    if command.is_empty() {
        return Ok(match platform {
            HookPlatform::Cursor => "{}".to_string(),
            _ => input.to_string(),
        });
    }

    // Don't intercept commands that are already piped through sqz.
    // Check the base command name specifically, not substring — so
    // "grep sqz logfile" or "cargo search sqz" aren't skipped.
    let base_cmd = extract_base_command(command);
    if base_cmd == "sqz" || command.starts_with("SQZ_CMD=") {
        return Ok(match platform {
            HookPlatform::Cursor => "{}".to_string(),
            _ => input.to_string(),
        });
    }

    // Don't intercept interactive or long-running commands
    if is_interactive_command(command) {
        return Ok(match platform {
            HookPlatform::Cursor => "{}".to_string(),
            _ => input.to_string(),
        });
    }

    // Don't intercept commands with shell operators that would break piping.
    // Compound commands (&&, ||, ;), redirects (>, <, >>), background (&),
    // heredocs (<<), and process substitution would misbehave when we append
    // `2>&1 | sqz compress` — the pipe only captures the last command.
    if has_shell_operators(command) {
        return Ok(match platform {
            HookPlatform::Cursor => "{}".to_string(),
            _ => input.to_string(),
        });
    }

    // Rewrite: pipe the command's output through sqz compress.
    // The command is a simple command (no operators), so direct piping is safe.
    let rewritten = format!(
        "SQZ_CMD={} {} 2>&1 | sqz compress",
        shell_escape(extract_base_command(command)),
        command
    );

    // Build platform-specific output.
    //
    // Each AI tool expects a different JSON response format. Using the wrong
    // format causes silent failures (the tool ignores the rewrite).
    //
    // Verified against official docs + RTK codebase (github.com/rtk-ai/rtk):
    //
    // Claude Code (docs.anthropic.com/en/docs/claude-code/hooks):
    //   hookSpecificOutput.hookEventName = "PreToolUse"
    //   hookSpecificOutput.permissionDecision = "allow"
    //   hookSpecificOutput.updatedInput = { "command": "..." }  (camelCase, replaces entire input)
    //
    // Cursor (confirmed by RTK hooks/cursor/rtk-rewrite.sh):
    //   permission = "allow"
    //   updated_input = { "command": "..." }  (snake_case, flat — NOT nested in hookSpecificOutput)
    //   Returns {} when no rewrite (Cursor requires JSON on all paths)
    //
    // Gemini CLI (geminicli.com/docs/hooks/reference):
    //   decision = "allow" | "deny"  (top-level)
    //   hookSpecificOutput.tool_input = { "command": "..." }  (merged with model args)
    //
    // Codex (developers.openai.com/codex/hooks):
    //   Only "deny" works in PreToolUse. "allow", updatedInput, additionalContext
    //   are parsed but NOT supported — they fail open. RTK uses AGENTS.md instead.
    //   We do NOT generate hooks for Codex.
    let output = match platform {
        HookPlatform::ClaudeCode => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "permissionDecisionReason": "sqz: command output will be compressed for token savings",
                "updatedInput": {
                    "command": rewritten
                }
            }
        }),
        HookPlatform::Cursor => serde_json::json!({
            "permission": "allow",
            "updated_input": {
                "command": rewritten
            }
        }),
        HookPlatform::GeminiCli => serde_json::json!({
            "decision": "allow",
            "hookSpecificOutput": {
                "tool_input": {
                    "command": rewritten
                }
            }
        }),
        HookPlatform::Windsurf => {
            // Windsurf hook support is unconfirmed for command rewriting.
            // Use Claude Code format as best-effort; the hook may only work
            // via exit codes (0 = allow, 2 = block).
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "permissionDecisionReason": "sqz: command output will be compressed for token savings",
                    "updatedInput": {
                        "command": rewritten
                    }
                }
            })
        }
    };

    serde_json::to_string(&output)
        .map_err(|e| crate::error::SqzError::Other(format!("hook: JSON serialize error: {e}")))
}

/// Generate hook configuration files for all supported AI tools.
pub fn generate_hook_configs(sqz_path: &str) -> Vec<ToolHookConfig> {
    // On Windows, `sqz_path` contains backslashes (C:\Users\...\sqz.exe).
    // Embedding the raw path into JSON string values produces invalid JSON
    // because `\` must be escaped as `\\` per RFC 8259. Same rule applies
    // to JS/TS string literals. See issue #2.
    //
    // We keep TWO versions of the path:
    //   - `sqz_path_raw` — the original, shown in markdown files the user
    //     reads (.windsurfrules, .clinerules) where backslashes should
    //     render as-is so the user can copy-paste the command.
    //   - `sqz_path` — JSON/JS-escaped, used in every .json / .ts config.
    let sqz_path_raw = sqz_path;
    let sqz_path_json = json_escape_string_value(sqz_path);
    let sqz_path = &sqz_path_json;

    vec![
        // Claude Code — goes in .claude/settings.local.json (nested format)
        // Includes PreToolUse for Bash compression AND SessionStart compact
        // for re-injecting context after compaction.
        ToolHookConfig {
            tool_name: "Claude Code".to_string(),
            config_path: PathBuf::from(".claude/settings.local.json"),
            config_content: format!(
                r#"{{
  "hooks": {{
    "PreToolUse": [
      {{
        "matcher": "Bash",
        "hooks": [
          {{
            "type": "command",
            "command": "{sqz_path} hook claude"
          }}
        ]
      }}
    ],
    "SessionStart": [
      {{
        "matcher": "compact",
        "hooks": [
          {{
            "type": "command",
            "command": "{sqz_path} resume"
          }}
        ]
      }}
    ]
  }}
}}"#
            ),
            scope: HookScope::Project,
        },
        // Cursor — beforeShellExecution hook can only allow/deny/ask; it
        // does NOT support rewriting the command via `updated_input` or any
        // other field (verified against Cursor's documented schema via
        // GitButler's deep-dive and Cupcake's reference, plus an empirical
        // trace against cursor-agent 2026.04.14-ee4b43a). We use the modern
        // .cursor/rules/*.mdc format (legacy .cursorrules is deprecated per
        // docs.cursor.com/en/context/rules). `alwaysApply: true` guarantees
        // the rule loads for every agent interaction.
        ToolHookConfig {
            tool_name: "Cursor".to_string(),
            config_path: PathBuf::from(".cursor/rules/sqz.mdc"),
            config_content: format!(
                r#"---
description: sqz — Token-Optimized CLI Output
alwaysApply: true
---

# sqz — Token-Optimized CLI Output

When running shell commands whose output may be long (directory listings,
git log/diff, test runners, build logs, `docker ps`, `kubectl get`, etc.),
pipe the output through `sqz compress` to reduce token consumption.

`sqz` is a stdin-to-stdout compressor, not a command wrapper. The correct
usage is to pipe the command's output into `sqz compress`:

```bash
# Instead of:     Use:
git status        git status 2>&1 | {sqz_path_raw} compress
cargo test        cargo test 2>&1 | {sqz_path_raw} compress
git log -10       git log -10 2>&1 | {sqz_path_raw} compress
docker ps         docker ps 2>&1 | {sqz_path_raw} compress
ls -la            ls -la 2>&1 | {sqz_path_raw} compress
```

The `2>&1` captures stderr too, which is useful for commands like `cargo
test` where diagnostics go to stderr. `sqz compress` filters and compresses
the combined output while preserving filenames, paths, and identifiers.
It typically saves 60-90% tokens on verbose commands.

Do NOT pipe output for:
- Interactive commands (`vim`, `ssh`, `python`, REPLs)
- Compound commands with operators (`cmd && other`, `cmd > file.txt`,
  `cmd; other`) — run those directly
- Short commands whose output is already a few lines

If `sqz` is not on PATH, run commands normally.
"#
            ),
            scope: HookScope::Project,
        },
        // Windsurf — no confirmed hook API for command rewriting.
        // RTK uses .windsurfrules (prompt-level guidance) instead of hooks.
        // We generate a rules file that instructs Windsurf to use sqz.
        ToolHookConfig {
            tool_name: "Windsurf".to_string(),
            config_path: PathBuf::from(".windsurfrules"),
            config_content: format!(
                r#"# sqz — Token-Optimized CLI Output

Pipe verbose shell command output through `sqz compress` to save tokens.
`sqz` reads from stdin and writes the compressed output to stdout — it is
NOT a command wrapper, so `{sqz_path_raw} git status` is not valid.

```bash
# Instead of:     Use:
git status        git status 2>&1 | {sqz_path_raw} compress
cargo test        cargo test 2>&1 | {sqz_path_raw} compress
git log -10       git log -10 2>&1 | {sqz_path_raw} compress
docker ps         docker ps 2>&1 | {sqz_path_raw} compress
```

sqz filters and compresses command outputs while preserving filenames,
paths, and identifiers (typically 60-90% token reduction on verbose
commands). Skip short commands, interactive commands (vim, ssh, python),
and commands with shell operators (`&&`, `||`, `;`, `>`, `<`). If sqz is
not on PATH, run commands normally.
"#
            ),
            scope: HookScope::Project,
        },
        // Cline / Roo Code — PreToolUse cannot rewrite commands (only cancel/allow).
        // RTK uses .clinerules (prompt-level guidance) instead of hooks.
        // We generate a rules file that instructs Cline to use sqz.
        ToolHookConfig {
            tool_name: "Cline".to_string(),
            config_path: PathBuf::from(".clinerules"),
            config_content: format!(
                r#"# sqz — Token-Optimized CLI Output

Pipe verbose shell command output through `sqz compress` to save tokens.
`sqz` reads from stdin and writes the compressed output to stdout — it is
NOT a command wrapper, so `{sqz_path_raw} git status` is not valid.

```bash
# Instead of:     Use:
git status        git status 2>&1 | {sqz_path_raw} compress
cargo test        cargo test 2>&1 | {sqz_path_raw} compress
git log -10       git log -10 2>&1 | {sqz_path_raw} compress
docker ps         docker ps 2>&1 | {sqz_path_raw} compress
```

sqz filters and compresses command outputs while preserving filenames,
paths, and identifiers (typically 60-90% token reduction on verbose
commands). Skip short commands, interactive commands (vim, ssh, python),
and commands with shell operators (`&&`, `||`, `;`, `>`, `<`). If sqz is
not on PATH, run commands normally.
"#
            ),
            scope: HookScope::Project,
        },
        // Gemini CLI — goes in .gemini/settings.json (BeforeTool event)
        ToolHookConfig {
            tool_name: "Gemini CLI".to_string(),
            config_path: PathBuf::from(".gemini/settings.json"),
            config_content: format!(
                r#"{{
  "hooks": {{
    "BeforeTool": [
      {{
        "matcher": "run_shell_command",
        "hooks": [
          {{
            "type": "command",
            "command": "{sqz_path} hook gemini"
          }}
        ]
      }}
    ]
  }}
}}"#
            ),
            scope: HookScope::Project,
        },
        // OpenCode — TypeScript plugin at ~/.config/opencode/plugins/sqz.ts
        // plus opencode.json config in project root. Unlike other tools,
        // OpenCode uses a TS plugin (not JSON hooks), so we generate a
        // placeholder config here and the actual plugin is installed
        // separately via install_opencode_plugin().
        ToolHookConfig {
            tool_name: "OpenCode".to_string(),
            config_path: PathBuf::from("opencode.json"),
            config_content: format!(
                r#"{{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {{
    "sqz": {{
      "type": "local",
      "command": ["sqz-mcp", "--transport", "stdio"]
    }}
  }},
  "plugin": ["sqz"]
}}"#
            ),
            scope: HookScope::Project,
        },
    ]
}

/// Install hook configs for detected AI tools in the given project directory.
///
/// Returns the list of tools that were configured.
pub fn install_tool_hooks(project_dir: &Path, sqz_path: &str) -> Vec<String> {
    let configs = generate_hook_configs(sqz_path);
    let mut installed = Vec::new();

    for config in &configs {
        let full_path = project_dir.join(&config.config_path);

        // Don't overwrite existing hook configs
        if full_path.exists() {
            continue;
        }

        // Create parent directories
        if let Some(parent) = full_path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                continue;
            }
        }

        if std::fs::write(&full_path, &config.config_content).is_ok() {
            installed.push(config.tool_name.clone());
        }
    }

    // Also install the OpenCode TypeScript plugin (user-level)
    if let Ok(true) = crate::opencode_plugin::install_opencode_plugin(sqz_path) {
        if !installed.iter().any(|n| n == "OpenCode") {
            installed.push("OpenCode".to_string());
        }
    }

    installed
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Extract the base command name from a full command string.
fn extract_base_command(cmd: &str) -> &str {
    cmd.split_whitespace()
        .next()
        .unwrap_or("unknown")
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
}

/// Escape a string for embedding as the contents of a double-quoted JSON
/// string value (per RFC 8259). Also valid for embedding in a double-quoted
/// JavaScript/TypeScript string literal — JS string-escape rules for the
/// characters that appear in filesystem paths (`\`, `"`, control chars) are
/// a strict subset of JSON's.
///
/// Needed because hook configs embed the sqz executable path into JSON/TS
/// files via `format!`. On Windows, `current_exe()` returns
/// `C:\Users\...\sqz.exe` — the raw backslashes produce invalid JSON that
/// Claude/Cursor/Gemini fail to parse. See issue #2.
pub(crate) fn json_escape_string_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                // Other control chars: use \u00XX escape
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Shell-escape a string for use in an environment variable assignment.
fn shell_escape(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Check if a command contains shell operators that would break piping.
/// Commands with these operators are passed through uncompressed rather
/// than risk incorrect behavior.
fn has_shell_operators(cmd: &str) -> bool {
    // Check for operators that would cause the pipe to only capture
    // the last command in a chain
    cmd.contains("&&")
        || cmd.contains("||")
        || cmd.contains(';')
        || cmd.contains('>')
        || cmd.contains('<')
        || cmd.contains('|') // already has a pipe
        || cmd.contains('&') && !cmd.contains("&&") // background &
        || cmd.contains("<<")  // heredoc
        || cmd.contains("$(")  // command substitution
        || cmd.contains('`')   // backtick substitution
}

/// Check if a command is interactive or long-running (should not be intercepted).
fn is_interactive_command(cmd: &str) -> bool {
    let base = extract_base_command(cmd);
    matches!(
        base,
        "vim" | "vi" | "nano" | "emacs" | "less" | "more" | "top" | "htop"
        | "ssh" | "python" | "python3" | "node" | "irb" | "ghci"
        | "psql" | "mysql" | "sqlite3" | "mongo" | "redis-cli"
    ) || cmd.contains("--watch")
        || cmd.contains("-w ")
        || cmd.ends_with(" -w")
        || cmd.contains("run dev")
        || cmd.contains("run start")
        || cmd.contains("run serve")
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_hook_rewrites_bash_command() {
        // Use the official Claude Code input format: tool_name + tool_input
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"git status"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // Claude Code format: hookSpecificOutput with updatedInput
        let hook_output = &parsed["hookSpecificOutput"];
        assert_eq!(hook_output["hookEventName"].as_str().unwrap(), "PreToolUse");
        assert_eq!(hook_output["permissionDecision"].as_str().unwrap(), "allow");
        // updatedInput for Claude Code (camelCase)
        let cmd = hook_output["updatedInput"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "should pipe through sqz: {cmd}");
        assert!(cmd.contains("git status"), "should preserve original command: {cmd}");
        assert!(cmd.contains("SQZ_CMD=git"), "should set SQZ_CMD: {cmd}");
        // Claude Code format should NOT have top-level decision/permission/continue
        assert!(parsed.get("decision").is_none(), "Claude Code format should not have top-level decision");
        assert!(parsed.get("permission").is_none(), "Claude Code format should not have top-level permission");
        assert!(parsed.get("continue").is_none(), "Claude Code format should not have top-level continue");
    }

    #[test]
    fn test_process_hook_passes_through_non_bash() {
        let input = r#"{"tool_name":"Read","tool_input":{"file_path":"file.txt"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "non-bash tools should pass through unchanged");
    }

    #[test]
    fn test_process_hook_skips_sqz_commands() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"sqz stats"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "sqz commands should not be double-wrapped");
    }

    #[test]
    fn test_process_hook_skips_interactive() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"vim file.txt"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "interactive commands should pass through");
    }

    #[test]
    fn test_process_hook_skips_watch_mode() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"npm run dev --watch"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "watch mode should pass through");
    }

    #[test]
    fn test_process_hook_empty_command() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":""}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_process_hook_gemini_format() {
        // Gemini CLI uses tool_name + tool_input (same field names as Claude Code)
        let input = r#"{"tool_name":"run_shell_command","tool_input":{"command":"git log"}}"#;
        let result = process_hook_gemini(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // Gemini uses top-level decision (not hookSpecificOutput.permissionDecision)
        assert_eq!(parsed["decision"].as_str().unwrap(), "allow");
        // Gemini format: hookSpecificOutput.tool_input.command (NOT updatedInput)
        let cmd = parsed["hookSpecificOutput"]["tool_input"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "should pipe through sqz: {cmd}");
        // Should NOT have Claude Code fields
        assert!(parsed.get("hookSpecificOutput").unwrap().get("updatedInput").is_none(),
            "Gemini format should not have updatedInput");
        assert!(parsed.get("hookSpecificOutput").unwrap().get("permissionDecision").is_none(),
            "Gemini format should not have permissionDecision");
    }

    #[test]
    fn test_process_hook_legacy_format() {
        // Test backward compatibility with older toolName/toolCall format
        let input = r#"{"toolName":"Bash","toolCall":{"command":"git status"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["hookSpecificOutput"]["updatedInput"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "legacy format should still work: {cmd}");
    }

    #[test]
    fn test_process_hook_cursor_format() {
        // Cursor uses tool_name "Shell" + tool_input.command (same as Claude Code input)
        let input = r#"{"tool_name":"Shell","tool_input":{"command":"git status"},"conversation_id":"abc"}"#;
        let result = process_hook_cursor(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // Cursor expects flat permission + updated_input (snake_case)
        assert_eq!(parsed["permission"].as_str().unwrap(), "allow");
        let cmd = parsed["updated_input"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "cursor format should work: {cmd}");
        assert!(cmd.contains("git status"));
        // Should NOT have Claude Code hookSpecificOutput
        assert!(parsed.get("hookSpecificOutput").is_none(),
            "Cursor format should not have hookSpecificOutput");
    }

    #[test]
    fn test_process_hook_cursor_passthrough_returns_empty_json() {
        // Cursor requires {} on all code paths, even when no rewrite happens
        let input = r#"{"tool_name":"Read","tool_input":{"file_path":"file.txt"}}"#;
        let result = process_hook_cursor(input).unwrap();
        assert_eq!(result, "{}", "Cursor passthrough must return empty JSON object");
    }

    #[test]
    fn test_process_hook_cursor_no_rewrite_returns_empty_json() {
        // sqz commands should not be double-wrapped; Cursor still needs {}
        let input = r#"{"tool_name":"Shell","tool_input":{"command":"sqz stats"}}"#;
        let result = process_hook_cursor(input).unwrap();
        assert_eq!(result, "{}", "Cursor no-rewrite must return empty JSON object");
    }

    #[test]
    fn test_process_hook_windsurf_format() {
        // Windsurf uses agent_action_name + tool_info.command_line
        let input = r#"{"agent_action_name":"pre_run_command","tool_info":{"command_line":"cargo test","cwd":"/project"}}"#;
        let result = process_hook_windsurf(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // Windsurf uses Claude Code format as best-effort
        let cmd = parsed["hookSpecificOutput"]["updatedInput"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "windsurf format should work: {cmd}");
        assert!(cmd.contains("cargo test"));
        assert!(cmd.contains("SQZ_CMD=cargo"));
    }

    #[test]
    fn test_process_hook_invalid_json() {
        let result = process_hook("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_base_command() {
        assert_eq!(extract_base_command("git status"), "git");
        assert_eq!(extract_base_command("/usr/bin/git log"), "git");
        assert_eq!(extract_base_command("cargo test --release"), "cargo");
    }

    #[test]
    fn test_is_interactive_command() {
        assert!(is_interactive_command("vim file.txt"));
        assert!(is_interactive_command("npm run dev --watch"));
        assert!(is_interactive_command("python3"));
        assert!(!is_interactive_command("git status"));
        assert!(!is_interactive_command("cargo test"));
    }

    #[test]
    fn test_generate_hook_configs() {
        let configs = generate_hook_configs("sqz");
        assert!(configs.len() >= 5, "should generate configs for multiple tools (including OpenCode)");
        assert!(configs.iter().any(|c| c.tool_name == "Claude Code"));
        assert!(configs.iter().any(|c| c.tool_name == "Cursor"));
        assert!(configs.iter().any(|c| c.tool_name == "OpenCode"));
        // Windsurf, Cline, and Cursor should generate rules files, not hook configs
        // (none of the three support transparent command rewriting via hooks).
        let windsurf = configs.iter().find(|c| c.tool_name == "Windsurf").unwrap();
        assert_eq!(windsurf.config_path, PathBuf::from(".windsurfrules"),
            "Windsurf should use .windsurfrules, not .windsurf/hooks.json");
        let cline = configs.iter().find(|c| c.tool_name == "Cline").unwrap();
        assert_eq!(cline.config_path, PathBuf::from(".clinerules"),
            "Cline should use .clinerules, not .clinerules/hooks/PreToolUse");
        // Cursor — empirically verified (forum/Cupcake/GitButler docs +
        // live cursor-agent trace) that beforeShellExecution cannot rewrite
        // commands. Use the modern .cursor/rules/*.mdc format.
        let cursor = configs.iter().find(|c| c.tool_name == "Cursor").unwrap();
        assert_eq!(cursor.config_path, PathBuf::from(".cursor/rules/sqz.mdc"),
            "Cursor should use .cursor/rules/sqz.mdc (modern rules), not \
             .cursor/hooks.json (non-functional) or .cursorrules (legacy)");
        assert!(cursor.config_content.starts_with("---"),
            "Cursor rule should start with YAML frontmatter");
        assert!(cursor.config_content.contains("alwaysApply: true"),
            "Cursor rule should use alwaysApply: true so the guidance loads \
             for every agent interaction");
        assert!(cursor.config_content.contains("sqz"),
            "Cursor rule body should mention sqz");
    }

    // ── Issue #2: Windows path escaping in hook configs ───────────────

    #[test]
    fn test_json_escape_string_value() {
        // Plain ASCII: unchanged
        assert_eq!(json_escape_string_value("sqz"), "sqz");
        assert_eq!(json_escape_string_value("/usr/local/bin/sqz"), "/usr/local/bin/sqz");
        // Backslash: escaped
        assert_eq!(json_escape_string_value(r"C:\Users\Alice\sqz.exe"),
                   r"C:\\Users\\Alice\\sqz.exe");
        // Double quote: escaped
        assert_eq!(json_escape_string_value(r#"path with "quotes""#),
                   r#"path with \"quotes\""#);
        // Control chars
        assert_eq!(json_escape_string_value("a\nb\tc"), r"a\nb\tc");
    }

    #[test]
    fn test_windows_path_produces_valid_json_for_claude() {
        // Issue #2 repro: on Windows, current_exe() returns a path with
        // backslashes. Without escaping, the generated JSON is invalid.
        let windows_path = r"C:\Users\SqzUser\.cargo\bin\sqz.exe";
        let configs = generate_hook_configs(windows_path);

        let claude = configs.iter().find(|c| c.tool_name == "Claude Code")
            .expect("Claude config should be generated");
        let parsed: serde_json::Value = serde_json::from_str(&claude.config_content)
            .expect("Claude hook config must be valid JSON on Windows paths");

        // Verify the command was written with the original path (not lossy-transformed).
        let cmd = parsed["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command field must be a string");
        assert!(cmd.contains(windows_path),
            "command '{cmd}' must contain the original Windows path '{windows_path}'");
    }

    #[test]
    fn test_windows_path_in_cursor_rules_file() {
        // Cursor's config is now .cursor/rules/sqz.mdc (markdown), not JSON.
        // Markdown doesn't escape backslashes — the user reads this rule
        // through the agent and needs to see the raw path so commands are
        // pasteable. See test_rules_files_use_raw_path_for_readability for
        // the same property on Windsurf/Cline.
        let windows_path = r"C:\Users\SqzUser\.cargo\bin\sqz.exe";
        let configs = generate_hook_configs(windows_path);

        let cursor = configs.iter().find(|c| c.tool_name == "Cursor").unwrap();
        assert_eq!(cursor.config_path, PathBuf::from(".cursor/rules/sqz.mdc"));
        assert!(cursor.config_content.contains(windows_path),
            "Cursor rule must contain the raw (unescaped) path so users can \
             copy-paste the shown commands — got:\n{}", cursor.config_content);
        assert!(!cursor.config_content.contains(r"C:\\Users"),
            "Cursor rule must NOT double-escape backslashes in markdown — \
             got:\n{}", cursor.config_content);
    }

    #[test]
    fn test_windows_path_produces_valid_json_for_gemini() {
        let windows_path = r"C:\Users\SqzUser\.cargo\bin\sqz.exe";
        let configs = generate_hook_configs(windows_path);

        let gemini = configs.iter().find(|c| c.tool_name == "Gemini CLI").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&gemini.config_content)
            .expect("Gemini hook config must be valid JSON on Windows paths");
        let cmd = parsed["hooks"]["BeforeTool"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.contains(windows_path));
    }

    #[test]
    fn test_rules_files_use_raw_path_for_readability() {
        // The .windsurfrules / .clinerules / .cursor/rules/sqz.mdc files are
        // markdown for humans. Backslashes should NOT be doubled there — the
        // user needs to copy-paste the command into their shell.
        let windows_path = r"C:\Users\SqzUser\.cargo\bin\sqz.exe";
        let configs = generate_hook_configs(windows_path);

        for tool in &["Windsurf", "Cline", "Cursor"] {
            let cfg = configs.iter().find(|c| &c.tool_name == tool).unwrap();
            assert!(cfg.config_content.contains(windows_path),
                "{tool} rules file must contain the raw (unescaped) path — got:\n{}",
                cfg.config_content);
            assert!(!cfg.config_content.contains(r"C:\\Users"),
                "{tool} rules file must NOT double-escape backslashes — got:\n{}",
                cfg.config_content);
        }
    }

    #[test]
    fn test_unix_path_still_works() {
        // Regression: make sure the escape path doesn't mangle Unix paths
        // (which have no backslashes to escape).
        let unix_path = "/usr/local/bin/sqz";
        let configs = generate_hook_configs(unix_path);

        let claude = configs.iter().find(|c| c.tool_name == "Claude Code").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&claude.config_content)
            .expect("Unix path should produce valid JSON");
        let cmd = parsed["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(cmd, "/usr/local/bin/sqz hook claude");
    }

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("git"), "git");
        assert_eq!(shell_escape("cargo-test"), "cargo-test");
    }

    #[test]
    fn test_shell_escape_special_chars() {
        assert_eq!(shell_escape("git log --oneline"), "'git log --oneline'");
    }

    #[test]
    fn test_install_tool_hooks_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let installed = install_tool_hooks(dir.path(), "sqz");
        // Should install at least some hooks
        assert!(!installed.is_empty(), "should install at least one hook config");
        // Verify files were created
        for name in &installed {
            let configs = generate_hook_configs("sqz");
            let config = configs.iter().find(|c| &c.tool_name == name).unwrap();
            let path = dir.path().join(&config.config_path);
            assert!(path.exists(), "hook config should exist: {}", path.display());
        }
    }

    #[test]
    fn test_install_tool_hooks_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        // First install
        install_tool_hooks(dir.path(), "sqz");
        // Write a custom file to one of the paths
        let custom_path = dir.path().join(".claude/settings.local.json");
        std::fs::write(&custom_path, "custom content").unwrap();
        // Second install should not overwrite
        install_tool_hooks(dir.path(), "sqz");
        let content = std::fs::read_to_string(&custom_path).unwrap();
        assert_eq!(content, "custom content", "should not overwrite existing config");
    }
}
