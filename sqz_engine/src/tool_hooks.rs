/// PreToolUse hook integration for AI coding tools.
///
/// Provides transparent command interception: when an AI tool (Claude Code,
/// Cursor, Copilot, etc.) executes a bash command, the hook rewrites it to
/// pipe output through sqz for compression. The AI tool never knows it
/// happened — it just sees smaller output.
///
/// Supported hook formats:
/// - Claude Code: .claude/settings.local.json (nested PreToolUse, matcher: "Bash")
/// - Cursor: .cursor/hooks.json (nested PreToolUse, matcher: "Bash")
/// - Windsurf: .windsurf/hooks.json (nested PreToolUse, matcher: "Bash")
/// - Gemini CLI: .gemini/settings.json (nested BeforeTool, matcher: "run_shell_command")
/// - Cline: .cline/hooks.json (nested PreToolUse, matcher: "Bash")
/// - OpenCode: ~/.config/opencode/plugins/sqz.ts (TypeScript plugin, tool.execute.before)

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
    let parsed: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| crate::error::SqzError::Other(format!("hook: invalid JSON input: {e}")))?;

    // Claude Code uses "toolName" + "toolCall", Gemini uses "tool_name" + "tool_input",
    // Cursor uses "toolName" or "tool_name" depending on version.
    let tool_name = parsed
        .get("toolName")
        .or_else(|| parsed.get("tool_name"))
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
    if !matches!(tool_name, "Bash" | "bash" | "shell" | "terminal"
        | "run_terminal_command" | "run_shell_command") {
        // Pass through non-bash tools unchanged
        return Ok(input.to_string());
    }

    // Claude Code puts command in toolCall.command, Gemini in tool_input.command
    let command = parsed
        .get("toolCall")
        .or_else(|| parsed.get("tool_input"))
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if command.is_empty() {
        return Ok(input.to_string());
    }

    // Don't intercept commands that are already piped through sqz
    if command.contains("sqz") || command.contains("SQZ_CMD") {
        return Ok(input.to_string());
    }

    // Don't intercept interactive or long-running commands
    if is_interactive_command(command) {
        return Ok(input.to_string());
    }

    // Rewrite: pipe the command's output through sqz compress
    let rewritten = format!(
        "SQZ_CMD={} {} 2>&1 | sqz compress",
        shell_escape(extract_base_command(command)),
        command
    );

    // Build the output in the format the tool expects.
    // Claude Code expects: { "decision": "approve", "updatedInput": { "command": "..." } }
    // Gemini expects: { "hookSpecificOutput": { "tool_input": { "command": "..." } } }
    // For maximum compatibility, output both formats.
    let output = serde_json::json!({
        "decision": "approve",
        "reason": "sqz: command output will be compressed for token savings",
        "updatedInput": {
            "command": rewritten
        },
        "hookSpecificOutput": {
            "tool_input": {
                "command": rewritten
            }
        }
    });

    serde_json::to_string(&output)
        .map_err(|e| crate::error::SqzError::Other(format!("hook: JSON serialize error: {e}")))
}

/// Generate hook configuration files for all supported AI tools.
pub fn generate_hook_configs(sqz_path: &str) -> Vec<ToolHookConfig> {
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
        // Cursor
        ToolHookConfig {
            tool_name: "Cursor".to_string(),
            config_path: PathBuf::from(".cursor/hooks.json"),
            config_content: format!(
                r#"{{
  "hooks": {{
    "PreToolUse": [
      {{
        "matcher": "Bash",
        "hooks": [
          {{
            "type": "command",
            "command": "{sqz_path} hook cursor"
          }}
        ]
      }}
    ]
  }}
}}"#
            ),
            scope: HookScope::Project,
        },
        // Windsurf
        ToolHookConfig {
            tool_name: "Windsurf".to_string(),
            config_path: PathBuf::from(".windsurf/hooks.json"),
            config_content: format!(
                r#"{{
  "hooks": {{
    "PreToolUse": [
      {{
        "matcher": "Bash",
        "hooks": [
          {{
            "type": "command",
            "command": "{sqz_path} hook windsurf"
          }}
        ]
      }}
    ]
  }}
}}"#
            ),
            scope: HookScope::Project,
        },
        // Cline
        ToolHookConfig {
            tool_name: "Cline".to_string(),
            config_path: PathBuf::from(".cline/hooks.json"),
            config_content: format!(
                r#"{{
  "hooks": {{
    "PreToolUse": [
      {{
        "matcher": "Bash",
        "hooks": [
          {{
            "type": "command",
            "command": "{sqz_path} hook cline"
          }}
        ]
      }}
    ]
  }}
}}"#
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

/// Shell-escape a string for use in an environment variable assignment.
fn shell_escape(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
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
        let input = r#"{"toolName":"Bash","toolCall":{"command":"git status"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["decision"].as_str().unwrap(), "approve");
        let cmd = parsed["updatedInput"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "should pipe through sqz: {cmd}");
        assert!(cmd.contains("git status"), "should preserve original command: {cmd}");
        assert!(cmd.contains("SQZ_CMD=git"), "should set SQZ_CMD: {cmd}");
    }

    #[test]
    fn test_process_hook_passes_through_non_bash() {
        let input = r#"{"toolName":"Read","toolCall":{"path":"file.txt"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "non-bash tools should pass through unchanged");
    }

    #[test]
    fn test_process_hook_skips_sqz_commands() {
        let input = r#"{"toolName":"Bash","toolCall":{"command":"sqz stats"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "sqz commands should not be double-wrapped");
    }

    #[test]
    fn test_process_hook_skips_interactive() {
        let input = r#"{"toolName":"Bash","toolCall":{"command":"vim file.txt"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "interactive commands should pass through");
    }

    #[test]
    fn test_process_hook_skips_watch_mode() {
        let input = r#"{"toolName":"Bash","toolCall":{"command":"npm run dev --watch"}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input, "watch mode should pass through");
    }

    #[test]
    fn test_process_hook_empty_command() {
        let input = r#"{"toolName":"Bash","toolCall":{"command":""}}"#;
        let result = process_hook(input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_process_hook_gemini_format() {
        // Gemini CLI uses tool_name + tool_input
        let input = r#"{"tool_name":"run_shell_command","tool_input":{"command":"git log"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["decision"].as_str().unwrap(), "approve");
        let cmd = parsed["hookSpecificOutput"]["tool_input"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "should pipe through sqz: {cmd}");
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
