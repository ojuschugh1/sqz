//! Kiro (IDE chat and CLI) integration.
//!
//! Kiro's PreToolUse hooks are allow/block only — there is no
//! `updatedInput`-style rewrite mechanism, so the command-rewriting hook
//! sqz used to install never worked (and its `when`/`then` JSON wasn't a
//! valid Kiro hook file either). The integration that actually works in
//! Kiro chat is:
//!
//! 1. A steering file at `.kiro/steering/sqz.md` (always included) that
//!    teaches the agent to pipe long command output through `sqz compress`.
//! 2. The `sqz-mcp` server registered in `.kiro/settings/mcp.json`.

use std::path::{Path, PathBuf};

use crate::error::{Result, SqzError};

const KIRO_STEERING_SENTINEL: &str = "<!-- sqz-kiro-steering (auto-installed by sqz init) -->";

pub fn kiro_steering_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".kiro").join("steering").join("sqz.md")
}

pub fn kiro_mcp_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".kiro").join("settings").join("mcp.json")
}

/// Kiro's user-level config root: `$KIRO_HOME` if set, else `~/.kiro`.
/// Steering and MCP config placed here apply to every workspace.
pub fn kiro_user_root() -> PathBuf {
    kiro_user_root_from(
        std::env::var("KIRO_HOME").ok(),
        dirs_next::home_dir(),
    )
}

fn kiro_user_root_from(kiro_home: Option<String>, home: Option<PathBuf>) -> PathBuf {
    if let Some(custom) = kiro_home {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    home.unwrap_or_else(|| PathBuf::from(".")).join(".kiro")
}

pub fn kiro_user_steering_path() -> PathBuf {
    kiro_user_root().join("steering").join("sqz.md")
}

pub fn kiro_user_mcp_path() -> PathBuf {
    kiro_user_root().join("settings").join("mcp.json")
}

/// The legacy hook file older sqz versions wrote. It used a schema Kiro
/// never recognized, so it can be deleted safely when we detect it.
pub fn kiro_legacy_hook_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".kiro").join("hooks").join("sqz-compress.json")
}

pub fn kiro_steering_content(sqz_path: &str) -> String {
    format!(
        "---\ninclusion: always\n---\n{KIRO_STEERING_SENTINEL}\n\n{}",
        crate::codex_integration::sqz_guidance(sqz_path)
    )
}

pub fn kiro_user_steering_content(sqz_path: &str) -> String {
    kiro_steering_content(sqz_path)
}

/// State of a steering file sqz may manage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteeringState {
    Missing,
    /// No sqz sentinel: the user wrote it, sqz never touches it.
    UserAuthored,
    /// Matches what this version of sqz writes.
    Current,
    /// sqz-managed but written by an older version (or a different
    /// template), so the agent is reading outdated guidance.
    Stale,
}

/// The sqz binary path a managed steering file was written for, read back
/// from its first example line.
pub(crate) fn embedded_sqz_path(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        line.split_once("git status 2>&1 | ")
            .and_then(|(_, rest)| rest.strip_suffix(" compress"))
    })
}

/// Compare a steering file against what this version would write for the
/// same sqz path. `global` selects the user-level template.
pub fn kiro_steering_state(path: &Path, global: bool) -> SteeringState {
    let Ok(content) = std::fs::read_to_string(path) else {
        return SteeringState::Missing;
    };
    if !content.contains(KIRO_STEERING_SENTINEL) {
        return SteeringState::UserAuthored;
    }
    let Some(sqz_path) = embedded_sqz_path(&content) else {
        return SteeringState::Stale;
    };
    let expected = if global {
        kiro_user_steering_content(sqz_path)
    } else {
        kiro_steering_content(sqz_path)
    };
    if content == expected {
        SteeringState::Current
    } else {
        SteeringState::Stale
    }
}

/// Whether sqz's user-level steering is installed. Kiro applies
/// `~/.kiro/steering` to every workspace, so a project copy next to it is
/// the same guidance sent twice on every request.
pub fn user_steering_installed() -> bool {
    matches!(
        kiro_steering_state(&kiro_user_steering_path(), true),
        SteeringState::Current | SteeringState::Stale
    )
}

/// Rewrite a stale sqz-managed steering file with this version's template,
/// keeping the sqz binary path it already points at when that binary still
/// exists (else `fallback_sqz_path`). Returns `Ok(false)` for missing,
/// current, or user-authored files.
pub fn refresh_kiro_steering(path: &Path, global: bool, fallback_sqz_path: &str) -> Result<bool> {
    if kiro_steering_state(path, global) != SteeringState::Stale {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("read {}: {e}", path.display())))?;
    let sqz_path = embedded_sqz_path(&existing)
        .filter(|p| Path::new(p).is_file())
        .unwrap_or(fallback_sqz_path)
        .to_string();
    let content = if global {
        kiro_user_steering_content(&sqz_path)
    } else {
        kiro_steering_content(&sqz_path)
    };
    install_kiro_steering_at(path, &content)
}

/// Write the steering file. Refreshes sqz-managed files in place, leaves
/// a user-authored `.kiro/steering/sqz.md` alone.
pub fn install_kiro_steering(project_dir: &Path, sqz_path: &str) -> Result<bool> {
    install_kiro_steering_at(&kiro_steering_path(project_dir), &kiro_steering_content(sqz_path))
}

/// Write the user-level steering file (`~/.kiro/steering/sqz.md`), which
/// Kiro applies to every workspace.
pub fn install_kiro_user_steering(sqz_path: &str) -> Result<bool> {
    install_kiro_steering_at(&kiro_user_steering_path(), &kiro_user_steering_content(sqz_path))
}

pub(crate) fn install_kiro_steering_at(path: &Path, content: &str) -> Result<bool> {
    if path.exists() {
        let existing = std::fs::read_to_string(&path).map_err(|e| {
            SqzError::Other(format!("read {}: {e}", path.display()))
        })?;
        if !existing.contains(KIRO_STEERING_SENTINEL) {
            return Ok(false);
        }
        if existing == content {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SqzError::Other(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, content)
        .map_err(|e| SqzError::Other(format!("write {}: {e}", path.display())))?;
    Ok(true)
}

pub fn remove_kiro_steering(project_dir: &Path) -> Result<bool> {
    remove_kiro_steering_at(&kiro_steering_path(project_dir))
}

pub fn remove_kiro_user_steering() -> Result<bool> {
    remove_kiro_steering_at(&kiro_user_steering_path())
}

pub(crate) fn remove_kiro_steering_at(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("read {}: {e}", path.display())))?;
    if !content.contains(KIRO_STEERING_SENTINEL) {
        return Ok(false);
    }
    std::fs::remove_file(path)
        .map_err(|e| SqzError::Other(format!("remove {}: {e}", path.display())))?;
    Ok(true)
}

/// Delete the legacy broken hook file if present and sqz-authored.
pub fn remove_kiro_legacy_hook(project_dir: &Path) -> Result<bool> {
    let path = kiro_legacy_hook_path(project_dir);
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    if !(content.contains("\"sqz compress\"") && content.contains("hook kiro")) {
        return Ok(false);
    }
    std::fs::remove_file(&path)
        .map_err(|e| SqzError::Other(format!("remove {}: {e}", path.display())))?;
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KiroMcpInstall {
    Created,
    Merged,
    AlreadyPresent,
    /// File exists but is not strict JSON; left untouched.
    SkippedUnparseable,
}

pub fn install_kiro_mcp_config(project_dir: &Path) -> Result<KiroMcpInstall> {
    install_kiro_mcp_config_at(&kiro_mcp_path(project_dir))
}

/// Register sqz-mcp in the user-level `~/.kiro/settings/mcp.json`, which
/// Kiro merges into every workspace.
pub fn install_kiro_user_mcp_config() -> Result<KiroMcpInstall> {
    install_kiro_mcp_config_at(&kiro_user_mcp_path())
}

pub(crate) fn install_kiro_mcp_config_at(path: &Path) -> Result<KiroMcpInstall> {
    let sqz_entry = serde_json::json!({
        "command": "sqz-mcp",
        "args": ["--transport", "stdio"],
        "disabled": false
    });

    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SqzError::Other(format!("create {}: {e}", parent.display())))?;
        }
        let root = serde_json::json!({ "mcpServers": { "sqz": sqz_entry } });
        std::fs::write(path, serde_json::to_string_pretty(&root).unwrap_or_default())
            .map_err(|e| SqzError::Other(format!("write {}: {e}", path.display())))?;
        return Ok(KiroMcpInstall::Created);
    }

    let text = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("read {}: {e}", path.display())))?;
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return Ok(KiroMcpInstall::SkippedUnparseable),
        }
    };

    let Some(root_obj) = root.as_object_mut() else {
        return Ok(KiroMcpInstall::SkippedUnparseable);
    };
    let servers = root_obj
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    let servers_obj = servers.as_object_mut().expect("just ensured object");

    if servers_obj.contains_key("sqz") {
        return Ok(KiroMcpInstall::AlreadyPresent);
    }
    servers_obj.insert("sqz".to_string(), sqz_entry);

    std::fs::write(path, serde_json::to_string_pretty(&root).unwrap_or_default())
        .map_err(|e| SqzError::Other(format!("write {}: {e}", path.display())))?;
    Ok(KiroMcpInstall::Merged)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KiroMcpRemove {
    Removed,
    NotPresent,
    SkippedUnparseable,
}

pub fn remove_kiro_mcp_config(project_dir: &Path) -> Result<KiroMcpRemove> {
    remove_kiro_mcp_config_at(&kiro_mcp_path(project_dir))
}

pub fn remove_kiro_user_mcp_config() -> Result<KiroMcpRemove> {
    remove_kiro_mcp_config_at(&kiro_user_mcp_path())
}

pub(crate) fn remove_kiro_mcp_config_at(path: &Path) -> Result<KiroMcpRemove> {
    if !path.exists() {
        return Ok(KiroMcpRemove::NotPresent);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| SqzError::Other(format!("read {}: {e}", path.display())))?;
    let mut root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            if text.contains("\"sqz\"") {
                return Ok(KiroMcpRemove::SkippedUnparseable);
            }
            return Ok(KiroMcpRemove::NotPresent);
        }
    };

    let Some(root_obj) = root.as_object_mut() else {
        return Ok(KiroMcpRemove::NotPresent);
    };
    let Some(servers) = root_obj.get_mut("mcpServers").and_then(|s| s.as_object_mut()) else {
        return Ok(KiroMcpRemove::NotPresent);
    };
    if servers.remove("sqz").is_none() {
        return Ok(KiroMcpRemove::NotPresent);
    }
    if servers.is_empty() {
        root_obj.remove("mcpServers");
    }
    std::fs::write(path, serde_json::to_string_pretty(&root).unwrap_or_default())
        .map_err(|e| SqzError::Other(format!("write {}: {e}", path.display())))?;
    Ok(KiroMcpRemove::Removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_install_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_kiro_steering(dir.path(), "/usr/bin/sqz").unwrap());
        let content = std::fs::read_to_string(kiro_steering_path(dir.path())).unwrap();
        assert!(content.starts_with("---\ninclusion: always\n---"));
        assert!(content.contains(KIRO_STEERING_SENTINEL));
        assert!(content.contains("/usr/bin/sqz compress"));
    }

    #[test]
    fn steering_install_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_kiro_steering(dir.path(), "sqz").unwrap());
        assert!(!install_kiro_steering(dir.path(), "sqz").unwrap());
    }

    #[test]
    fn steering_install_refreshes_stale_managed_file() {
        let dir = tempfile::tempdir().unwrap();
        install_kiro_steering(dir.path(), "/old/sqz").unwrap();
        assert!(install_kiro_steering(dir.path(), "/new/sqz").unwrap());
        let content = std::fs::read_to_string(kiro_steering_path(dir.path())).unwrap();
        assert!(content.contains("/new/sqz"));
        assert!(!content.contains("/old/sqz"));
    }

    #[test]
    fn steering_install_leaves_user_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = kiro_steering_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "my own sqz notes\n").unwrap();
        assert!(!install_kiro_steering(dir.path(), "sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "my own sqz notes\n");
    }

    #[test]
    fn steering_remove_only_deletes_managed_file() {
        let dir = tempfile::tempdir().unwrap();
        install_kiro_steering(dir.path(), "sqz").unwrap();
        assert!(remove_kiro_steering(dir.path()).unwrap());
        assert!(!kiro_steering_path(dir.path()).exists());

        let path = kiro_steering_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "user file\n").unwrap();
        assert!(!remove_kiro_steering(dir.path()).unwrap());
        assert!(path.exists());
    }

    #[test]
    fn legacy_hook_is_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let hook = kiro_legacy_hook_path(dir.path());
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, r#"{"name":"sqz compress","then":{"command":"sqz hook kiro"}}"#).unwrap();
        assert!(remove_kiro_legacy_hook(dir.path()).unwrap());
        assert!(!hook.exists());
    }

    #[test]
    fn legacy_cleanup_skips_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let hook = kiro_legacy_hook_path(dir.path());
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, r#"{"version":"v1","hooks":[{"name":"mine"}]}"#).unwrap();
        assert!(!remove_kiro_legacy_hook(dir.path()).unwrap());
        assert!(hook.exists());
    }

    #[test]
    fn mcp_creates_fresh_config() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            install_kiro_mcp_config(dir.path()).unwrap(),
            KiroMcpInstall::Created
        );
        let parsed: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(kiro_mcp_path(dir.path())).unwrap(),
        )
        .unwrap();
        assert_eq!(parsed["mcpServers"]["sqz"]["command"], "sqz-mcp");
        assert_eq!(parsed["mcpServers"]["sqz"]["disabled"], false);
    }

    #[test]
    fn mcp_merges_preserving_other_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = kiro_mcp_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"aws-docs":{"command":"uvx"}}}"#).unwrap();
        assert_eq!(install_kiro_mcp_config_at(&path).unwrap(), KiroMcpInstall::Merged);
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["aws-docs"]["command"], "uvx");
        assert_eq!(parsed["mcpServers"]["sqz"]["command"], "sqz-mcp");
    }

    #[test]
    fn mcp_leaves_existing_sqz_entry_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = kiro_mcp_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"sqz":{"command":"/custom/sqz-mcp"}}}"#).unwrap();
        assert_eq!(
            install_kiro_mcp_config_at(&path).unwrap(),
            KiroMcpInstall::AlreadyPresent
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["sqz"]["command"], "/custom/sqz-mcp");
    }

    #[test]
    fn mcp_skips_unparseable_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = kiro_mcp_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bad = "{ not json ";
        std::fs::write(&path, bad).unwrap();
        assert_eq!(
            install_kiro_mcp_config_at(&path).unwrap(),
            KiroMcpInstall::SkippedUnparseable
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
    }

    #[test]
    fn mcp_remove_is_surgical() {
        let dir = tempfile::tempdir().unwrap();
        let path = kiro_mcp_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"mcpServers":{"sqz":{"command":"sqz-mcp"},"aws-docs":{"command":"uvx"}}}"#,
        )
        .unwrap();
        assert_eq!(remove_kiro_mcp_config_at(&path).unwrap(), KiroMcpRemove::Removed);
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(parsed["mcpServers"].get("sqz").is_none());
        assert_eq!(parsed["mcpServers"]["aws-docs"]["command"], "uvx");
    }

    #[test]
    fn user_root_honors_kiro_home() {
        assert_eq!(
            kiro_user_root_from(Some("/custom/kiro".into()), Some(PathBuf::from("/home/u"))),
            PathBuf::from("/custom/kiro")
        );
        assert_eq!(
            kiro_user_root_from(Some("  ".into()), Some(PathBuf::from("/home/u"))),
            PathBuf::from("/home/u/.kiro")
        );
        assert_eq!(
            kiro_user_root_from(None, Some(PathBuf::from("/home/u"))),
            PathBuf::from("/home/u/.kiro")
        );
    }

    #[test]
    fn user_and_project_steering_carry_the_shared_guidance() {
        let content = kiro_user_steering_content("/usr/bin/sqz");
        assert!(content.contains(KIRO_STEERING_SENTINEL));
        assert!(content.ends_with(&crate::codex_integration::sqz_guidance("/usr/bin/sqz")));
        assert_eq!(content, kiro_steering_content("/usr/bin/sqz"));
    }

    #[test]
    fn user_level_steering_roundtrip_via_at_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("steering").join("sqz.md");
        let content = kiro_user_steering_content("/usr/bin/sqz");
        assert!(install_kiro_steering_at(&path, &content).unwrap());
        assert!(path.exists());
        // Idempotent on identical content.
        assert!(!install_kiro_steering_at(&path, &content).unwrap());
        // Removes only sqz-authored files.
        assert!(remove_kiro_steering_at(&path).unwrap());
        assert!(!path.exists());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "my own steering\n").unwrap();
        assert!(!remove_kiro_steering_at(&path).unwrap());
        assert!(path.exists());
    }

    #[test]
    fn steering_state_tracks_current_stale_and_user_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqz.md");
        assert_eq!(kiro_steering_state(&path, true), SteeringState::Missing);

        std::fs::write(&path, kiro_user_steering_content("/opt/bin/sqz")).unwrap();
        assert_eq!(kiro_steering_state(&path, true), SteeringState::Current);
        assert_eq!(kiro_steering_state(&path, false), SteeringState::Current);

        // An older template: sentinel present, content no longer matches.
        let old = kiro_user_steering_content("/opt/bin/sqz").replace("sqz_recall", "recall");
        std::fs::write(&path, old).unwrap();
        assert_eq!(kiro_steering_state(&path, true), SteeringState::Stale);

        std::fs::write(&path, "my own notes about sqz\n").unwrap();
        assert_eq!(kiro_steering_state(&path, true), SteeringState::UserAuthored);
    }

    #[test]
    fn refresh_keeps_the_existing_binary_path_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("sqz");
        std::fs::write(&bin, "").unwrap();
        let bin = bin.to_string_lossy().to_string();
        let path = dir.path().join("steering.md");
        let stale = kiro_user_steering_content(&bin).replace("sqz_recall", "recall");
        std::fs::write(&path, stale).unwrap();

        assert!(refresh_kiro_steering(&path, true, "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), kiro_user_steering_content(&bin));
        assert!(!refresh_kiro_steering(&path, true, "/fallback/sqz").unwrap(), "now current");
    }

    #[test]
    fn refresh_falls_back_when_the_old_binary_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("steering.md");
        let stale = kiro_steering_content("/deleted/target/debug/sqz").replace("sqz_recall", "recall");
        std::fs::write(&path, stale).unwrap();
        assert!(refresh_kiro_steering(&path, false, "/fallback/sqz").unwrap());
        assert!(std::fs::read_to_string(&path).unwrap().contains("/fallback/sqz compress"));
    }

    #[test]
    fn refresh_never_touches_user_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("steering.md");
        std::fs::write(&path, "mine\n").unwrap();
        assert!(!refresh_kiro_steering(&path, true, "/fallback/sqz").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\n");
    }

    #[test]
    fn embedded_path_round_trips_through_template() {
        let content = kiro_steering_content("/Users/me/.cargo/bin/sqz");
        assert_eq!(embedded_sqz_path(&content), Some("/Users/me/.cargo/bin/sqz"));
    }

    #[test]
    fn user_mcp_merge_preserves_existing_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings").join("mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"mcpServers":{"builder-mcp":{"command":"/x/builder-mcp","args":[]}}}"#,
        )
        .unwrap();
        assert_eq!(
            install_kiro_mcp_config_at(&path).unwrap(),
            KiroMcpInstall::Merged
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["builder-mcp"]["command"], "/x/builder-mcp");
        assert_eq!(parsed["mcpServers"]["sqz"]["command"], "sqz-mcp");
    }
}
