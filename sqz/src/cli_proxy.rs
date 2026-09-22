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

/// Per-call options for [`CliProxy::intercept_output_with_options`].
///
/// Builder-lite pattern so callers that want the default path stay on the
/// short-form `intercept_output`. Note the [`Default`] impl is hand-written
/// (not derived) because one field — `abbreviate` — defaults to `true`,
/// which a derive could not express.
#[derive(Debug, Clone, Copy)]
pub struct InterceptOptions {
    /// Skip both L1 and L2 dedup lookups. The compression pipeline still
    /// runs; the 13-token `§ref:…§` shortcut never fires. Useful for
    /// models that can't parse inline refs (reported for GLM 5.1 on
    /// Synthetic).
    pub no_cache: bool,
    /// Apply n-gram phrase abbreviation to the generic-compressed output
    /// (replace recurring multi-word phrases with `«A1»` symbols + a
    /// legend). **Defaults to `true`** to preserve historical behaviour.
    ///
    /// Turn this OFF (`--no-abbrev`, or `SQZ_NO_ABBREV=1`) when the output
    /// carries identifiers an agent will copy-paste verbatim — SHAs, file
    /// paths, URLs. The abbreviator keeps only the FIRST occurrence of a
    /// repeated phrase and rewrites the rest to `«A1»`; if a SHA or path
    /// lives inside that phrase, every later reference is silently
    /// replaced, and the next `git checkout`/`cat` on it fails. See the
    /// regression test `abbreviator_opt_out_preserves_repeated_identifiers`.
    pub abbreviate: bool,
}

impl Default for InterceptOptions {
    fn default() -> Self {
        Self {
            no_cache: false,
            // Default ON — matches upstream behaviour. Opt out per-call or
            // via SQZ_NO_ABBREV=1 when output contains paste-critical tokens.
            abbreviate: true,
        }
    }
}

impl InterceptOptions {
    /// Build an `InterceptOptions` reflecting the current environment.
    ///
    /// Reads two env vars, both following the same "any non-empty, non-`0`
    /// value is true" rule the rest of sqz uses:
    ///   * `SQZ_NO_DEDUP=1`  → `no_cache = true`
    ///   * `SQZ_NO_ABBREV=1` → `abbreviate = false`
    ///
    /// Everything else keeps its [`Default`] (dedup on, abbreviation on),
    /// so a bare environment yields the historical behaviour unchanged.
    pub fn from_env() -> Self {
        let env_true = |name: &str| match std::env::var(name) {
            Ok(v) => !v.is_empty() && v != "0",
            Err(_) => false,
        };
        Self {
            no_cache: env_true("SQZ_NO_DEDUP"),
            abbreviate: !env_true("SQZ_NO_ABBREV"),
        }
    }
}

/// Suppress informational stderr lines when `SQZ_QUIET=1`.
/// Checked per print site rather than cached: replay flips the env var
/// at runtime, and these paths are nowhere near hot enough to care.
fn quiet() -> bool {
    sqz_engine::SqzEngine::quiet_stderr()
}

pub struct CliProxy {
    engine: SqzEngine,
    /// In-memory L1 dedup cache (fast hash → seen).
    /// On miss, falls through to the persistent CacheManager (L2).
    l1_cache: std::cell::RefCell<HashSet<u64>>,
    /// Dependency mapper for predictive pre-caching (in-memory, rebuilt per session).
    dep_mapper: std::cell::RefCell<DependencyMapper>,
    /// Session-level n-gram abbreviator for recurring phrase compression.
    /// Only applied when `InterceptOptions::abbreviate` is set (the default).
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
        self.intercept_output_with_options(cmd, output, InterceptOptions::default())
    }

    /// Variant of [`intercept_output`] that lets the caller disable the
    /// dedup cache on a per-call basis.
    ///
    /// Added after SquireNed reported on the Synthetic discord that
    /// GLM 5.1 hits a thrash loop when served a `§ref:…§` token it
    /// can't parse. `InterceptOptions { no_cache: true }` skips both
    /// L1 and L2 dedup lookups so the agent gets the full compressed
    /// output every time — strictly more tokens, strictly less
    /// ambiguous. Callers still benefit from per-command formatters
    /// and the compression pipeline; this flag only disables the
    /// 13-token `§ref§` shortcut.
    ///
    /// The flag is also honoured via the `SQZ_NO_DEDUP=1` environment
    /// variable so that shell hooks and agent tools can flip it
    /// without any plumbing change. See `cmd_compress` in main.rs for
    /// the env parsing.
    pub fn intercept_output_with_options(
        &self,
        cmd: &str,
        output: &str,
        opts: InterceptOptions,
    ) -> String {
        // Always track file reads for cross-command context refs,
        // even if the content is a dedup hit (the file is still "known").
        self.track_file(cmd, output);

        // Advance the turn counter for compaction-aware dedup.
        // Each intercept_output call represents one LLM interaction turn.
        self.engine.cache_manager().advance_turn();

        // Step 1: L1 in-memory dedup check (fast path)
        let fast_hash = content_hash(output);
        if !opts.no_cache && self.l1_cache.borrow().contains(&fast_hash) {
            // L1 hit — check L2 persistent cache for the actual ref
            if let Ok(Some(hit)) = self.engine.cache_manager().check_dedup_with_meta(output.as_bytes()) {
                if !quiet() {
                    eprintln!("[sqz] dedup hit: {} (L1+L2)", hit.inline_ref);
                }
                self.maybe_log_rerun_regret(cmd, &hit);
                self.log_dedup_hit(cmd, output);
                return hit.inline_ref;
            }
        }

        // Step 2: L2 persistent SHA-256 dedup check (survives restarts)
        if !opts.no_cache {
            if let Ok(Some(hit)) = self.engine.cache_manager().check_dedup_with_meta(output.as_bytes()) {
                // Promote to L1 for faster future lookups
                self.l1_cache.borrow_mut().insert(fast_hash);
                if !quiet() {
                    eprintln!("[sqz] dedup hit: {} (L2)", hit.inline_ref);
                }
                self.maybe_log_rerun_regret(cmd, &hit);
                self.log_dedup_hit(cmd, output);
                return hit.inline_ref;
            }
        }

        // Step 2b: a line range of a file the model already has in full
        // (`sed -n '40,80p' f`, `head -50 f` after `cat f`).
        if !opts.no_cache {
            if let Ok(Some(hit)) = self.engine.cache_manager().check_slice_with_meta(output.as_bytes()) {
                if !quiet() {
                    eprintln!(
                        "[sqz] slice ref: lines {}-{} of {} already in context",
                        hit.start_line, hit.end_line, hit.total_lines
                    );
                }
                self.log_slice_hit(output, &hit);
                return hit.inline_ref;
            }
        }

        // Step 3: Try per-command formatter. Same net-win gate as the
        // pipeline path: the stats header costs the agent ~12 tokens, so a
        // formatter that trims a few blank lines is a net loss.
        const NET_WIN_MIN_TOKENS: u32 = 16;
        if let Some(formatted) = format_command(cmd, output) {
            let tokens_original = (output.len() as u32 + 3) / 4;
            let tokens_compressed = (formatted.len() as u32 + 3) / 4;
            if tokens_original.saturating_sub(tokens_compressed) >= NET_WIN_MIN_TOKENS {
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
                let truncated = compressed
                    .stages_applied
                    .iter()
                    .any(|s| s == "entropy_truncate");
                // Persist to L2 cache — skip if content was routed to Safe mode
                // (may contain secrets, API keys, passwords)
                let mode = self.engine.route_compression_mode(output);
                let stored = if mode != sqz_engine::CompressionMode::Safe {
                    self.engine
                        .cache_manager()
                        .store_compressed(output.as_bytes(), &compressed)
                        .is_ok()
                } else {
                    false
                };
                self.l1_cache.borrow_mut().insert(fast_hash);

                // N-gram abbreviation (opt-out, default ON).
                //
                // The abbreviator replaces every-occurrence-after-the-first of a
                // repeated multi-word phrase with a «A1» symbol + a legend. This
                // saves tokens on genuinely repetitive prose, but it is LOSSY for
                // identifiers: when a SHA, path, or URL lives inside the repeated
                // phrase (build logs, lint sweeps, `git show $SHA:path` fan-outs),
                // every reference after the first becomes «A1». An agent that then
                // copy-pastes from a later line gets the symbol, not the value —
                // and the next `git checkout «A1»` / `cat «A1»` fails silently.
                //
                // We keep the historical default (abbreviate = true) so behaviour
                // is unchanged out of the box, but callers can opt out per-call
                // (`InterceptOptions { abbreviate: false, .. }`), via the
                // `--no-abbrev` CLI flag, or via `SQZ_NO_ABBREV=1` in the shell
                // hook environment. See the regression tests
                // `abbreviator_opt_out_preserves_repeated_identifiers` and
                // `abbreviator_default_on_still_abbreviates` below.
                let is_source = compressed.stages_applied.iter().any(|s| s == "source_code");
                let abbreviated = if opts.abbreviate && !is_source {
                    let mut abbr = self.abbreviator.borrow_mut();
                    abbr.observe(&compressed.data);
                    match abbr.abbreviate(&compressed.data) {
                        Ok(result) if result.total_tokens_saved > 0 => {
                            if !quiet() {
                                eprintln!(
                                    "[sqz] n-gram abbreviation: {} tokens saved",
                                    result.total_tokens_saved
                                );
                            }
                            result.text
                        }
                        _ => compressed.data,
                    }
                } else {
                    compressed.data
                };

                // Spill ref: entropy truncation drops whole segments, but
                // the untruncated original is sitting in the cache we just
                // wrote. Tell the agent how to get it back, so truncation
                // is a preview rather than a loss. Costs ~10 tokens; only
                // added when the original was actually stored (a ref that
                // can't resolve would be worse than nothing).
                let abbreviated = if stored && truncated {
                    let hash = sqz_engine::CacheManager::sha256_hex(output.as_bytes());
                    format!("{abbreviated}\n[full output: sqz expand {}]", &hash[..16])
                } else {
                    abbreviated
                };

                // Net-win gate: the stats header itself costs context tokens
                // in the hook pipeline, and marker-laden output that saves
                // only a handful of tokens is a bad trade. Judged on the
                // final output (post-abbreviation); below the threshold, the
                // original passes through and no savings are recorded. The
                // L2 store above still happens, so a repeat of this content
                // resolves to a dedup ref.
                let tokens_final = self.engine.count_tokens(&abbreviated);
                if tokens_original.saturating_sub(tokens_final) < NET_WIN_MIN_TOKENS {
                    return self.apply_context_refs(output);
                }

                self.log_compression(cmd, tokens_original, tokens_final.min(tokens_compressed));
                self.apply_context_refs(&abbreviated)
            }
            Err(e) => {
                eprintln!("[sqz] fallback: compression error for command '{cmd}': {e}");
                output.to_owned()
            }
        }
    }

    /// Record a rerun-regret signal when a dedup hit lands within the
    /// regret window of the content's previous sighting. The agent just
    /// re-produced byte-identical output it already had — the repeat
    /// bought nothing, which usually means the compressed first serving
    /// was missing something. Window defaults to 120s; override with
    /// `SQZ_REGRET_WINDOW_SECS` (0 disables).
    fn maybe_log_rerun_regret(&self, cmd: &str, hit: &sqz_engine::DedupHit) {
        const DEFAULT_WINDOW_SECS: f64 = 120.0;
        let window = std::env::var("SQZ_REGRET_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(DEFAULT_WINDOW_SECS);
        if window <= 0.0 {
            return;
        }
        let Some(prev) = hit.prev_accessed_at else { return };
        let gap = (chrono::Utc::now() - prev).num_milliseconds() as f64 / 1000.0;
        if gap >= 0.0 && gap <= window {
            let _ = self
                .engine
                .session_store()
                .log_regret("rerun", cmd, &hit.hash, gap);
        }
    }

    /// Log compression stats to stderr.
    fn log_compression(&self, cmd: &str, original: u32, compressed: u32) {
        let saved = original.saturating_sub(compressed);
        let pct = if original > 0 { (saved as f64 / original as f64 * 100.0) as u32 } else { 0 };
        if !quiet() {
            eprintln!("[sqz] {}/{} tokens ({}% reduction) [{}]", compressed, original, pct, cmd);
        }
        let project = std::env::current_dir().ok();
        let project_str = project.as_ref().map(|p| p.to_string_lossy().to_string());
        let _ = self.engine.session_store().log_compression_with_project(
            original, compressed, &[], cmd,
            project_str.as_deref(),
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
        const DEDUP_REF_TOKENS: u32 = 13;
        let project = std::env::current_dir().ok();
        let project_str = project.as_ref().map(|p| p.to_string_lossy().to_string());
        let _ = self.engine.session_store().log_compression_with_project(
            tokens_original,
            DEDUP_REF_TOKENS,
            &["dedup".to_string()],
            "dedup",
            project_str.as_deref(),
        );
    }

    fn log_slice_hit(&self, output: &str, hit: &sqz_engine::SliceHit) {
        let tokens_original = (output.len() as u32 + 3) / 4;
        let project = std::env::current_dir().ok();
        let project_str = project.as_ref().map(|p| p.to_string_lossy().to_string());
        let _ = self.engine.session_store().log_compression_with_project(
            tokens_original,
            hit.token_cost,
            &["slice".to_string()],
            "slice",
            project_str.as_deref(),
        );
    }

    /// Internal: run `output` through the engine pipeline.
    /// Uses adaptive compression: escalates to aggressive mode when
    /// session token pressure is high (research-backed: ACC-RAG, ACON).
    fn compress_output(
        &self,
        _cmd: &str,
        output: &str,
    ) -> sqz_engine::Result<CompressedContent> {
        // Adaptive pressure: check how many tokens have been injected
        // in the last 30 minutes. If above threshold, compress harder.
        let pressure = self.engine.session_store()
            .session_pressure(30)
            .unwrap_or(0);

        // Thresholds based on typical 200k context windows:
        // >80k tokens in 30min = high pressure → aggressive mode
        // >120k tokens in 30min = critical → aggressive mode
        if pressure > 80_000 {
            if !quiet() {
                eprintln!("[sqz] adaptive: high session pressure ({} tokens/30min), escalating compression", pressure);
            }
            self.engine.compress_with_mode(output, sqz_engine::CompressionMode::Aggressive)
        } else {
            self.engine.compress(output)
        }
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

        if precached > 0 && !quiet() {
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

    /// A `CliProxy` backed by an isolated temp-dir session DB. Tests that share
    /// the default `~/.sqz/sessions.db` race and corrupt it under parallel
    /// `cargo test` (the CI flake on #33). The returned `TempDir` must be kept
    /// alive for the duration of the test.
    fn isolated_proxy() -> (CliProxy, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("sessions.db");
        let engine = SqzEngine::with_preset_and_store(sqz_engine::Preset::default(), &store)
            .expect("engine init");
        (CliProxy::with_engine(engine), dir)
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
        let (proxy, _dir) = isolated_proxy();
        let output = "hello world\nsome output\n";
        let result = proxy.intercept_output("echo", output);
        // Result must be non-empty (either compressed or original fallback).
        assert!(!result.is_empty());
    }

    #[test]
    fn test_intercept_output_fallback_on_empty() {
        let (proxy, _dir) = isolated_proxy();
        // Empty input should not panic and should return something.
        let result = proxy.intercept_output("git", "");
        // Empty input may compress to empty — just ensure no panic.
        let _ = result;
    }

    #[test]
    fn test_dedup_cache_returns_ref_on_second_call() {
        let (proxy, _dir) = isolated_proxy();
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
        let (proxy, _dir) = isolated_proxy();
        let content = "use std::io;\nfn main() {}\n";
        proxy.intercept_output("cat src/main.rs", content);
        // File should be persisted in the session store
        let known = proxy.engine.session_store().known_files().unwrap();
        assert!(known.contains(&"src/main.rs".to_string()), "cat should track the file path");
    }

    #[test]
    fn truncated_output_carries_spill_ref() {
        let (proxy, _dir) = isolated_proxy();
        // Entropy-truncation fixture (same shape as the entropy_truncator
        // unit tests): low-entropy repeat-heavy segments get dropped,
        // high-entropy prose survives. Unique tag defeats cross-test dedup.
        let unique_tag = format!("spill-{}-{:?}", std::process::id(), std::time::SystemTime::now());
        let mut input = format!("run marker {unique_tag}\n\n");
        for i in 0..12u8 {
            // Distinct letter per segment so template/dedup stages don't
            // collapse them before the entropy truncator sees the text.
            let word: String = std::iter::repeat((b'a' + i) as char).take(10).collect();
            input.push_str(&format!("{} filler-{i}\n\n", format!("{word} ").repeat(30)));
        }
        input.push_str("The quick brown fox jumps over the lazy dog by silver rivers today.\n\n");
        input.push_str("Zephyrs quickly vex jumbled gnomes while quartz dwarfs mix pyjamas.\n");

        let result = proxy.intercept_output("mytool", &input);
        assert!(
            result.contains("[full output: sqz expand "),
            "expected spill ref hint in truncated output, got: {result}"
        );

        // The hinted prefix must expand back to the exact original bytes.
        let prefix_start = result.find("[full output: sqz expand ").unwrap()
            + "[full output: sqz expand ".len();
        let prefix: String = result[prefix_start..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        assert_eq!(prefix.len(), 16, "hint must carry a 16-char hash prefix");
        match proxy.engine.cache_manager().expand_prefix(&prefix).unwrap() {
            Some(sqz_engine::ExpandResult::Original { bytes, .. }) => {
                assert_eq!(bytes, input.as_bytes(), "expand must recover the exact original");
            }
            other => panic!("expected Original expansion, got {other:?}"),
        }
    }

    #[test]
    fn quick_rerun_logs_a_regret_signal() {
        let (proxy, _dir) = isolated_proxy();
        let unique_tag = format!(
            "regret-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now(),
        );
        let output = format!(
            "regret test content with {}\n{}",
            unique_tag,
            "line of output long enough to be stored in the dedup cache\n".repeat(5),
        );

        let store = proxy.engine.session_store();
        assert_eq!(store.regret_stats().unwrap().reruns, 0);

        // First sighting: compressed and cached. No regret.
        proxy.intercept_output("git log", &output);
        assert_eq!(store.regret_stats().unwrap().reruns, 0);

        // Identical output seconds later: dedup hit inside the 120s
        // regret window — one rerun signal, attributed to the command.
        let second = proxy.intercept_output("git log", &output);
        assert!(second.starts_with("§ref:"), "expected dedup ref, got: {second}");
        let stats = store.regret_stats().unwrap();
        assert_eq!(stats.reruns, 1, "quick rerun must log exactly one regret");
        assert!(stats.avg_rerun_gap_secs < 120.0);
        let top = store.regret_by_command(5).unwrap();
        assert_eq!(top[0].0, "git log");
    }

    #[test]
    fn test_context_refs_annotate_known_files() {
        let (proxy, _dir) = isolated_proxy();
        // Simulate reading a file (persists to session store)
        let _ = proxy.engine.session_store().add_known_file("src/auth.rs");
        // Error output referencing that file
        let error = "error[E0308]: mismatched types\n --> src/auth.rs:42:5\n";
        let result = proxy.apply_context_refs(error);
        assert!(result.contains("[in context]"), "should annotate known file: {}", result);
    }

    #[test]
    fn test_context_refs_no_annotation_for_unknown_files() {
        let (proxy, _dir) = isolated_proxy();
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
        let (proxy, _dir) = isolated_proxy();
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
        let (proxy, _dir) = isolated_proxy();
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
        let (proxy, _dir) = isolated_proxy();
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
        let (proxy, _dir) = isolated_proxy();
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

    // ── n-gram abbreviator: opt-out behaviour ─────────────────────────────
    //
    // The abbreviator replaces repeated multi-word phrases with «A1» symbols,
    // keeping only the first occurrence intact. When a SHA or path lives
    // inside the repeated phrase, every later line loses it — an agent
    // copy-pasting from a later line gets «A1» instead of the real value,
    // failing the next command (`git checkout «A1»`, `cat «A1»`).
    //
    // Abbreviation is ON by default (matches upstream). These two tests pin
    // both halves of the contract: the opt-out path is lossless, and the
    // default path still abbreviates. Mirrors the shell repro at
    // /tmp/sqzrepro/repro.sh.

    /// Build a unique 40-hex "SHA" from pid + nanos. Uniqueness matters:
    /// a fixed SHA could hit a stale §ref:…§ in the shared sessions.db and
    /// make the test pass vacuously — that's how the original regression
    /// nearly slipped through.
    fn unique_sha() -> String {
        let seed = format!(
            "{:016x}{:032x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        let sha: String = seed.chars().filter(|c| c.is_ascii_hexdigit()).take(40).collect();
        assert_eq!(sha.len(), 40, "test seed must yield a 40-char hex SHA");
        sha
    }

    /// With abbreviation opted out (`SQZ_NO_ABBREV` equivalent), a SHA that
    /// repeats inside an identical phrase must survive on EVERY line — never
    /// collapsed to «A1». This is the fix for the corruption bug.
    /// Net-win gate: output whose compression saves fewer than the
    /// threshold tokens passes through verbatim — no markers, no header
    /// worth of savings eaten by the rewrite.
    #[test]
    fn net_win_gate_passes_small_savings_through() {
        let (proxy, _dir) = isolated_proxy();
        let tag = unique_sha();
        // Short unique prose: nothing here compresses meaningfully.
        let output = format!("one unique diagnostic line mentioning {tag} and nothing else\n");
        let opts = InterceptOptions { no_cache: true, abbreviate: true };
        let result = proxy.intercept_output_with_options("build", &output, opts);
        assert_eq!(result, output, "sub-threshold savings must pass through verbatim");
    }

    #[test]
    fn source_code_is_served_verbatim_and_slices_of_it_become_refs() {
        let (proxy, _dir) = isolated_proxy();
        let tag = unique_sha();
        let file: String = (1..=50)
            .map(|i| format!("def handler_{i:03}(request):\n    token = request.headers.get('Authorization-{tag}')\n    return verify(token, scope='handler_{i:03}')\n\n"))
            .collect();

        let first = proxy.intercept_output_with_options("cat auth.py", &file, InterceptOptions::default());
        assert_eq!(first, file, "source code must not be abbreviated, folded, or truncated");

        let lines: Vec<&str> = file.split_inclusive('\n').collect();
        let slice: String = lines[40..80].concat();
        let second = proxy.intercept_output_with_options("sed -n '41,80p' auth.py", &slice, InterceptOptions::default());
        assert!(second.starts_with("§ref:") && second.ends_with(":L41-80§"), "{second}");

        let no_cache = InterceptOptions { no_cache: true, ..InterceptOptions::default() };
        let third = proxy.intercept_output_with_options("sed -n '41,80p' auth.py", &slice, no_cache);
        assert_eq!(third, slice);
    }

    #[test]
    fn abbreviator_opt_out_preserves_repeated_identifiers() {
        let (proxy, _dir) = isolated_proxy();
        let sha = unique_sha();

        let output = format!(
            "Resolving dependencies for revision {sha} in module-a\n\
             Resolving dependencies for revision {sha} in module-b\n\
             Resolving dependencies for revision {sha} in module-c\n\
             Resolving dependencies for revision {sha} in module-d\n"
        );

        let opts = InterceptOptions { no_cache: false, abbreviate: false };
        let result = proxy.intercept_output_with_options("build", &output, opts);

        // Unique content ⇒ first intercept is a cache MISS, so no §ref.
        assert!(
            !result.starts_with("§ref:"),
            "unexpected dedup ref for unique content — test guard broken:\n{result}"
        );
        // No abbreviation markers when opted out.
        assert!(
            !result.contains("«A"),
            "abbreviation symbol leaked despite opt-out:\n{result}"
        );
        assert!(
            !result.contains("[Abbreviations]"),
            "abbreviation legend present despite opt-out:\n{result}"
        );
        // The SHA survives on every line, not just the first.
        let sha_count = result.matches(sha.as_str()).count();
        assert!(
            sha_count >= 4,
            "SHA must appear on all 4 lines, found {sha_count}:\n{result}"
        );
    }

    /// The default (abbreviation ON) preserves upstream behaviour: a phrase
    /// repeated enough times still gets collapsed to a «A1» legend. This
    /// guards against the opt-out plumbing accidentally disabling
    /// abbreviation for everyone.
    #[test]
    fn abbreviator_default_on_still_abbreviates() {
        let (proxy, _dir) = isolated_proxy();
        // Unique tag keeps this a cache miss; the repeated long phrase is
        // what the abbreviator should fold.
        let tag = unique_sha();
        let phrase = format!("recurring diagnostic phrase with extra words {tag} marker");
        // Enough repetitions that abbreviation clears the net-win gate.
        let output: String = (0..12)
            .map(|i| format!("{phrase} occurrence number {i}\n"))
            .collect();

        // Explicit default — abbreviation enabled.
        let opts = InterceptOptions::default();
        assert!(opts.abbreviate, "abbreviate must default to true");
        let result = proxy.intercept_output_with_options("build", &output, opts);

        // A dedup ref would be a (lossless) correct answer too — only assert
        // the abbreviation contract when we actually got compressed text.
        if result.starts_with("§ref:") {
            return;
        }
        assert!(
            result.contains("[Abbreviations]") && result.contains("«A"),
            "default path should still abbreviate a 4×-repeated phrase:\n{result}"
        );
    }

    #[test]
    fn test_ls_output_preserves_all_filenames_through_rle() {
        // Reddit repro end-to-end. When the output is new (cache miss), the
        // pipeline must preserve every filename. When it's a cache hit, the
        // §ref:...§ response is a correct compression.
        let (proxy, _dir) = isolated_proxy();
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
