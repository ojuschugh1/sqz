/// Shell hook installation for Bash, Zsh, Fish, Nushell, and PowerShell.
///
/// Each variant knows how to detect its RC file and append the sqz hook
/// function that pipes command output through `sqz compress`.

use std::path::{Path, PathBuf};

// ── Hook script templates ─────────────────────────────────────────────────

const BASH_HOOK: &str = r#"
# sqz — context intelligence layer (auto-installed)
__sqz_preexec() {
    export __SQZ_CMD="$BASH_COMMAND"
}
trap '__sqz_preexec' DEBUG
__sqz_postexec() {
    local exit_code=$?
    return $exit_code
}
PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__sqz_postexec"
sqz_run() {
    "$@" 2>&1 | SQZ_CMD="$*" sqz compress
}
# sudo passthrough: preserve compression for privileged commands
sqz_sudo() {
    sudo "$@" 2>&1 | SQZ_CMD="sudo $*" sqz compress
}
"#;

const ZSH_HOOK: &str = r#"
# sqz — context intelligence layer (auto-installed)
sqz_run() {
    "$@" 2>&1 | SQZ_CMD="$*" sqz compress
}
sqz_sudo() {
    sudo "$@" 2>&1 | SQZ_CMD="sudo $*" sqz compress
}
preexec() {
    export __SQZ_CMD="$1"
}
"#;

const FISH_HOOK: &str = r#"
# sqz — context intelligence layer (auto-installed)
function sqz_run
    set -lx SQZ_CMD (string join " " $argv)
    $argv 2>&1 | sqz compress
end
function sqz_sudo
    set -lx SQZ_CMD "sudo "(string join " " $argv)
    sudo $argv 2>&1 | sqz compress
end
"#;

const NUSHELL_HOOK: &str = r#"
# sqz — context intelligence layer (auto-installed)
def sqz_run [...args: string] {
    $env.SQZ_CMD = ($args | str join " ")
    run-external $args.0 ...$args[1..] | sqz compress
}
"#;

const POWERSHELL_HOOK: &str = r#"
# sqz — context intelligence layer (auto-installed)
function Invoke-SqzRun {
    param([string[]]$Command)
    $env:SQZ_CMD = ($Command -join " ")
    & @Command 2>&1 | sqz compress
}
Set-Alias sqz_run Invoke-SqzRun
function Invoke-SqzSudo {
    param([string[]]$Command)
    $env:SQZ_CMD = "sudo " + ($Command -join " ")
    Start-Process -Verb RunAs -FilePath $Command[0] -ArgumentList $Command[1..] 2>&1 | sqz compress
}
"#;

// ── ShellHook enum ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellHook {
    Bash,
    Zsh,
    Fish,
    Nushell,
    PowerShell,
}

impl ShellHook {
    /// Detect the current shell from the `SHELL` environment variable.
    /// Falls back to `Bash` when detection fails.
    pub fn detect() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_default();
        Self::detect_from_str(&shell)
    }

    /// Detect shell type from a shell path string (testable without env mutation).
    pub fn detect_from_str(shell: &str) -> Self {
        let shell = shell.to_lowercase();
        if shell.contains("zsh") {
            ShellHook::Zsh
        } else if shell.contains("fish") {
            ShellHook::Fish
        } else if shell.contains("nu") || shell.contains("nushell") {
            ShellHook::Nushell
        } else if shell.contains("pwsh") || shell.contains("powershell") {
            ShellHook::PowerShell
        } else {
            ShellHook::Bash
        }
    }

    /// Return the path to the shell RC / profile file.
    pub fn rc_path(&self) -> PathBuf {
        let home = home_dir();
        match self {
            ShellHook::Bash => home.join(".bashrc"),
            ShellHook::Zsh => home.join(".zshrc"),
            ShellHook::Fish => home
                .join(".config")
                .join("fish")
                .join("config.fish"),
            ShellHook::Nushell => home
                .join(".config")
                .join("nushell")
                .join("config.nu"),
            ShellHook::PowerShell => powershell_profile_path(),
        }
    }

    /// The hook script text to append.
    pub fn hook_script(&self) -> &'static str {
        match self {
            ShellHook::Bash => BASH_HOOK,
            ShellHook::Zsh => ZSH_HOOK,
            ShellHook::Fish => FISH_HOOK,
            ShellHook::Nushell => NUSHELL_HOOK,
            ShellHook::PowerShell => POWERSHELL_HOOK,
        }
    }

    /// A unique sentinel comment used to detect whether the hook is already
    /// installed, preventing duplicate entries.
    pub fn sentinel(&self) -> &'static str {
        "# sqz — context intelligence layer (auto-installed)"
    }

    /// Install the hook into the shell RC file.
    ///
    /// Returns `Ok(true)` when the hook was written, `Ok(false)` when it was
    /// already present.  On failure the error is logged and the function
    /// returns `Err`.
    pub fn install(&self) -> Result<bool, HookError> {
        let rc = self.rc_path();
        install_hook_to_file(&rc, self.hook_script(), self.sentinel())
    }

    /// Remove the hook from the shell RC file.
    ///
    /// Returns `Ok(true)` when the hook was removed, `Ok(false)` when it was
    /// not present.
    pub fn uninstall(&self) -> Result<bool, HookError> {
        let rc = self.rc_path();
        uninstall_hook_from_file(&rc, self.sentinel())
    }
}

// ── Error type ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct HookError {
    pub path: PathBuf,
    pub message: String,
}

impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "sqz hook installation failed for {}: {}",
            self.path.display(),
            self.message
        )
    }
}

impl std::error::Error for HookError {}

// ── Helpers ───────────────────────────────────────────────────────────────

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn powershell_profile_path() -> PathBuf {
    // $PROFILE is typically Documents\PowerShell\Microsoft.PowerShell_profile.ps1
    if let Ok(profile) = std::env::var("PROFILE") {
        return PathBuf::from(profile);
    }
    let home = home_dir();
    home.join("Documents")
        .join("PowerShell")
        .join("Microsoft.PowerShell_profile.ps1")
}

/// Append `script` to `path` unless `sentinel` is already present.
pub fn install_hook_to_file(
    path: &Path,
    script: &str,
    sentinel: &str,
) -> Result<bool, HookError> {
    // Read existing content (file may not exist yet).
    let existing = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| HookError {
            path: path.to_owned(),
            message: format!("read error: {e}"),
        })?
    } else {
        String::new()
    };

    if existing.contains(sentinel) {
        return Ok(false); // already installed
    }

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HookError {
            path: path.to_owned(),
            message: format!("create dir error: {e}"),
        })?;
    }

    // Append the hook.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| HookError {
            path: path.to_owned(),
            message: format!("open error: {e}"),
        })?;

    writeln!(file, "{script}").map_err(|e| HookError {
        path: path.to_owned(),
        message: format!("write error: {e}"),
    })?;

    Ok(true)
}

/// Remove the sqz hook block from `path`.
///
/// Finds the sentinel line and removes everything from that line to the
/// next blank line (or end of file). Returns `Ok(true)` if removed,
/// `Ok(false)` if not present.
pub fn uninstall_hook_from_file(path: &Path, sentinel: &str) -> Result<bool, HookError> {
    if !path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(path).map_err(|e| HookError {
        path: path.to_owned(),
        message: format!("read error: {e}"),
    })?;

    if !content.contains(sentinel) {
        return Ok(false); // not installed
    }

    // Remove the hook block: from the sentinel line to the next blank line
    let mut result = Vec::new();
    let mut in_hook = false;
    let mut removed = false;

    for line in content.lines() {
        if line.contains(sentinel) {
            in_hook = true;
            removed = true;
            continue;
        }
        if in_hook {
            // End of hook block: blank line or next comment section
            if line.trim().is_empty() {
                in_hook = false;
                continue; // skip the trailing blank line too
            }
            continue; // skip hook body lines
        }
        result.push(line);
    }

    if removed {
        std::fs::write(path, result.join("\n")).map_err(|e| HookError {
            path: path.to_owned(),
            message: format!("write error: {e}"),
        })?;
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn tmp_rc(dir: &TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    #[test]
    fn test_install_writes_hook() {
        let dir = TempDir::new().unwrap();
        let rc = tmp_rc(&dir, ".bashrc");
        let result = install_hook_to_file(&rc, BASH_HOOK, "# sqz — context intelligence layer (auto-installed)");
        assert!(result.unwrap());
        let content = fs::read_to_string(&rc).unwrap();
        assert!(content.contains("sqz compress"));
    }

    #[test]
    fn test_install_idempotent() {
        let dir = TempDir::new().unwrap();
        let rc = tmp_rc(&dir, ".bashrc");
        install_hook_to_file(&rc, BASH_HOOK, "# sqz — context intelligence layer (auto-installed)").unwrap();
        let result = install_hook_to_file(&rc, BASH_HOOK, "# sqz — context intelligence layer (auto-installed)").unwrap();
        assert!(!result, "second install should be a no-op");
    }

    #[test]
    fn test_install_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let rc = dir.path().join("nested").join("dir").join("config.fish");
        install_hook_to_file(&rc, FISH_HOOK, "# sqz — context intelligence layer (auto-installed)").unwrap();
        assert!(rc.exists());
    }

    #[test]
    fn test_detect_bash_default() {
        assert_eq!(ShellHook::detect_from_str(""), ShellHook::Bash);
        assert_eq!(ShellHook::detect_from_str("unknown"), ShellHook::Bash);
    }

    #[test]
    fn test_detect_zsh() {
        assert_eq!(ShellHook::detect_from_str("/bin/zsh"), ShellHook::Zsh);
    }

    #[test]
    fn test_detect_fish() {
        assert_eq!(ShellHook::detect_from_str("/usr/bin/fish"), ShellHook::Fish);
    }

    #[test]
    fn test_detect_nushell() {
        assert_eq!(ShellHook::detect_from_str("/usr/bin/nu"), ShellHook::Nushell);
        assert_eq!(ShellHook::detect_from_str("nushell"), ShellHook::Nushell);
    }

    #[test]
    fn test_detect_powershell() {
        assert_eq!(ShellHook::detect_from_str("pwsh"), ShellHook::PowerShell);
        assert_eq!(ShellHook::detect_from_str("/usr/bin/powershell"), ShellHook::PowerShell);
    }

    #[test]
    fn test_all_shells_have_sudo_or_equivalent() {
        // Bash, Zsh, Fish, PowerShell all have sudo passthrough
        // Nushell doesn't have sudo in the same way — that's expected
        for hook in &[ShellHook::Bash, ShellHook::Zsh, ShellHook::Fish] {
            assert!(
                hook.hook_script().contains("sudo"),
                "{hook:?} should have sudo passthrough"
            );
        }
    }

    #[test]
    fn test_nushell_hook_has_sqz_run() {
        assert!(NUSHELL_HOOK.contains("sqz_run"));
        assert!(NUSHELL_HOOK.contains("sqz compress"));
    }

    #[test]
    fn test_uninstall_removes_hook() {
        let dir = TempDir::new().unwrap();
        let rc = tmp_rc(&dir, ".bashrc");
        let sentinel = "# sqz — context intelligence layer (auto-installed)";

        // Install first
        install_hook_to_file(&rc, BASH_HOOK, sentinel).unwrap();
        let content = fs::read_to_string(&rc).unwrap();
        assert!(content.contains(sentinel));

        // Uninstall
        let removed = uninstall_hook_from_file(&rc, sentinel).unwrap();
        assert!(removed, "should return true when hook was removed");

        let content_after = fs::read_to_string(&rc).unwrap();
        assert!(!content_after.contains(sentinel), "sentinel should be gone after uninstall");
    }

    #[test]
    fn test_uninstall_nonexistent_is_noop() {
        let dir = TempDir::new().unwrap();
        let rc = tmp_rc(&dir, ".bashrc_nonexistent");
        let result = uninstall_hook_from_file(&rc, "# sqz — context intelligence layer (auto-installed)");
        assert!(!result.unwrap(), "uninstall on missing file should return false");
    }

    #[test]
    fn test_uninstall_not_installed_is_noop() {
        let dir = TempDir::new().unwrap();
        let rc = tmp_rc(&dir, ".bashrc");
        fs::write(&rc, "# some existing content\nexport PATH=$PATH:/usr/local/bin\n").unwrap();
        let result = uninstall_hook_from_file(&rc, "# sqz — context intelligence layer (auto-installed)").unwrap();
        assert!(!result, "uninstall when not installed should return false");
        // Original content preserved
        let content = fs::read_to_string(&rc).unwrap();
        assert!(content.contains("some existing content"));
    }

    #[test]
    fn test_rc_paths_distinct_for_all_shells() {
        let paths: Vec<_> = [
            ShellHook::Bash,
            ShellHook::Zsh,
            ShellHook::Fish,
            ShellHook::Nushell,
            ShellHook::PowerShell,
        ]
        .iter()
        .map(|h| h.rc_path())
        .collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len(), "each shell must have a unique RC path");
    }
}
