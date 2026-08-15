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

/// The legacy hook file older sqz versions wrote. It used a schema Kiro
/// never recognized, so it can be deleted safely when we detect it.
pub fn kiro_legacy_hook_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".kiro").join("hooks").join("sqz-compress.json")
}

pub fn kiro_steering_content(sqz_path: &str) -> String {
    format!(
        "\
---
inclusion: always
---
{sentinel}

# sqz — Token-Optimized Output

When running shell commands whose output may be long (git log/diff, test
runners, build logs, `ls -la`, `docker ps`), pipe the output through
`sqz compress` to cut token consumption:

```bash
git status 2>&1 | {sqz_path} compress
cargo test 2>&1 | {sqz_path} compress
```

Do NOT pipe interactive commands (REPLs, `ssh`), compound commands with
shell operators (`&&`, `>`, `;`), or output that is already short.

The `sqz` MCP server is configured in `.kiro/settings/mcp.json`. Prefer
its tools when they fit:

- `sqz_read_file` / `sqz_grep` / `sqz_list_dir` — file access with
  caching; repeat reads of unchanged content cost ~13 tokens.
- `compress` — compress a blob of text you already have.
- `passthrough` — return text unchanged (escape hatch).
- `expand` — resolve a `§ref:HASH§` token back to the original bytes.

If you see a `§ref:HASH§` token you can't parse, run
`{sqz_path} expand <prefix>` or set `SQZ_NO_DEDUP=1` on the command.
If a `«A1»` symbol replaced a value you need verbatim, re-run with
`SQZ_NO_ABBREV=1` (or `--no-abbrev`).
",
        sentinel = KIRO_STEERING_SENTINEL,
    )
}

/// Write the steering file. Refreshes sqz-managed files in place, leaves
/// a user-authored `.kiro/steering/sqz.md` alone.
pub fn install_kiro_steering(project_dir: &Path, sqz_path: &str) -> Result<bool> {
    let path = kiro_steering_path(project_dir);
    let content = kiro_steering_content(sqz_path);

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
    let path = kiro_steering_path(project_dir);
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| SqzError::Other(format!("read {}: {e}", path.display())))?;
    if !content.contains(KIRO_STEERING_SENTINEL) {
        return Ok(false);
    }
    std::fs::remove_file(&path)
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
}
