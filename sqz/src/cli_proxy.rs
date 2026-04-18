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
use sqz_engine::{format_command, CompressedContent, DependencyMapper, NgramAbbreviator, SqzEngine};

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
    /// Dependency mapper for predictive pre-caching (in-memory, rebuilt per session).
    dep_mapper: std::cell::RefCell<DependencyMapper>,
    /// Session-level n-gram abbreviator for recurring phrase compression.
    abbreviator: std::cell::RefCell<NgramAbbreviator>,
}

impl CliProxy {
    /// Create a new `CliProxy` backed by a default `SqzEngine`.
    pub fn new() -> sqz_engine::Result<Self> {
        let engine = SqzEngine::new()?;
        Ok(Self {
            engine,
            l1_cache: std::cell::RefCell::new(HashSet::new()),
            dep_mapper: std::cell::RefCell::new(DependencyMapper::new()),
            abbreviator: std::cell::RefCell::new(NgramAbbreviator::new()),
        })
    }

    /// Create a `CliProxy` with an existing engine (useful in tests).
    #[allow(dead_code)]
    pub fn with_engine(engine: SqzEngine) -> Self {
        Self {
            engine,
            l1_cache: std::cell::RefCell::new(HashSet::new()),
            dep_mapper: std::cell::RefCell::new(DependencyMapper::new()),
            abbreviator: std::cell::RefCell::new(NgramAbbreviator::new()),
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
        // Always track file reads for cross-command context refs,
        // even if the content is a dedup hit (the file is still "known").
        self.track_file(cmd, output);

        // Advance the turn counter for compaction-aware dedup.
        // Each intercept_output call represents one LLM interaction turn.
        self.engine.cache_manager().advance_turn();

        // Step 1: L1 in-memory dedup check (fast path)
        let fast_hash = content_hash(output);
        if self.l1_cache.borrow().contains(&fast_hash) {
            // L1 hit — check L2 persistent cache for the actual ref
            if let Ok(Some(inline_ref)) = self.engine.cache_manager().check_dedup(output.as_bytes()) {
                eprintln!("[sqz] dedup hit: {} (L1+L2)", inline_ref);
                self.log_dedup_hit(cmd, output);
                return inline_ref;
            }
        }

        // Step 2: L2 persistent SHA-256 dedup check (survives restarts)
        if let Ok(Some(inline_ref)) = self.engine.cache_manager().check_dedup(output.as_bytes()) {
            // Promote to L1 for faster future lookups
            self.l1_cache.borrow_mut().insert(fast_hash);
            eprintln!("[sqz] dedup hit: {} (L2)", inline_ref);
            self.log_dedup_hit(cmd, output);
            return inline_ref;
        }

        // Step 3: Try per-command formatter
        if let Some(formatted) = format_command(cmd, output) {
            let tokens_original = (output.len() as u32 + 3) / 4;
            let tokens_compressed = (formatted.len() as u32 + 3) / 4;
            if tokens_compressed < tokens_original {
                // Persist to L2 cache — but skip if content contains secrets
                // (confidence router detected high-risk patterns like API keys)
                let mode = self.engine.route_compression_mode(output);
                if mode != sqz_engine::CompressionMode::Safe {
                    if let Ok(compressed) = self.engine.compress(&formatted) {
                        let _ = self.engine.cache_manager().store_compressed(output.as_bytes(), &compressed);
                    }
                }
                self.l1_cache.borrow_mut().insert(fast_hash);
                self.log_compression(cmd, tokens_original, tokens_compressed);
                return self.apply_context_refs(&formatted);
            }
        }

        // Step 4: Generic compression pipeline
        match self.compress_output(cmd, output) {
            Ok(compressed) => {
                let tokens_original = compressed.tokens_original;
                let tokens_compressed = compressed.tokens_compressed;
                // Persist to L2 cache — skip if content was routed to Safe mode
                // (may contain secrets, API keys, passwords)
                let mode = self.engine.route_compression_mode(output);
                if mode != sqz_engine::CompressionMode::Safe {
                    let _ = self.engine.cache_manager().store_compressed(output.as_bytes(), &compressed);
                }
                self.l1_cache.borrow_mut().insert(fast_hash);
                self.log_compression(cmd, tokens_original, tokens_compressed);

                // Technique 3: N-gram abbreviation — observe output for phrase
                // frequency tracking, then apply abbreviations to the result
                let mut abbr = self.abbreviator.borrow_mut();
                abbr.observe(&compressed.data);
                let abbreviated = match abbr.abbreviate(&compressed.data) {
                    Ok(result) if result.total_tokens_saved > 0 => {
                        eprintln!("[sqz] n-gram abbreviation: {} tokens saved", result.total_tokens_saved);
                        result.text
                    }
                    _ => compressed.data,
                };

                self.apply_context_refs(&abbreviated)
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

    /// Record a dedup hit in the compression log so `sqz stats` and
    /// `sqz gain` reflect the savings.
    ///
    /// A dedup hit replaces the full content with a 13-token §ref:hash§
    /// marker (hard-coded to match `CacheResult::Dedup { token_cost: 13 }`
    /// in cache_manager.rs). Without this call the dominant savings path
    /// is invisible to users — they'd see ~15% average reduction in
    /// `sqz stats` while actually getting 99%+ on repeat reads.
    ///
    /// `tokens_original` uses the same byte/4 heuristic the formatter
    /// path uses (cli_proxy.rs line ~161). Switching both paths to real
    /// tiktoken counts is a separate follow-up; using the same heuristic
    /// keeps the reporting internally consistent.
    fn log_dedup_hit(&self, _cmd: &str, output: &str) {
        let tokens_original = (output.len() as u32 + 3) / 4;
        // A dedup ref is approximately 13 tokens; matches token_cost in
        // CacheResult::Dedup and the tests that assert on it.
        const DEDUP_REF_TOKENS: u32 = 13;
        // Tag with mode "dedup" so downstream analysis can distinguish
        // pipeline compressions from cache hits.
        let _ = self.engine.session_store().log_compression(
            tokens_original,
            DEDUP_REF_TOKENS,
            &["dedup".to_string()],
            "dedup",
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

    /// Scan `text` for file paths that are already in the persistent known_files
    /// store. When an error message references a file the LLM has already seen,
    /// annotate it so the LLM knows not to re-read it.
    fn apply_context_refs(&self, text: &str) -> String {
        let known = match self.engine.session_store().known_files() {
            Ok(files) => files,
            Err(_) => return text.to_string(),
        };
        if known.is_empty() {
            return text.to_string();
        }

        let mut result = text.to_string();
        for file_path in &known {
            // Look for error location patterns: "  --> path:line:col"
            let marker = format!("--> {}", file_path);
            if result.contains(&marker) {
                let note = format!("{} [in context]", marker);
                result = result.replace(&marker, &note);
            }
            // Also check for bare path references in error output
            // e.g. "at src/auth.rs:42" or "file: src/auth.rs"
            let at_marker = format!("at {}:", file_path);
            if result.contains(&at_marker) {
                let note = format!("at {} [in context]:", file_path);
                result = result.replace(&at_marker, &note);
            }
        }
        result
    }

    /// Track a file path as "in context" — persists to SessionStore so it
    /// survives across sqz process invocations (each shell hook call is a
    /// separate process).
    fn track_file(&self, cmd: &str, output: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let base = parts.first().map(|s| s.rsplit('/').next().unwrap_or(s)).unwrap_or("");

        match base {
            "cat" | "head" | "tail" | "less" | "bat" => {
                if let Some(path) = parts.last() {
                    if Path::new(path).extension().is_some() {
                        // Persist to SQLite so next sqz invocation sees it
                        let _ = self.engine.session_store().add_known_file(path);
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
                        // Persist to known_files so cross-command refs work
                        let _ = self.engine.session_store().add_known_file(&dep_str);
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

    /// Main event loop: read all stdin, compress, write to stdout.
    /// Reads `SQZ_CMD` env var for command identity (set by shell hooks).
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

        let cmd = std::env::var("SQZ_CMD").unwrap_or_else(|_| "stdin".to_string());
        let compressed = self.intercept_output(&cmd, &buf);
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
        // Use unique content so this test doesn't depend on prior test state
        // in the shared ~/.sqz/sessions.db cache.
        let unique_tag = format!(
            "tag-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now(),
        );
        let output = format!(
            "dedup test content with {}\n{}",
            unique_tag,
            "some repeated output that is long enough to be meaningful\n".repeat(5),
        );

        // Capture the compression count before the two calls.
        let store = proxy.engine.session_store();
        let count_before = store.compression_stats().unwrap_or_default().total_compressions;

        let first = proxy.intercept_output("echo", &output);
        let second = proxy.intercept_output("echo", &output);

        // Second call must be a dedup ref.
        assert!(
            second.starts_with("§ref:"),
            "expected dedup ref, got: {}",
            second
        );
        assert!(
            second.len() < first.len(),
            "dedup ref should be shorter than original"
        );

        // Both calls must be recorded in the log. Before the April 18
        // reporting fix, dedup hits returned early and never logged — so
        // `sqz stats` undercounted. This assertion locks in the fix.
        let count_after = store.compression_stats().unwrap_or_default().total_compressions;
        assert!(
            count_after >= count_before + 2,
            "both intercept calls must be logged (including dedup hit); \
             before={count_before}, after={count_after}"
        );
    }

    #[test]
    fn test_file_tracking_on_cat() {
        let proxy = CliProxy::new().expect("engine init");
        let content = "use std::io;\nfn main() {}\n";
        proxy.intercept_output("cat src/main.rs", content);
        // File should be persisted in the session store
        let known = proxy.engine.session_store().known_files().unwrap();
        assert!(known.contains(&"src/main.rs".to_string()), "cat should track the file path");
    }

    #[test]
    fn test_context_refs_annotate_known_files() {
        let proxy = CliProxy::new().expect("engine init");
        // Simulate reading a file (persists to session store)
        let _ = proxy.engine.session_store().add_known_file("src/auth.rs");
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

    // ── Regression tests for Reddit bug report ────────────────────────────
    // https://github.com/ojuschugh1/sqz/issues/1 (related discussion)
    //
    // Word abbreviation was silently rewriting directory names, file paths,
    // and identifiers in command output. "packages" → "pkgs" broke paths,
    // "configuration" → "config" broke directory listings, etc.

    // Helper for the Reddit-bug regressions below. Each `intercept_output`
    // call may either return the original content (first call in a fresh
    // cache) or a §ref:...§ dedup marker if an earlier run put the same
    // content in the persistent ~/.sqz/sessions.db cache. The tests below
    // check the bug patterns, not the dedup state — they must accept
    // either outcome.
    fn assert_not_abbreviated(result: &str, bug_patterns: &[(&str, &str)]) {
        if result.starts_with("§ref:") && result.trim().ends_with('§') {
            // Dedup hit — the agent was already told about this content.
            // The bug patterns can't possibly appear in a ref token.
            return;
        }
        for &(wrong, why) in bug_patterns {
            assert!(
                !result.contains(wrong),
                "output must not contain '{wrong}' ({why}) — got:\n{result}"
            );
        }
    }

    #[test]
    fn test_reddit_packages_not_abbreviated() {
        let proxy = CliProxy::new().expect("engine init");
        let output = "drwxr-xr-x  5 user user 4096 Apr 15 10:00 packages\n\
                      drwxr-xr-x  3 user user 4096 Apr 15 10:00 configuration\n\
                      drwxr-xr-x  2 user user 4096 Apr 15 10:00 documentation\n";
        let result = proxy.intercept_output("ls -la", output);
        assert_not_abbreviated(&result, &[
            ("pkgs", "packages→pkgs regression"),
            (" config/", "configuration→config path rewrite"),
            (" docs/", "documentation→docs path rewrite"),
        ]);
        // If not a dedup hit, the original identifiers must survive.
        if !result.starts_with("§ref:") {
            assert!(result.contains("packages"), "{}", result);
            assert!(result.contains("configuration"), "{}", result);
            assert!(result.contains("documentation"), "{}", result);
        }
    }

    #[test]
    fn test_paths_preserved_in_output() {
        let proxy = CliProxy::new().expect("engine init");
        let output = "/etc/myapp/configuration/default.yml\n\
                      /usr/share/documentation/readme.md\n\
                      /home/user/.local/environment/config\n";
        let result = proxy.intercept_output("find /etc -name '*.yml'", output);
        assert_not_abbreviated(&result, &[
            ("/etc/myapp/config/", "configuration→config path rewrite"),
            ("/usr/share/docs/", "documentation→docs path rewrite"),
            (".local/env/", "environment→env path rewrite"),
        ]);
        if !result.starts_with("§ref:") {
            assert!(result.contains("configuration"), "{}", result);
            assert!(result.contains("documentation"), "{}", result);
            assert!(result.contains("environment"), "{}", result);
        }
    }

    #[test]
    fn test_git_urls_preserved() {
        let proxy = CliProxy::new().expect("engine init");
        let output = "origin\thttps://github.com/example/repository.git (fetch)\n\
                      origin\thttps://github.com/example/repository.git (push)\n";
        let result = proxy.intercept_output("git remote -v", output);
        assert_not_abbreviated(&result, &[
            ("github.com/example/repo.git", "repository→repo URL rewrite"),
        ]);
        if !result.starts_with("§ref:") {
            assert!(result.contains("repository"), "{}", result);
        }
    }

    #[test]
    fn test_identifiers_preserved_in_code_output() {
        let proxy = CliProxy::new().expect("engine init");
        let output = "error[E0433]: failed to resolve: use of undeclared crate or module `implementation`\n\
                      --> src/main.rs:5:5\n\
                      5 | use implementation::Config;\n";
        let result = proxy.intercept_output("cargo build", output);
        assert_not_abbreviated(&result, &[
            ("use impl::Config", "implementation→impl identifier rewrite"),
        ]);
        if !result.starts_with("§ref:") {
            assert!(result.contains("implementation"), "{}", result);
        }
    }

    #[test]
    fn test_ls_output_preserves_all_filenames_through_rle() {
        // Reddit repro end-to-end. When the output is new (cache miss), the
        // pipeline must preserve every filename. When it's a cache hit, the
        // §ref:...§ response is a correct compression.
        let proxy = CliProxy::new().expect("engine init");
        let output = "total 24\n\
                      drwxr-xr-x  6 user user  192 Apr 18 10:00 packages\n\
                      drwxr-xr-x  3 user user   96 Apr 18 10:00 configuration\n\
                      drwxr-xr-x  4 user user  128 Apr 18 10:00 documentation\n\
                      drwxr-xr-x  2 user user   64 Apr 18 10:00 environment\n\
                      -rw-r--r--  1 user user 1024 Apr 18 10:00 README.md\n\
                      -rw-r--r--  1 user user  512 Apr 18 10:00 Cargo.toml\n\
                      -rw-r--r--  1 user user  256 Apr 18 10:00 LICENSE\n";
        let result = proxy.intercept_output("ls -la", output);
        assert_not_abbreviated(&result, &[
            ("unique values", "RLE pattern-run must not summarize filenames away"),
            ("pkgs/", "packages→pkgs rewrite"),
        ]);
        if !result.starts_with("§ref:") {
            for name in &["packages", "configuration", "documentation", "environment",
                          "README.md", "Cargo.toml", "LICENSE"] {
                assert!(result.contains(name),
                    "filename '{name}' must appear in output — got:\n{result}");
            }
        }
    }
}
