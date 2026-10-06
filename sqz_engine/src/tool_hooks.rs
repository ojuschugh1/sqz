/// PreToolUse hook integration for AI coding tools.
///
/// Provides transparent command interception: when an AI tool (Claude Code,
/// Cursor, Copilot, etc.) executes a bash command, the hook rewrites it to
/// pipe output through sqz for compression. The AI tool never knows it
/// happened — it just sees smaller output.
///
/// Supported hook formats (tools that support command rewriting via hooks):
/// - Claude Code: .claude/settings.local.json or ~/.claude/settings.json (nested
///   PreToolUse, matcher "Bash|PowerShell", plus PostToolUse for Bash and
///   two allow rules for the parts the rewrite adds)
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
    /// Kiro: STDOUT with rewritten tool_input JSON, exit 0 = allow
    Kiro,
    /// Copilot CLI: { permissionDecision, modifiedArgs } (camelCase input:
    /// toolName + toolArgs)
    Copilot,
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

/// Process a hook invocation for Kiro (IDE and CLI).
///
/// Kiro's PreToolUse hook receives JSON on STDIN with `tool_name` and
/// `tool_input`. The hook outputs the modified tool_input JSON to STDOUT.
/// Exit code 0 = allow (proceed with modified input).
/// Exit code 2 = block.
///
/// Kiro uses `execute_bash` (alias `shell`) as the tool name for shell
/// commands, with `tool_input.command` containing the command string.
pub fn process_hook_kiro(input: &str) -> Result<String> {
    process_hook_for_platform(input, HookPlatform::Kiro)
}

/// Process a hook invocation for GitHub Copilot CLI.
///
/// Copilot's camelCase preToolUse payload carries `toolName` + `toolArgs`;
/// the response uses `permissionDecision` + `modifiedArgs`, which
/// substitutes the tool arguments wholesale, so unchanged fields are
/// carried over. Shell tools are `bash` and `powershell`.
pub fn process_hook_copilot(input: &str) -> Result<String> {
    process_hook_for_platform(input, HookPlatform::Copilot)
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
    // hooks entirely. A PostToolUse hook can now replace a result, but the
    // replacement must match each tool's own result shape and Read's
    // feeds Claude Code's edit tracking, so only Bash results are
    // replaced there (`claude_post_tool_use_target`). The MCP server
    // (sqz-mcp) provides compressed alternatives to the built-in tools.
    let is_shell = matches!(tool_name, "Bash" | "bash" | "Shell" | "shell" | "terminal"
        | "run_terminal_command" | "run_shell_command" | "execute_bash"
        | "PowerShell" | "powershell" | "pwsh")
        || matches!(hook_event, "beforeShellExecution" | "pre_run_command" | "preToolUse");

    if !is_shell {
        // Pass through non-bash tools unchanged.
        // Cursor requires valid JSON on all code paths (empty object = passthrough).
        return Ok(match platform {
            HookPlatform::Cursor | HookPlatform::Copilot => "{}".to_string(),
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
        .or_else(|| {
            // Copilot CLI camelCase payload: toolArgs.command
            parsed
                .get("toolArgs")
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");

    if command.is_empty() {
        return Ok(match platform {
            HookPlatform::Cursor | HookPlatform::Copilot => "{}".to_string(),
            _ => input.to_string(),
        });
    }

    // Don't intercept commands that are already piped through sqz.
    // Check the base command name specifically, not substring — so
    // "grep sqz logfile" or "cargo search sqz" aren't skipped.
    //
    // Guards:
    //   * base command is `sqz` (running sqz directly)
    //   * legacy `SQZ_CMD=…` sh-style prefix (from older sqz versions)
    //   * new shell-neutral `--cmd NAME` form — we need this because
    //     the new emission (issue #10 fix) uses `| sqz compress --cmd …`
    //     and without a guard the hook would re-wrap commands it had
    //     already rewritten once (runaway-prefix bug from issue #5).
    let base_cmd = extract_base_command(command);
    if base_cmd == "sqz"
        || command.starts_with("SQZ_CMD=")
        || command.contains("sqz compress --cmd ")
        || command.contains("sqz.exe compress --cmd ")
    {
        return Ok(match platform {
            HookPlatform::Cursor | HookPlatform::Copilot => "{}".to_string(),
            _ => input.to_string(),
        });
    }

    // A background call streams its output to the agent while it runs;
    // through sqz nothing would show until the command exits.
    if runs_in_background(&parsed) {
        return Ok(passthrough(platform, input));
    }

    // Rewrite: pipe the command's output through sqz compress.
    //
    // Issue #10: use `--cmd NAME` instead of a `SQZ_CMD=NAME` prefix so
    // the rewrite works in every shell. The sh-style inline env-var
    // assignment doesn't parse in PowerShell (the reporter's default on
    // Windows) or cmd.exe — both treat `SQZ_CMD=cmd` as a literal
    // command name and raise `CommandNotFoundException`. `--cmd NAME`
    // is a normal CLI argument and all three shells parse it fine.
    let is_powershell = matches!(tool_name, "PowerShell" | "powershell" | "pwsh");
    let rewritten = if platform == HookPlatform::ClaudeCode {
        let settings = claude_settings_files(&parsed);
        let shell = crate::shell_scan::Shell::from_env();
        match claude_rewrite(&parsed, command, is_powershell, &settings, shell) {
            Some(rewritten) => rewritten,
            None => return Ok(passthrough(platform, input)),
        }
    } else if is_powershell {
        // PowerShell pipes objects, so only plain commands are rewritten.
        if is_interactive_command(command) || has_shell_operators(command) {
            return Ok(passthrough(platform, input));
        }
        crate::exit_marker::powershell_rewrite(command, &shell_escape(command))
    } else {
        match crate::shell_scan::plan(command) {
            Ok(plan) => posix_rewrite_plan(&plan),
            Err(_) => return Ok(passthrough(platform, input)),
        }
    };

    // Claude Code's updatedInput and Gemini's tool_input both REPLACE the
    // arguments, so unchanged fields like run_in_background, timeout,
    // description or dir_path must be carried over or they are silently
    // dropped. Start from the original tool_input and swap only the command.
    let mut updated_input = parsed
        .get("tool_input")
        .or_else(|| parsed.get("toolArgs"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    updated_input.insert(
        "command".to_string(),
        serde_json::Value::String(rewritten.clone()),
    );
    let updated_input = serde_json::Value::Object(updated_input);

    // Build platform-specific output.
    //
    // Each AI tool expects a different JSON response format. Using the wrong
    // format causes silent failures (the tool ignores the rewrite).
    //
    // No output claims a permission decision. In Claude Code and Copilot CLI
    // an "allow" skips the approval prompt, so the user's own rules and
    // prompts would stop applying to every command sqz rewrites. Without a
    // decision the host applies the new input and runs its normal checks.
    //
    // Claude Code (code.claude.com/docs/en/hooks):
    //   hookSpecificOutput.hookEventName = "PreToolUse"
    //   hookSpecificOutput.updatedInput = { ... }  (replaces the entire input)
    //
    // Cursor (confirmed against working hook configs in the wild):
    //   permission = "allow"
    //   updated_input = { "command": "..." }  (snake_case, flat — NOT nested in hookSpecificOutput)
    //   Returns {} when no rewrite (Cursor requires JSON on all paths)
    //
    // Gemini CLI (geminicli.com/docs/hooks/reference):
    //   hookSpecificOutput.tool_input = { ... }. The scheduler replaces the
    //   call's arguments with it rather than merging, so it carries every
    //   original field.
    //
    // Codex (developers.openai.com/codex/hooks):
    //   Only "deny" works in PreToolUse. "allow", updatedInput, additionalContext
    //   are parsed but NOT supported — they fail open. AGENTS.md guidance is
    //   the working alternative; we do NOT generate hooks for Codex.
    let output = match platform {
        HookPlatform::ClaudeCode => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "updatedInput": updated_input
            }
        }),
        HookPlatform::Cursor => serde_json::json!({
            "permission": "allow",
            "updated_input": {
                "command": rewritten
            }
        }),
        HookPlatform::GeminiCli => serde_json::json!({
            "hookSpecificOutput": {
                "tool_input": updated_input
            }
        }),
        HookPlatform::Windsurf => {
            // Windsurf hook support is unconfirmed for command rewriting.
            // Use Claude Code format as best-effort; the hook may only work
            // via exit codes (0 = allow, 2 = block).
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "updatedInput": updated_input
                }
            })
        }
        HookPlatform::Copilot => serde_json::json!({
            "modifiedArgs": updated_input
        }),
        HookPlatform::Kiro => {
            // Kiro CLI/IDE: output the modified tool_input as JSON to STDOUT.
            // Exit code 0 means "allow with modifications".
            // The hook runner merges this back into the tool call.
            serde_json::json!({
                "tool_input": {
                    "command": rewritten
                }
            })
        }
    };

    serde_json::to_string(&output)
        .map_err(|e| crate::error::SqzError::Other(format!("hook: JSON serialize error: {e}")))
}

const SQZ_RULES_SENTINEL: &str = "<!-- sqz-rules (auto-installed by sqz init) -->";
const LEGACY_RULES_HEADING: &str = "# sqz \u{2014} Token-Optimized CLI Output";
const LEGACY_CURSOR_FRONTMATTER: &str = "---\ndescription: sqz \u{2014} Token-Optimized CLI Output\n";
const LEGACY_RULES_LAST_LINES: [&str; 3] = [
    "path is still what you want for most output.",
    "not on PATH, run commands normally.",
    "If sqz is not available, run commands normally.",
];

pub fn sqz_rules_content(tool_name: &str, sqz_path: &str) -> String {
    if tool_name == "Cline" {
        return crate::codex_integration::agents_md_guidance_block(sqz_path);
    }
    let body = format!(
        "{SQZ_RULES_SENTINEL}\n\n{}",
        crate::codex_integration::sqz_guidance(sqz_path)
    );
    if tool_name == "Cursor" {
        format!("---\ndescription: sqz output compression\nalwaysApply: true\n---\n{body}")
    } else {
        body
    }
}

fn is_sqz_rules(content: &str) -> bool {
    content.contains(SQZ_RULES_SENTINEL)
        || content.starts_with(LEGACY_RULES_HEADING)
        || content.starts_with(LEGACY_CURSOR_FRONTMATTER)
}

fn legacy_rules_end(content: &str) -> Option<usize> {
    if !content.starts_with(LEGACY_RULES_HEADING) {
        return None;
    }
    LEGACY_RULES_LAST_LINES.iter().find_map(|last| {
        let mut start = 0;
        content.split_inclusive('\n').find_map(|line| {
            let text = line.trim_end();
            let end = start + text.len();
            start += line.len();
            text.ends_with(last).then_some(end)
        })
    })
}

fn without_sqz_rules(content: &str) -> Option<String> {
    match legacy_rules_end(content) {
        Some(end) => Some(content[end..].trim_start_matches(['\r', '\n']).to_string()),
        None => crate::codex_integration::without_guidance(content),
    }
}

pub fn has_sqz_rules(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| without_sqz_rules(&content))
        .is_some()
}

pub fn remove_sqz_rules(path: &Path) -> Result<bool> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(false);
    };
    let Some(rest) = without_sqz_rules(&content) else {
        return Ok(false);
    };
    if rest.trim().is_empty() {
        std::fs::remove_file(path).map_err(|e| {
            crate::error::SqzError::Other(format!("remove {}: {e}", path.display()))
        })?;
    } else {
        std::fs::write(path, rest)
            .map_err(|e| crate::error::SqzError::Other(format!("write {}: {e}", path.display())))?;
    }
    Ok(true)
}

pub fn sqz_rules_path(project_dir: &Path, tool_name: &str) -> Option<PathBuf> {
    match tool_name {
        "Cursor" => Some(project_dir.join(".cursor").join("rules").join("sqz.mdc")),
        "Windsurf" => Some(project_dir.join(".windsurf").join("rules").join("sqz.md")),
        "Cline" => {
            let base = project_dir.join(".clinerules");
            Some(if base.is_dir() { base.join("sqz.md") } else { base })
        }
        _ => None,
    }
}

pub fn sqz_rules_state(path: &Path, tool_name: &str) -> crate::kiro_integration::SteeringState {
    use crate::kiro_integration::SteeringState;
    let Ok(content) = std::fs::read_to_string(path) else {
        return SteeringState::Missing;
    };
    if tool_name == "Cline" {
        return if legacy_rules_end(&content).is_some() {
            SteeringState::Stale
        } else {
            crate::codex_integration::guidance_state(&content)
        };
    }
    if !is_sqz_rules(&content) {
        return SteeringState::UserAuthored;
    }
    match crate::kiro_integration::embedded_sqz_path(&content) {
        Some(p) if content == sqz_rules_content(tool_name, p) => SteeringState::Current,
        _ => SteeringState::Stale,
    }
}

pub fn refresh_sqz_rules(path: &Path, tool_name: &str, fallback_sqz_path: &str) -> Result<bool> {
    if sqz_rules_state(path, tool_name) != crate::kiro_integration::SteeringState::Stale {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(path)
        .map_err(|e| crate::error::SqzError::Other(format!("read {}: {e}", path.display())))?;
    if tool_name == "Cline" && legacy_rules_end(&existing).is_none() {
        return crate::codex_integration::refresh_agents_md_guidance(path, fallback_sqz_path);
    }
    let sqz_path = crate::kiro_integration::embedded_sqz_path(&existing)
        .filter(|p| Path::new(p).is_file())
        .unwrap_or(fallback_sqz_path)
        .to_string();
    install_sqz_rules(path, tool_name, &sqz_path)
}

pub fn sqz_rules_change(
    path: &Path,
    tool_name: &str,
    sqz_path: &str,
) -> crate::codex_integration::GuidanceChange {
    use crate::codex_integration::GuidanceChange;
    match rules_update(path, tool_name, sqz_path) {
        None => GuidanceChange::Unchanged,
        Some(_) if path.exists() => GuidanceChange::Replace,
        Some(_) => GuidanceChange::Create,
    }
}

fn rules_update(path: &Path, tool_name: &str, sqz_path: &str) -> Option<String> {
    let content = sqz_rules_content(tool_name, sqz_path);
    let Ok(existing) = std::fs::read_to_string(path) else {
        return (!path.exists()).then_some(content);
    };
    let updated = if tool_name != "Cline" {
        if !is_sqz_rules(&existing) {
            return None;
        }
        content
    } else if let Some(end) = legacy_rules_end(&existing) {
        format!("{}{}", content.trim_end_matches('\n'), &existing[end..])
    } else {
        crate::codex_integration::replaced_guidance(&existing, sqz_path)?
    };
    (updated != existing).then_some(updated)
}

fn install_sqz_rules(path: &Path, tool_name: &str, sqz_path: &str) -> Result<bool> {
    let Some(content) = rules_update(path, tool_name, sqz_path) else {
        return Ok(false);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::error::SqzError::Other(format!("create {}: {e}", parent.display()))
        })?;
    }
    std::fs::write(path, content)
        .map_err(|e| crate::error::SqzError::Other(format!("write {}: {e}", path.display())))?;
    Ok(true)
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
        // Four hooks, each addressing a different concern, plus the two
        // allow rules the PreToolUse rewrite needs:
        //
        //   PreToolUse:   compress shell tool output before the agent sees it
        //                 (matcher "Bash|PowerShell" covers both Unix and
        //                 Windows shells without firing on Read/Edit/etc.)
        //   PostToolUse:  compress the output of a successful Bash command
        //                 the PreToolUse hook left alone
        //   PreCompact:   mark sqz's dedup refs stale before Claude Code
        //                 summarises older turns. Otherwise our §ref:HASH§
        //                 tokens would outlive the content they pointed at,
        //                 leading to dangling refs the agent can't resolve.
        //                 Documented by Anthropic at
        //                 docs.anthropic.com/en/docs/claude-code/hooks-guide.
        //   SessionStart: if the session was resumed via /compact, re-inject
        //                 sqz's session guide (handled by `sqz resume`).
        ToolHookConfig {
            tool_name: "Claude Code".to_string(),
            config_path: PathBuf::from(".claude/settings.local.json"),
            config_content: claude_settings_content(sqz_path_raw),
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
            config_content: sqz_rules_content("Cursor", sqz_path_raw),
            scope: HookScope::Project,
        },
        // Windsurf — no hook API; prompt-level rules. Current docs only
        // document the .windsurf/rules/ directory; .windsurfrules is legacy.
        ToolHookConfig {
            tool_name: "Windsurf".to_string(),
            config_path: PathBuf::from(".windsurf/rules/sqz.md"),
            config_content: sqz_rules_content("Windsurf", sqz_path_raw),
            scope: HookScope::Project,
        },
        // Cline / Roo Code — prompt-level guidance. .clinerules can be a
        // single file or a directory; the install path handles both.
        ToolHookConfig {
            tool_name: "Cline".to_string(),
            config_path: PathBuf::from(".clinerules"),
            config_content: sqz_rules_content("Cline", sqz_path_raw),
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
        // Kiro — steering file + workspace MCP registration. Kiro's
        // PreToolUse hooks can't rewrite tool input, so the old hook-based
        // integration never worked in Kiro chat; the real install goes
        // through crate::kiro_integration.
        ToolHookConfig {
            tool_name: "Kiro".to_string(),
            config_path: PathBuf::from(".kiro/steering/sqz.md"),
            config_content: crate::kiro_integration::kiro_steering_content(sqz_path_raw),
            scope: HookScope::Project,
        },
        // OpenCode — TypeScript plugin at ~/.config/opencode/plugins/sqz.ts
        // plus a config file in project root (opencode.json or
        // opencode.jsonc). Unlike other tools, OpenCode uses a TS
        // plugin (not JSON hooks). The `config_path` below is the
        // fresh-install default; `install_tool_hooks` detects a
        // pre-existing `.jsonc` and merges into it instead. The actual
        // plugin (sqz.ts) is installed separately via
        // `install_opencode_plugin()`.
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
        // Codex (openai/codex) — no stable per-tool-call hook, only a
        // turn-end `notify` that fires after the agent is done and can't
        // rewrite tool output. Native integration is therefore two-part:
        //
        //   1. AGENTS.md at project root — prompt-level guidance telling
        //      Codex to pipe shell output through `sqz compress`. This is
        //      the shape Codex expects (the cross-tool AGENTS.md standard).
        //   2. ~/.codex/config.toml user-level [mcp_servers.sqz] — Codex
        //      merges this with any existing entries. Handled specially
        //      in `install_tool_hooks` via `install_codex_mcp_config`.
        //
        // The config_content below is the AGENTS.md guidance block; it
        // is only used as a placeholder for the (project-level) file and
        // for surfacing the "create AGENTS.md" line in the install plan.
        // The actual install goes through
        // `crate::codex_integration::install_agents_md_guidance` so
        // pre-existing AGENTS.md files are appended to, not clobbered.
        ToolHookConfig {
            tool_name: "Codex".to_string(),
            config_path: PathBuf::from("AGENTS.md"),
            config_content: crate::codex_integration::agents_md_guidance_block(
                sqz_path_raw,
            ),
            scope: HookScope::Project,
        },
        // Zed (issue #38) has no command-rewrite hooks, so the
        // integration is instructions + MCP registration:
        //
        //   1. Project guidance appended to the file Zed's Agent
        //      actually reads (`.rules` when present — it shadows
        //      everything — otherwise `AGENTS.md`).
        //   2. `context_servers.sqz` merged into Zed's settings.json
        //      (user-level), which exposes the sqz MCP tools to the
        //      Zed Agent — including local models.
        //
        // The config_path/content below are placeholders for the
        // install plan display; the real install goes through
        // `crate::zed_integration` so existing files are appended to,
        // not clobbered, and JSONC settings are never rewritten.
        ToolHookConfig {
            tool_name: "Zed".to_string(),
            config_path: PathBuf::from("AGENTS.md"),
            config_content: crate::zed_integration::zed_guidance_block(sqz_path_raw),
            scope: HookScope::Project,
        },
        // Copilot CLI — user-level hook file at ~/.copilot/hooks/sqz.json.
        // preToolUse supports modifiedArgs (documented), so this is a full
        // rewrite host like Claude Code. Install goes through
        // install_copilot_hook and only runs when ~/.copilot exists.
        ToolHookConfig {
            tool_name: "Copilot CLI".to_string(),
            config_path: PathBuf::from(".copilot/hooks/sqz.json"),
            config_content: copilot_hook_content(sqz_path_raw),
            scope: HookScope::User,
        },
    ]
}

/// Install hook configs for detected AI tools in the given project directory.
///
/// Install hook configs for detected AI tools in the given project directory.
///
/// Returns the list of tools that were configured.
pub fn install_tool_hooks(project_dir: &Path, sqz_path: &str) -> Vec<String> {
    install_tool_hooks_scoped(project_dir, sqz_path, InstallScope::Project)
}

/// Where hooks should be written.
///
/// The Claude Code scope table (docs.claude.com/en/docs/claude-code/settings)
/// defines four settings locations: managed, user, project, and local.
/// `sqz init` cares about the last three:
///
/// * `Project` — writes `.claude/settings.local.json` (per-project, gitignored).
///   This is what the bare `sqz init` has always done. Good for "I only
///   want sqz active inside this repo", but a common foot-gun because the
///   user expects it to work everywhere and then sees "caching nothing"
///   in every other project. Reported by 76vangel.
///
/// * `Global` — writes `~/.claude/settings.json` (user scope, applies to
///   every Claude Code session on this machine regardless of cwd).
///   This is what most users actually want on first install. Verified
///   against the official Anthropic scope table.
///
/// Precedence in Claude Code (highest to lowest): managed > local > project > user.
/// That means a project-level install can still override a global one —
/// and a user with `.claude/settings.local.json` in their worktree will
/// silently shadow the global setting. We do NOT auto-delete the local
/// file; the uninstall flow is responsible for whichever scope was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallScope {
    /// Project-local (gitignored): `.claude/settings.local.json`, `.cursor/rules/`,
    /// etc. under `project_dir`.
    Project,
    /// User-level: `~/.claude/settings.json` and similar home-directory paths.
    /// Applies to every project on this machine.
    Global,
}

/// Which tools `sqz init` should configure.
///
/// By default sqz init writes hook configs for every supported tool
/// (Claude Code, Cursor, Windsurf, Cline, Gemini CLI, OpenCode, Codex,
/// Zed). Users who only use one agent have asked (issue #11,
/// @shochdoerfer) for a way to say "just OpenCode, please, leave the
/// rest alone." This filter is the plumbing for that.
///
/// Matching is by canonical tool name. The [`canonicalize_tool_name`]
/// helper normalises user input (lowercase, hyphens/underscores/spaces
/// collapsed, known aliases) so `Opencode`, `open-code`, `opencode`,
/// `OPENCODE` all refer to the same tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFilter {
    /// Install hook configs for every supported tool. The historical
    /// default of `sqz init`.
    All,
    /// Install hook configs only for the named tools. Unknown names are
    /// surfaced to the caller as errors by the canonicalisation layer
    /// so we don't silently ignore typos.
    Only(Vec<String>),
    /// Install hook configs for every supported tool EXCEPT the named
    /// tools. Useful when the user is fine with everything but wants
    /// one integration skipped (e.g. a project shared with collaborators
    /// who don't want a `.windsurfrules` file in the repo).
    Skip(Vec<String>),
}

impl Default for ToolFilter {
    fn default() -> Self {
        ToolFilter::All
    }
}

impl ToolFilter {
    /// Return `true` if `tool_name` (as produced by [`generate_hook_configs`])
    /// should be installed under this filter.
    ///
    /// `tool_name` is the display name sqz uses internally:
    ///   * `"Claude Code"`, `"Cursor"`, `"Windsurf"`, `"Cline"`,
    ///     `"Gemini CLI"`, `"OpenCode"`, `"Codex"`.
    ///
    /// The caller is expected to have already canonicalised the filter
    /// entries via [`canonicalize_tool_name`] so the strings line up
    /// case- and alias-wise.
    pub fn includes(&self, tool_name: &str) -> bool {
        let canon = canonicalize_tool_name(tool_name);
        match self {
            ToolFilter::All => true,
            ToolFilter::Only(allow) => allow.iter().any(|n| {
                // `allow` entries have already been canonicalised by
                // parse_tool_list; compare canonical to canonical.
                n == &canon
            }),
            ToolFilter::Skip(deny) => !deny.iter().any(|n| n == &canon),
        }
    }
}

/// Every canonical tool name sqz knows about, in the same order
/// [`generate_hook_configs`] emits them. Used by the CLI to list valid
/// options in `--only`/`--skip` error messages, and by tests that
/// need to enumerate the supported set.
pub const SUPPORTED_TOOL_NAMES: &[&str] = &[
    "Claude Code",
    "Cursor",
    "Windsurf",
    "Cline",
    "Gemini CLI",
    "Kiro",
    "OpenCode",
    "Codex",
    "Zed",
    "Copilot CLI",
];

/// How sqz reaches a client's tool output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationTier {
    /// A hook rewrites shell tool calls before they run, so output is
    /// compressed without the agent's help.
    Transparent,
    /// sqz can only ask: guidance files plus MCP tools the agent has to
    /// choose to call. The client's built-in shell and read tools bypass it.
    Advisory,
}

/// Which tier sqz's integration for `tool_name` falls in. Keep in step
/// with what [`generate_hook_configs`] actually installs.
pub fn integration_tier(tool_name: &str) -> IntegrationTier {
    match canonicalize_tool_name(tool_name).as_str() {
        "claudecode" | "copilot" | "gemini" | "opencode" => IntegrationTier::Transparent,
        _ => IntegrationTier::Advisory,
    }
}

/// Path of the user-level Copilot CLI hooks file sqz manages.
/// Honors `COPILOT_HOME` the same way the CLI does.
pub fn copilot_hooks_path() -> PathBuf {
    copilot_home().join("hooks").join("sqz.json")
}

fn copilot_home() -> PathBuf {
    if let Ok(home) = std::env::var("COPILOT_HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home);
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".copilot")
}

/// Path of the repo-level CI hook file, read by both local Copilot CLI
/// sessions in this repo and Copilot cloud agent jobs (the sandbox loads
/// only `.github/hooks/*.json` from the clone).
pub fn ci_hooks_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".github").join("hooks").join("sqz.json")
}

/// CI hook config for Copilot cloud agent + repo-scoped Copilot CLI.
///
/// The cloud agent sandbox has no sqz preinstalled and restricts outbound
/// network to GitHub hosts — which is exactly where sqz's install script
/// and release binaries live, so a sessionStart hook can bootstrap it.
/// The preToolUse hook is fail-closed on non-zero exit in Copilot, so
/// every path ends in `exit 0` with either a decision or `{}` (fall
/// through) — a missing binary must never block the tool call.
pub fn ci_hook_content() -> String {
    r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [
      {
        "type": "command",
        "bash": "command -v sqz >/dev/null 2>&1 || curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh >/dev/null 2>&1 || true",
        "timeoutSec": 120
      }
    ],
    "preToolUse": [
      {
        "type": "command",
        "matcher": "bash|powershell",
        "bash": "SQZ_BIN=$(command -v sqz || echo $HOME/.local/bin/sqz); [ -x \"$SQZ_BIN\" ] && \"$SQZ_BIN\" hook copilot 2>/dev/null || echo '{}'",
        "timeoutSec": 10
      }
    ]
  }
}
"#
    .to_string()
}

/// Install the repo-level CI hook file. Skip-if-exists unless the existing
/// file is sqz-authored and stale.
pub fn install_ci_hook(project_dir: &Path) -> Result<bool> {
    let path = ci_hooks_path(project_dir);
    let content = ci_hook_content();
    if path.exists() {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing == content || !existing.contains("hook copilot") {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::error::SqzError::Other(format!("create {}: {e}", parent.display()))
        })?;
    }
    std::fs::write(&path, content).map_err(|e| {
        crate::error::SqzError::Other(format!("write {}: {e}", path.display()))
    })?;
    Ok(true)
}

/// Remove the CI hook file when sqz-authored.
pub fn remove_ci_hook(project_dir: &Path) -> Result<bool> {
    let path = ci_hooks_path(project_dir);
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    if !content.contains("hook copilot") {
        return Ok(false);
    }
    std::fs::remove_file(&path).map_err(|e| {
        crate::error::SqzError::Other(format!("remove {}: {e}", path.display()))
    })?;
    Ok(true)
}

/// The Copilot CLI hook config sqz installs (v1 camelCase format).
pub fn copilot_hook_content(sqz_path: &str) -> String {
    let escaped = json_escape_string_value(sqz_path);
    format!(
        r#"{{
  "version": 1,
  "hooks": {{
    "preToolUse": [
      {{
        "type": "command",
        "matcher": "bash|powershell",
        "bash": "{escaped} hook copilot",
        "powershell": "{escaped} hook copilot",
        "timeoutSec": 10
      }}
    ]
  }}
}}
"#
    )
}

/// Install the Copilot CLI hook file. Returns `Ok(None)` when Copilot CLI
/// is not installed (no `~/.copilot` directory), `Ok(Some(changed))`
/// otherwise. Refreshes the file in place when the content is stale.
pub fn install_copilot_hook(sqz_path: &str) -> Result<Option<bool>> {
    let home = copilot_home();
    if !home.exists() {
        return Ok(None);
    }
    let path = copilot_hooks_path();
    let content = copilot_hook_content(sqz_path);
    if path.exists() {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing == content {
            return Ok(Some(false));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::error::SqzError::Other(format!("create {}: {e}", parent.display()))
        })?;
    }
    std::fs::write(&path, content).map_err(|e| {
        crate::error::SqzError::Other(format!("write {}: {e}", path.display()))
    })?;
    Ok(Some(true))
}

/// Remove the sqz-managed Copilot hook file if present.
pub fn remove_copilot_hook() -> Result<bool> {
    let path = copilot_hooks_path();
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path).map_err(|e| {
        crate::error::SqzError::Other(format!("remove {}: {e}", path.display()))
    })?;
    Ok(true)
}

/// Normalise a tool name or alias to its canonical form.
///
/// Canonical forms are lowercase and hyphen-free. Accepts common
/// variants:
///
/// | Input                                  | Canonical      |
/// |----------------------------------------|----------------|
/// | `Claude Code`, `claude-code`, `claude` | `claudecode`   |
/// | `Cursor`, `cursor`                     | `cursor`       |
/// | `Windsurf`, `windsurf`                 | `windsurf`     |
/// | `Cline`, `roo`, `roo-code`, `roocode`  | `cline`        |
/// | `Gemini CLI`, `gemini-cli`, `gemini`   | `gemini`       |
/// | `OpenCode`, `opencode`                 | `opencode`     |
/// | `Codex`, `codex`                       | `codex`        |
///
/// Returns the canonical string unchanged if no alias matches — the
/// caller decides whether unknown names are an error. This function
/// never fails.
pub fn canonicalize_tool_name(name: &str) -> String {
    let lowered: String = name
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .filter(|c| *c != '-' && *c != '_')
        .collect();
    match lowered.as_str() {
        "claude" | "claudecode" => "claudecode".to_string(),
        "cursor" => "cursor".to_string(),
        "windsurf" => "windsurf".to_string(),
        // Cline is also sold as "Roo Code" — treat the two as one
        // integration because that's what sqz actually targets (same
        // .clinerules file).
        "cline" | "roo" | "roocode" => "cline".to_string(),
        "gemini" | "geminicli" => "gemini".to_string(),
        "kiro" | "kirocli" | "kiroide" => "kiro".to_string(),
        "opencode" => "opencode".to_string(),
        "codex" => "codex".to_string(),
        "zed" | "zededitor" | "zedagent" => "zed".to_string(),
        "copilot" | "copilotcli" | "githubcopilot" | "ghcopilot" => "copilot".to_string(),
        other => other.to_string(),
    }
}

/// Parse a user-supplied tool list (comma-separated, whitespace-tolerant)
/// into a vector of canonical names.
///
/// Returns an error if any entry does not match a known tool — we never
/// silently drop typos because the failure mode ("my filter didn't
/// work") is hard to debug.
///
/// The error message lists every accepted name so the user can see
/// exactly what's valid.
pub fn parse_tool_list(raw: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let known: std::collections::HashSet<String> = SUPPORTED_TOOL_NAMES
        .iter()
        .map(|n| canonicalize_tool_name(n))
        .collect();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canon = canonicalize_tool_name(trimmed);
        if !known.contains(&canon) {
            let valid: Vec<String> = SUPPORTED_TOOL_NAMES
                .iter()
                .map(|n| canonicalize_tool_name(n))
                .collect();
            return Err(crate::error::SqzError::Other(format!(
                "unknown agent name '{}'. Valid options: {}",
                trimmed,
                valid.join(", ")
            )));
        }
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    Ok(out)
}

/// Like [`install_tool_hooks`] but lets the caller choose between
/// project-local and user-global scope. This is the function `sqz init`
/// and `sqz init --global` both call.
///
/// For `InstallScope::Global`:
///
/// * Claude Code hook is merged into `~/.claude/settings.json` (the user
///   settings file). We merge rather than overwrite because the user may
///   already have permissions, env, statusLine, or other hooks there —
///   blindly writing would nuke their config. Any existing sqz hook
///   entries are replaced in place; unrelated fields are preserved.
///
/// * Cursor, Windsurf, Cline, Gemini CLI rules files don't have a
///   user-level equivalent that Cursor/etc. actually load. We keep those
///   at project scope and note it in the plan. Users who want Cursor
///   compressed across all projects should follow the Cursor docs
///   (docs.cursor.com/en/context/rules) and add the rule at user scope
///   manually — Cursor honours ~/.cursor/rules/*.mdc but only within
///   workspaces that opt in.
///
/// * OpenCode plugin is already user-level by design (lives at
///   `~/.config/opencode/plugins/sqz.ts`), so scope doesn't matter here.
///
/// * Codex MCP config is always user-level (`~/.codex/config.toml`).
///   AGENTS.md stays per-project because that's where it belongs.
pub fn install_tool_hooks_scoped(
    project_dir: &Path,
    sqz_path: &str,
    scope: InstallScope,
) -> Vec<String> {
    install_tool_hooks_scoped_filtered(project_dir, sqz_path, scope, &ToolFilter::All)
}

/// Like [`install_tool_hooks_scoped`] but honours a [`ToolFilter`] so
/// callers can restrict `sqz init` to a subset of the supported tools.
///
/// The filter applies to hook-config writes AND to the OpenCode
/// TypeScript plugin at `~/.config/opencode/plugins/sqz.ts` — we only
/// install the plugin file when OpenCode passes the filter. Writing
/// the plugin file to a machine where the user filtered OpenCode out
/// would be surprising (they'd see sqz fire next time they opened
/// OpenCode even though they never asked for it).
///
/// Shell hook (rc file) and the default preset are NOT gated by this
/// filter — they're user-scoped and not specific to any agent.
/// `cmd_init` handles those separately.
pub fn install_tool_hooks_scoped_filtered(
    project_dir: &Path,
    sqz_path: &str,
    scope: InstallScope,
    filter: &ToolFilter,
) -> Vec<String> {
    let configs = generate_hook_configs(sqz_path);
    let mut installed = Vec::new();

    for config in &configs {
        // Apply the user's agent filter before we touch anything.
        // Each tool the filter rejects is completely skipped — no
        // plan lines, no files written, no logging.
        if !filter.includes(&config.tool_name) {
            continue;
        }

        // OpenCode config files are special: they live alongside the
        // user's own config and must be *merged* rather than clobbered.
        // The placeholder `config_content` is only used on a fresh
        // install; `update_opencode_config_detailed` handles both the
        // create-new and merge-into-existing cases, AND picks the
        // right file extension (opencode.jsonc vs opencode.json) —
        // fixes issue #6 where the old write-if-missing logic created
        // a parallel `opencode.json` next to an existing `.jsonc`.
        if config.tool_name == "OpenCode" {
            match crate::opencode_plugin::update_opencode_config_detailed(project_dir) {
                Ok((updated, _comments_lost)) => {
                    if updated && !installed.iter().any(|n| n == "OpenCode") {
                        installed.push("OpenCode".to_string());
                    }
                }
                Err(_e) => {
                    // Non-fatal — leave OpenCode out of the installed
                    // list and continue with other tools.
                }
            }
            continue;
        }

        // Codex has the same merge-not-clobber concern on two fronts:
        // the project-level AGENTS.md (may contain unrelated user
        // content) and the USER-level ~/.codex/config.toml (may contain
        // other MCP servers). Both go through the surgical helpers.
        if config.tool_name == "Codex" {
            let agents_changed = crate::codex_integration::install_agents_md_guidance(
                project_dir, sqz_path,
            )
            .unwrap_or(false);
            let mcp_changed = crate::codex_integration::install_codex_mcp_config()
                .unwrap_or(false);
            if (agents_changed || mcp_changed)
                && !installed.iter().any(|n| n == "Codex")
            {
                installed.push("Codex".to_string());
            }
            continue;
        }

        // Windsurf: current docs only document .windsurf/rules/; migrate a
        // legacy sqz-authored .windsurfrules to the new location.
        if config.tool_name == "Windsurf" {
            let legacy = project_dir.join(".windsurfrules");
            let target = project_dir.join(&config.config_path);
            let mut changed = install_sqz_rules(&target, "Windsurf", sqz_path).unwrap_or(false);
            if target.exists() && remove_sqz_rules(&legacy).unwrap_or(false) {
                changed = true;
            }
            if changed && !installed.iter().any(|n| n == "Windsurf") {
                installed.push("Windsurf".to_string());
            }
            continue;
        }

        if config.tool_name == "Cursor" {
            let target = project_dir.join(&config.config_path);
            if install_sqz_rules(&target, "Cursor", sqz_path).unwrap_or(false)
                && !installed.iter().any(|n| n == "Cursor")
            {
                installed.push("Cursor".to_string());
            }
            continue;
        }

        // Cline: .clinerules can be a single file or a directory of md
        // files; when it's a directory, install .clinerules/sqz.md.
        if config.tool_name == "Cline" {
            let Some(target) = sqz_rules_path(project_dir, "Cline") else {
                continue;
            };
            if install_sqz_rules(&target, "Cline", sqz_path).unwrap_or(false)
                && !installed.iter().any(|n| n == "Cline")
            {
                installed.push("Cline".to_string());
            }
            continue;
        }

        // Copilot CLI: user-level hook file, only when Copilot is installed.
        if config.tool_name == "Copilot CLI" {
            if let Ok(Some(changed)) = install_copilot_hook(sqz_path) {
                if changed && !installed.iter().any(|n| n == "Copilot CLI") {
                    installed.push("Copilot CLI".to_string());
                }
            }
            continue;
        }

        // Kiro: steering file + MCP config, plus cleanup of the legacy
        // hook file that Kiro never recognized. At global scope both go
        // to the user level (~/.kiro), which Kiro applies to every
        // workspace; at project scope they stay in the workspace.
        if config.tool_name == "Kiro" {
            let _ = crate::kiro_integration::remove_kiro_legacy_hook(project_dir);
            let (steering_changed, mcp_changed) = if scope == InstallScope::Global {
                let steering =
                    crate::kiro_integration::install_kiro_user_steering(sqz_path)
                        .unwrap_or(false);
                let mcp = matches!(
                    crate::kiro_integration::install_kiro_user_mcp_config(),
                    Ok(crate::kiro_integration::KiroMcpInstall::Created)
                        | Ok(crate::kiro_integration::KiroMcpInstall::Merged)
                );
                (steering, mcp)
            } else {
                // The user-level copy already reaches this workspace.
                let steering = !crate::kiro_integration::user_steering_installed()
                    && crate::kiro_integration::install_kiro_steering(project_dir, sqz_path)
                        .unwrap_or(false);
                let mcp = matches!(
                    crate::kiro_integration::install_kiro_mcp_config(project_dir),
                    Ok(crate::kiro_integration::KiroMcpInstall::Created)
                        | Ok(crate::kiro_integration::KiroMcpInstall::Merged)
                );
                (steering, mcp)
            };
            if (steering_changed || mcp_changed) && !installed.iter().any(|n| n == "Kiro") {
                installed.push("Kiro".to_string());
            }
            continue;
        }

        // Zed (issue #38): guidance into the instruction file Zed
        // actually reads, plus context_servers.sqz in the user-level
        // settings.json. Both are surgical; JSONC settings are never
        // rewritten (the CLI surfaces manual instructions instead).
        if config.tool_name == "Zed" {
            let guidance_changed =
                crate::zed_integration::install_zed_guidance(project_dir, sqz_path)
                    .unwrap_or(false);
            let mcp_changed = matches!(
                crate::zed_integration::install_zed_mcp_config(),
                Ok(crate::zed_integration::ZedMcpInstall::Created)
                    | Ok(crate::zed_integration::ZedMcpInstall::Merged)
            );
            if (guidance_changed || mcp_changed) && !installed.iter().any(|n| n == "Zed") {
                installed.push("Zed".to_string());
            }
            continue;
        }

        // Claude Code at global scope: merge into ~/.claude/settings.json
        // instead of writing a fresh .claude/settings.local.json in cwd.
        // This is the fix for "sqz init does nothing outside the project
        // I ran it in" — reported by 76vangel.
        //
        // Also triggers the issue #12 companion installs (CLAUDE.md
        // guidance + ~/.claude.json MCP server registration) since those
        // belong to Claude Code conceptually — if you're installing the
        // hook, you want the guidance and MCP wiring too.
        if config.tool_name == "Claude Code" && scope == InstallScope::Global {
            let hook_installed = match install_claude_global(sqz_path) {
                Ok(v) => v,
                Err(_) => false,
            };
            let md_changed = crate::claude_md_integration::install_claude_md_guidance(
                project_dir, sqz_path,
            )
            .unwrap_or(false);
            let mcp_changed =
                crate::claude_md_integration::install_claude_mcp_config()
                    .unwrap_or(false);
            if (hook_installed || md_changed || mcp_changed)
                && !installed.iter().any(|n| n == "Claude Code")
            {
                installed.push("Claude Code".to_string());
            }
            continue;
        }

        let full_path = project_dir.join(&config.config_path);

        // Don't overwrite existing hook configs
        if full_path.exists() {
            // Project-scope Claude Code file already exists. Don't skip:
            // merge our entries in, replacing any stale sqz entries. This
            // is what upgrades a pre-issue-#26 `"matcher": "Bash"` install
            // to `"Bash|PowerShell"` when the user re-runs `sqz init` —
            // with the old skip-if-exists behavior, Windows installs kept
            // the stale matcher forever and PowerShell tool calls never
            // reached the hook. User keys (permissions, env, other hooks)
            // are preserved; only sqz's own entries are replaced.
            if config.tool_name == "Claude Code" {
                let hook_changed =
                    merge_sqz_hooks_into_settings(&full_path, sqz_path).unwrap_or(false);
                let md_changed =
                    crate::claude_md_integration::install_claude_md_guidance(
                        project_dir, sqz_path,
                    )
                    .unwrap_or(false);
                let mcp_changed =
                    crate::claude_md_integration::install_claude_mcp_config()
                        .unwrap_or(false);
                if (hook_changed || md_changed || mcp_changed)
                    && !installed.iter().any(|n| n == "Claude Code")
                {
                    installed.push("Claude Code".to_string());
                }
            }
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
            // Claude Code at project scope: also install the issue #12
            // companion artifacts (CLAUDE.md guidance + ~/.claude.json
            // MCP registration). The agent needs all three pieces to
            // actually use sqz — the hook catches Bash, the MCP server
            // provides sqz_read_file/grep/list_dir, and the CLAUDE.md
            // tells it when to pick which.
            if config.tool_name == "Claude Code" {
                let _ = crate::claude_md_integration::install_claude_md_guidance(
                    project_dir, sqz_path,
                );
                let _ = crate::claude_md_integration::install_claude_mcp_config();
            }
        }
    }

    // Also install the OpenCode TypeScript plugin (user-level). The
    // config merge above has already put OpenCode in `installed` if it
    // wrote anything, so this call only matters for machines where no
    // project config existed — we still want the user-level plugin so
    // future OpenCode sessions see sqz.
    //
    // Gated by the filter: if the user ran `sqz init --skip opencode`
    // we must NOT drop the plugin file in `~/.config/opencode/plugins/`.
    // Leaving it there would surprise them on their next OpenCode run
    // (sqz would start firing uninvited, reverting the skip).
    if filter.includes("OpenCode") {
        if let Ok(true) = crate::opencode_plugin::install_opencode_plugin(sqz_path) {
            if !installed.iter().any(|n| n == "OpenCode") {
                installed.push("OpenCode".to_string());
            }
        }
    }

    installed
}

// ── Claude Code user-scope hook install ──────────────────────────────────

/// Resolve `~/.claude/settings.json` for the current user.
///
/// This is the "User" scope file per the Anthropic scope table
/// (docs.claude.com/en/docs/claude-code/settings). Applies to every
/// Claude Code session on this machine regardless of cwd.
///
/// Precedence: Managed > Local (`.claude/settings.local.json`) >
/// Project (`.claude/settings.json`) > User (this file). Users with a
/// local settings file in a worktree can still override the global
/// sqz hook — that's intended.
pub fn claude_user_settings_path() -> Option<PathBuf> {
    dirs_next::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// Merge sqz's PreToolUse / PreCompact / SessionStart hook entries
/// into `~/.claude/settings.json`.
///
/// * Creates the file if missing, with just our hooks.
/// * If the file exists, parses it as JSON, replaces any existing sqz
///   entries (matched by `command` containing `sqz hook` / `sqz resume` /
///   `sqz hook precompact`), and inserts ours. Everything else — the
///   user's permissions, env, statusLine, other PreToolUse matchers —
///   stays untouched.
/// * Writes atomically (temp file + rename) so a crash halfway through
///   can't leave the user with a corrupted settings.json.
///
/// Returns `Ok(true)` if the file was created or changed, `Ok(false)`
/// if our hook entries were already present identically.
fn install_claude_global(sqz_path: &str) -> Result<bool> {
    install_claude_global_at(sqz_path, None)
}

/// Internal: home-dir-injectable counterpart used by tests. Avoids
/// `std::env::set_var("HOME")` which races with parallel tests that
/// also read HOME (e.g. the api_proxy property tests that open
/// `~/.sqz/sessions.db`).
fn install_claude_global_at(sqz_path: &str, home_override: Option<&Path>) -> Result<bool> {
    let path = match home_override {
        Some(h) => h.join(".claude").join("settings.json"),
        None => claude_user_settings_path().ok_or_else(|| {
            crate::error::SqzError::Other(
                "Could not resolve home directory for ~/.claude/settings.json".to_string(),
            )
        })?,
    };
    merge_sqz_hooks_into_settings(&path, sqz_path)
}

/// Dry-run probe for `sqz init`'s plan phase: would merging sqz's hook
/// entries change the Claude Code settings file at `path`?
///
/// Returns `Ok(true)` when the file is missing, or exists with stale or
/// absent sqz entries (e.g. the pre-issue-#26 `"Bash"`-only matcher, or
/// an outdated binary path). Returns `Ok(false)` when our entries are
/// already present identically. Errors on unparseable files — same as
/// the real merge, which refuses to touch those.
///
/// Without this probe the CLI's plan phase treated any existing file as
/// "nothing to do" and exited before the installer ran, so the healing
/// merge below was unreachable in practice.
pub fn claude_project_settings_needs_update(path: &Path, sqz_path: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    let content = std::fs::read_to_string(path).map_err(|e| {
        crate::error::SqzError::Other(format!("read {}: {e}", path.display()))
    })?;
    let mut root: serde_json::Value = if content.trim().is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(&content).map_err(|e| {
            crate::error::SqzError::Other(format!("parse {}: {e}", path.display()))
        })?
    };
    let Some(root_obj) = root.as_object_mut() else {
        return Err(crate::error::SqzError::Other(format!(
            "{} is not a JSON object",
            path.display()
        )));
    };

    let before = serde_json::to_string(&root_obj).unwrap_or_default();
    let hooks = root_obj
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(hooks_obj) = hooks.as_object_mut() else {
        return Ok(true);
    };
    for (event, entry, sentinel) in sqz_hook_entries(sqz_path) {
        upsert_sqz_hook_entry(hooks_obj, event, entry, sentinel);
    }
    upsert_claude_allow_rules(root_obj);
    let after = serde_json::to_string(&root_obj).unwrap_or_default();
    Ok(before != after)
}

/// A fresh Claude Code settings file holding only sqz's hooks and rules.
fn claude_settings_content(sqz_path: &str) -> String {
    let mut hooks = serde_json::Map::new();
    for (event, entry, sentinel) in sqz_hook_entries(sqz_path) {
        upsert_sqz_hook_entry(&mut hooks, event, entry, sentinel);
    }
    let mut root = serde_json::Map::new();
    root.insert("hooks".to_string(), serde_json::Value::Object(hooks));
    upsert_claude_allow_rules(&mut root);
    serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .expect("a map of strings always serializes")
}

/// The hook entries sqz installs into Claude Code settings files, as
/// `(event_name, entry_json, sentinel)` tuples. Single source of truth for
/// the real merge, the dry-run probe and uninstall.
fn sqz_hook_entries(sqz_path: &str) -> [(&'static str, serde_json::Value, &'static str); 4] {
    [
        (
            "PreToolUse",
            serde_json::json!({
                "matcher": "Bash|PowerShell",
                "hooks": [{ "type": "command", "command": format!("{sqz_path} hook claude") }]
            }),
            "hook claude",
        ),
        (
            "PostToolUse",
            serde_json::json!({
                "matcher": "Bash",
                "hooks": [{ "type": "command", "command": format!("{sqz_path} hook claude-post") }]
            }),
            "hook claude-post",
        ),
        (
            "PreCompact",
            serde_json::json!({
                "hooks": [{ "type": "command", "command": format!("{sqz_path} hook precompact") }]
            }),
            "hook precompact",
        ),
        (
            "SessionStart",
            serde_json::json!({
                "matcher": "compact",
                "hooks": [{ "type": "command", "command": format!("{sqz_path} resume") }]
            }),
            "resume",
        ),
    ]
}

/// Merge sqz's three hook entries into a Claude Code settings JSON at
/// `path` — either `~/.claude/settings.json` (global scope) or a
/// project's `.claude/settings.local.json`. Shared by both install
/// scopes so re-running `sqz init` upgrades stale sqz entries (an old
/// `"Bash"`-only matcher, an outdated binary path) at project scope
/// too, instead of skipping any file that already exists. That skip is
/// how Windows installs from before the issue #26 fix stayed broken:
/// their project settings kept the pre-PowerShell matcher forever.
fn merge_sqz_hooks_into_settings(path: &Path, sqz_path: &str) -> Result<bool> {
    // Parse the existing file, or start from an empty object.
    let mut root: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| {
            crate::error::SqzError::Other(format!(
                "read {}: {e}",
                path.display()
            ))
        })?;
        if content.trim().is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&content).map_err(|e| {
                crate::error::SqzError::Other(format!(
                    "parse {}: {e} — please fix or move the file before re-running sqz init",
                    path.display()
                ))
            })?
        }
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    // Ensure root is an object (users occasionally have arrays or
    // corrupted files; we refuse to touch those).
    let root_obj = root.as_object_mut().ok_or_else(|| {
        crate::error::SqzError::Other(format!(
            "{} is not a JSON object — refusing to overwrite",
            path.display()
        ))
    })?;

    // Snapshot the "before" state for change detection.
    let before = serde_json::to_string(&root_obj).unwrap_or_default();

    // Get or create the top-level "hooks" object.
    let hooks = root_obj
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let hooks_obj = hooks.as_object_mut().ok_or_else(|| {
        crate::error::SqzError::Other(format!(
            "{}: `hooks` is not an object — refusing to overwrite",
            path.display()
        ))
    })?;

    for (event, entry, sentinel) in sqz_hook_entries(sqz_path) {
        upsert_sqz_hook_entry(hooks_obj, event, entry, sentinel);
    }
    upsert_claude_allow_rules(root_obj);

    let after = serde_json::to_string(&root_obj).unwrap_or_default();
    if before == after && path.exists() {
        // Already present and unchanged — no write needed.
        return Ok(false);
    }

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::error::SqzError::Other(format!(
                "create {}: {e}",
                parent.display()
            ))
        })?;
    }

    // Atomic write: tempfile in same directory + rename. Keeps the old
    // file intact if serialization or write fails halfway.
    let parent = path.parent().ok_or_else(|| {
        crate::error::SqzError::Other(format!(
            "path {} has no parent directory",
            path.display()
        ))
    })?;
    let tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "create temp file in {}: {e}",
            parent.display()
        ))
    })?;
    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(root_obj.clone()))
        .map_err(|e| crate::error::SqzError::Other(format!("serialize settings.json: {e}")))?;
    std::fs::write(tmp.path(), serialized).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "write to temp file {}: {e}",
            tmp.path().display()
        ))
    })?;
    tmp.persist(&path).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "rename temp file into place at {}: {e}",
            path.display()
        ))
    })?;

    Ok(true)
}

/// Remove sqz's hook entries from `~/.claude/settings.json` without
/// touching any other keys. Symmetric with [`install_claude_global`].
///
/// Returns:
/// * `Ok(Some((path, true)))` — file existed, sqz entries found and
///   stripped. If the resulting `hooks` object is empty, we also remove
///   the `hooks` key entirely. If the resulting root object is empty,
///   we remove the file — matches the uninstall UX of every other sqz
///   surface.
/// * `Ok(Some((path, false)))` — file existed but contained no sqz
///   entries. No write.
/// * `Ok(None)` — file did not exist.
/// * `Err(_)` — file existed but could not be read or parsed.
pub fn remove_claude_global_hook() -> Result<Option<(PathBuf, bool)>> {
    remove_claude_global_hook_at(None)
}

/// Internal: home-dir-injectable counterpart used by tests. See
/// `install_claude_global_at` for rationale.
fn remove_claude_global_hook_at(
    home_override: Option<&Path>,
) -> Result<Option<(PathBuf, bool)>> {
    let path = match home_override {
        Some(h) => h.join(".claude").join("settings.json"),
        None => match claude_user_settings_path() {
            Some(p) => p,
            None => return Ok(None),
        },
    };
    remove_sqz_from_claude_settings(path)
}

/// Remove sqz's entries from a project's `.claude/settings.local.json`,
/// keeping whatever else is in it, such as the permissions Claude Code
/// saves there when you pick "don't ask again". Same results as
/// [`remove_claude_global_hook`].
pub fn remove_claude_project_hook(project_dir: &Path) -> Result<Option<(PathBuf, bool)>> {
    remove_sqz_from_claude_settings(project_dir.join(".claude").join("settings.local.json"))
}

/// Strip sqz's hook entries and allow rules from the Claude Code settings
/// file at `path`, deleting the file if nothing else is left in it.
fn remove_sqz_from_claude_settings(path: PathBuf) -> Result<Option<(PathBuf, bool)>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path).map_err(|e| {
        crate::error::SqzError::Other(format!("read {}: {e}", path.display()))
    })?;
    if content.trim().is_empty() {
        return Ok(Some((path, false)));
    }

    let mut root: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "parse {}: {e} — refusing to rewrite an unparseable file",
            path.display()
        ))
    })?;
    let Some(root_obj) = root.as_object_mut() else {
        return Ok(Some((path, false)));
    };

    let mut changed = remove_claude_allow_rules(root_obj);
    if let Some(hooks) = root_obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (event, _, sentinel) in &sqz_hook_entries("") {
            if let Some(arr) = hooks.get_mut(*event).and_then(|v| v.as_array_mut()) {
                let before = arr.len();
                arr.retain(|entry| !hook_entry_command_contains(entry, sentinel));
                if arr.len() != before {
                    changed = true;
                }
            }
        }

        // Drop any now-empty hook event arrays so we don't leave
        // `"PreToolUse": []` clutter in the user's settings.
        hooks.retain(|_, v| match v {
            serde_json::Value::Array(a) => !a.is_empty(),
            _ => true,
        });

        // If the whole `hooks` object is now empty, drop it so sqz's
        // uninstall leaves no trace.
        let hooks_empty = hooks.is_empty();
        if hooks_empty {
            root_obj.remove("hooks");
            changed = true;
        }
    }

    if !changed {
        return Ok(Some((path, false)));
    }

    // If root is now completely empty, delete the file — matches the
    // "leave nothing behind" behaviour of the OpenCode/Codex uninstall
    // paths.
    if root_obj.is_empty() {
        std::fs::remove_file(&path).map_err(|e| {
            crate::error::SqzError::Other(format!(
                "remove {}: {e}",
                path.display()
            ))
        })?;
        return Ok(Some((path, true)));
    }

    // Atomic rewrite.
    let parent = path.parent().ok_or_else(|| {
        crate::error::SqzError::Other(format!(
            "path {} has no parent directory",
            path.display()
        ))
    })?;
    let tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "create temp file in {}: {e}",
            parent.display()
        ))
    })?;
    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(root_obj.clone()))
        .map_err(|e| {
            crate::error::SqzError::Other(format!("serialize settings.json: {e}"))
        })?;
    std::fs::write(tmp.path(), serialized).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "write to temp file {}: {e}",
            tmp.path().display()
        ))
    })?;
    tmp.persist(&path).map_err(|e| {
        crate::error::SqzError::Other(format!(
            "rename temp file into place at {}: {e}",
            path.display()
        ))
    })?;

    Ok(Some((path, true)))
}

/// Replace (or insert) sqz's hook entry in the array under
/// `hooks[event_name]`. Entries are matched by the `command` substring
/// `sentinel` — that way, an upgrade from `sqz hook claude` to a future
/// renamed command won't accumulate stale entries.
///
/// Idempotent: calling this twice yields the same JSON.
fn upsert_sqz_hook_entry(
    hooks_obj: &mut serde_json::Map<String, serde_json::Value>,
    event_name: &str,
    new_entry: serde_json::Value,
    sentinel: &str,
) {
    let arr = hooks_obj
        .entry(event_name.to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(arr) = arr.as_array_mut() else {
        // `hooks[event]` exists but isn't an array — overwrite it with
        // just our entry. Not ideal but matches the behavior the user
        // would get on a fresh install.
        hooks_obj.insert(
            event_name.to_string(),
            serde_json::Value::Array(vec![new_entry]),
        );
        return;
    };

    // Drop any existing entry whose command matches our sentinel.
    arr.retain(|entry| !hook_entry_command_contains(entry, sentinel));

    arr.push(new_entry);
}

/// Add [`CLAUDE_ALLOW_RULES`] to `permissions.allow`, keeping the user's
/// own rules. A `permissions` or `allow` of an unexpected type is left alone.
fn upsert_claude_allow_rules(root: &mut serde_json::Map<String, serde_json::Value>) {
    let perms = root
        .entry("permissions".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(perms) = perms.as_object_mut() else { return };
    let allow = perms
        .entry("allow".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(allow) = allow.as_array_mut() else { return };
    for rule in CLAUDE_ALLOW_RULES {
        if !allow.iter().any(|r| r.as_str() == Some(rule)) {
            allow.push(serde_json::Value::String(rule.to_string()));
        }
    }
}

/// Take [`CLAUDE_ALLOW_RULES`] back out, dropping `allow` and `permissions`
/// if that leaves them empty. Returns whether anything was removed.
fn remove_claude_allow_rules(root: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let Some(perms) = root.get_mut("permissions").and_then(|p| p.as_object_mut()) else {
        return false;
    };
    let Some(allow) = perms.get_mut("allow").and_then(|a| a.as_array_mut()) else {
        return false;
    };
    let before = allow.len();
    allow.retain(|r| !r.as_str().is_some_and(|s| CLAUDE_ALLOW_RULES.contains(&s)));
    if allow.len() == before {
        return false;
    }
    if allow.is_empty() {
        perms.remove("allow");
    }
    if perms.is_empty() {
        root.remove("permissions");
    }
    true
}

/// The binary path in a settings file's sqz PreToolUse hook, if it has one.
fn claude_settings_sqz_path(root: &serde_json::Value) -> Option<String> {
    root.pointer("/hooks/PreToolUse")?.as_array()?.iter().find_map(|entry| {
        entry.get("hooks")?.as_array()?.iter().find_map(|h| {
            let command = h.get("command")?.as_str()?;
            command.strip_suffix(" hook claude").map(str::to_string)
        })
    })
}

/// Whether a Claude Code settings file has sqz's PreToolUse hook but not
/// the allow rules or PostToolUse hook `sqz init` now writes, as an
/// install from an older sqz does. Without the rules the hook leaves
/// every command alone outside bypass mode.
pub fn claude_settings_stale(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    if claude_settings_sqz_path(&root).is_none() {
        return false;
    }
    let post_hook = root
        .pointer("/hooks/PostToolUse")
        .and_then(|v| v.as_array())
        .is_some_and(|a| a.iter().any(|e| hook_entry_command_contains(e, "hook claude-post")));
    !post_hook || !claude_rules_installed(&[path.to_path_buf()])
}

/// Bring a stale settings file (see [`claude_settings_stale`]) up to date,
/// keeping the sqz binary its hook already runs when that still exists.
pub fn refresh_claude_settings(path: &Path, fallback_sqz: &str) -> Result<bool> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::error::SqzError::Other(format!("read {}: {e}", path.display())))?;
    let root: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| crate::error::SqzError::Other(format!("parse {}: {e}", path.display())))?;
    let sqz_path = claude_settings_sqz_path(&root)
        .filter(|p| Path::new(p).exists())
        .unwrap_or_else(|| fallback_sqz.to_string());
    merge_sqz_hooks_into_settings(path, &sqz_path)
}

/// True if any command in a hook entry contains the given substring.
/// Used to locate sqz's own entries without pinning to an exact command
/// (so future format changes still upgrade cleanly).
fn hook_entry_command_contains(entry: &serde_json::Value, needle: &str) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks_arr| {
            hooks_arr.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| c.contains(needle))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
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

/// What a hook returns when it leaves the tool call alone. Cursor and
/// Copilot need a JSON object on every path.
fn passthrough(platform: HookPlatform, input: &str) -> String {
    match platform {
        HookPlatform::Cursor | HookPlatform::Copilot => "{}".to_string(),
        _ => input.to_string(),
    }
}

/// Background, async or detached shell calls, under each client's name
/// for the flag.
fn runs_in_background(parsed: &serde_json::Value) -> bool {
    let Some(args) = parsed
        .get("tool_input")
        .or_else(|| parsed.get("toolArgs"))
        .or_else(|| parsed.get("args"))
    else {
        return false;
    };
    ["run_in_background", "is_background", "background", "async", "detach"]
        .iter()
        .any(|k| args.get(k).and_then(|v| v.as_bool()) == Some(true))
        || args.get("mode").and_then(|v| v.as_str()) == Some("async")
}

/// Allow rules `sqz init` adds to Claude Code settings so the two parts the
/// ungrouped rewrite adds never need approval of their own. Neither runs
/// anything: one sets a shell option, the other reads stdin.
pub const CLAUDE_ALLOW_RULES: [&str; 2] = ["Bash(set -o pipefail)", "Bash(sqz compress --cmd *)"];

/// Claude Code checks permissions against the rewritten command, so a
/// rewrite must not make an allowed command prompt. Outside bypass mode
/// that holds only for the ungrouped form, and only once the user has
/// allowed its extra parts; grouped commands are judged too complex to
/// match any rule. Everything else is left alone.
fn claude_rewrite(
    parsed: &serde_json::Value,
    command: &str,
    is_powershell: bool,
    settings: &[PathBuf],
    shell: crate::shell_scan::Shell,
) -> Option<String> {
    let bypass = parsed.get("permission_mode").and_then(|v| v.as_str()) == Some("bypassPermissions");
    if is_powershell {
        if !bypass || is_interactive_command(command) || has_shell_operators(command) {
            return None;
        }
        return Some(crate::exit_marker::powershell_rewrite(command, &shell_escape(command)));
    }
    if let Some(cmds) = crate::shell_scan::flat_for(command, shell) {
        return (bypass || claude_rules_installed(settings))
            .then(|| crate::exit_marker::posix_flat_rewrite(command, &cmds));
    }
    if bypass {
        return crate::shell_scan::plan(command).ok().map(|plan| posix_rewrite_plan(&plan));
    }
    None
}

/// A successful Bash result the Claude Code PostToolUse hook may shrink.
#[derive(Debug, Clone, PartialEq)]
pub struct PostToolUseTarget {
    /// The tool's result object. A replacement must keep its shape, so
    /// only `stdout` and `stderr` should change.
    pub response: serde_json::Map<String, serde_json::Value>,
    /// The stats and formatter label for its output.
    pub label: String,
}

/// Decide whether a Claude Code PostToolUse payload is worth compressing:
/// a Bash call the PreToolUse hook didn't already pipe through sqz, with
/// text output that Claude Code hasn't truncated. Claude Code only runs
/// PostToolUse after a command exits 0; a failure goes to
/// PostToolUseFailure, which can't replace output.
pub fn claude_post_tool_use_target(input: &str) -> Option<PostToolUseTarget> {
    let parsed: serde_json::Value = serde_json::from_str(input).ok()?;
    if parsed.get("hook_event_name").and_then(|v| v.as_str()) != Some("PostToolUse")
        || parsed.get("tool_name").and_then(|v| v.as_str()) != Some("Bash")
        || runs_in_background(&parsed)
    {
        return None;
    }
    let command = parsed.pointer("/tool_input/command")?.as_str()?;
    if command.contains("sqz compress")
        || crate::shell_scan::plan(command) == Err(crate::shell_scan::Skip::AlreadySqz)
    {
        return None;
    }
    let response = parsed.get("tool_response")?.as_object()?.clone();
    let flag = |k: &str| response.get(k).and_then(|v| v.as_bool()) == Some(true);
    if flag("isImage") || flag("interrupted") {
        return None;
    }
    // Past this many characters Claude Code has already cut the output and
    // shows only a preview, so a smaller replacement changes nothing.
    let limit = std::env::var("BASH_MAX_OUTPUT_LENGTH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(30_000);
    let long = |k: &str| {
        response
            .get(k)
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.chars().count() >= limit)
    };
    if response.get("stdout").and_then(|v| v.as_str()).is_none() || long("stdout") || long("stderr") {
        return None;
    }
    Some(PostToolUseTarget { response, label: post_tool_use_label(command) })
}

/// Whether sqz's Claude Code hooks, as `sqz init` installs them, compress a
/// Bash command's output in a default-mode session: the PreToolUse rewrite
/// takes it, or it exits 0 and the PostToolUse hook does. Followers and
/// background jobs never finish with output to replace.
pub fn claude_hooks_compress(command: &str, failed: bool) -> bool {
    use crate::shell_scan::{flat_for, plan, Shell, Skip};
    if command.contains("sqz compress") {
        return false;
    }
    match plan(command) {
        Err(Skip::AlreadySqz) => false,
        _ if flat_for(command, Shell::from_env()).is_some() => true,
        Err(Skip::LongRunning | Skip::Background) => false,
        _ => !failed,
    }
}

/// One command's label when only one command in the list has output worth
/// compressing, so its formatter can run; the generic `sh:` label otherwise.
fn post_tool_use_label(command: &str) -> String {
    if let Some(cmds) = crate::shell_scan::flat(command) {
        let mut labels = cmds.into_iter().filter_map(|c| c.label);
        if let (Some(only), None) = (labels.next(), labels.next()) {
            return only;
        }
    }
    list_label(command)
}

/// The settings files Claude Code reads allow rules from, managed policy
/// aside: the user file, then the project's shared and local files.
fn claude_settings_files(parsed: &serde_json::Value) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let user = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs_next::home_dir().map(|h| h.join(".claude")));
    if let Some(dir) = user {
        files.push(dir.join("settings.json"));
    }
    let project = std::env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .or_else(|| parsed.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from));
    if let Some(dir) = project {
        files.push(dir.join(".claude").join("settings.json"));
        files.push(dir.join(".claude").join("settings.local.json"));
    }
    files
}

/// Whether the allow rules across `files` cover both parts the ungrouped
/// rewrite adds.
fn claude_rules_installed(files: &[PathBuf]) -> bool {
    let (mut pipefail, mut compress) = (false, false);
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let Some(rules) = root.pointer("/permissions/allow").and_then(|a| a.as_array()) else {
            continue;
        };
        for rule in rules.iter().filter_map(|r| r.as_str()) {
            pipefail |= bash_rule_allows(rule, "set -o pipefail");
            compress |= bash_rule_allows(rule, "sqz compress --cmd ls");
        }
    }
    pipefail && compress
}

/// Claude Code's matching for the rule shapes that could cover `command`:
/// bare `Bash`, an exact command, or one trailing wildcard (`x *`, `x:*`
/// and `x*`). Anything else counts as not covering it.
fn bash_rule_allows(rule: &str, command: &str) -> bool {
    let rule = rule.trim();
    if rule == "Bash" {
        return true;
    }
    let Some(spec) = rule.strip_prefix("Bash(").and_then(|r| r.strip_suffix(')')) else {
        return false;
    };
    if let Some(prefix) = spec.strip_suffix(":*").or_else(|| spec.strip_suffix(" *")) {
        if prefix.contains('*') {
            return false;
        }
        return command == prefix
            || command.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(' '));
    }
    match spec.strip_suffix('*') {
        Some(prefix) if !prefix.contains('*') => command.starts_with(prefix),
        Some(_) => false,
        None => spec == command,
    }
}

/// The POSIX rewrite for a command `shell_scan` accepted.
fn posix_rewrite_plan(plan: &crate::shell_scan::Plan<'_>) -> String {
    let grouped = if plan.simple {
        // The full command is the label so subcommand formatters
        // (`git status`, `cargo test`) dispatch.
        let cmd = plan.body.trim();
        let label = cmd.strip_suffix(" 2>&1").map_or(cmd, str::trim_end);
        crate::exit_marker::posix_rewrite(cmd, &shell_escape(label))
    } else {
        crate::exit_marker::posix_rewrite_list(plan.body, &shell_escape(&list_label(plan.body)))
    };
    if plan.prefix.is_empty() {
        grouped
    } else {
        format!("{} {grouped}", plan.prefix)
    }
}

/// Label for a list or pipeline. The `sh:` prefix keeps per-command
/// formatters off output that is partial or comes from several commands.
fn list_label(body: &str) -> String {
    let first = body.trim().lines().next().unwrap_or("");
    let label = format!("sh: {}", first.split_whitespace().collect::<Vec<_>>().join(" "));
    if label.chars().count() > 100 {
        format!("{}…", label.chars().take(99).collect::<String>())
    } else {
        label
    }
}

/// Check if a command contains shell operators that would break piping.
/// Commands with these operators are passed through uncompressed rather
/// than risk incorrect behavior.
pub(crate) fn has_shell_operators(cmd: &str) -> bool {
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
    fn test_copilot_hook_rewrites_bash_with_modified_args() {
        let input = r#"{"sessionId":"s","timestamp":1,"cwd":"/x","toolName":"bash","toolArgs":{"command":"git log -20","timeoutSec":30}}"#;
        let result = process_hook_copilot(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // An "allow" would skip Copilot's own approval prompt.
        assert!(parsed.get("permissionDecision").is_none(), "{parsed}");
        let args = &parsed["modifiedArgs"];
        assert!(args["command"].as_str().unwrap().contains("sqz compress"));
        assert_eq!(args["timeoutSec"], 30, "sibling toolArgs fields carried over");
    }

    #[test]
    fn test_copilot_hook_passes_non_shell_through() {
        let input = r#"{"sessionId":"s","timestamp":1,"cwd":"/x","toolName":"view","toolArgs":{"path":"a.rs"}}"#;
        let result = process_hook_copilot(input).unwrap();
        assert_eq!(result, "{}", "non-shell tools fall through with empty object");
    }

    #[test]
    fn test_copilot_hook_skips_risky_shell_operators() {
        // Copilot keeps one shell per session, so state changes must not
        // move into a subshell.
        for command in ["source .venv/bin/activate && pytest", "npm run build &"] {
            let input = serde_json::json!({"toolName": "bash", "toolArgs": {"command": command}}).to_string();
            assert_eq!(process_hook_copilot(&input).unwrap(), "{}", "{command}");
        }
    }

    #[test]
    fn test_claude_updated_input_preserves_sibling_fields() {
        // Claude Code's updatedInput replaces the whole input object, so
        // run_in_background / timeout / description must survive the rewrite.
        let input = r#"{"tool_name":"Bash","permission_mode":"bypassPermissions","tool_input":{"command":"cargo test","run_in_background":false,"timeout":120000,"description":"Run tests"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let updated = &parsed["hookSpecificOutput"]["updatedInput"];
        assert!(updated["command"].as_str().unwrap().contains("sqz compress"));
        assert_eq!(updated["run_in_background"], false);
        assert_eq!(updated["timeout"], 120000);
        assert_eq!(updated["description"], "Run tests");
    }

    fn rewritten(input: &str) -> Option<String> {
        let out: serde_json::Value = serde_json::from_str(&process_hook(input).unwrap()).unwrap();
        out.pointer("/hookSpecificOutput/updatedInput/command")
            .and_then(|c| c.as_str())
            .map(str::to_string)
    }

    /// A Claude Code Bash call in bypass mode, where the rewrite doesn't
    /// depend on the allow rules in this machine's settings files.
    fn bash(command: &str) -> String {
        serde_json::json!({
            "tool_name": "Bash",
            "permission_mode": "bypassPermissions",
            "tool_input": {"command": command}
        })
        .to_string()
    }

    fn claude_default(command: &str, settings: &[PathBuf]) -> Option<String> {
        claude_default_in(command, settings, crate::shell_scan::Shell::Zsh)
    }

    fn claude_default_in(
        command: &str,
        settings: &[PathBuf],
        shell: crate::shell_scan::Shell,
    ) -> Option<String> {
        let parsed = serde_json::json!({
            "tool_name": "Bash",
            "permission_mode": "default",
            "tool_input": {"command": command}
        });
        claude_rewrite(&parsed, command, false, settings, shell)
    }

    fn settings_with_rules(dir: &Path, rules: &[&str]) -> PathBuf {
        let path = dir.join("settings.json");
        std::fs::write(&path, serde_json::json!({"permissions": {"allow": rules}}).to_string())
            .unwrap();
        path
    }

    #[test]
    fn claude_default_mode_rewrites_only_with_the_allow_rules() {
        let dir = tempfile::tempdir().unwrap();
        let none = dir.path().join("missing.json");
        assert_eq!(claude_default("git status", &[none.clone()]), None);

        let partial = settings_with_rules(dir.path(), &["Bash(sqz compress --cmd *)"]);
        assert_eq!(claude_default("git status", &[partial]), None, "pipefail rule missing");

        let full = settings_with_rules(dir.path(), &CLAUDE_ALLOW_RULES);
        assert_eq!(
            claude_default("git status", &[none, full.clone()]).as_deref(),
            Some("set -o pipefail; git status 2>&1 | sqz compress --cmd 'git status'")
        );
        // Grouping is judged too complex for any allow rule to match, so
        // pipelines are left alone outside bypass mode.
        assert_eq!(claude_default("git log | head -5", &[full.clone()]), None);
        assert_eq!(claude_default("for f in *; do wc -l $f; done", &[full]), None);
    }

    #[test]
    fn claude_default_mode_keeps_builtins_in_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let rules = settings_with_rules(dir.path(), &CLAUDE_ALLOW_RULES);
        assert_eq!(
            claude_default("cd sub && export CI=1 && cargo test", &[rules]).as_deref(),
            Some("set -o pipefail; cd sub && export CI=1 && cargo test 2>&1 | sqz compress --cmd 'cargo test'")
        );
    }

    #[test]
    fn claude_flat_form_stops_where_the_shell_would() {
        use crate::shell_scan::Shell;
        let dir = tempfile::tempdir().unwrap();
        let rules = [settings_with_rules(dir.path(), &CLAUDE_ALLOW_RULES)];
        for shell in [Shell::Bash, Shell::Zsh] {
            assert_eq!(claude_default_in("ls ${X:?unset}; echo next", &rules, shell), None);
            assert_eq!(claude_default_in("ls $((1/0)) || echo next", &rules, shell), None);
            assert_eq!(
                claude_default_in("ls ${X:?unset} && echo next", &rules, shell).as_deref(),
                Some("set -o pipefail; ls ${X:?unset} 2>&1 | sqz compress --cmd ls && echo next")
            );
        }
        assert_eq!(claude_default_in("ls *.rs; echo next", &rules, Shell::Zsh), None);
        assert!(claude_default_in("ls *.rs; echo next", &rules, Shell::Bash).is_some());
        // Bypass mode falls back to the grouped form, which stops with the shell.
        let parsed = serde_json::json!({"permission_mode": "bypassPermissions"});
        let command = "ls ${X:?unset}; echo next";
        let cmd = claude_rewrite(&parsed, command, false, &[], Shell::Bash).unwrap();
        assert!(cmd.starts_with("{ ( ls ${X:?unset}; echo next\n); printf"), "{cmd}");
    }

    #[test]
    fn claude_powershell_is_rewritten_only_in_bypass_mode() {
        let zsh = crate::shell_scan::Shell::Zsh;
        let parsed = serde_json::json!({"tool_name": "PowerShell", "permission_mode": "default"});
        assert_eq!(claude_rewrite(&parsed, "Get-ChildItem", true, &[], zsh), None);
        let parsed = serde_json::json!({"tool_name": "PowerShell", "permission_mode": "bypassPermissions"});
        assert!(claude_rewrite(&parsed, "Get-ChildItem", true, &[], zsh).is_some());
    }

    #[test]
    fn bash_rule_matching() {
        for (rule, command, expected) in [
            ("Bash", "set -o pipefail", true),
            ("Bash(*)", "set -o pipefail", true),
            ("Bash(set -o pipefail)", "set -o pipefail", true),
            ("Bash(set -o pipefail)", "set -o pipefail -x", false),
            ("Bash(set *)", "set -o pipefail", true),
            ("Bash(set:*)", "set -o pipefail", true),
            ("Bash(sqz compress --cmd *)", "sqz compress --cmd ls", true),
            ("Bash(sqz *)", "sqz compress --cmd ls", true),
            ("Bash(sqz*)", "sqz compress --cmd ls", true),
            ("Bash(sq *)", "sqz compress --cmd ls", false),
            ("Bash(* --cmd ls)", "sqz compress --cmd ls", false),
            ("Read(./x)", "sqz compress --cmd ls", false),
        ] {
            assert_eq!(bash_rule_allows(rule, command), expected, "{rule} vs {command}");
        }
    }

    #[test]
    fn stale_claude_settings_are_detected_and_refreshed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let bin = dir.path().join("sqz");
        std::fs::write(&bin, "").unwrap();
        let bin = bin.to_string_lossy().to_string();
        // What a 1.9.0 install wrote, plus one of the user's own hooks.
        let old = serde_json::json!({"hooks": {
            "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": format!("{bin} hook claude")}]}],
            "PostToolUse": [{"matcher": "Edit", "hooks": [{"type": "command", "command": "prettier --write"}]}],
        }});
        std::fs::write(&path, old.to_string()).unwrap();
        assert!(claude_settings_stale(&path));

        assert!(refresh_claude_settings(&path, "/fallback/sqz").unwrap());
        assert!(!claude_settings_stale(&path));
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let post = v["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post.len(), 2, "the user's own PostToolUse hook stays: {post:?}");
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            format!("{bin} hook claude"),
            "keeps the binary the hook already used"
        );

        std::fs::write(&path, r#"{"permissions":{}}"#).unwrap();
        assert!(!claude_settings_stale(&path), "no sqz hook, nothing to refresh");
    }

    #[test]
    fn background_calls_pass_through() {
        for input in [
            r#"{"tool_name":"Bash","tool_input":{"command":"cargo build","run_in_background":true}}"#,
            r#"{"tool_name":"run_shell_command","tool_input":{"command":"cargo build","is_background":true}}"#,
            r#"{"toolName":"bash","toolArgs":{"command":"cargo build","mode":"async"}}"#,
        ] {
            assert_eq!(rewritten(input), None, "{input}");
        }
    }

    #[test]
    fn lists_and_pipelines_are_grouped_with_a_generic_label() {
        let cmd = rewritten(&bash("cargo test 2>&1 | tail -30")).unwrap();
        assert_eq!(
            cmd,
            "{ ( cargo test 2>&1 | tail -30\n); printf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; } 2>&1 | sqz compress --cmd 'sh: cargo test 2>&1 | tail -30'"
        );
        // A trailing comment no longer swallows the closing brace.
        let cmd = rewritten(&bash("ls | head # list")).unwrap();
        assert!(cmd.starts_with("{ ( ls | head # list\n); printf"), "{cmd}");
        // Without a pipe nothing needs grouping, and the comment stays last.
        let cmd = rewritten(&bash("ls # list")).unwrap();
        assert_eq!(cmd, "set -o pipefail; ls 2>&1 | sqz compress --cmd ls # list");
    }

    #[test]
    fn leading_cd_runs_outside_the_group() {
        let cmd = rewritten(&bash("cd sqz_engine && cargo test 2>&1 | tail -5")).unwrap();
        assert_eq!(
            cmd,
            "cd sqz_engine && { (  cargo test 2>&1 | tail -5\n); printf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; } 2>&1 | sqz compress --cmd 'sh: cargo test 2>&1 | tail -5'"
        );
        let cmd = rewritten(&bash("cd sqz_engine && cargo test 2>&1")).unwrap();
        assert_eq!(
            cmd,
            "set -o pipefail; cd sqz_engine && cargo test 2>&1 | sqz compress --cmd 'cargo test'"
        );
    }

    #[test]
    fn state_changes_pipe_only_the_commands_around_them() {
        assert_eq!(
            rewritten(&bash("export X=1 && make")).as_deref(),
            Some("set -o pipefail; export X=1 && make 2>&1 | sqz compress --cmd make")
        );
        assert_eq!(
            rewritten(&bash("make && cd build")).as_deref(),
            Some("set -o pipefail; make 2>&1 | sqz compress --cmd make && cd build")
        );
    }

    #[test]
    fn followers_background_jobs_and_sqz_pass_through() {
        for command in ["tail -f app.log", "make &", "git log | sqz compress", "cd x && npm run dev"] {
            assert_eq!(rewritten(&bash(command)), None, "{command}");
        }
    }

    #[test]
    fn test_process_hook_rewrites_bash_command() {
        // Use the official Claude Code input format: tool_name + tool_input
        let input = bash("git status");
        let result = process_hook(&input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // Claude Code format: hookSpecificOutput with updatedInput
        let hook_output = &parsed["hookSpecificOutput"];
        assert_eq!(hook_output["hookEventName"].as_str().unwrap(), "PreToolUse");
        // "allow" would skip the permission prompt for the command.
        assert!(hook_output.get("permissionDecision").is_none(), "{hook_output}");
        assert!(hook_output.get("permissionDecisionReason").is_none(), "{hook_output}");
        // updatedInput for Claude Code (camelCase)
        let cmd = hook_output["updatedInput"]["command"].as_str().unwrap();
        assert!(cmd.contains("sqz compress"), "should pipe through sqz: {cmd}");
        assert!(cmd.contains("git status"), "should preserve original command: {cmd}");
        // Issue #10: the label is now passed as `--cmd NAME`, not as a
        // `SQZ_CMD=NAME` prefix (sh-specific, broken on PowerShell/cmd.exe).
        assert!(
            cmd.contains("--cmd 'git status'"),
            "should pass the full command as --cmd so subcommand formatters dispatch: {cmd}"
        );
        assert!(
            !cmd.contains("SQZ_CMD="),
            "new rewrites must not emit the legacy sh-style env prefix: {cmd}"
        );
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
    fn test_process_hook_rewrites_powershell_command() {
        let input = r#"{"tool_name":"PowerShell","permission_mode":"bypassPermissions","tool_input":{"command":"Get-Content big.log"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .expect("PowerShell tool calls should be rewritten");
        assert!(
            cmd.contains("Get-Content big.log"),
            "should preserve original PowerShell command: {cmd}"
        );
        assert!(
            cmd.contains("sqz compress"),
            "should pipe through sqz compress: {cmd}"
        );
        assert!(
            cmd.contains("--cmd 'Get-Content big.log'"),
            "should pass full command via --cmd: {cmd}"
        );
    }

    #[test]
    fn test_process_hook_rewrites_pwsh_command() {
        let input = r#"{"tool_name":"pwsh","permission_mode":"bypassPermissions","tool_input":{"command":"ls"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            parsed["hookSpecificOutput"]["updatedInput"]["command"].is_string(),
            "pwsh tool calls should be rewritten"
        );
    }

    #[test]
    fn bash_rewrite_carries_exit_status_trailer() {
        let input = r#"{"tool_name":"run_shell_command","tool_input":{"command":"cargo test"}}"#;
        let result = process_hook_gemini(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["hookSpecificOutput"]["tool_input"]["command"]
            .as_str()
            .unwrap();
        assert_eq!(
            cmd,
            "{ ( cargo test\n); printf '\\n__SQZ_EXIT_%d__\\n' \"$?\"; } 2>&1 | sqz compress --cmd 'cargo test'",
            "POSIX rewrite must send the command's exit status through the pipe"
        );
        // Claude Code gets the ungrouped form, where pipefail carries it.
        let cmd = rewritten(&bash("cargo test")).unwrap();
        assert_eq!(cmd, "set -o pipefail; cargo test 2>&1 | sqz compress --cmd 'cargo test'");
    }

    #[test]
    fn powershell_rewrite_uses_lastexitcode_not_printf() {
        let input = r#"{"tool_name":"PowerShell","permission_mode":"bypassPermissions","tool_input":{"command":"cargo test"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains("$LASTEXITCODE"), "PowerShell form uses $LASTEXITCODE: {cmd}");
        assert!(cmd.contains("__SQZ_EXIT_"), "PowerShell form emits the trailer: {cmd}");
        assert!(!cmd.contains("printf"), "printf is not valid PowerShell: {cmd}");
    }

    #[test]
    fn exit_trailer_rewrite_is_not_rewrapped() {
        // Feed the hook its own output — the anti-rewrap guard must catch
        // the new brace-group form (it contains `sqz compress --cmd`).
        let first = process_hook(&bash("git log")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&first).unwrap();
        let rewritten = parsed["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        let second_input = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": rewritten }
        })
        .to_string();
        let second = process_hook(&second_input).unwrap();
        assert_eq!(
            second, second_input,
            "already-rewritten commands must pass through unchanged"
        );
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
        assert!(parsed.get("decision").is_none(), "the policy engine decides: {parsed}");
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
    fn gemini_tool_input_keeps_every_argument() {
        // The scheduler replaces the call's arguments with tool_input.
        let input = r#"{"tool_name":"run_shell_command","tool_input":{"command":"git log","dir_path":"sub","description":"log"}}"#;
        let parsed: serde_json::Value =
            serde_json::from_str(&process_hook_gemini(input).unwrap()).unwrap();
        let args = &parsed["hookSpecificOutput"]["tool_input"];
        assert_eq!(args["dir_path"], "sub");
        assert_eq!(args["description"], "log");
        assert!(args["command"].as_str().unwrap().contains("sqz compress"));
    }

    #[test]
    fn test_process_hook_legacy_format() {
        // Test backward compatibility with older toolName/toolCall format
        let input = r#"{"toolName":"Bash","permission_mode":"bypassPermissions","toolCall":{"command":"git status"}}"#;
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
        // Issue #10: label is passed as `--cmd`, not `SQZ_CMD=` prefix.
        assert!(
            cmd.contains("--cmd 'cargo test'"),
            "full command must be passed via --cmd flag: {cmd}"
        );
        assert!(!cmd.contains("SQZ_CMD="), "must not emit legacy env prefix: {cmd}");
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

    // ── Issue #10: Windows shell compatibility ────────────────────────────

    /// The rewritten command must use shell-neutral syntax so it works
    /// in PowerShell and cmd.exe on Windows, not just POSIX shells.
    ///
    /// The old form `SQZ_CMD=val cmd` is sh-specific: PowerShell parses
    /// `SQZ_CMD=val` as a command name (CommandNotFoundException), and
    /// cmd.exe does the same. OpenCode Desktop on Windows routes the
    /// bash tool through PowerShell (or cmd.exe when $SHELL is unset),
    /// so the old form produced zero compression and a spurious error
    /// dialog.
    ///
    /// Reported in issue #10. The fix: pass the label as `--cmd NAME`,
    /// a normal CLI argument that every shell accepts.
    #[test]
    fn issue_10_rewrite_is_shell_neutral() {
        let input = bash("dotnet build NewNeonCheckers3.sln");
        let result = process_hook(&input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let cmd = parsed["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();

        // Must use the --cmd flag form, carrying the full command.
        assert!(
            cmd.contains("--cmd 'dotnet build NewNeonCheckers3.sln'"),
            "issue #10: rewrite must pass label via --cmd, got: {cmd}"
        );
        // Must NOT use the sh-specific inline-env-var form.
        assert!(
            !cmd.contains("SQZ_CMD="),
            "issue #10: rewrite must NOT emit `SQZ_CMD=` prefix \
             (broken in PowerShell and cmd.exe), got: {cmd}"
        );
        // Reporter's original command must still be intact.
        assert!(
            cmd.contains("dotnet build NewNeonCheckers3.sln"),
            "original command must be preserved verbatim: {cmd}"
        );
        // And the pipe to sqz must be there.
        assert!(cmd.contains("| sqz compress"), "must pipe through sqz: {cmd}");
    }

    /// The already-wrapped guard must recognise the new `--cmd` form so
    /// that a command the hook has already rewritten doesn't get
    /// wrapped again (causing `… | sqz compress --cmd X 2>&1 | sqz
    /// compress --cmd sqz` chains).
    ///
    /// This is the runaway-prefix bug from issue #5 rephrased for the
    /// new emission form.
    #[test]
    fn issue_10_already_wrapped_command_passes_through() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"git status 2>&1 | sqz compress --cmd git"}}"#;
        let result = process_hook(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // The hook must leave an already-wrapped command alone.
        // (When the guard short-circuits we return the input verbatim.)
        assert_eq!(
            result, input,
            "already-wrapped command must pass through unchanged; \
             otherwise each pass accumulates another `| sqz compress` tail"
        );
        // And explicitly verify the non-rewritten `command` is still the
        // original, so someone reading the hook response doesn't think
        // we silently re-wrapped.
        let _ = parsed; // suppress "unused" in case of future assertion adds
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
        assert_eq!(windsurf.config_path, PathBuf::from(".windsurf/rules/sqz.md"),
            "Windsurf should use the .windsurf/rules/ directory (legacy .windsurfrules is deprecated)");
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

    #[test]
    fn test_claude_config_includes_precompact_hook() {
        // The PreCompact hook is what keeps sqz's dedup refs from dangling
        // after Claude Code auto-compacts. Without this entry, cached refs
        // can point at content the LLM no longer has in context.
        // Documented at docs.anthropic.com/en/docs/claude-code/hooks-guide.
        let configs = generate_hook_configs("sqz");
        let claude = configs.iter().find(|c| c.tool_name == "Claude Code").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&claude.config_content)
            .expect("Claude Code config must be valid JSON");

        let precompact = parsed["hooks"]["PreCompact"]
            .as_array()
            .expect("PreCompact hook array must be present");
        assert!(
            !precompact.is_empty(),
            "PreCompact must have at least one registered hook"
        );

        let cmd = precompact[0]["hooks"][0]["command"]
            .as_str()
            .expect("command field must be a string");
        assert!(
            cmd.ends_with(" hook precompact"),
            "PreCompact hook should invoke `sqz hook precompact`; got: {cmd}"
        );
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
        // Verify files were created. Copilot CLI installs at user scope
        // (~/.copilot or $COPILOT_HOME), not under the project dir, so its
        // placeholder config_path is not checked here.
        for name in &installed {
            if name == "Copilot CLI" {
                continue;
            }
            let configs = generate_hook_configs("sqz");
            let config = configs.iter().find(|c| &c.tool_name == name).unwrap();
            // With user-level Kiro steering on this machine, project scope
            // writes only the MCP config.
            let path = if name == "Kiro" && crate::kiro_integration::user_steering_installed() {
                dir.path().join(".kiro/settings/mcp.json")
            } else {
                dir.path().join(&config.config_path)
            };
            assert!(path.exists(), "hook config should exist: {}", path.display());
        }
    }

    #[test]
    fn test_install_tool_hooks_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        // First install
        install_tool_hooks(dir.path(), "sqz");
        // Write a custom file to one of the paths. It is deliberately NOT
        // valid JSON: the Claude Code merge path refuses to touch files it
        // can't parse, and every other tool's config is skip-if-exists.
        let custom_path = dir.path().join(".claude/settings.local.json");
        std::fs::write(&custom_path, "custom content").unwrap();
        // Second install must leave the unparseable file alone.
        install_tool_hooks(dir.path(), "sqz");
        let content = std::fs::read_to_string(&custom_path).unwrap();
        assert_eq!(content, "custom content", "should not overwrite existing config");
    }

    /// Regression for the issue #26 leftover: a project-scope settings file
    /// from a pre-PowerShell install (matcher "Bash", old binary path) must
    /// be upgraded in place when `sqz init` runs again — while every key
    /// the user added themselves survives untouched. The old behavior was
    /// skip-if-exists, which left Windows installs on the stale matcher
    /// forever.
    #[test]
    fn test_reinit_upgrades_stale_project_scope_matcher() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join(".claude").join("settings.local.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        // A pre-issue-#26 install: Bash-only matcher, stale binary path,
        // plus keys the user added by hand.
        let stale = serde_json::json!({
            "permissions": { "allow": ["Bash(ls:*)"] },
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [{ "type": "command", "command": "/old/path/sqz hook claude" }]
                    },
                    {
                        "matcher": "Edit",
                        "hooks": [{ "type": "command", "command": "my-linter --check" }]
                    }
                ]
            }
        });
        std::fs::write(&settings_path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();

        install_tool_hooks(dir.path(), "/new/path/sqz");

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // sqz's entry: matcher upgraded, binary path refreshed, no duplicate.
        let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        let sqz_entries: Vec<_> = pre
            .iter()
            .filter(|e| {
                e["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|c| c.contains("hook claude"))
            })
            .collect();
        assert_eq!(sqz_entries.len(), 1, "exactly one sqz PreToolUse entry: {content}");
        assert_eq!(
            sqz_entries[0]["matcher"].as_str().unwrap(),
            "Bash|PowerShell",
            "stale Bash-only matcher must be upgraded"
        );
        assert!(
            sqz_entries[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("/new/path/sqz"),
            "binary path must be refreshed"
        );

        // The user's own entries survive.
        assert!(
            pre.iter().any(|e| e["matcher"] == "Edit"),
            "user's non-sqz PreToolUse entry must be preserved"
        );
        assert_eq!(
            parsed["permissions"]["allow"][0].as_str().unwrap(),
            "Bash(ls:*)",
            "user's permissions must be preserved"
        );

        // The other two sqz hooks got added too (full heal, not partial).
        assert!(parsed["hooks"]["PreCompact"].is_array());
        assert!(parsed["hooks"]["SessionStart"].is_array());
    }

    #[test]
    fn test_windsurf_installs_to_rules_directory() {
        let dir = tempfile::tempdir().unwrap();
        install_tool_hooks(dir.path(), "sqz");
        assert!(dir.path().join(".windsurf/rules/sqz.md").exists());
        assert!(!dir.path().join(".windsurfrules").exists());
    }

    fn legacy_rules(sqz: &str) -> String {
        format!(
            "# sqz \u{2014} Token-Optimized CLI Output\n\n```bash\ngit status        git status 2>&1 | {sqz} compress\n```\n\nUse these when you genuinely need raw data. The default compressed\npath is still what you want for most output.\n"
        )
    }

    #[test]
    fn test_windsurf_migrates_legacy_sqz_rules_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".windsurfrules"), legacy_rules("sqz")).unwrap();
        install_tool_hooks(dir.path(), "sqz");
        assert!(dir.path().join(".windsurf/rules/sqz.md").exists());
        assert!(!dir.path().join(".windsurfrules").exists(), "legacy file migrated");
    }

    #[test]
    fn test_windsurf_migration_keeps_user_rules_after_the_sqz_text() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(".windsurfrules");
        std::fs::write(&legacy, format!("{}\n## Team rules\n\nUse tabs.\n", legacy_rules("sqz"))).unwrap();
        install_tool_hooks(dir.path(), "sqz");
        assert!(dir.path().join(".windsurf/rules/sqz.md").exists());
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), "## Team rules\n\nUse tabs.\n");
    }

    #[test]
    fn test_windsurf_leaves_user_legacy_rules_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".windsurfrules"), "my own rules\n").unwrap();
        install_tool_hooks(dir.path(), "sqz");
        assert!(dir.path().join(".windsurf/rules/sqz.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".windsurfrules")).unwrap(),
            "my own rules\n"
        );
    }

    #[test]
    fn test_cline_installs_into_rules_directory_when_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".clinerules")).unwrap();
        std::fs::write(dir.path().join(".clinerules/mine.md"), "user rule\n").unwrap();
        install_tool_hooks(dir.path(), "sqz");
        assert!(dir.path().join(".clinerules/sqz.md").exists());
        assert!(dir.path().join(".clinerules/mine.md").exists());
    }

    #[test]
    fn test_cline_single_file_untouched_when_user_authored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".clinerules"), "user rules\n").unwrap();
        install_tool_hooks(dir.path(), "sqz");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".clinerules")).unwrap(),
            "user rules\n"
        );
    }

    /// Re-running init on an already-current project file is a no-op.
    #[test]
    fn test_reinit_is_idempotent_at_project_scope() {
        let dir = tempfile::tempdir().unwrap();
        install_tool_hooks(dir.path(), "/usr/bin/sqz");
        let settings_path = dir.path().join(".claude").join("settings.local.json");
        let first = std::fs::read_to_string(&settings_path).unwrap();

        install_tool_hooks(dir.path(), "/usr/bin/sqz");
        let second = std::fs::read_to_string(&settings_path).unwrap();

        let a: serde_json::Value = serde_json::from_str(&first).unwrap();
        let b: serde_json::Value = serde_json::from_str(&second).unwrap();
        assert_eq!(a, b, "second init must not change project settings");
        // No duplicated sqz entries.
        assert_eq!(b["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn init_refreshes_old_sqz_rules_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let filter = ToolFilter::Only(vec!["cursor".into(), "windsurf".into(), "cline".into()]);
        let old_cursor = "---\ndescription: sqz \u{2014} Token-Optimized CLI Output\nalwaysApply: true\n---\n\n# sqz \u{2014} Token-Optimized CLI Output\n\ngit status        git status 2>&1 | /old/sqz compress\n";
        std::fs::create_dir_all(dir.path().join(".cursor/rules")).unwrap();
        std::fs::write(dir.path().join(".cursor/rules/sqz.mdc"), old_cursor).unwrap();
        std::fs::write(dir.path().join(".clinerules"), legacy_rules("/old/sqz")).unwrap();

        let installed =
            install_tool_hooks_scoped_filtered(dir.path(), "/new/sqz", InstallScope::Project, &filter);
        assert_eq!(installed, vec!["Cursor", "Windsurf", "Cline"]);
        for (tool, rel) in [
            ("Cursor", ".cursor/rules/sqz.mdc"),
            ("Windsurf", ".windsurf/rules/sqz.md"),
            ("Cline", ".clinerules"),
        ] {
            let content = std::fs::read_to_string(dir.path().join(rel)).unwrap();
            assert_eq!(content, sqz_rules_content(tool, "/new/sqz"), "{tool}");
            assert!(!content.contains('%') && !content.contains('\u{2014}'), "{tool}");
        }
        assert!(
            install_tool_hooks_scoped_filtered(dir.path(), "/new/sqz", InstallScope::Project, &filter)
                .is_empty()
        );
    }

    #[test]
    fn init_leaves_user_written_rules_alone() {
        let dir = tempfile::tempdir().unwrap();
        let cursor = dir.path().join(".cursor/rules/sqz.mdc");
        std::fs::create_dir_all(cursor.parent().unwrap()).unwrap();
        std::fs::write(&cursor, "my own rule\n").unwrap();
        let filter = ToolFilter::Only(vec!["cursor".into()]);
        assert!(
            install_tool_hooks_scoped_filtered(dir.path(), "/new/sqz", InstallScope::Project, &filter)
                .is_empty()
        );
        assert_eq!(std::fs::read_to_string(&cursor).unwrap(), "my own rule\n");
    }

    #[test]
    fn rules_refresh_keeps_the_binary_the_old_text_points_at() {
        use crate::kiro_integration::SteeringState;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("sqz");
        std::fs::write(&bin, "").unwrap();
        let bin = bin.to_string_lossy().to_string();
        let path = dir.path().join("sqz.md");
        std::fs::write(
            &path,
            format!("# sqz \u{2014} Token-Optimized CLI Output\n\ngit status        git status 2>&1 | {bin} compress\n"),
        )
        .unwrap();

        assert_eq!(sqz_rules_state(&path, "Windsurf"), SteeringState::Stale);
        assert!(refresh_sqz_rules(&path, "Windsurf", "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), sqz_rules_content("Windsurf", &bin));
        assert_eq!(sqz_rules_state(&path, "Windsurf"), SteeringState::Current);
        assert!(!refresh_sqz_rules(&path, "Windsurf", "/fallback/sqz").unwrap());

        std::fs::write(&path, "my own rules\n").unwrap();
        assert_eq!(sqz_rules_state(&path, "Windsurf"), SteeringState::UserAuthored);
        assert!(!refresh_sqz_rules(&path, "Windsurf", "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "my own rules\n");
    }

    #[test]
    fn cline_rules_keep_the_user_rules_after_them() {
        use crate::kiro_integration::SteeringState;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".clinerules");
        let filter = ToolFilter::Only(vec!["cline".into()]);
        let install = |sqz: &str| {
            install_tool_hooks_scoped_filtered(dir.path(), sqz, InstallScope::Project, &filter)
        };
        let mine = "\n## Team rules\n\nUse tabs.\n";
        std::fs::write(&path, format!("{}{mine}", legacy_rules("/old/sqz"))).unwrap();

        assert_eq!(sqz_rules_state(&path, "Cline"), SteeringState::Stale);
        assert_eq!(install("/new/sqz"), vec!["Cline"]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}{mine}", sqz_rules_content("Cline", "/new/sqz"))
        );
        assert_eq!(sqz_rules_state(&path, "Cline"), SteeringState::Current);
        assert!(install("/new/sqz").is_empty());

        assert_eq!(install("/other/sqz"), vec!["Cline"]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}{mine}", sqz_rules_content("Cline", "/other/sqz"))
        );

        assert!(remove_sqz_rules(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "## Team rules\n\nUse tabs.\n");
        assert!(!has_sqz_rules(&path));
    }

    #[test]
    fn cline_refresh_keeps_the_old_binary_and_the_user_rules() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("sqz");
        std::fs::write(&bin, "").unwrap();
        let bin = bin.to_string_lossy().to_string();
        let path = dir.path().join(".clinerules");
        let v2_ending = "and commands with shell operators. If sqz is\nnot on PATH, run commands normally.\n";
        let old = legacy_rules(&bin).replace(
            "Use these when you genuinely need raw data. The default compressed\npath is still what you want for most output.\n",
            v2_ending,
        );
        std::fs::write(&path, format!("{old}\nUse tabs.\n")).unwrap();

        assert!(refresh_sqz_rules(&path, "Cline", "/fallback/sqz").unwrap());
        let block = sqz_rules_content("Cline", &bin);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{block}\nUse tabs.\n"));
        assert!(!refresh_sqz_rules(&path, "Cline", "/fallback/sqz").unwrap());

        std::fs::write(&path, format!("{}\nUse tabs.\n", block.replace("sqz_recall", "recall"))).unwrap();
        assert!(refresh_sqz_rules(&path, "Cline", "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{block}\nUse tabs.\n"));
    }

    #[test]
    fn cline_rules_without_a_known_end_are_left_alone() {
        use crate::kiro_integration::SteeringState;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".clinerules");
        let edited = "# sqz \u{2014} Token-Optimized CLI Output\n\nPipe output through sqz.\n\n## Team rules\n";
        std::fs::write(&path, edited).unwrap();
        let filter = ToolFilter::Only(vec!["cline".into()]);

        assert_eq!(sqz_rules_state(&path, "Cline"), SteeringState::UserAuthored);
        assert!(
            install_tool_hooks_scoped_filtered(dir.path(), "/new/sqz", InstallScope::Project, &filter)
                .is_empty()
        );
        assert!(!refresh_sqz_rules(&path, "Cline", "/fallback/sqz").unwrap());
        assert!(!remove_sqz_rules(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }
}

#[cfg(test)]
mod global_install_tests {
    use super::*;

    #[test]
    fn global_install_creates_fresh_settings_json() {
        let tmp = tempfile::tempdir().unwrap();
        let changed = install_claude_global_at("/usr/local/bin/sqz", Some(tmp.path())).unwrap();
        assert!(changed, "first install should report a change");

        let path = tmp.path().join(".claude").join("settings.json");
        assert!(path.exists(), "user settings.json should be created");

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // All three hook entries should be present.
        let pre = &parsed["hooks"]["PreToolUse"];
        assert!(pre.is_array(), "PreToolUse should be an array");
        assert_eq!(pre.as_array().unwrap().len(), 1);
        let cmd = pre[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(
            cmd.contains("/usr/local/bin/sqz"),
            "hook command should use the passed sqz_path, got: {cmd}"
        );
        assert!(cmd.contains("hook claude"));

        let precompact = &parsed["hooks"]["PreCompact"];
        assert!(precompact.is_array());
        let precompact_cmd = precompact[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(precompact_cmd.contains("hook precompact"));

        let session = &parsed["hooks"]["SessionStart"];
        assert!(session.is_array());
        assert_eq!(
            session[0]["matcher"].as_str().unwrap(),
            "compact",
            "SessionStart should only match /compact resume"
        );
    }

    #[test]
    fn global_install_preserves_existing_user_config() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();

        let existing = serde_json::json!({
            "permissions": {
                "allow": ["Bash(npm test *)"],
                "deny":  ["Read(./.env)"]
            },
            "env": { "FOO": "bar" },
            "statusLine": {
                "type": "command",
                "command": "~/.claude/statusline.sh"
            },
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Edit",
                        "hooks": [
                            {
                                "type": "command",
                                "command": "~/.claude/hooks/format-on-edit.sh"
                            }
                        ]
                    }
                ]
            }
        });
        std::fs::write(&settings, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        let changed = install_claude_global_at("/usr/local/bin/sqz", Some(tmp.path())).unwrap();
        assert!(changed, "install should report a change on new hook");

        let content = std::fs::read_to_string(&settings).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // User's permissions survived.
        assert_eq!(
            parsed["permissions"]["allow"][0].as_str().unwrap(),
            "Bash(npm test *)"
        );
        assert_eq!(
            parsed["permissions"]["deny"][0].as_str().unwrap(),
            "Read(./.env)"
        );
        // User's env block survived.
        assert_eq!(parsed["env"]["FOO"].as_str().unwrap(), "bar");
        // User's statusLine survived.
        assert_eq!(
            parsed["statusLine"]["command"].as_str().unwrap(),
            "~/.claude/statusline.sh"
        );

        // PreToolUse should now contain BOTH the user's format-on-edit
        // hook and sqz's Bash hook — our install appends, not replaces.
        let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "expected user's hook + sqz's hook, got: {pre:?}");
        let matchers: Vec<&str> = pre
            .iter()
            .map(|e| e["matcher"].as_str().unwrap_or(""))
            .collect();
        assert!(matchers.contains(&"Edit"), "user's Edit hook must survive");
        assert!(
            matchers.contains(&"Bash|PowerShell"),
            "sqz shell hook must be present (Bash|PowerShell matcher covers Windows CC, issue #26)"
        );
    }

    #[test]
    fn global_install_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(install_claude_global_at("sqz", Some(tmp.path())).unwrap());
        assert!(
            !install_claude_global_at("sqz", Some(tmp.path())).unwrap(),
            "second install with identical args should report no change"
        );

        let path = tmp.path().join(".claude").join("settings.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for event in &["PreToolUse", "PreCompact", "SessionStart"] {
            let arr = parsed["hooks"][event].as_array().unwrap();
            assert_eq!(
                arr.len(),
                1,
                "{event} must have exactly one sqz entry after 2 installs, got {arr:?}"
            );
        }
    }

    /// Regression for issue #21: on Windows, `sqz_path` contains `.exe`
    /// so the stored command is `C:\...\sqz.exe hook claude`. The old
    /// sentinel was `"sqz hook claude"` which is NOT a substring of
    /// `"sqz.exe hook claude"` — causing every re-run to append a
    /// duplicate instead of replacing. The fix uses `"hook claude"` as
    /// the sentinel which matches regardless of `.exe`.
    #[test]
    fn global_install_is_idempotent_with_exe_path() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_path = r"C:\Users\user\.cargo\bin\sqz.exe";
        assert!(install_claude_global_at(exe_path, Some(tmp.path())).unwrap());
        assert!(
            !install_claude_global_at(exe_path, Some(tmp.path())).unwrap(),
            "second install with .exe path should report no change (issue #21)"
        );

        let path = tmp.path().join(".claude").join("settings.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for event in &["PreToolUse", "PreCompact", "SessionStart"] {
            let arr = parsed["hooks"][event].as_array().unwrap();
            assert_eq!(
                arr.len(),
                1,
                "{event} must have exactly one entry after 2 installs with .exe path, got {arr:?}"
            );
        }
    }

    #[test]
    fn global_install_upgrades_stale_sqz_hook_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        install_claude_global_at("/old/path/sqz", Some(tmp.path())).unwrap();
        let changed = install_claude_global_at("/new/path/sqz", Some(tmp.path())).unwrap();
        assert!(changed, "different sqz_path must be seen as a change");

        let path = tmp.path().join(".claude").join("settings.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "stale sqz entry must be replaced, not duplicated");
        let cmd = pre[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.contains("/new/path/sqz"));
        assert!(!cmd.contains("/old/path/sqz"));
    }

    #[test]
    fn global_uninstall_removes_sqz_and_preserves_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            serde_json::json!({
                "permissions": { "allow": ["Bash(git status)"] },
                "hooks": {
                    "PreToolUse": [
                        {
                            "matcher": "Edit",
                            "hooks": [
                                { "type": "command", "command": "~/format.sh" }
                            ]
                        }
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();

        install_claude_global_at("/usr/local/bin/sqz", Some(tmp.path())).unwrap();
        let result = remove_claude_global_hook_at(Some(tmp.path())).unwrap().unwrap();
        assert_eq!(result.0, settings);
        assert!(result.1, "should report that the file was modified");

        assert!(settings.exists(), "settings.json should be preserved");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();

        assert_eq!(
            parsed["permissions"]["allow"][0].as_str().unwrap(),
            "Bash(git status)"
        );

        let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "only the user's Edit hook should remain");
        assert_eq!(pre[0]["matcher"].as_str().unwrap(), "Edit");

        assert!(parsed["hooks"].get("PreCompact").is_none());
        assert!(parsed["hooks"].get("SessionStart").is_none());
    }

    #[test]
    fn global_uninstall_deletes_settings_json_if_it_was_sqz_only() {
        let tmp = tempfile::tempdir().unwrap();
        install_claude_global_at("sqz", Some(tmp.path())).unwrap();
        let path = tmp.path().join(".claude").join("settings.json");
        assert!(path.exists(), "precondition: install created the file");

        let result = remove_claude_global_hook_at(Some(tmp.path())).unwrap().unwrap();
        assert!(result.1);
        assert!(!path.exists(), "sqz-only settings.json should be removed on uninstall");
    }

    #[test]
    fn global_uninstall_on_missing_file_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            remove_claude_global_hook_at(Some(tmp.path())).unwrap().is_none(),
            "missing file should return None, not error"
        );
    }

    #[test]
    fn global_uninstall_refuses_to_touch_unparseable_file() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{ invalid json because").unwrap();

        assert!(
            remove_claude_global_hook_at(Some(tmp.path())).is_err(),
            "bad JSON must surface as an error"
        );

        let after = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(after, "{ invalid json because");
    }
}

#[cfg(test)]
mod issue_11_tool_filter_tests {
    //! Regression tests for issue #11 (@shochdoerfer): let the user
    //! choose for which agent sqz init creates configs.
    //!
    //! Every assertion here pins a behaviour the filter should have.
    //! If any one of these flips, users are either getting configs for
    //! tools they asked to skip OR getting no config for tools they
    //! explicitly asked for — both are issue #11 regressions.

    use super::*;

    #[test]
    fn canonicalize_collapses_common_aliases() {
        // Each row: list of aliases a user might type, followed by the
        // canonical form they should all normalise to.
        for aliases in &[
            (vec!["Claude Code", "claude-code", "claude", "CLAUDE", "ClaudeCode"], "claudecode"),
            (vec!["Cursor", "cursor", "CURSOR"], "cursor"),
            (vec!["Windsurf", "WINDSURF"], "windsurf"),
            // Cline is also marketed as "Roo Code" — sqz treats them
            // as one integration (same .clinerules file) so the
            // aliases must collapse.
            (vec!["Cline", "cline", "Roo", "roo-code", "RooCode"], "cline"),
            (vec!["Gemini CLI", "gemini-cli", "gemini", "GEMINI"], "gemini"),
            (vec!["OpenCode", "open-code", "opencode", "OPENCODE"], "opencode"),
            (vec!["Codex", "codex"], "codex"),
        ] {
            for alias in &aliases.0 {
                assert_eq!(
                    canonicalize_tool_name(alias),
                    aliases.1,
                    "alias '{}' must canonicalise to '{}'",
                    alias,
                    aliases.1
                );
            }
        }
    }

    #[test]
    fn canonicalize_leaves_unknown_names_unchanged_but_normalised() {
        // Unknown names fall through — we don't guess. The caller
        // (parse_tool_list) is responsible for turning unknown values
        // into user-facing errors; canonicalize itself just
        // normalises case, whitespace, and hyphens/underscores.
        assert_eq!(canonicalize_tool_name("unknown-tool"), "unknowntool");
        assert_eq!(canonicalize_tool_name("Some Thing"), "something");
    }

    #[test]
    fn parse_tool_list_accepts_comma_separated_with_whitespace() {
        // Typical shell invocation: `--only opencode,codex` or
        // `--only "opencode, codex"` — both should work.
        let names = parse_tool_list("opencode,codex").unwrap();
        assert_eq!(names, vec!["opencode", "codex"]);

        let names = parse_tool_list(" opencode ,  codex ").unwrap();
        assert_eq!(names, vec!["opencode", "codex"]);

        // Single entry.
        let names = parse_tool_list("opencode").unwrap();
        assert_eq!(names, vec!["opencode"]);

        // Alias with hyphen.
        let names = parse_tool_list("claude-code").unwrap();
        assert_eq!(names, vec!["claudecode"]);
    }

    #[test]
    fn parse_tool_list_dedupes_repeated_entries() {
        // `--only opencode,opencode` shouldn't produce two "opencode"
        // entries that the filter then matches twice. Harmless today
        // but tomorrow someone could code `filter.allow.len()` against
        // it and silently count wrong.
        let names = parse_tool_list("opencode,opencode").unwrap();
        assert_eq!(names, vec!["opencode"]);

        // Same name via different aliases still dedupes because they
        // canonicalise to the same string.
        let names = parse_tool_list("Claude Code, claude, claude-code").unwrap();
        assert_eq!(names, vec!["claudecode"]);
    }

    #[test]
    fn parse_tool_list_rejects_unknown_names_with_helpful_error() {
        // This is the critical failure path: a typo in --only must
        // fail, not silently drop. Otherwise the user runs `sqz init
        // --only opncode` (typo), sees "OK" with nothing installed,
        // and spends 20 minutes figuring out why. The error message
        // must list valid options verbatim so they can spot their
        // typo.
        let err = parse_tool_list("opncode").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown agent name 'opncode'"),
            "error must quote the bad input: {msg}"
        );
        assert!(msg.contains("opencode"), "error must list valid options: {msg}");
        assert!(msg.contains("cursor"), "error must list valid options: {msg}");
    }

    #[test]
    fn parse_tool_list_rejects_one_bad_entry_in_a_list() {
        // `--only opencode,xyz` — fail the whole list, don't just drop
        // xyz. User either typed a typo they need to see, or they
        // don't understand the vocabulary — in both cases pretending
        // the input was valid hurts them.
        let err = parse_tool_list("opencode,xyz").unwrap_err();
        assert!(err.to_string().contains("xyz"));
    }

    #[test]
    fn parse_tool_list_empty_and_whitespace_return_empty_vec() {
        // `--only ""` and `--only "   "` produce an empty filter —
        // semantically "install nothing" rather than "install all".
        // cmd_init surfaces this as ToolFilter::Only(empty) which
        // skips every config file but still installs the shell hook
        // and preset.
        assert_eq!(parse_tool_list("").unwrap(), Vec::<String>::new());
        assert_eq!(parse_tool_list("   ").unwrap(), Vec::<String>::new());
        assert_eq!(parse_tool_list(" , , ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn tool_filter_all_includes_every_supported_tool() {
        let filter = ToolFilter::All;
        for tool in SUPPORTED_TOOL_NAMES {
            assert!(
                filter.includes(tool),
                "default filter must include {tool}"
            );
        }
    }

    #[test]
    fn tool_filter_only_opencode_excludes_everything_else() {
        // The exact scenario from issue #11.
        let filter = ToolFilter::Only(vec!["opencode".to_string()]);
        assert!(filter.includes("OpenCode"));
        // Every other supported tool is rejected.
        for tool in SUPPORTED_TOOL_NAMES {
            if *tool == "OpenCode" {
                continue;
            }
            assert!(
                !filter.includes(tool),
                "--only opencode must not include {tool}"
            );
        }
    }

    #[test]
    fn tool_filter_only_multi_tool_includes_exactly_those() {
        let filter = ToolFilter::Only(vec!["opencode".to_string(), "codex".to_string()]);
        assert!(filter.includes("OpenCode"));
        assert!(filter.includes("Codex"));
        // Everything else: excluded.
        assert!(!filter.includes("Claude Code"));
        assert!(!filter.includes("Cursor"));
        assert!(!filter.includes("Windsurf"));
        assert!(!filter.includes("Cline"));
        assert!(!filter.includes("Gemini CLI"));
    }

    #[test]
    fn tool_filter_skip_inverts_the_set() {
        // `--skip cursor,windsurf` means "install everything except
        // those two." Opposite of --only.
        let filter = ToolFilter::Skip(vec!["cursor".to_string(), "windsurf".to_string()]);
        assert!(!filter.includes("Cursor"));
        assert!(!filter.includes("Windsurf"));
        // Everything else stays on.
        assert!(filter.includes("Claude Code"));
        assert!(filter.includes("Cline"));
        assert!(filter.includes("Gemini CLI"));
        assert!(filter.includes("OpenCode"));
        assert!(filter.includes("Codex"));
    }

    #[test]
    fn tool_filter_only_empty_excludes_everything() {
        // Edge: `--only ""` → empty filter → nothing passes. This is
        // semantically "install no AI-tool configs, just the shell
        // hook and preset." Surprising if the user typed it by
        // accident, but consistent — the plan output will show only
        // shell/preset lines and the user can abort.
        let filter = ToolFilter::Only(vec![]);
        for tool in SUPPORTED_TOOL_NAMES {
            assert!(
                !filter.includes(tool),
                "empty --only must exclude every tool, got {tool}"
            );
        }
    }

    #[test]
    fn tool_filter_only_accepts_display_name_or_canonical() {
        // The filter lives in the engine; callers pass the
        // display-name strings ("Claude Code", "Gemini CLI") straight
        // from generate_hook_configs. But filter entries come from
        // the CLI via parse_tool_list, which canonicalises. Both
        // sides must line up — assert the cross-path works.
        let filter = ToolFilter::Only(vec!["claudecode".to_string()]);
        assert!(filter.includes("Claude Code"));
        assert!(!filter.includes("Cursor"));

        let filter = ToolFilter::Only(vec!["gemini".to_string()]);
        assert!(filter.includes("Gemini CLI"));
    }

    #[test]
    fn supported_tool_names_matches_generate_hook_configs_exactly() {
        // Invariant: the SUPPORTED_TOOL_NAMES constant (used in error
        // messages, docs, tests) must match the tool_name fields the
        // config generator actually emits. If someone adds a new tool
        // config but forgets to add its name to the constant, this
        // test fails loudly.
        let configs = generate_hook_configs("sqz");
        let emitted: std::collections::HashSet<&str> =
            configs.iter().map(|c| c.tool_name.as_str()).collect();
        let declared: std::collections::HashSet<&str> =
            SUPPORTED_TOOL_NAMES.iter().copied().collect();
        assert_eq!(
            emitted, declared,
            "SUPPORTED_TOOL_NAMES must equal the set of tool_name values \
             from generate_hook_configs. emitted={:?}, declared={:?}",
            emitted, declared
        );
    }

    #[test]
    fn filtered_install_only_opencode_writes_only_opencode_files() {
        // End-to-end: exercise install_tool_hooks_scoped_filtered
        // against a tempdir and assert that only OpenCode's files
        // appear. This is the single most important regression for
        // issue #11: no Cursor rules, no Windsurf rules, no Cline
        // rules, no Gemini settings, no AGENTS.md, no .claude/.
        let dir = tempfile::tempdir().unwrap();
        let filter = ToolFilter::Only(vec!["opencode".to_string()]);
        let _installed = install_tool_hooks_scoped_filtered(
            dir.path(),
            "sqz",
            InstallScope::Project,
            &filter,
        );

        // OpenCode SHOULD be there.
        assert!(
            dir.path().join("opencode.json").exists(),
            "OpenCode config must be written when --only opencode is used"
        );

        // None of the other agents' files should exist.
        for (path, tool) in &[
            (".claude/settings.local.json", "Claude Code"),
            (".cursor/rules/sqz.mdc", "Cursor"),
            (".windsurfrules", "Windsurf"),
            (".clinerules", "Cline"),
            (".gemini/settings.json", "Gemini CLI"),
            ("AGENTS.md", "Codex"),
        ] {
            assert!(
                !dir.path().join(path).exists(),
                "filter rejected {tool} but the installer still wrote {path}"
            );
        }
    }

    #[test]
    fn filtered_install_skip_cursor_omits_only_cursor() {
        // Symmetric: --skip cursor should leave everything else intact.
        let dir = tempfile::tempdir().unwrap();
        let filter = ToolFilter::Skip(vec!["cursor".to_string()]);
        let _installed = install_tool_hooks_scoped_filtered(
            dir.path(),
            "sqz",
            InstallScope::Project,
            &filter,
        );

        // Cursor rules must NOT exist.
        assert!(
            !dir.path().join(".cursor/rules/sqz.mdc").exists(),
            "skip cursor: .cursor/rules/sqz.mdc must not be written"
        );
        // Windsurf and Cline rules SHOULD still exist.
        assert!(
            dir.path().join(".windsurf/rules/sqz.md").exists(),
            "skip cursor should not skip windsurf"
        );
        assert!(
            dir.path().join(".clinerules").exists(),
            "skip cursor should not skip cline"
        );
    }
}

#[cfg(test)]
mod ci_hook_tests {
    use super::*;

    #[test]
    fn ci_hook_content_is_valid_v1_config() {
        let parsed: serde_json::Value =
            serde_json::from_str(&ci_hook_content()).expect("CI hook must be valid JSON");
        assert_eq!(parsed["version"], 1);
        let session_start = parsed["hooks"]["sessionStart"].as_array().unwrap();
        assert!(session_start[0]["bash"]
            .as_str()
            .unwrap()
            .contains("install.sh"));
        let pre = parsed["hooks"]["preToolUse"].as_array().unwrap();
        let bash = pre[0]["bash"].as_str().unwrap();
        assert!(bash.contains("hook copilot"));
        // Fail-closed platform: every branch must end in output, never
        // a bare non-zero exit.
        assert!(bash.ends_with("echo '{}'"));
        assert_eq!(pre[0]["matcher"], "bash|powershell");
    }

    #[test]
    fn install_ci_hook_creates_then_noops() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_ci_hook(dir.path()).unwrap());
        let path = ci_hooks_path(dir.path());
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ci_hook_content());
        // Second run: unchanged content, no rewrite reported.
        assert!(!install_ci_hook(dir.path()).unwrap());
    }

    #[test]
    fn install_ci_hook_refreshes_stale_sqz_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = ci_hooks_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"version":1,"hooks":{"preToolUse":[{"bash":"sqz hook copilot"}]}}"#).unwrap();
        assert!(install_ci_hook(dir.path()).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ci_hook_content());
    }

    #[test]
    fn install_ci_hook_leaves_user_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = ci_hooks_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"version":1,"hooks":{}}"#).unwrap();
        assert!(!install_ci_hook(dir.path()).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"version":1,"hooks":{}}"#
        );
    }

    #[test]
    fn remove_ci_hook_only_removes_sqz_authored() {
        let dir = tempfile::tempdir().unwrap();
        // Missing file: no-op.
        assert!(!remove_ci_hook(dir.path()).unwrap());
        // sqz-authored: removed.
        install_ci_hook(dir.path()).unwrap();
        assert!(remove_ci_hook(dir.path()).unwrap());
        assert!(!ci_hooks_path(dir.path()).exists());
        // User-owned: left alone.
        let path = ci_hooks_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{}").unwrap();
        assert!(!remove_ci_hook(dir.path()).unwrap());
        assert!(path.exists());
    }

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn claude_install_adds_the_post_hook_and_allow_rules_once() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"permissions":{"allow":["Bash(npm test)"],"deny":["Bash(rm *)"]}}"#)
            .unwrap();
        assert!(install_claude_global_at("/usr/local/bin/sqz", Some(tmp.path())).unwrap());
        let v = read_json(&path);
        assert_eq!(
            v["permissions"]["allow"],
            serde_json::json!(["Bash(npm test)", "Bash(set -o pipefail)", "Bash(sqz compress --cmd *)"])
        );
        assert_eq!(v["permissions"]["deny"], serde_json::json!(["Bash(rm *)"]));
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            v["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "/usr/local/bin/sqz hook claude-post"
        );
        assert!(claude_rules_installed(&[path.clone()]));
        assert!(!install_claude_global_at("/usr/local/bin/sqz", Some(tmp.path())).unwrap());

        let (_, changed) = remove_claude_global_hook_at(Some(tmp.path())).unwrap().unwrap();
        assert!(changed);
        assert_eq!(
            read_json(&path),
            serde_json::json!({"permissions": {"allow": ["Bash(npm test)"], "deny": ["Bash(rm *)"]}})
        );
    }

    #[test]
    fn fresh_project_settings_carry_the_rules_and_post_hook() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.local.json");
        std::fs::write(&path, claude_settings_content("/usr/bin/sqz")).unwrap();
        assert!(claude_rules_installed(&[path.clone()]));
        assert_eq!(
            read_json(&path)["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "/usr/bin/sqz hook claude-post"
        );
        assert!(!claude_project_settings_needs_update(&path, "/usr/bin/sqz").unwrap());

        // A file from before the rules existed needs the update.
        let mut old = read_json(&path);
        old.as_object_mut().unwrap().remove("permissions");
        std::fs::write(&path, old.to_string()).unwrap();
        assert!(claude_settings_stale(&path));
        assert!(claude_project_settings_needs_update(&path, "/usr/bin/sqz").unwrap());
        assert!(merge_sqz_hooks_into_settings(&path, "/usr/bin/sqz").unwrap());
        assert!(!claude_settings_stale(&path));
    }

    #[test]
    fn project_uninstall_keeps_permissions_claude_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude").join("settings.local.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, claude_settings_content("/usr/bin/sqz")).unwrap();
        // "Yes, and don't ask again" lands in the same file.
        let mut v = read_json(&path);
        v["permissions"]["allow"].as_array_mut().unwrap().push(serde_json::json!("Bash(make)"));
        std::fs::write(&path, v.to_string()).unwrap();

        let (_, changed) = remove_claude_project_hook(dir.path()).unwrap().unwrap();
        assert!(changed);
        assert_eq!(read_json(&path), serde_json::json!({"permissions": {"allow": ["Bash(make)"]}}));

        // A file holding only sqz's entries goes away entirely.
        std::fs::write(&path, claude_settings_content("/usr/bin/sqz")).unwrap();
        remove_claude_project_hook(dir.path()).unwrap();
        assert!(!path.exists());
        assert!(remove_claude_project_hook(dir.path()).unwrap().is_none());
    }

    fn post(command: &str, response: serde_json::Value) -> String {
        serde_json::json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "tool_response": response,
        })
        .to_string()
    }

    #[test]
    fn post_tool_use_picks_successful_bash_output_sqz_never_saw() {
        let ok = serde_json::json!({"stdout": "out", "stderr": "", "interrupted": false, "isImage": false});
        let t = claude_post_tool_use_target(&post("cargo test | tail -5", ok.clone())).unwrap();
        assert_eq!(t.label, "sh: cargo test | tail -5");
        assert_eq!(serde_json::Value::Object(t.response), ok);
        // One command with output: its formatter can run.
        let t = claude_post_tool_use_target(&post("cd a && cargo test", ok.clone())).unwrap();
        assert_eq!(t.label, "cargo test");

        let skipped = [
            post("set -o pipefail; ls 2>&1 | sqz compress --cmd ls", ok.clone()),
            post("sqz stats", ok.clone()),
            post("ls", serde_json::json!({"stdout": "x", "isImage": true})),
            post("ls", serde_json::json!({"stdout": "x", "interrupted": true})),
            post("ls", serde_json::json!({"content": "x"})),
            post("ls", serde_json::json!({"stdout": "a".repeat(30_000), "stderr": ""})),
            serde_json::json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
                "tool_input": {"command": "ls"}, "tool_response": ok}).to_string(),
            serde_json::json!({"hook_event_name": "PostToolUse", "tool_name": "Read",
                "tool_input": {"file_path": "/x"}, "tool_response": ok}).to_string(),
            serde_json::json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
                "tool_input": {"command": "make", "run_in_background": true}, "tool_response": ok}).to_string(),
            "not json".to_string(),
        ];
        for input in skipped {
            assert_eq!(claude_post_tool_use_target(&input), None, "{input}");
        }
    }
}

