/// Confidence-driven compression mode router.
///
/// Analyzes input content and selects the appropriate compression aggressiveness:
/// - High-risk content (stack traces, configs, migrations) → safe preset
/// - Low-entropy repetitive content → aggressive preset
/// - Normal content → default preset
///
/// Based on entropy analysis and content pattern detection.

use crate::entropy_analyzer::EntropyAnalyzer;

/// The compression mode selected by the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMode {
    /// Safe mode: minimal compression, preserve all structure.
    /// Used for stack traces, migrations and credentials.
    Safe,
    /// Default mode: balanced compression.
    Default,
    /// Aggressive mode: maximum compression.
    /// Used for repetitive/boilerplate content with low information density.
    Aggressive,
}

impl CompressionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Default => "default",
            Self::Aggressive => "aggressive",
        }
    }
}

/// Routes content to the appropriate compression mode.
pub struct ConfidenceRouter {
    entropy_analyzer: EntropyAnalyzer,
}

impl Default for ConfidenceRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfidenceRouter {
    pub fn new() -> Self {
        Self {
            entropy_analyzer: EntropyAnalyzer::new(),
        }
    }

    /// Analyze content and return the recommended compression mode.
    pub fn route(&self, content: &str) -> CompressionMode {
        let content = &*without_ansi(content);
        if content.len() < 100 {
            return CompressionMode::Default;
        }

        // Check for high-risk content patterns first (always → Safe)
        if self.is_high_risk(content) {
            return CompressionMode::Safe;
        }

        // Use entropy to distinguish repetitive from normal content
        let blocks = self.entropy_analyzer.analyze(content);
        if blocks.is_empty() {
            return CompressionMode::Default;
        }

        let avg_entropy: f64 = blocks.iter().map(|b| b.entropy).sum::<f64>() / blocks.len() as f64;

        // Very low average entropy → mostly boilerplate → aggressive
        if avg_entropy < 2.5 {
            return CompressionMode::Aggressive;
        }

        // High entropy = information-dense content (code, git log, varied text).
        // This is NOT the same as high-risk. Route to Default, not Safe.
        // Safe mode is only for content that is_high_risk() explicitly identifies.
        CompressionMode::Default
    }

    /// Whether content must stay out of the persistent cache.
    pub fn never_store(&self, content: &str) -> bool {
        let content = &*without_ansi(content);
        content.len() >= 100 && (self.mentions_high_risk(content) || self.is_high_risk(content))
    }

    /// Returns true if content contains high-risk patterns that should
    /// never be aggressively compressed.
    ///
    /// These checks require structural context — not just keyword presence.
    /// A commit message saying "fix: password reset" is NOT a secret.
    /// A stack trace with its frames IS high-risk.
    fn is_high_risk(&self, content: &str) -> bool {
        let lines: Vec<&str> = content.lines().map(str::trim).collect();
        (0..lines.len()).any(|i| is_risk_line(&lines, i))
    }

    fn mentions_high_risk(&self, content: &str) -> bool {
        let lower = content.to_lowercase();

        // Stack traces — require structural markers, not just keywords.
        // "panicked at" alone in a commit message is not a stack trace.
        // A real stack trace has "panicked at" PLUS frame numbers or "stack backtrace".
        if (lower.contains("panicked at") || lower.contains("traceback"))
            && (lower.contains("stack backtrace") || lower.contains("  0:") || lower.contains("  at "))
        {
            return true;
        }
        if lower.contains("stack trace") && lower.contains("  at ") {
            return true;
        }

        // Database migrations — require SQL keywords, not just the word "migration".
        // "fix: migration path" in a commit message is NOT a database migration.
        if (lower.contains("alter table") || lower.contains("create table") || lower.contains("drop table"))
            && (lower.contains("column") || lower.contains("index") || lower.contains("constraint")
                || lower.contains("primary key") || lower.contains("references"))
        {
            return true;
        }

        // Security/auth configs — require structural context.
        // "fix: password reset" is not a secret. But "password: abc123" is.
        if lower.contains("-----begin") {
            // PEM headers are always high-risk
            return true;
        }
        if (lower.contains("private_key") || lower.contains("secret_key") || lower.contains("api_key"))
            && (lower.contains('=') || lower.contains(':'))
            && !self.looks_like_commit_log(&lower)
        {
            return true;
        }
        // "password" only high-risk when it looks like a config value, not a commit message
        if lower.contains("password")
            && (lower.contains("password:") || lower.contains("password=") || lower.contains("password \""))
            && !self.looks_like_commit_log(&lower)
        {
            return true;
        }

        // Kubernetes/infrastructure configs with critical fields
        if lower.contains("apiversion:") && lower.contains("kind:")
            && (lower.contains("secret") || lower.contains("configmap"))
        {
            return true;
        }

        false
    }

    /// Heuristic: does this content look like a git commit log?
    /// Commit logs have lines starting with short hashes (7+ hex chars)
    /// or conventional commit prefixes (feat:, fix:, chore:, etc.).
    fn looks_like_commit_log(&self, lower: &str) -> bool {
        let lines: Vec<&str> = lower.lines().take(10).collect();
        let mut commit_lines = 0;
        for line in &lines {
            let trimmed = line.trim();
            // Short hash prefix: "abc1234 feat: ...". Take chars (not a
            // byte slice) — byte 7 can land inside a multi-byte character
            // and panic (issue #34: Cyrillic output + a password marker).
            if trimmed.len() > 8
                && trimmed.chars().take(7).all(|c| c.is_ascii_hexdigit())
                && trimmed.chars().count() > 7
            {
                commit_lines += 1;
            }
            // Conventional commit prefix
            if trimmed.starts_with("feat:") || trimmed.starts_with("fix:")
                || trimmed.starts_with("chore:") || trimmed.starts_with("docs:")
                || trimmed.starts_with("refactor:") || trimmed.starts_with("test:")
                || trimmed.starts_with("ci:") || trimmed.starts_with("style:")
                || trimmed.starts_with("perf:")
            {
                commit_lines += 1;
            }
        }
        commit_lines > 0 && commit_lines >= lines.len() / 3
    }
}

pub(crate) fn protected_lines(content: &str) -> Vec<&str> {
    if without_ansi(content).len() < 100 {
        return Vec::new();
    }
    let raw: Vec<&str> = content.lines().collect();
    let plain: Vec<std::borrow::Cow<str>> = raw.iter().map(|l| without_ansi(l)).collect();
    let lines: Vec<&str> = plain.iter().map(|l| l.trim()).collect();
    let mut protected = Vec::new();
    let mut start = 0;
    for end in 0..=lines.len() {
        if lines.get(end).is_some_and(|l| !l.is_empty()) {
            continue;
        }
        if (start..end).any(|i| is_risk_line(&lines, i)) {
            protected.extend_from_slice(&raw[start..end]);
        }
        start = end + 1;
    }
    protected
}

fn without_ansi(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\x1b') {
        crate::ansi_strip::strip_ansi(text).into()
    } else {
        text.into()
    }
}

fn is_risk_line(lines: &[&str], i: usize) -> bool {
    let line = lines[i];
    let next = lines.get(i + 1).copied().unwrap_or("");
    let after = lines.get(i + 2).copied().unwrap_or("");
    starts_trace(line, next, after) || is_ddl(line) || is_pem(line) || is_secret_line(line)
}

fn starts_trace(line: &str, next: &str, after: &str) -> bool {
    (line.eq_ignore_ascii_case("stack backtrace:") && next.starts_with("0: "))
        || (line.starts_with("Traceback (most recent call last)") && next.starts_with("File \""))
        || (is_frame(next) && ["Error", "Exception", "Throwable"].iter().any(|w| line.contains(w)))
        || (is_goroutine_header(line) && next.ends_with(')') && after.contains(".go:"))
}

fn is_frame(line: &str) -> bool {
    line.strip_prefix("at ").is_some_and(|call| {
        (call.contains('(') && call.contains(')')) || ends_with_line_col(call)
    })
}

fn ends_with_line_col(call: &str) -> bool {
    let mut parts = call.rsplitn(3, ':');
    let (Some(col), Some(row), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [col, row].iter().all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn is_goroutine_header(line: &str) -> bool {
    line.strip_prefix("goroutine ")
        .and_then(|rest| rest.split_once(" ["))
        .is_some_and(|(id, state)| {
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && state.ends_with("]:")
        })
}

fn is_ddl(line: &str) -> bool {
    const DDL: &[&str] = &[
        "create table ", "alter table ", "drop table ", "create index ", "create unique index ",
        "drop index ",
    ];
    DDL.iter()
        .any(|k| line.get(..k.len()).is_some_and(|head| head.eq_ignore_ascii_case(k)))
}

fn is_pem(line: &str) -> bool {
    line.starts_with("-----BEGIN ") && line.ends_with("-----")
}

fn is_secret_line(line: &str) -> bool {
    const NAMES: &[&str] = &[
        "password", "passwd", "passphrase", "secret", "apikey", "privatekey", "secretkey",
        "accesskey", "token", "authorization",
    ];
    const NOT_SECRET: &[&str] = &["nexttoken", "pagetoken", "continuationtoken"];
    const WORDS: &[&str] = &["null", "none", "nil", "true", "false", "read", "write", "str", "string"];
    let norm = |s: &str| -> String {
        s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
    };
    let line = line.strip_prefix("export ").unwrap_or(line);
    let Some(at) = line.find(['=', ':']) else { return false };
    let key = line[..at].trim_end().trim_matches(['"', '\'']);
    if key.is_empty()
        || key.len() > 64
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
    {
        return false;
    }
    let name = norm(key);
    if !NAMES.iter().any(|n| name.ends_with(n)) || NOT_SECRET.iter().any(|n| name.ends_with(n)) {
        return false;
    }
    let rest = line[at + 1..].trim();
    let assigned = match rest.split_once(" = ") {
        Some((_, v)) if !rest.starts_with(['"', '\'']) => v.trim(),
        _ => rest,
    };
    let value = assigned.trim_end_matches(',').trim_matches(['"', '\'']);
    let code = !rest.starts_with(['"', '\''])
        && (rest.ends_with([',', ';']) || rest.contains(['(', ')', '[', ']', '{', '}', '<', '>', '&']));
    let bare = value.trim_start_matches(['^', '~', '=', '>', '<']);
    let bare = bare.strip_prefix('v').unwrap_or(bare);
    let numbers = bare.split(['-', '+']).next().unwrap_or(bare);
    let version = numbers.contains('.')
        && numbers.split('.').all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        && bare.bytes().all(|b| b.is_ascii_alphanumeric() || b"-+.".contains(&b));
    if rest.starts_with(':')
        || code
        || value.is_empty()
        || WORDS.iter().any(|w| value.eq_ignore_ascii_case(w))
        || version
    {
        return false;
    }
    let last = norm(value.rsplit('.').next().unwrap_or(value));
    let reference = (line.as_bytes()[at] == b'=' || assigned.len() < rest.len())
        && !assigned.starts_with(['"', '\''])
        && value.split('.').all(|s| {
            s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        && (last == norm(key.rsplit('.').next().unwrap_or(key))
            || (value.contains('.') && NAMES.iter().any(|n| last.ends_with(n))));
    !reference
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_stack_trace_to_safe() {
        let router = ConfidenceRouter::new();
        let trace = "thread 'main' panicked at 'index out of bounds', src/main.rs:42\nstack backtrace:\n  0: std::panicking::begin_panic\n  1: myapp::process\n  2: main";
        assert_eq!(router.route(trace), CompressionMode::Safe);
    }

    #[test]
    fn routes_migration_to_safe() {
        let router = ConfidenceRouter::new();
        let migration = "ALTER TABLE users ADD COLUMN email VARCHAR(255) NOT NULL;\nCREATE TABLE sessions (id UUID PRIMARY KEY, user_id INT REFERENCES users(id));";
        assert_eq!(router.route(migration), CompressionMode::Safe);
    }

    /// Regression for issue #34: routing multi-byte text through the
    /// commit-log heuristic panicked on `trimmed[..7]` when byte 7 landed
    /// inside a multi-byte character. The password marker is what pulls
    /// the input into `looks_like_commit_log`; the Cyrillic line is what
    /// used to blow up there.
    #[test]
    fn multibyte_input_with_password_marker_does_not_panic() {
        let router = ConfidenceRouter::new();
        // Must clear the router's 100-byte minimum or is_high_risk (and
        // the vulnerable heuristic behind it) never runs.
        let content = "Привет мир строка номер 0\n\
                       Привет мир строка номер 1\n\
                       Привет мир строка номер 2\n\
                       пароль тут\n\
                       password: тайна";
        assert!(content.len() >= 100, "fixture must reach is_high_risk");
        // High-risk (password) content routes to Safe; the point of the
        // test is that `route` returns at all instead of panicking.
        assert_eq!(router.route(content), CompressionMode::Safe);
        assert!(router.never_store(content));
    }

    /// Same class, CJK + emoji: no byte offset in these strings may be
    /// assumed to be a char boundary.
    #[test]
    fn cjk_and_emoji_input_does_not_panic() {
        let router = ConfidenceRouter::new();
        let cjk = "私はガラスを食べられます。それは私を傷つけません。\n\
                   api_key = 秘密🦀\n\
                   日本語のテキストの行がもう一つあります";
        assert!(cjk.len() >= 100);
        let _ = router.route(cjk);
        let _ = router.never_store(cjk);
    }

    #[test]
    fn git_log_with_fix_is_not_high_risk() {
        let router = ConfidenceRouter::new();
        let git_log = "abc1234 fix: password reset flow\ndef5678 feat: migrate user auth to OAuth\n\
                       1234567 chore: update dependencies\n8901234 fix: migration path handling\n\
                       abcdef0 refactor: clean up error handling\n1111111 docs: update README\n\
                       2222222 fix: handle edge case in parser\n3333333 feat: add new API endpoint";
        let mode = router.route(git_log);
        assert_ne!(mode, CompressionMode::Safe,
            "git log with 'fix:' and 'migration' should NOT trigger safe mode");
    }

    #[test]
    fn commit_message_with_password_is_not_high_risk() {
        let router = ConfidenceRouter::new();
        let git_log = "abc1234 fix: password reset flow broken on mobile\n\
                       def5678 feat: add password strength indicator\n\
                       1234567 chore: update password hashing library\n\
                       8901234 fix: password validation regex\n\
                       abcdef0 test: add password reset integration tests";
        let mode = router.route(git_log);
        assert_ne!(mode, CompressionMode::Safe,
            "git log mentioning 'password' should NOT trigger safe mode");
    }

    #[test]
    fn actual_password_config_is_high_risk() {
        let router = ConfidenceRouter::new();
        let config = "database:\n  host: localhost\n  port: 5432\n  username: admin\n  password: super_secret_123\n  database: myapp_prod\n  ssl: true\n  pool_size: 10";
        assert_eq!(router.route(config), CompressionMode::Safe,
            "config file with password: value should trigger safe mode");
    }

    #[test]
    fn routes_repetitive_logs_to_aggressive() {
        let router = ConfidenceRouter::new();
        // Very repetitive content → low entropy → aggressive
        let logs = "// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n// comment\n";
        let mode = router.route(logs);
        // Should be aggressive or default (low entropy)
        assert!(mode == CompressionMode::Aggressive || mode == CompressionMode::Default);
    }

    #[test]
    fn routes_normal_code_to_default() {
        let router = ConfidenceRouter::new();
        let code = "fn process(items: &[Item]) -> Result<Vec<Output>, Error> {\n    let mut results = Vec::new();\n    for item in items {\n        let output = transform(item)?;\n        results.push(output);\n    }\n    Ok(results)\n}";
        let mode = router.route(code);
        assert_eq!(mode, CompressionMode::Default,
            "normal code should route to Default, not Safe");
    }

    #[test]
    fn short_content_is_default() {
        let router = ConfidenceRouter::new();
        assert_eq!(router.route("hello"), CompressionMode::Default);
    }

    #[test]
    fn pem_key_routes_to_safe() {
        let router = ConfidenceRouter::new();
        // PEM key must be > 100 chars to trigger routing
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOP\nQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz==\n-----END RSA PRIVATE KEY-----\n";
        assert_eq!(router.route(key), CompressionMode::Safe);
    }

    #[test]
    fn routes_every_trace_format_to_safe() {
        let router = ConfidenceRouter::new();
        let padding = "worker started, waiting for jobs on the default queue\n".repeat(2);
        for trace in [
            "Traceback (most recent call last):\n  File \"/srv/app/main.py\", line 42, in <module>\n    main()\nValueError: bad refund amount\n",
            "Exception in thread \"main\" java.lang.IllegalStateException: queue closed\n\tat com.acme.Queue.pop(Queue.java:88)\n\tat com.acme.Main.main(Main.java:12)\n",
            "TypeError: Cannot read properties of undefined (reading 'id')\n    at refund (/srv/app/refund.js:14:22)\n    at /srv/app/index.js:3:9\n",
            "panic: runtime error: index out of range [5] with length 3\n\ngoroutine 1 [running]:\nmain.main()\n\t/srv/app/main.go:8 +0x1d\nexit status 2\n",
            "Error: config missing\n\nStack backtrace:\n   0: anyhow::error::object_drop\n   1: app::main\n",
        ] {
            let content = format!("{padding}{trace}");
            assert_eq!(router.route(&content), CompressionMode::Safe, "{trace}");
            assert!(router.never_store(&content), "{trace}");
        }
    }

    #[test]
    fn routes_credential_lines_to_safe() {
        let router = ConfidenceRouter::new();
        let padding = "service: billing\nregion: us-east-1\nreplicas: 3\nlog_level: info\n";
        for line in [
            "export STRIPE_SECRET_KEY=sk_test_51Pq9AbCd",
            "    \"apiKey\": \"ak_live_0123456789\",",
            "db_password = \"hunter2\"",
            "GITHUB_TOKEN=ghp_0123456789abcdef",
            "aws_secret_access_key: wJalrXUtnFEMI",
            "POSTGRES_PASSWORD: password",
            "password: password",
            "token=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
            "api_key: str = \"sk_live_0123456789\"",
            "token: v4.public.eyJzdWIiOiIxMjM0In0",
            "token: 07401b.f395accd246ae52d",
            "api_key = \"sk-proj-0123456789abcdef\"",
        ] {
            let content = format!("{padding}{line}\n{padding}");
            assert_eq!(router.route(&content), CompressionMode::Safe, "{line}");
        }
        for line in [
            "password:",
            "next_token: eyJvZmZzZXQiOjEwfQ",
            "secretName: db-creds",
            "\"token\": null,",
            "self.token = token",
            "self._api_key = api_key",
            "token = None",
            "password = password",
            "secret = secret",
            "api_key = settings.STRIPE_API_KEY",
            "id-token: write",
            "\"jsonwebtoken\": \"^9.0.0\",",
            "\"jsonwebtoken\": \"~1.2\",",
            "\"jsonwebtoken\": \">=2.0.1\",",
            "\"jsonwebtoken\": \"v1.2.3-beta.1\",",
            "\"jsonwebtoken\": \"1.2.3+build.5\",",
            "token: str = \"\"",
            "token: str = \"\", grant_json: str | None = None,",
            "password: str",
        ] {
            let content = format!("{padding}{line}\n{padding}");
            assert_ne!(router.route(&content), CompressionMode::Safe, "{line}");
        }
    }

    #[test]
    fn markers_outside_their_structure_are_not_routed_but_never_stored() {
        let router = ConfidenceRouter::new();
        let log: String = (0..20)
            .map(|i| format!("2026-10-05T12:00:{i:02}Z INFO  GET /api/orders/{i} 200 3ms\n"))
            .collect();
        for content in [
            format!("{log}2026-10-05T12:30:00Z DEBUG db config host=db.internal user=app password=hunter2\n{log}"),
            format!("commit 1a2b3c4\n\n    fix(router): no panic on a marker that panicked at byte 7, which ran\n    at a momentarily-invalid home path.\n{log}"),
            format!("+    conn.execute(\"CREATE TABLE sessions (id TEXT PRIMARY KEY)\")\n{log}"),
            format!("let api_key = env::var(\"API_KEY\")?;\n{log}"),
        ] {
            assert_ne!(router.route(&content), CompressionMode::Safe, "{content}");
            assert!(router.never_store(&content), "{content}");
        }
    }

    #[test]
    fn protected_lines_are_the_paragraphs_that_route_safe() {
        let router = ConfidenceRouter::new();
        let trace = "---- tests::refund stdout ----\nthread 'tests::refund' panicked at src/lib.rs:9:5:\nstack backtrace:\n   0: rust_begin_unwind\n   1: app::tests::refund\n             at ./src/lib.rs:9:5";
        let output = format!("running 3 tests\ntest tests::a ... ok\ntest tests::b ... ok\n\nfailures:\n\n{trace}\n\n\ntest result: FAILED. 2 passed; 1 failed\n");
        assert_eq!(protected_lines(&output), trace.lines().collect::<Vec<_>>());
        assert_eq!(router.route(&output), CompressionMode::Safe);
        let clean = output.replace("stack backtrace:", "note: run with `RUST_BACKTRACE=1`");
        assert!(protected_lines(&clean).is_empty());
        assert_ne!(router.route(&clean), CompressionMode::Safe);
        assert!(protected_lines("password: hunter2\n").is_empty());
    }

    #[test]
    fn colored_content_is_routed_like_plain_content() {
        let router = ConfidenceRouter::new();
        let color = |s: &str| s.lines().map(|l| format!("\x1b[31m{l}\x1b[0m\n")).collect::<String>();
        for plain in [
            "thread 'main' panicked at src/q.rs:217:31:\ncalled `Option::unwrap()` on a `None` value\nstack backtrace:\n   0: rust_begin_unwind\n   1: app::main\n             at ./src/main.rs:3:5\n",
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, user_id INT);\nALTER TABLE users ADD COLUMN email VARCHAR(255) NOT NULL;\n",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0123456789abcdefghijklmnopqrstuvwxyz\n-----END RSA PRIVATE KEY-----\n",
            "service: billing\nregion: us-east-1\nGITHUB_TOKEN=ghp_0123456789abcdef\nreplicas: 3\nlog_level: info\npool_size: 10\n",
        ] {
            let colored = color(plain);
            assert_eq!(router.route(plain), CompressionMode::Safe, "{plain}");
            assert_eq!(router.route(&colored), CompressionMode::Safe, "{plain}");
            assert!(router.never_store(&colored), "{plain}");
            assert_eq!(protected_lines(&colored), colored.lines().collect::<Vec<_>>(), "{plain}");
        }
    }

    #[test]
    fn legal_text_is_routed_and_stored_like_any_page() {
        let router = ConfidenceRouter::new();
        let page = "<div class=\"row\"><a href=\"/item/1\">Item 1</a></div>\n".repeat(8)
            + "<footer><a href=\"/privacy\">Privacy Policy</a> <a href=\"/terms\">Terms of Service</a></footer>\n";
        assert_ne!(router.route(&page), CompressionMode::Safe);
        assert!(!router.never_store(&page));
        assert!(!router.never_store("password=hunter2"));
    }
}
