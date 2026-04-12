use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::error::{Result, SqzError};

/// Environment variable names for credential passthrough.
/// These are inherited from the parent process so that sandbox code
/// can use authenticated CLIs (gh, aws, gcloud, kubectl, docker)
/// without exposing credentials to the conversation context.
const CREDENTIAL_ENV_PREFIXES: &[&str] = &[
    // AWS
    "AWS_",
    // Google Cloud
    "GCLOUD_",
    "GOOGLE_",
    "CLOUDSDK_",
    // GitHub CLI
    "GH_",
    "GITHUB_",
    // Kubernetes
    "KUBECONFIG",
    // Docker
    "DOCKER_",
    // General
    "HOME",
    "PATH",
    "USER",
    "LANG",
    "TERM",
    "SHELL",
    "TMPDIR",
    "XDG_",
];

/// A detected runtime with its binary path.
#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    pub name: &'static str,
    pub binary: String,
    pub language: &'static str,
}

/// Result of a sandbox execution.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// Captured stdout (the only thing that enters the context window).
    pub stdout: String,
    /// Process exit code.
    pub exit_code: i32,
    /// Whether output was truncated due to max_output_bytes.
    pub truncated: bool,
    /// True if output was indexed into FTS5 due to size + intent filtering.
    pub indexed: bool,
}

/// Threshold in bytes above which intent-driven filtering kicks in.
const OUTPUT_FILTER_THRESHOLD: usize = 5 * 1024; // 5 KB

/// Result of intent-driven output filtering via FTS5 BM25 search.
#[derive(Debug, Clone)]
pub struct FilteredOutput {
    /// BM25-matched sections from the original output.
    pub matched_sections: Vec<String>,
    /// Vocabulary of searchable terms for follow-up queries.
    pub vocabulary: Vec<String>,
    /// Total number of chunks the output was split into.
    pub total_chunks: usize,
    /// Number of chunks that matched the intent.
    pub matched_chunks: usize,
}

/// Executes code in isolated subprocesses.
///
/// Only stdout enters the context window — stderr, file system side effects,
/// and environment variables never leak into the LLM context.
pub struct SandboxExecutor {
    timeout: Duration,
    max_output_bytes: usize,
    runtimes: HashMap<String, RuntimeInfo>,
}

// ── OutputFilter ──────────────────────────────────────────────────────────────

/// Indexes large text output into an in-memory FTS5 table and returns
/// BM25-matched sections plus a vocabulary of searchable terms.
pub(crate) struct OutputFilter;

impl OutputFilter {
    /// Chunk `text` by double-newline paragraphs (or every ~512 bytes for
    /// long runs without blank lines), index into FTS5, and return the
    /// BM25-matched sections for `intent`.
    pub fn filter(text: &str, intent: &str) -> Result<FilteredOutput> {
        let chunks = Self::chunk_output(text);
        let total_chunks = chunks.len();

        let conn = Connection::open_in_memory()
            .map_err(|e| SqzError::Other(format!("FTS5 in-memory open failed: {e}")))?;

        conn.execute_batch(
            r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS sandbox_fts USING fts5(
                chunk_id,
                body,
                tokenize='porter ascii'
            );
            "#,
        )
        .map_err(|e| SqzError::Other(format!("FTS5 schema creation failed: {e}")))?;

        // Insert chunks
        for (i, chunk) in chunks.iter().enumerate() {
            conn.execute(
                "INSERT INTO sandbox_fts(chunk_id, body) VALUES (?1, ?2)",
                params![i.to_string(), chunk],
            )
            .map_err(|e| SqzError::Other(format!("FTS5 insert failed: {e}")))?;
        }

        // BM25 search
        let matched_sections = Self::bm25_search(&conn, intent, &chunks)?;
        let matched_chunks = matched_sections.len();

        // Extract vocabulary
        let vocabulary = Self::extract_vocabulary(&conn)?;

        Ok(FilteredOutput {
            matched_sections,
            vocabulary,
            total_chunks,
            matched_chunks,
        })
    }

    /// Split output into chunks on double-newline boundaries. If a chunk
    /// exceeds 512 bytes, split it further on single newlines.
    fn chunk_output(text: &str) -> Vec<String> {
        const MAX_CHUNK_BYTES: usize = 512;

        let paragraphs: Vec<&str> = text.split("\n\n").collect();
        let mut chunks = Vec::new();

        for para in paragraphs {
            let trimmed = para.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() <= MAX_CHUNK_BYTES {
                chunks.push(trimmed.to_string());
            } else {
                // Sub-split on single newlines
                let mut current = String::new();
                for line in trimmed.lines() {
                    if !current.is_empty() && current.len() + line.len() + 1 > MAX_CHUNK_BYTES {
                        chunks.push(std::mem::take(&mut current));
                    }
                    if !current.is_empty() {
                        current.push('\n');
                    }
                    current.push_str(line);
                }
                if !current.is_empty() {
                    chunks.push(current);
                }
            }
        }

        // Guarantee at least one chunk even for empty-ish input
        if chunks.is_empty() && !text.trim().is_empty() {
            chunks.push(text.trim().to_string());
        }

        chunks
    }

    /// Query the FTS5 table with the intent and return matching chunk bodies
    /// ranked by BM25.
    fn bm25_search(conn: &Connection, intent: &str, _chunks: &[String]) -> Result<Vec<String>> {
        // Sanitize intent for FTS5 query: keep alphanumeric and spaces
        let sanitized: String = intent
            .chars()
            .map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' })
            .collect();
        let terms: Vec<&str> = sanitized.split_whitespace().collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        // Build an OR query so partial matches still return results
        let fts_query = terms.join(" OR ");

        let mut stmt = conn
            .prepare(
                r#"SELECT body FROM sandbox_fts
                   WHERE sandbox_fts MATCH ?1
                   ORDER BY rank
                   LIMIT 20"#,
            )
            .map_err(|e| SqzError::Other(format!("FTS5 query prepare failed: {e}")))?;

        let rows = stmt
            .query_map(params![fts_query], |row| row.get::<_, String>(0))
            .map_err(|e| SqzError::Other(format!("FTS5 query failed: {e}")))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(
                row.map_err(|e| SqzError::Other(format!("FTS5 row read failed: {e}")))?,
            );
        }
        Ok(results)
    }

    /// Extract a vocabulary of distinct searchable terms from the indexed
    /// content. Uses the FTS5 `vocab` virtual table to pull out tokens.
    fn extract_vocabulary(conn: &Connection) -> Result<Vec<String>> {
        // Create a vocab table over the FTS5 index using 'col' detail
        // which gives (term, col, doc, cnt) columns.
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS sandbox_vocab USING fts5vocab(sandbox_fts, col);",
        )
        .map_err(|e| SqzError::Other(format!("FTS5 vocab table creation failed: {e}")))?;

        let mut stmt = conn
            .prepare(
                r#"SELECT term FROM sandbox_vocab
                   WHERE col = 'body'
                   ORDER BY doc DESC
                   LIMIT 100"#,
            )
            .map_err(|e| SqzError::Other(format!("vocab query prepare failed: {e}")))?;

        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| SqzError::Other(format!("vocab query failed: {e}")))?;

        let mut vocab = Vec::new();
        for row in rows {
            vocab.push(
                row.map_err(|e| SqzError::Other(format!("vocab row read failed: {e}")))?,
            );
        }
        Ok(vocab)
    }
}

impl SandboxExecutor {
    /// Default timeout: 30 seconds.
    pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
    /// Default max output: 1 MB.
    pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1_048_576;

    /// Create a new executor, auto-detecting available runtimes.
    pub fn new() -> Self {
        Self::with_config(
            Duration::from_secs(Self::DEFAULT_TIMEOUT_SECS),
            Self::DEFAULT_MAX_OUTPUT_BYTES,
        )
    }

    /// Create with custom timeout and max output size.
    pub fn with_config(timeout: Duration, max_output_bytes: usize) -> Self {
        let runtimes = detect_runtimes();
        Self {
            timeout,
            max_output_bytes,
            runtimes,
        }
    }

    /// Execute code in the given language runtime.
    ///
    /// Only stdout is captured and returned. Stderr is discarded.
    /// Credentials for gh, aws, gcloud, kubectl, docker are passed through
    /// via environment variable inheritance.
    pub fn execute(&self, code: &str, language: &str) -> Result<SandboxResult> {
        let lang = language.to_lowercase();
        let runtime = self
            .runtimes
            .get(&lang)
            .ok_or_else(|| SqzError::Other(format!("unsupported or unavailable runtime: {lang}")))?;

        let env = build_credential_env();

        let result = match lang.as_str() {
            "go" => self.execute_go(code, runtime, &env),
            "rust" => self.execute_rust(code, runtime, &env),
            _ => self.execute_interpreted(code, runtime, &env),
        }?;

        Ok(result)
    }

    /// Execute code and, when stdout exceeds 5 KB and `intent` is provided,
    /// index the full output into an in-memory FTS5 table and return only
    /// BM25-matched sections plus a vocabulary of searchable terms.
    ///
    /// When the output is small or no intent is given, behaves identically
    /// to [`execute`] (returns full stdout, `filtered` is `None`).
    pub fn execute_with_intent(
        &self,
        code: &str,
        language: &str,
        intent: Option<&str>,
    ) -> Result<(SandboxResult, Option<FilteredOutput>)> {
        let mut result = self.execute(code, language)?;

        let should_filter = result.stdout.len() > OUTPUT_FILTER_THRESHOLD
            && intent.map_or(false, |i| !i.trim().is_empty());

        if should_filter {
            let intent_str = intent.unwrap(); // safe: checked above
            let filtered = OutputFilter::filter(&result.stdout, intent_str)?;
            result.indexed = true;
            // Replace stdout with only the matched sections so the LLM
            // context window receives the filtered view.
            result.stdout = filtered.matched_sections.join("\n\n");
            Ok((result, Some(filtered)))
        } else {
            Ok((result, None))
        }
    }

    /// Languages that this executor currently supports (only those detected on this system).
    pub fn available_languages(&self) -> Vec<&str> {
        self.runtimes.values().map(|r| r.language).collect()
    }

    /// All languages the executor can potentially support.
    pub fn supported_languages(&self) -> &[&str] {
        &["js", "ts", "python", "shell", "ruby", "go", "rust"]
    }

    /// Check whether a specific language runtime is available.
    pub fn is_available(&self, language: &str) -> bool {
        self.runtimes.contains_key(&language.to_lowercase())
    }

    /// Current timeout setting.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Current max output size in bytes.
    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    // ── Private helpers ───────────────────────────────────────────────────

    /// Execute an interpreted language (JS, TS, Python, Shell, Ruby) by
    /// writing code to a temp file and invoking the runtime binary.
    fn execute_interpreted(
        &self,
        code: &str,
        runtime: &RuntimeInfo,
        env: &HashMap<String, String>,
    ) -> Result<SandboxResult> {
        let ext = match runtime.language {
            "js" => "js",
            "ts" => "ts",
            "python" => "py",
            "shell" => "sh",
            "ruby" => "rb",
            _ => "tmp",
        };

        let tmp_dir = tempfile::tempdir().map_err(|e| SqzError::Io(e))?;
        let script_path = tmp_dir.path().join(format!("sandbox_script.{ext}"));
        {
            let mut f = std::fs::File::create(&script_path)?;
            f.write_all(code.as_bytes())?;
        }

        let mut cmd = Command::new(&runtime.binary);

        // Special case: TypeScript via npx needs `tsx` as the first argument
        if runtime.language == "ts" && runtime.name == "npx" {
            cmd.arg("tsx");
        }

        cmd.arg(&script_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // stderr never enters context
            .envs(env);

        self.run_with_timeout(cmd)
    }

    /// Execute Go code: write to temp file, run with `go run`.
    fn execute_go(
        &self,
        code: &str,
        runtime: &RuntimeInfo,
        env: &HashMap<String, String>,
    ) -> Result<SandboxResult> {
        let tmp_dir = tempfile::tempdir()?;
        let script_path = tmp_dir.path().join("main.go");
        {
            let mut f = std::fs::File::create(&script_path)?;
            f.write_all(code.as_bytes())?;
        }

        let mut cmd = Command::new(&runtime.binary);
        cmd.arg("run")
            .arg(&script_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .envs(env);

        self.run_with_timeout(cmd)
    }

    /// Execute Rust code: write to temp file, compile with rustc, then run.
    fn execute_rust(
        &self,
        code: &str,
        runtime: &RuntimeInfo,
        env: &HashMap<String, String>,
    ) -> Result<SandboxResult> {
        let tmp_dir = tempfile::tempdir()?;
        let src_path = tmp_dir.path().join("sandbox.rs");
        let bin_path = tmp_dir.path().join("sandbox_bin");
        {
            let mut f = std::fs::File::create(&src_path)?;
            f.write_all(code.as_bytes())?;
        }

        // Compile
        let compile = Command::new(&runtime.binary)
            .arg(&src_path)
            .arg("-o")
            .arg(&bin_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .envs(env)
            .status();

        match compile {
            Ok(status) if status.success() => {}
            Ok(status) => {
                return Ok(SandboxResult {
                    stdout: String::new(),
                    exit_code: status.code().unwrap_or(1),
                    truncated: false,
                    indexed: false,
                });
            }
            Err(e) => return Err(SqzError::Io(e)),
        }

        // Run the compiled binary
        let mut cmd = Command::new(&bin_path);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::null())
            .envs(env);

        self.run_with_timeout(cmd)
    }

    /// Spawn the command, enforce timeout, capture stdout, and truncate if needed.
    fn run_with_timeout(&self, mut cmd: Command) -> Result<SandboxResult> {
        let mut child = cmd.spawn().map_err(SqzError::Io)?;

        // Wait with timeout
        let status = match wait_with_timeout(&mut child, self.timeout) {
            Ok(status) => status,
            Err(_) => {
                // Timeout — kill the process
                let _ = child.kill();
                let _ = child.wait();
                return Err(SqzError::Other(format!(
                    "sandbox execution timed out after {}s",
                    self.timeout.as_secs()
                )));
            }
        };

        // Read stdout
        let stdout_raw = if let Some(mut stdout) = child.stdout.take() {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        } else {
            Vec::new()
        };

        // Truncate if needed
        let truncated = stdout_raw.len() > self.max_output_bytes;
        let stdout_bytes = if truncated {
            &stdout_raw[..self.max_output_bytes]
        } else {
            &stdout_raw[..]
        };

        let stdout = String::from_utf8_lossy(stdout_bytes).into_owned();

        Ok(SandboxResult {
            stdout,
            exit_code: status.code().unwrap_or(-1),
            truncated,
            indexed: false,
        })
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Wait for a child process with a timeout. Returns the exit status on success,
/// or an error if the timeout is exceeded.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> std::result::Result<std::process::ExitStatus, ()> {
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(50);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return Err(());
                }
                std::thread::sleep(poll_interval);
            }
            Err(_) => return Err(()),
        }
    }
}

/// Build an environment map containing only credential-related variables
/// from the current process environment.
fn build_credential_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    for (key, value) in std::env::vars() {
        if CREDENTIAL_ENV_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
        {
            env.insert(key, value);
        }
    }
    env
}

/// Probe the system for available runtimes.
fn detect_runtimes() -> HashMap<String, RuntimeInfo> {
    let mut runtimes = HashMap::new();

    let candidates: &[(&str, &[&str], &str)] = &[
        // (language key, [binary candidates], language label)
        ("js", &["node", "bun"], "js"),
        ("ts", &["bun", "npx"], "ts"),
        ("python", &["python3", "python"], "python"),
        ("shell", &["bash", "sh"], "shell"),
        ("ruby", &["ruby"], "ruby"),
        ("go", &["go"], "go"),
        ("rust", &["rustc"], "rust"),
    ];

    for &(lang_key, binaries, lang_label) in candidates {
        for &bin in binaries {
            if is_binary_available(bin) {
                // For ts via npx, we use `npx tsx` as the actual command
                let effective_binary = if lang_key == "ts" && bin == "npx" {
                    "npx".to_string()
                } else {
                    bin.to_string()
                };

                runtimes.insert(
                    lang_key.to_string(),
                    RuntimeInfo {
                        name: bin,
                        binary: effective_binary,
                        language: lang_label,
                    },
                );
                break; // use first available binary
            }
        }
    }

    runtimes
}

/// Check if a binary is available on PATH.
fn is_binary_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_detects_runtimes() {
        let executor = SandboxExecutor::new();
        // At minimum, shell (bash/sh) should be available on any Unix system
        assert!(
            !executor.runtimes.is_empty(),
            "should detect at least one runtime"
        );
    }

    #[test]
    fn test_supported_languages_list() {
        let executor = SandboxExecutor::new();
        let supported = executor.supported_languages();
        assert!(supported.len() >= 6, "should list at least 6 supported languages");
        assert!(supported.contains(&"js"));
        assert!(supported.contains(&"python"));
        assert!(supported.contains(&"shell"));
        assert!(supported.contains(&"ruby"));
        assert!(supported.contains(&"go"));
        assert!(supported.contains(&"rust"));
    }

    #[test]
    fn test_default_config() {
        let executor = SandboxExecutor::new();
        assert_eq!(executor.timeout(), Duration::from_secs(30));
        assert_eq!(executor.max_output_bytes(), 1_048_576);
    }

    #[test]
    fn test_custom_config() {
        let executor = SandboxExecutor::with_config(Duration::from_secs(10), 4096);
        assert_eq!(executor.timeout(), Duration::from_secs(10));
        assert_eq!(executor.max_output_bytes(), 4096);
    }

    #[test]
    fn test_execute_shell_echo() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("shell") {
            return; // skip if no shell
        }
        let result = executor.execute("echo hello sandbox", "shell").unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello sandbox");
        assert!(!result.truncated);
    }

    #[test]
    fn test_execute_shell_captures_only_stdout() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("shell") {
            return;
        }
        // Write to both stdout and stderr — only stdout should appear
        let code = r#"echo "visible"
echo "hidden" >&2
echo "also visible""#;
        let result = executor.execute(code, "shell").unwrap();
        assert!(result.stdout.contains("visible"));
        assert!(result.stdout.contains("also visible"));
        assert!(!result.stdout.contains("hidden"));
    }

    #[test]
    fn test_execute_python() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("python") {
            return;
        }
        let result = executor.execute("print('hello from python')", "python").unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello from python");
    }

    #[test]
    fn test_execute_nonzero_exit() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("shell") {
            return;
        }
        let result = executor.execute("exit 42", "shell").unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn test_execute_timeout() {
        let executor = SandboxExecutor::with_config(Duration::from_secs(1), 1024);
        if !executor.is_available("shell") {
            return;
        }
        let result = executor.execute("sleep 30", "shell");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("timed out"));
    }

    #[test]
    fn test_execute_output_truncation() {
        let executor = SandboxExecutor::with_config(Duration::from_secs(10), 32);
        if !executor.is_available("shell") {
            return;
        }
        // Generate output larger than 32 bytes
        let result = executor
            .execute("for i in $(seq 1 100); do echo \"line $i\"; done", "shell")
            .unwrap();
        assert!(result.truncated);
        assert!(result.stdout.len() <= 32);
    }

    #[test]
    fn test_unsupported_runtime() {
        let executor = SandboxExecutor::new();
        let result = executor.execute("code", "brainfuck");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("unsupported or unavailable runtime"));
    }

    #[test]
    fn test_case_insensitive_language() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("shell") {
            return;
        }
        let result = executor.execute("echo ok", "Shell");
        assert!(result.is_ok());
    }

    #[test]
    fn test_credential_env_includes_path() {
        let env = build_credential_env();
        assert!(env.contains_key("PATH"), "PATH should be inherited");
    }

    #[test]
    fn test_credential_env_includes_aws() {
        // Temporarily set an AWS var to verify it's picked up
        std::env::set_var("AWS_TEST_SANDBOX", "test_value");
        let env = build_credential_env();
        assert_eq!(env.get("AWS_TEST_SANDBOX").map(|s| s.as_str()), Some("test_value"));
        std::env::remove_var("AWS_TEST_SANDBOX");
    }

    #[test]
    fn test_is_binary_available() {
        // `sh` should always be available on Unix
        assert!(is_binary_available("sh"));
        assert!(!is_binary_available("definitely_not_a_real_binary_xyz"));
    }

    // ── OutputFilter unit tests ───────────────────────────────────────────

    #[test]
    fn test_chunk_output_splits_on_double_newline() {
        let text = "first paragraph\n\nsecond paragraph\n\nthird paragraph";
        let chunks = OutputFilter::chunk_output(text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "first paragraph");
        assert_eq!(chunks[1], "second paragraph");
        assert_eq!(chunks[2], "third paragraph");
    }

    #[test]
    fn test_chunk_output_splits_large_paragraphs() {
        // Build a single paragraph > 512 bytes with many lines
        let line = "a]".repeat(30); // 60 chars per line
        let big_para = (0..20).map(|i| format!("{line} line{i}")).collect::<Vec<_>>().join("\n");
        assert!(big_para.len() > 512);

        let chunks = OutputFilter::chunk_output(&big_para);
        assert!(chunks.len() > 1, "large paragraph should be sub-split");
        for chunk in &chunks {
            assert!(chunk.len() <= 600, "each sub-chunk should be roughly ≤512 bytes");
        }
    }

    #[test]
    fn test_chunk_output_empty_input() {
        let chunks = OutputFilter::chunk_output("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_filter_returns_matching_sections() {
        let text = "error: compilation failed at line 42\n\n\
                    warning: unused variable `x`\n\n\
                    info: build started at 10:00\n\n\
                    error: type mismatch in function foo\n\n\
                    success: 3 tests passed";
        let result = OutputFilter::filter(text, "error compilation").unwrap();
        assert!(!result.matched_sections.is_empty(), "should find error-related chunks");
        // At least one matched section should contain "error"
        assert!(
            result.matched_sections.iter().any(|s| s.contains("error")),
            "matched sections should contain the intent keyword"
        );
        assert!(result.total_chunks >= 4);
    }

    #[test]
    fn test_filter_returns_vocabulary() {
        let text = "the quick brown fox jumps over the lazy dog\n\n\
                    rust programming language is fast and safe\n\n\
                    memory safety without garbage collection";
        let result = OutputFilter::filter(text, "rust").unwrap();
        assert!(!result.vocabulary.is_empty(), "vocabulary should not be empty");
        // Vocabulary should contain stemmed terms from the content
        // (porter stemmer may stem words, so check for presence of some terms)
        let vocab_joined = result.vocabulary.join(" ");
        assert!(
            vocab_joined.contains("rust") || vocab_joined.contains("fast") || vocab_joined.contains("safe"),
            "vocabulary should contain terms from the indexed content"
        );
    }

    #[test]
    fn test_filter_no_match_returns_empty() {
        let text = "hello world\n\nfoo bar baz";
        let result = OutputFilter::filter(text, "zzzznonexistent").unwrap();
        assert!(result.matched_sections.is_empty());
        assert_eq!(result.matched_chunks, 0);
    }

    #[test]
    fn test_filter_special_chars_in_intent() {
        // Intent with special characters should not crash FTS5
        let text = "error: something went wrong\n\nwarning: check this";
        let result = OutputFilter::filter(text, "error: (something) [wrong]");
        assert!(result.is_ok(), "special chars in intent should be sanitized");
    }

    #[test]
    fn test_execute_with_intent_small_output_no_filter() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("shell") {
            return;
        }
        // Small output (< 5KB) should not trigger filtering
        let (result, filtered) = executor
            .execute_with_intent("echo hello", "shell", Some("hello"))
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(!result.indexed);
        assert!(filtered.is_none());
    }

    #[test]
    fn test_execute_with_intent_no_intent_no_filter() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("shell") {
            return;
        }
        // Even large output without intent should not filter
        let code = "for i in $(seq 1 1000); do echo \"line $i: some padding text to make it bigger\"; done";
        let (result, filtered) = executor
            .execute_with_intent(code, "shell", None)
            .unwrap();
        assert!(!result.indexed);
        assert!(filtered.is_none());
    }

    #[test]
    fn test_execute_with_intent_large_output_filters() {
        let executor = SandboxExecutor::new();
        if !executor.is_available("shell") {
            return;
        }
        // Generate > 5KB of output with identifiable sections
        let code = r#"
for i in $(seq 1 50); do echo "error: compilation failed at module $i"; done
echo ""
for i in $(seq 1 50); do echo "info: processing file $i of 200"; done
echo ""
for i in $(seq 1 50); do echo "warning: deprecated API usage in handler $i"; done
echo ""
for i in $(seq 1 50); do echo "success: test suite $i passed with 100% coverage"; done
"#;
        let (result, filtered) = executor
            .execute_with_intent(code, "shell", Some("error compilation"))
            .unwrap();
        assert!(result.indexed, "large output with intent should be indexed");
        let filtered = filtered.expect("should have filtered output");
        assert!(!filtered.matched_sections.is_empty(), "should have matched sections");
        assert!(!filtered.vocabulary.is_empty(), "should have vocabulary");
        assert!(filtered.total_chunks > 0);
    }

    // ── Property-based tests ──────────────────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        /// Generate a random alphanumeric label safe for use in shell echo commands.
        fn safe_label() -> impl Strategy<Value = String> {
            "[a-zA-Z0-9]{1,20}"
        }

        /// **Validates: Requirements 30.1, 30.2**
        ///
        /// Property 35: Sandbox execution isolation — stdout only
        ///
        /// For any code execution that writes to both stdout and stderr,
        /// only stdout content appears in the returned SandboxResult.
        /// We use distinct prefixes to guarantee stdout and stderr
        /// messages are distinguishable.
        proptest! {
            #[test]
            fn prop_only_stdout_captured(
                label in safe_label(),
            ) {
                let executor = SandboxExecutor::new();
                if !executor.is_available("shell") {
                    return Ok(());
                }

                let stdout_msg = format!("OUT_{label}");
                let stderr_msg = format!("ERR_{label}");

                // Script writes distinct messages to stdout and stderr
                let code = format!(
                    "echo \"{stdout_msg}\"\necho \"{stderr_msg}\" >&2"
                );
                let result = executor.execute(&code, "shell").unwrap();

                // stdout content must be present
                prop_assert!(
                    result.stdout.contains(&stdout_msg),
                    "stdout should contain the stdout message '{}', got: '{}'",
                    stdout_msg, result.stdout
                );
                // stderr content must never appear
                prop_assert!(
                    !result.stdout.contains(&stderr_msg),
                    "stdout should NOT contain the stderr message '{}', got: '{}'",
                    stderr_msg, result.stdout
                );
            }
        }

        /// **Validates: Requirements 30.1, 30.2**
        ///
        /// Property 35: Sandbox execution isolation — subprocess isolation
        ///
        /// Each execution runs in an isolated subprocess with no shared
        /// state. Setting an env var in one execution must not be visible
        /// in a subsequent execution.
        proptest! {
            #[test]
            fn prop_no_shared_state_between_executions(
                var_name in "[A-Z]{3,8}",
                var_value in "[a-z0-9]{1,10}",
            ) {
                let executor = SandboxExecutor::new();
                if !executor.is_available("shell") {
                    return Ok(());
                }

                let unique_var = format!("SQZ_PROP_{var_name}");

                // First execution: export an env var
                let code1 = format!(
                    "export {unique_var}={var_value}\necho \"set {unique_var}\""
                );
                let result1 = executor.execute(&code1, "shell").unwrap();
                prop_assert!(
                    result1.stdout.contains(&format!("set {unique_var}")),
                    "first execution should succeed"
                );

                // Second execution: try to read that env var — it should be empty
                let code2 = format!(
                    "echo \"val=${{{unique_var}:-UNSET}}\""
                );
                let result2 = executor.execute(&code2, "shell").unwrap();
                prop_assert!(
                    result2.stdout.contains("val=UNSET"),
                    "env var from first execution should not leak into second; got: '{}'",
                    result2.stdout
                );
            }
        }
    }
}
