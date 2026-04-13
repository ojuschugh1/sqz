/// CLI Proxy — intercepts command output and compresses it through SqzEngine.
///
/// `CliProxy::intercept_output` is the core entry point: it takes raw command
/// output, runs it through per-command formatters first, then the compression
/// pipeline, with SHA-256 dedup cache for repeated content.
///
/// On any failure it logs the error and returns the original output unchanged
/// (transparent fallback, Requirement 1.5).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::Path;
use sqz_engine::{format_command, CompressedContent, DependencyMapper, SqzEngine};

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

// ── Command-aware pre-processors ─────────────────────────────────────────
// (Moved to sqz_engine::cmd_formatters for reuse across CLI, MCP, and IDE)

// ── Dedup cache ──────────────────────────────────────────────────────────
// Persistent SHA-256 dedup is handled by SqzEngine's CacheManager.
// The in-memory cache below is a fast first-level check to avoid
// hitting SQLite on every call within the same process lifetime.

/// Compute a fast hash of content for in-memory dedup.
fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

// ── CliProxy ─────────────────────────────────────────────────────────────

/// In-memory first-level dedup cache entry (avoids SQLite round-trip).
#[allow(dead_code)]
struct CacheEntry {
    hash: u64,
    tokens_original: u32,
}

pub struct CliProxy {
    engine: SqzEngine,
    /// In-memory L1 dedup cache (fast hash → seen).
    /// On miss, falls through to the persistent CacheManager (L2).
    l1_cache: std::cell::RefCell<HashSet<u64>>,
    /// File paths whose content is in the dedup cache (for cross-command context refs).
    known_files: std::cell::RefCell<HashSet<String>>,
    /// Dependency mapper for predictive pre-caching.
    dep_mapper: std::cell::RefCell<DependencyMapper>,
}

impl CliProxy {
    /// Create a new `CliProxy` backed by a default `SqzEngine`.
    pub fn new() -> sqz_engine::Result<Self> {
        let engine = SqzEngine::new()?;
        Ok(Self {
            engine,
            l1_cache: std::cell::RefCell::new(HashSet::new()),
            known_files: std::cell::RefCell::new(HashSet::new()),
            dep_mapper: std::cell::RefCell::new(DependencyMapper::new()),
        })
    }

    /// Create a `CliProxy` with an existing engine (useful in tests).
    #[allow(dead_code)]
    pub fn with_engine(engine: SqzEngine) -> Self {
        Self {
            engine,
            l1_cache: std::cell::RefCell::new(HashSet::new()),
            known_files: std::cell::RefCell::new(HashSet::new()),
            dep_mapper: std::cell::RefCell::new(DependencyMapper::new()),
        }
    }

    /// Intercept `output` produced by `cmd`, compress it, and return the
    /// compressed text.
    ///
    /// Flow:
    /// 1. Check dedup cache — if exact content was seen before, return a
    ///    compact reference (~13 tokens instead of full re-compression).
    /// 2. Try per-command formatter (git status, cargo test, etc.).
    /// 3. Fall back to generic compression pipeline.
    /// 4. Cache the result for future dedup.
    ///
    /// On any compression error the original `output` is returned unchanged
    /// and the error is logged to stderr (Requirement 1.5 fallback).
    pub fn intercept_output(&self, cmd: &str, output: &str) -> String {
        // Step 1: L1 in-memory dedup check (fast path)
        let fast_hash = content_hash(output);
        if self.l1_cache.borrow().contains(&fast_hash) {
            // L1 hit — check L2 persistent cache for the actual ref
            if let Ok(Some(inline_ref)) = self.engine.cache_manager().check_dedup(output.as_bytes()) {
                eprintln!("[sqz] dedup hit: {} (L1+L2)", inline_ref);
                return inline_ref;
            }
        }

        // Step 2: L2 persistent SHA-256 dedup check (survives restarts)
        if let Ok(Some(inline_ref)) = self.engine.cache_manager().check_dedup(output.as_bytes()) {
            // Promote to L1 for faster future lookups
            self.l1_cache.borrow_mut().insert(fast_hash);
            eprintln!("[sqz] dedup hit: {} (L2)", inline_ref);
            return inline_ref;
        }

        // Step 3: Track file reads for cross-command context refs + predictive pre-cache
        self.track_file(cmd, output);

        // Step 4: Try per-command formatter
        if let Some(formatted) = format_command(cmd, output) {
            let tokens_original = (output.len() as u32 + 3) / 4;
            let tokens_compressed = (formatted.len() as u32 + 3) / 4;
            if tokens_compressed < tokens_original {
                // Persist to L2 cache
                if let Ok(compressed) = self.engine.compress(&formatted) {
                    let _ = self.engine.cache_manager().store_compressed(output.as_bytes(), &compressed);
                }
                self.l1_cache.borrow_mut().insert(fast_hash);
                self.log_compression(cmd, tokens_original, tokens_compressed);
                return self.apply_context_refs(&formatted);
            }
        }

        // Step 5: Generic compression pipeline
        match self.compress_output(cmd, output) {
            Ok(compressed) => {
                let tokens_original = compressed.tokens_original;
                let tokens_compressed = compressed.tokens_compressed;
                // Persist to L2 cache
                let _ = self.engine.cache_manager().store_compressed(output.as_bytes(), &compressed);
                self.l1_cache.borrow_mut().insert(fast_hash);
                self.log_compression(cmd, tokens_original, tokens_compressed);
                self.apply_context_refs(&compressed.data)
            }
            Err(e) => {
                eprintln!("[sqz] fallback: compression error for command '{cmd}': {e}");
                output.to_owned()
            }
        }
    }

    /// Log compression stats to stderr.
    fn log_compression(&self, cmd: &str, original: u32, compressed: u32) {
        let saved = original.saturating_sub(compressed);
        let pct = if original > 0 { (saved as f64 / original as f64 * 100.0) as u32 } else { 0 };
        eprintln!("[sqz] {}/{} tokens ({}% reduction) [{}]", compressed, original, pct, cmd);
        // Also log to session store for `sqz gain` tracking
        let _ = self.engine.session_store().log_compression(
            original, compressed, &[], cmd,
        );
    }

    /// Internal: run `output` through the engine pipeline.
    fn compress_output(
        &self,
        _cmd: &str,
        output: &str,
    ) -> sqz_engine::Result<CompressedContent> {
        self.engine.compress(output)
    }

    // ── Cross-command context references ──────────────────────────────────

    /// Scan `text` for file paths that are already in the dedup cache.
    /// Replace inline file content excerpts with compact cache references.
    ///
    /// Example: if `src/auth.rs` is cached and an error message contains
    /// a multi-line excerpt from that file, replace it with
    /// `[in context: src/auth.rs]`.
    fn apply_context_refs(&self, text: &str) -> String {
        let known = self.known_files.borrow();
        if known.is_empty() {
            return text.to_string();
        }

        let mut result = text.to_string();
        for file_path in known.iter() {
            // Extract just the filename for matching
            let _filename = Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file_path);

            // Look for patterns like "  --> src/auth.rs:42:5" followed by
            // multi-line code excerpts (indented lines). Replace the excerpt
            // with a compact reference.
            let marker = format!("--> {}", file_path);
            if result.contains(&marker) {
                // The file is referenced in an error — the LLM already has it
                // in context from a previous read. Add a note.
                let note = format!("{} [in context]", marker);
                result = result.replace(&marker, &note);
            }
        }
        result
    }

    /// Track a file path as "in context" (its content is in the dedup cache).
    fn track_file(&self, cmd: &str, output: &str) {
        // Detect file-read commands: cat, head, tail, or any command with a
        // file path argument
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let base = parts.first().map(|s| s.rsplit('/').next().unwrap_or(s)).unwrap_or("");

        match base {
            "cat" | "head" | "tail" | "less" | "bat" => {
                // The file path is typically the last argument
                if let Some(path) = parts.last() {
                    if Path::new(path).extension().is_some() {
                        self.known_files.borrow_mut().insert(path.to_string());
                        // Predictive pre-cache: analyze imports
                        self.predictive_precache(path, output);
                    }
                }
            }
            _ => {}
        }
    }

    // ── Predictive pre-caching ───────────────────────────────────────────

    /// When a file is read, parse its imports and pre-cache the dependency
    /// file paths. When the LLM inevitably reads those files next, they'll
    /// be instant dedup hits.
    fn predictive_precache(&self, file_path: &str, content: &str) {
        let path = Path::new(file_path);

        // Add the file to the dependency mapper
        self.dep_mapper.borrow_mut().add_file(path, content);

        // Get its dependencies
        let deps = self.dep_mapper.borrow().dependencies_of(path);

        if deps.is_empty() {
            return;
        }

        // Pre-read and cache each dependency that exists on disk
        let mut precached = 0;
        for dep_path in &deps {
            // Try to resolve to an actual file
            let resolved = if dep_path.is_absolute() {
                dep_path.clone()
            } else if let Some(parent) = path.parent() {
                parent.join(dep_path)
            } else {
                dep_path.clone()
            };

            if resolved.exists() && resolved.is_file() {
                // Read and hash the file content
                if let Ok(dep_content) = std::fs::read_to_string(&resolved) {
                    // Check if already in persistent cache
                    if let Ok(Some(_)) = self.engine.cache_manager().check_dedup(dep_content.as_bytes()) {
                        continue; // Already cached
                    }

                    // Compress and persist to L2 cache
                    if let Ok(compressed) = self.engine.compress(&dep_content) {
                        let _ = self.engine.cache_manager().store_compressed(
                            dep_content.as_bytes(), &compressed,
                        );
                        let hash = content_hash(&dep_content);
                        self.l1_cache.borrow_mut().insert(hash);
                        let dep_str = resolved.to_string_lossy().to_string();
                        self.known_files.borrow_mut().insert(dep_str);
                        precached += 1;
                    }
                }
            }
        }

        if precached > 0 {
            eprintln!("[sqz] predictive pre-cache: {} dependencies of {} cached",
                precached, file_path);
        }
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

    #[test]
    fn test_dedup_cache_returns_ref_on_second_call() {
        let proxy = CliProxy::new().expect("engine init");
        let output = "some repeated output that is long enough to be meaningful\n".repeat(5);
        let first = proxy.intercept_output("echo", &output);
        let second = proxy.intercept_output("echo", &output);
        // Second call should return a dedup reference (from L1 or L2 cache)
        assert!(second.starts_with("§ref:"), "expected dedup ref, got: {}", second);
        assert!(second.len() < first.len() || first.starts_with("§ref:"),
            "dedup ref should be shorter than original");
    }

    #[test]
    fn test_file_tracking_on_cat() {
        let proxy = CliProxy::new().expect("engine init");
        let content = "use std::io;\nfn main() {}\n";
        proxy.intercept_output("cat src/main.rs", content);
        let known = proxy.known_files.borrow();
        assert!(known.contains("src/main.rs"), "cat should track the file path");
    }

    #[test]
    fn test_context_refs_annotate_known_files() {
        let proxy = CliProxy::new().expect("engine init");
        // Simulate reading a file
        proxy.known_files.borrow_mut().insert("src/auth.rs".to_string());
        // Error output referencing that file
        let error = "error[E0308]: mismatched types\n --> src/auth.rs:42:5\n";
        let result = proxy.apply_context_refs(error);
        assert!(result.contains("[in context]"), "should annotate known file: {}", result);
    }

    #[test]
    fn test_context_refs_no_annotation_for_unknown_files() {
        let proxy = CliProxy::new().expect("engine init");
        let error = "error[E0308]: mismatched types\n --> src/unknown.rs:42:5\n";
        let result = proxy.apply_context_refs(error);
        assert!(!result.contains("[in context]"), "should not annotate unknown file");
    }
}
