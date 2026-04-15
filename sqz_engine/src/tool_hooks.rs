/// PreToolUse hook integration for AI coding tools.
///
/// Provides transparent command interception: when an AI tool (Claude Code,
/// Cursor, Copilot, etc.) executes a bash command, the hook rewrites it to
/// pipe output through sqz for compression. The AI tool never knows it
/// happened — it just sees smaller output.
///
/// Supported hook formats:
/// - Claude Code: ~/.claude/hooks/hooks.json (PreToolUse matcher)
/// - Cursor: .cursor/hooks.json
/// - Copilot: .github/copilot-hooks.json
/// - Windsurf: .windsurfrules (PreToolUse)
/// - Gemini CLI: .gemini/settings.json (BeforeTool)
/// - Cline: .cline/hooks.json

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

    let tool_name = parsed
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Only intercept Bash/shell tool calls
    if !matches!(tool_name, "Bash" | "bash" | "shell" | "terminal" | "run_terminal_command") {
        // Pass through non-bash tools unchanged
        return Ok(input.to_string());
    }

    let command = parsed
        .get("tool_input")
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

    // Build the modified payload
    let mut output = parsed.clone();
    if let Some(tool_input) = output.get_mut("tool_input") {
        if let Some(cmd) = tool_input.get_mut("command") {
            *cmd = serde_json::Value::String(rewritten);
        }
    }

    serde_json::to_string(&output)
        .map_err(|e| crate::error::SqzError::Other(format!("hook: JSON serialize error: {e}")))
}

/// Generate hook configuration files for all supported AI tools.
pub fn generate_hook_configs(sqz_path: &str) -> Vec<ToolHookConfig> {
    vec![
        // Claude Code
        ToolHookConfig {
            tool_name: "Claude Code".to_string(),
            config_path: PathBuf::from(".claude/hooks/sqz.json"),
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
    "preToolUse": [
      {{
        "matcher": "terminal",
        "command": "{sqz_path} hook cursor"
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
    "preToolUse": [
      {{
        "matcher": "terminal",
        "command": "{sqz_path} hook windsurf"
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
    "preToolUse": [
      {{
        "matcher": "execute_command",
        "command": "{sqz_path} hook cline"
      }}
    ]
  }}
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
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"git status"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["tool_input"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "should pipe through sqz: {cmd}");
        assert!(cmd.contains("git status"), "should preserve original command: {cmd}");
        assert!(cmd.contains("SQZ_CMD=git"), "should set SQZ_CMD: {cmd}");
    }

    #[test]
    fn test_process_hook_passes_through_non_bash() {
        let input = r#"{"tool_name":"Read","tool_input":{"path":"file.txt"}}"#;
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
        assert!(configs.len() >= 4, "should generate configs for multiple tools");
        assert!(configs.iter().any(|c| c.tool_name == "Claude Code"));
        assert!(configs.iter().any(|c| c.tool_name == "Cursor"));
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
        let custom_path = dir.path().join(".claude/hooks/sqz.json");
        std::fs::write(&custom_path, "custom content").unwrap();
        // Second install should not overwrite
        install_tool_hooks(dir.path(), "sqz");
        let content = std::fs::read_to_string(&custom_path).unwrap();
        assert_eq!(content, "custom content", "should not overwrite existing config");
    }
}
