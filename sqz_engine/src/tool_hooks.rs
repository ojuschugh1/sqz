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
        // Three hooks, each addressing a different concern:
        //
        //   PreToolUse:   compress Bash tool output before the agent sees it
        //                 (matcher "Bash" keeps other tools untouched)
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
    "PreCompact": [
      {{
        "hooks": [
          {{
            "type": "command",
            "command": "{sqz_path} hook precompact"
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
        //      the same approach RTK uses for Codex and the shape Codex
        //      expects (the cross-tool AGENTS.md standard).
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
///   This is what RTK's `rtk init -g` does and what most users actually
///   want on first install. Verified against the official Anthropic scope
///   table; verified against rtk-ai/rtk's `resolve_claude_dir` helper.
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
    let configs = generate_hook_configs(sqz_path);
    let mut installed = Vec::new();

    for config in &configs {
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

        // Claude Code at global scope: merge into ~/.claude/settings.json
        // instead of writing a fresh .claude/settings.local.json in cwd.
        // This is the fix for "sqz init does nothing outside the project
        // I ran it in" — reported by 76vangel. Design mirrors rtk init -g.
        if config.tool_name == "Claude Code" && scope == InstallScope::Global {
            match install_claude_global(sqz_path) {
                Ok(true) => installed.push("Claude Code".to_string()),
                Ok(false) => { /* nothing new to install — already present */ }
                Err(_e) => {
                    // Non-fatal: leave Claude Code out and continue.
                }
            }
            continue;
        }

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

    // Also install the OpenCode TypeScript plugin (user-level). The
    // config merge above has already put OpenCode in `installed` if it
    // wrote anything, so this call only matters for machines where no
    // project config existed — we still want the user-level plugin so
    // future OpenCode sessions see sqz.
    if let Ok(true) = crate::opencode_plugin::install_opencode_plugin(sqz_path) {
        if !installed.iter().any(|n| n == "OpenCode") {
            installed.push("OpenCode".to_string());
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
    let path = claude_user_settings_path().ok_or_else(|| {
        crate::error::SqzError::Other(
            "Could not resolve home directory for ~/.claude/settings.json".to_string(),
        )
    })?;

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

    // Build our three hook entries as fresh JSON values.
    let pre_tool_use = serde_json::json!({
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": format!("{sqz_path} hook claude") }]
    });
    let pre_compact = serde_json::json!({
        "hooks": [{ "type": "command", "command": format!("{sqz_path} hook precompact") }]
    });
    let session_start = serde_json::json!({
        "matcher": "compact",
        "hooks": [{ "type": "command", "command": format!("{sqz_path} resume") }]
    });

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

    upsert_sqz_hook_entry(hooks_obj, "PreToolUse", pre_tool_use, "sqz hook claude");
    upsert_sqz_hook_entry(hooks_obj, "PreCompact", pre_compact, "sqz hook precompact");
    upsert_sqz_hook_entry(hooks_obj, "SessionStart", session_start, "sqz resume");

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

    // Atomic write: tempfile in same directory + rename. Modelled after
    // rtk's `atomic_write` in src/hooks/init.rs. Keeps the old file
    // intact if serialization or write fails halfway.
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
    let Some(path) = claude_user_settings_path() else {
        return Ok(None);
    };
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

    let mut changed = false;
    if let Some(hooks) = root_obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (event, sentinel) in &[
            ("PreToolUse", "sqz hook claude"),
            ("PreCompact", "sqz hook precompact"),
            ("SessionStart", "sqz resume"),
        ] {
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

#[cfg(test)]
mod global_install_tests {
    use super::*;

    /// Run `body` with `HOME` (and `USERPROFILE` on Windows) pointing at
    /// `tmp`, then restore the original values. Without this, the tests
    /// would write to the real user's `~/.claude/settings.json` and
    /// wreck their config.
    ///
    /// `dirs_next::home_dir` reads `HOME` on Unix and `USERPROFILE` on
    /// Windows, so we set both — keeps the tests portable.
    ///
    /// SAFETY: `set_var` / `remove_var` are marked unsafe on the
    /// unstable `std::env` edition; this helper stays on the stable
    /// API that doesn't require `unsafe`. Tests that run in parallel
    /// must serialize through a mutex because the process-level env
    /// is shared.
    fn with_fake_home<R>(tmp: &std::path::Path, body: impl FnOnce() -> R) -> R {
        use std::sync::Mutex;
        // Serialize so parallel tests don't race on $HOME.
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let prev_home = std::env::var_os("HOME");
        let prev_userprofile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", tmp);
        std::env::set_var("USERPROFILE", tmp);
        let result = body();
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_userprofile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        result
    }

    #[test]
    fn global_install_creates_fresh_settings_json() {
        let tmp = tempfile::tempdir().unwrap();
        with_fake_home(tmp.path(), || {
            let changed = install_claude_global("/usr/local/bin/sqz").unwrap();
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
        });
    }

    #[test]
    fn global_install_preserves_existing_user_config() {
        // This is the big safety guarantee: if the user already has
        // permissions, env, statusLine, or unrelated hooks in
        // ~/.claude/settings.json, sqz must NOT stomp on them.
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

        with_fake_home(tmp.path(), || {
            let changed = install_claude_global("/usr/local/bin/sqz").unwrap();
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
            assert!(matchers.contains(&"Bash"), "sqz Bash hook must be present");
        });
    }

    #[test]
    fn global_install_is_idempotent() {
        // Running `sqz init --global` twice should leave exactly ONE sqz
        // hook entry per event, not two. This is the foot-gun the
        // upsert_sqz_hook_entry helper defends against.
        let tmp = tempfile::tempdir().unwrap();
        with_fake_home(tmp.path(), || {
            assert!(install_claude_global("sqz").unwrap());
            // Second call: same sqz_path → no change reported, file is
            // byte-identical.
            assert!(
                !install_claude_global("sqz").unwrap(),
                "second install with identical args should report no change"
            );

            let path = tmp.path().join(".claude").join("settings.json");
            let parsed: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            // Exactly one entry per hook event.
            for event in &["PreToolUse", "PreCompact", "SessionStart"] {
                let arr = parsed["hooks"][event].as_array().unwrap();
                assert_eq!(
                    arr.len(),
                    1,
                    "{event} must have exactly one sqz entry after 2 installs, got {arr:?}"
                );
            }
        });
    }

    #[test]
    fn global_install_upgrades_stale_sqz_hook_in_place() {
        // If a previous sqz release wrote a hook with a different sqz
        // path, re-running `sqz init --global` should replace it, not
        // leave two entries pointing at different binaries.
        let tmp = tempfile::tempdir().unwrap();
        with_fake_home(tmp.path(), || {
            // First install with old path.
            install_claude_global("/old/path/sqz").unwrap();
            // Second install with new path.
            let changed = install_claude_global("/new/path/sqz").unwrap();
            assert!(changed, "different sqz_path must be seen as a change");

            let path = tmp.path().join(".claude").join("settings.json");
            let parsed: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
            assert_eq!(pre.len(), 1, "stale sqz entry must be replaced, not duplicated");
            let cmd = pre[0]["hooks"][0]["command"].as_str().unwrap();
            assert!(cmd.contains("/new/path/sqz"));
            assert!(!cmd.contains("/old/path/sqz"));
        });
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

        with_fake_home(tmp.path(), || {
            // Install so there's something to remove.
            install_claude_global("/usr/local/bin/sqz").unwrap();
            // And remove.
            let result = remove_claude_global_hook().unwrap().unwrap();
            assert_eq!(result.0, settings);
            assert!(result.1, "should report that the file was modified");

            // File must still exist (user had non-sqz config in it).
            assert!(settings.exists(), "settings.json should be preserved");
            let parsed: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();

            // Permissions must survive.
            assert_eq!(
                parsed["permissions"]["allow"][0].as_str().unwrap(),
                "Bash(git status)"
            );

            // User's Edit hook must survive; sqz's Bash hook must be gone.
            let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
            assert_eq!(pre.len(), 1, "only the user's Edit hook should remain");
            assert_eq!(pre[0]["matcher"].as_str().unwrap(), "Edit");

            // sqz-only hook events should be cleaned up entirely.
            assert!(parsed["hooks"].get("PreCompact").is_none());
            assert!(parsed["hooks"].get("SessionStart").is_none());
        });
    }

    #[test]
    fn global_uninstall_deletes_settings_json_if_it_was_sqz_only() {
        // If the user's ~/.claude/settings.json contained ONLY sqz's
        // hooks (common case: sqz installed it from scratch), uninstall
        // removes the whole file so there's no trace left.
        let tmp = tempfile::tempdir().unwrap();
        with_fake_home(tmp.path(), || {
            install_claude_global("sqz").unwrap();
            let path = tmp.path().join(".claude").join("settings.json");
            assert!(path.exists(), "precondition: install created the file");

            let result = remove_claude_global_hook().unwrap().unwrap();
            assert!(result.1);
            assert!(!path.exists(), "sqz-only settings.json should be removed on uninstall");
        });
    }

    #[test]
    fn global_uninstall_on_missing_file_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        with_fake_home(tmp.path(), || {
            assert!(
                remove_claude_global_hook().unwrap().is_none(),
                "missing file should return None, not error"
            );
        });
    }

    #[test]
    fn global_uninstall_refuses_to_touch_unparseable_file() {
        // If the user's ~/.claude/settings.json is corrupt (or they
        // started editing it manually and saved mid-flight), uninstall
        // should refuse rather than delete data.
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{ invalid json because").unwrap();

        with_fake_home(tmp.path(), || {
            assert!(
                remove_claude_global_hook().is_err(),
                "bad JSON must surface as an error"
            );
        });

        // File preserved, no data loss.
        let after = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(after, "{ invalid json because");
    }
}
