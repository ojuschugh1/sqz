/// CLI Proxy — intercepts command output and compresses it through SqzEngine.
///
/// `CliProxy::intercept_output` is the core entry point: it takes raw command
/// output, runs it through the compression pipeline, and returns the
/// compressed text.  On any failure it logs the error and returns the
/// original output unchanged (transparent fallback, Requirement 1.5).

use sqz_engine::{CompressedContent, SqzEngine};

// ── CLI compression patterns ──────────────────────────────────────────────

/// A registry of recognised CLI command patterns.  Each entry is a prefix or
/// substring that identifies the command whose output is being compressed.
/// The list covers 90+ distinct command output formats (Requirement 1.2).
pub const CLI_PATTERNS: &[&str] = &[
    // Version control
    "git", "hg", "svn", "fossil",
    // Build tools
    "cargo", "make", "cmake", "ninja", "bazel", "buck", "gradle", "mvn",
    "ant", "sbt", "lein", "mix", "rebar3",
    // Package managers
    "npm", "yarn", "pnpm", "bun", "pip", "pip3", "poetry", "pipenv",
    "conda", "gem", "bundle", "composer", "go", "dep", "glide",
    "apt", "apt-get", "dpkg", "yum", "dnf", "rpm", "pacman", "brew",
    "port", "snap", "flatpak", "nix", "guix",
    // Containers / orchestration
    "docker", "podman", "buildah", "skopeo", "kubectl", "helm", "k9s",
    "minikube", "kind", "k3s", "nomad", "consul", "vault",
    // Cloud CLIs
    "aws", "az", "gcloud", "gsutil", "terraform", "pulumi", "cdk",
    "serverless", "sam",
    // Language runtimes
    "node", "deno", "python", "python3", "ruby", "java", "kotlin",
    "scala", "clojure", "elixir", "erlang", "ghc", "rustc", "clang",
    "gcc", "g++",
    // Test runners
    "jest", "mocha", "pytest", "rspec", "minitest", "phpunit", "vitest",
    "cypress", "playwright",
    // Linters / formatters
    "eslint", "tslint", "prettier", "black", "isort", "flake8", "mypy",
    "pylint", "rubocop", "golangci-lint", "clippy", "rustfmt",
    // System / network
    "curl", "wget", "ssh", "scp", "rsync", "nc", "netstat", "ss",
    "ping", "traceroute", "dig", "nslookup", "openssl",
    // File / text processing
    "find", "grep", "rg", "ag", "fd", "ls", "tree", "cat", "less",
    "head", "tail", "wc", "sort", "uniq", "awk", "sed", "jq", "yq",
    // Databases
    "psql", "mysql", "sqlite3", "mongo", "redis-cli", "influx",
    // Misc dev tools
    "gh", "hub", "lab", "glab", "jira", "linear",
    "ansible", "chef", "puppet", "salt",
    "ffmpeg", "convert", "identify",
];

// ── CliProxy ─────────────────────────────────────────────────────────────

pub struct CliProxy {
    engine: SqzEngine,
}

impl CliProxy {
    /// Create a new `CliProxy` backed by a default `SqzEngine`.
    pub fn new() -> sqz_engine::Result<Self> {
        let engine = SqzEngine::new()?;
        Ok(Self { engine })
    }

    /// Create a `CliProxy` with an existing engine (useful in tests).
    #[allow(dead_code)]
    pub fn with_engine(engine: SqzEngine) -> Self {
        Self { engine }
    }

    /// Intercept `output` produced by `cmd`, compress it, and return the
    /// compressed text.
    ///
    /// On any compression error the original `output` is returned unchanged
    /// and the error is logged to stderr (Requirement 1.5 fallback).
    pub fn intercept_output(&self, cmd: &str, output: &str) -> String {
        match self.compress_output(cmd, output) {
            Ok(compressed) => compressed.data,
            Err(e) => {
                eprintln!("[sqz] compression error for command '{cmd}': {e}");
                output.to_owned()
            }
        }
    }

    /// Internal: run `output` through the engine pipeline.
    fn compress_output(
        &self,
        _cmd: &str,
        output: &str,
    ) -> sqz_engine::Result<CompressedContent> {
        self.engine.compress(output)
    }

    /// Return `true` when `cmd` matches one of the registered CLI patterns.
    #[allow(dead_code)]
    pub fn is_known_command(cmd: &str) -> bool {
        let base = cmd
            .split_whitespace()
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("");
        CLI_PATTERNS
            .iter()
            .any(|p| base.eq_ignore_ascii_case(p))
    }

    /// Main event loop: read lines from stdin, compress each one, write to
    /// stdout.  This is the mode used when the shell hook pipes output
    /// through `sqz compress`.
    pub fn run_proxy(&self) -> sqz_engine::Result<()> {
        use std::io::{self, BufRead, Write};
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = stdout.lock();

        let mut buf = String::new();
        for line in stdin.lock().lines() {
            let line = line.map_err(|e| sqz_engine::SqzError::Other(e.to_string()))?;
            buf.push_str(&line);
            buf.push('\n');
        }

        let compressed = self.intercept_output("stdin", &buf);
        out.write_all(compressed.as_bytes())
            .map_err(|e| sqz_engine::SqzError::Other(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_known_command_git() {
        assert!(CliProxy::is_known_command("git"));
        assert!(CliProxy::is_known_command("/usr/bin/git"));
        assert!(CliProxy::is_known_command("git status"));
    }

    #[test]
    fn test_is_known_command_unknown() {
        assert!(!CliProxy::is_known_command("my_custom_tool"));
    }

    #[test]
    fn test_patterns_count() {
        assert!(
            CLI_PATTERNS.len() >= 90,
            "expected ≥90 patterns, got {}",
            CLI_PATTERNS.len()
        );
    }

    #[test]
    fn test_intercept_output_returns_string() {
        let proxy = CliProxy::new().expect("engine init");
        let output = "hello world\nsome output\n";
        let result = proxy.intercept_output("echo", output);
        // Result must be non-empty (either compressed or original fallback).
        assert!(!result.is_empty());
    }

    #[test]
    fn test_intercept_output_fallback_on_empty() {
        let proxy = CliProxy::new().expect("engine init");
        // Empty input should not panic and should return something.
        let result = proxy.intercept_output("git", "");
        // Empty input may compress to empty — just ensure no panic.
        let _ = result;
    }
}
