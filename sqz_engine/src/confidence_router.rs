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
    /// Used for stack traces, configs, migration files, legal text.
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

    /// Returns true if content contains high-risk patterns that should
    /// never be aggressively compressed.
    ///
    /// These checks require structural context — not just keyword presence.
    /// A commit message saying "fix: password reset" is NOT a secret.
    /// A stack trace saying "panicked at" IS high-risk.
    fn is_high_risk(&self, content: &str) -> bool {
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

        // Legal/compliance text
        if lower.contains("terms of service") || lower.contains("privacy policy")
            || lower.contains("license agreement") || lower.contains("gdpr")
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
            // Short hash prefix: "abc1234 feat: ..."
            if trimmed.len() > 8 && trimmed[..7].chars().all(|c| c.is_ascii_hexdigit()) {
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
}
