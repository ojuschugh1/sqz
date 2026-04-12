use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The 5 hook types supported by the sqz hook system.
///
/// Requirements: 44.1
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookType {
    /// Fires before a tool is executed. Can block, redirect, or inject context.
    PreToolUse,
    /// Fires after a tool completes. Captures structured events.
    PostToolUse,
    /// Fires before context compaction. Builds session snapshot.
    PreCompact,
    /// Fires on session start or resume. Restores from snapshot.
    SessionStart,
    /// Fires when the user submits a prompt. Captures decisions and corrections.
    UserPromptSubmit,
}

impl HookType {
    /// Returns all hook types in canonical order.
    pub fn all() -> &'static [HookType] {
        &[
            HookType::PreToolUse,
            HookType::PostToolUse,
            HookType::PreCompact,
            HookType::SessionStart,
            HookType::UserPromptSubmit,
        ]
    }

    /// Human-readable label for this hook type.
    pub fn label(&self) -> &'static str {
        match self {
            HookType::PreToolUse => "pre_tool_use",
            HookType::PostToolUse => "post_tool_use",
            HookType::PreCompact => "pre_compact",
            HookType::SessionStart => "session_start",
            HookType::UserPromptSubmit => "user_prompt_submit",
        }
    }
}

/// Actions a hook can take when fired.
///
/// Requirements: 44.2, 44.3, 44.4, 44.5
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookAction {
    /// Allow the operation to proceed (no-op).
    Allow,
    /// Block the operation with a reason (PreToolUse).
    Block { reason: String },
    /// Redirect to a different tool (PreToolUse).
    Redirect { to_tool: String },
    /// Inject additional context into the operation (PreToolUse).
    InjectContext { content: String },
    /// Capture a structured event (PostToolUse).
    CaptureEvent { event_type: String, data: String },
    /// Build a session snapshot (PreCompact).
    BuildSnapshot,
    /// Restore session state from snapshot (SessionStart).
    RestoreSnapshot,
    /// Capture a user decision or correction (UserPromptSubmit).
    CaptureDecision { decision: String },
}

/// A registered hook with its type, action, and optional filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    pub hook_type: HookType,
    pub action: HookAction,
    /// Optional filter — when set, the hook only fires if the context
    /// matches this pattern (e.g. a tool name for PreToolUse).
    pub filter: Option<String>,
}

/// Context passed to hooks when they fire.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    /// The tool name (relevant for PreToolUse / PostToolUse).
    pub tool_name: Option<String>,
    /// The command being executed.
    pub command: Option<String>,
    /// Arbitrary key-value metadata.
    pub metadata: HashMap<String, String>,
}

/// Manages hook registration and dispatch.
///
/// Requirements: 44.1–44.6
pub struct HookManager {
    hooks: HashMap<HookType, Vec<Hook>>,
}

impl HookManager {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    /// Register a hook.
    pub fn register(&mut self, hook: Hook) {
        self.hooks
            .entry(hook.hook_type)
            .or_default()
            .push(hook);
    }

    /// Fire all hooks of the given type and return the first non-Allow action,
    /// or `HookAction::Allow` if no hook matched.
    pub fn fire(&self, hook_type: HookType, context: &HookContext) -> HookAction {
        let Some(hooks) = self.hooks.get(&hook_type) else {
            return HookAction::Allow;
        };

        for hook in hooks {
            if let Some(ref filter) = hook.filter {
                // Check filter against tool_name or command.
                let matches = context
                    .tool_name
                    .as_deref()
                    .map_or(false, |t| t == filter)
                    || context
                        .command
                        .as_deref()
                        .map_or(false, |c| c.contains(filter));
                if !matches {
                    continue;
                }
            }
            // Return the first matching non-Allow action.
            if hook.action != HookAction::Allow {
                return hook.action.clone();
            }
        }

        HookAction::Allow
    }

    /// Return all hooks registered for a given type.
    pub fn hooks_for(&self, hook_type: HookType) -> &[Hook] {
        self.hooks.get(&hook_type).map_or(&[], |v| v.as_slice())
    }

    /// Total number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}


// ── Platform config generation ────────────────────────────────────────────

/// Known platforms for `sqz init --agent <platform>`.
const KNOWN_PLATFORMS: &[&str] = &[
    "claude-code",
    "cursor",
    "kiro",
    "copilot",
    "windsurf",
    "cline",
    "gemini-cli",
    "codex",
    "opencode",
    "goose",
    "aider",
    "amp",
    "continue",
    "zed",
    "amazon-q",
];

/// Generate a platform-specific hook configuration for `sqz init --agent <platform>`.
///
/// Returns a TOML string for Level 2 platforms (shell hook + MCP) and a JSON
/// string for Level 1 platforms (MCP-only).
///
/// Requirements: 44.6
pub fn generate_platform_config(platform: &str) -> Option<String> {
    match platform {
        // ── Level 1: MCP config only ──────────────────────────────────
        "continue" | "zed" | "amazon-q" => Some(generate_level1_config(platform)),

        // ── Level 2: Shell hook + MCP + hooks ─────────────────────────
        "claude-code" | "cursor" | "kiro" | "copilot" | "windsurf" | "cline"
        | "gemini-cli" | "codex" | "opencode" | "goose" | "aider" | "amp" => {
            Some(generate_level2_config(platform))
        }

        _ => None,
    }
}

/// Returns the list of known platform identifiers.
pub fn known_platforms() -> &'static [&'static str] {
    KNOWN_PLATFORMS
}

fn generate_level1_config(platform: &str) -> String {
    let config_path = match platform {
        "continue" => "~/.continue/config.json",
        "zed" => "~/.config/zed/settings.json",
        "amazon-q" => "~/.aws/amazonq/mcp.json",
        _ => "mcp.json",
    };

    format!(
        r#"{{
  "_comment": "sqz MCP config for {platform}",
  "_path": "{config_path}",
  "mcpServers": {{
    "sqz": {{
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {{}}
    }}
  }}
}}"#
    )
}

fn generate_level2_config(platform: &str) -> String {
    let config_path = match platform {
        "claude-code" => ".claude/mcp_servers.json",
        "cursor" => "~/.cursor/mcp.json",
        "kiro" => ".kiro/settings/mcp.json",
        "copilot" => ".github/copilot/mcp.json",
        "windsurf" => "~/.windsurf/mcp.json",
        "cline" => "~/.cline/mcp.json",
        _ => "mcp.json",
    };

    format!(
        r#"# sqz hook config for {platform}
# MCP config path: {config_path}

[hooks.pre_tool_use]
enabled = true
block_dangerous = true
sandbox_redirect = ["shell", "bash", "exec"]
inject_context = true

[hooks.post_tool_use]
enabled = true
capture_events = ["file_edit", "git_op", "task_update", "error"]

[hooks.pre_compact]
enabled = true
build_snapshot = true

[hooks.session_start]
enabled = true
restore_snapshot = true

[hooks.user_prompt_submit]
enabled = true
capture_decisions = true
capture_corrections = true

[mcp]
command = "sqz-mcp"
args = ["--transport", "stdio"]
config_path = "{config_path}"
"#
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── HookType ──────────────────────────────────────────────────────

    #[test]
    fn test_hook_type_all_returns_5_variants() {
        assert_eq!(HookType::all().len(), 5);
    }

    #[test]
    fn test_hook_type_labels_are_unique() {
        let labels: Vec<&str> = HookType::all().iter().map(|h| h.label()).collect();
        let mut deduped = labels.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(labels.len(), deduped.len());
    }

    // ── HookManager basics ────────────────────────────────────────────

    #[test]
    fn test_new_manager_is_empty() {
        let mgr = HookManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn test_register_and_count() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Block {
                reason: "dangerous".into(),
            },
            filter: None,
        });
        assert_eq!(mgr.len(), 1);
        assert!(!mgr.is_empty());
    }

    #[test]
    fn test_hooks_for_returns_registered_hooks() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PostToolUse,
            action: HookAction::CaptureEvent {
                event_type: "file_edit".into(),
                data: "{}".into(),
            },
            filter: None,
        });
        assert_eq!(mgr.hooks_for(HookType::PostToolUse).len(), 1);
        assert_eq!(mgr.hooks_for(HookType::PreToolUse).len(), 0);
    }

    // ── fire() dispatch ───────────────────────────────────────────────

    #[test]
    fn test_fire_returns_allow_when_no_hooks() {
        let mgr = HookManager::new();
        let ctx = HookContext::default();
        assert_eq!(mgr.fire(HookType::PreToolUse, &ctx), HookAction::Allow);
    }

    #[test]
    fn test_fire_returns_first_matching_action() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Block {
                reason: "blocked".into(),
            },
            filter: None,
        });
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Redirect {
                to_tool: "sandbox".into(),
            },
            filter: None,
        });

        let ctx = HookContext::default();
        // First non-Allow wins.
        assert_eq!(
            mgr.fire(HookType::PreToolUse, &ctx),
            HookAction::Block {
                reason: "blocked".into()
            }
        );
    }

    #[test]
    fn test_fire_with_filter_matches_tool_name() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Redirect {
                to_tool: "sandbox".into(),
            },
            filter: Some("exec_shell".into()),
        });

        // No match → Allow.
        let ctx_miss = HookContext {
            tool_name: Some("read_file".into()),
            ..Default::default()
        };
        assert_eq!(mgr.fire(HookType::PreToolUse, &ctx_miss), HookAction::Allow);

        // Match → Redirect.
        let ctx_hit = HookContext {
            tool_name: Some("exec_shell".into()),
            ..Default::default()
        };
        assert_eq!(
            mgr.fire(HookType::PreToolUse, &ctx_hit),
            HookAction::Redirect {
                to_tool: "sandbox".into()
            }
        );
    }

    #[test]
    fn test_fire_with_filter_matches_command_substring() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Block {
                reason: "rm blocked".into(),
            },
            filter: Some("rm -rf".into()),
        });

        let ctx = HookContext {
            command: Some("rm -rf /tmp/stuff".into()),
            ..Default::default()
        };
        assert_eq!(
            mgr.fire(HookType::PreToolUse, &ctx),
            HookAction::Block {
                reason: "rm blocked".into()
            }
        );
    }

    // ── PreToolUse actions ────────────────────────────────────────────

    #[test]
    fn test_pre_tool_use_block() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Block {
                reason: "dangerous command".into(),
            },
            filter: None,
        });
        let action = mgr.fire(HookType::PreToolUse, &HookContext::default());
        assert!(matches!(action, HookAction::Block { .. }));
    }

    #[test]
    fn test_pre_tool_use_redirect() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Redirect {
                to_tool: "sandbox_exec".into(),
            },
            filter: None,
        });
        let action = mgr.fire(HookType::PreToolUse, &HookContext::default());
        assert!(matches!(action, HookAction::Redirect { .. }));
    }

    #[test]
    fn test_pre_tool_use_inject_context() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::InjectContext {
                content: "extra context".into(),
            },
            filter: None,
        });
        let action = mgr.fire(HookType::PreToolUse, &HookContext::default());
        assert!(matches!(action, HookAction::InjectContext { .. }));
    }

    // ── PostToolUse capture ───────────────────────────────────────────

    #[test]
    fn test_post_tool_use_capture_event() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PostToolUse,
            action: HookAction::CaptureEvent {
                event_type: "file_edit".into(),
                data: r#"{"path":"src/main.rs"}"#.into(),
            },
            filter: None,
        });
        let action = mgr.fire(HookType::PostToolUse, &HookContext::default());
        assert!(matches!(action, HookAction::CaptureEvent { .. }));
    }

    // ── PreCompact snapshot ───────────────────────────────────────────

    #[test]
    fn test_pre_compact_build_snapshot() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreCompact,
            action: HookAction::BuildSnapshot,
            filter: None,
        });
        let action = mgr.fire(HookType::PreCompact, &HookContext::default());
        assert_eq!(action, HookAction::BuildSnapshot);
    }

    // ── SessionStart restore ──────────────────────────────────────────

    #[test]
    fn test_session_start_restore_snapshot() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::SessionStart,
            action: HookAction::RestoreSnapshot,
            filter: None,
        });
        let action = mgr.fire(HookType::SessionStart, &HookContext::default());
        assert_eq!(action, HookAction::RestoreSnapshot);
    }

    // ── UserPromptSubmit capture ──────────────────────────────────────

    #[test]
    fn test_user_prompt_submit_capture_decision() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::UserPromptSubmit,
            action: HookAction::CaptureDecision {
                decision: "use async/await".into(),
            },
            filter: None,
        });
        let action = mgr.fire(HookType::UserPromptSubmit, &HookContext::default());
        assert!(matches!(action, HookAction::CaptureDecision { .. }));
    }

    // ── Platform config generation ────────────────────────────────────

    #[test]
    fn test_generate_config_unknown_platform_returns_none() {
        assert!(generate_platform_config("unknown-platform").is_none());
    }

    #[test]
    fn test_generate_config_level1_platforms_produce_json() {
        for platform in &["continue", "zed", "amazon-q"] {
            let config = generate_platform_config(platform).unwrap();
            assert!(config.contains("mcpServers"), "missing mcpServers for {platform}");
            assert!(config.contains("sqz-mcp"), "missing sqz-mcp for {platform}");
        }
    }

    #[test]
    fn test_generate_config_level2_platforms_produce_toml() {
        for platform in &[
            "claude-code", "cursor", "kiro", "copilot", "windsurf", "cline",
            "gemini-cli", "codex", "opencode", "goose", "aider", "amp",
        ] {
            let config = generate_platform_config(platform).unwrap();
            assert!(
                config.contains("[hooks.pre_tool_use]"),
                "missing pre_tool_use section for {platform}"
            );
            assert!(
                config.contains("[hooks.session_start]"),
                "missing session_start section for {platform}"
            );
            assert!(
                config.contains("sqz-mcp"),
                "missing sqz-mcp for {platform}"
            );
        }
    }

    #[test]
    fn test_generate_config_claude_code_has_correct_path() {
        let config = generate_platform_config("claude-code").unwrap();
        assert!(config.contains(".claude/mcp_servers.json"));
    }

    #[test]
    fn test_generate_config_kiro_has_correct_path() {
        let config = generate_platform_config("kiro").unwrap();
        assert!(config.contains(".kiro/settings/mcp.json"));
    }

    #[test]
    fn test_generate_config_cursor_has_correct_path() {
        let config = generate_platform_config("cursor").unwrap();
        assert!(config.contains("~/.cursor/mcp.json"));
    }

    #[test]
    fn test_known_platforms_covers_all() {
        assert_eq!(known_platforms().len(), 15);
        // Every known platform should produce a config.
        for p in known_platforms() {
            assert!(
                generate_platform_config(p).is_some(),
                "no config for known platform: {p}"
            );
        }
    }

    #[test]
    fn test_level2_config_contains_all_5_hook_sections() {
        let config = generate_platform_config("claude-code").unwrap();
        assert!(config.contains("[hooks.pre_tool_use]"));
        assert!(config.contains("[hooks.post_tool_use]"));
        assert!(config.contains("[hooks.pre_compact]"));
        assert!(config.contains("[hooks.session_start]"));
        assert!(config.contains("[hooks.user_prompt_submit]"));
    }

    // ── Multiple hooks per type ───────────────────────────────────────

    #[test]
    fn test_multiple_hooks_same_type_different_filters() {
        let mut mgr = HookManager::new();
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Block {
                reason: "shell blocked".into(),
            },
            filter: Some("exec_shell".into()),
        });
        mgr.register(Hook {
            hook_type: HookType::PreToolUse,
            action: HookAction::Redirect {
                to_tool: "sandbox".into(),
            },
            filter: Some("run_code".into()),
        });

        assert_eq!(mgr.len(), 2);

        let ctx_shell = HookContext {
            tool_name: Some("exec_shell".into()),
            ..Default::default()
        };
        assert!(matches!(
            mgr.fire(HookType::PreToolUse, &ctx_shell),
            HookAction::Block { .. }
        ));

        let ctx_code = HookContext {
            tool_name: Some("run_code".into()),
            ..Default::default()
        };
        assert!(matches!(
            mgr.fire(HookType::PreToolUse, &ctx_code),
            HookAction::Redirect { .. }
        ));
    }
}
