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

  return {{
    "tool.execute.before": async (input: any, output: any) => {{
      const tool = input.tool ?? "";
      if (!shouldIntercept(tool)) return;

      const cmd = output.args?.command ?? "";
      if (!cmd || cmd.includes("sqz") || isInteractive(cmd)) return;

      // Rewrite: pipe through sqz compress
      const base = cmd.split(/\s+/)[0]?.split("/").pop() ?? "unknown";
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

    if command.is_empty() || command.contains("sqz") {
        return Ok(input.to_string());
    }

    // Check for interactive commands
    let base = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("");

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
    let base_cmd = command
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .rsplit('/')
        .next()
        .unwrap_or("unknown");

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
    fn test_generate_opencode_plugin_has_interactive_check() {
        let content = generate_opencode_plugin("sqz");
        assert!(content.contains("isInteractive"));
        assert!(content.contains("vim"));
        assert!(content.contains("--watch"));
    }

    #[test]
    fn test_generate_opencode_plugin_has_sqz_guard() {
        let content = generate_opencode_plugin("sqz");
        assert!(
            content.contains(r#"cmd.includes("sqz")"#),
            "should skip commands already containing sqz"
        );
    }

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
}
