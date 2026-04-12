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

        // High average entropy → information-dense → safe
        if avg_entropy > 4.5 {
            return CompressionMode::Safe;
        }

        CompressionMode::Default
    }

    /// Returns true if content contains high-risk patterns that should
    /// never be aggressively compressed.
    fn is_high_risk(&self, content: &str) -> bool {
        let lower = content.to_lowercase();

        // Stack traces
        if lower.contains("stack trace") || lower.contains("traceback")
            || lower.contains("at line ") || lower.contains("panicked at")
        {
            return true;
        }

        // Database migrations
        if lower.contains("alter table") || lower.contains("create table")
            || lower.contains("drop table") || lower.contains("migration")
        {
            return true;
        }

        // Security/auth configs
        if lower.contains("private_key") || lower.contains("secret_key")
            || lower.contains("api_key") || lower.contains("password")
            || lower.contains("-----begin") // PEM headers
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
        let trace = "thread 'main' panicked at 'index out of bounds', src/main.rs:42\nstack trace:\n  0: std::panicking::begin_panic\n  1: myapp::process\n  2: main";
        assert_eq!(router.route(trace), CompressionMode::Safe);
    }

    #[test]
    fn routes_migration_to_safe() {
        let router = ConfidenceRouter::new();
        let migration = "ALTER TABLE users ADD COLUMN email VARCHAR(255) NOT NULL;\nCREATE TABLE sessions (id UUID PRIMARY KEY, user_id INT REFERENCES users(id));";
        assert_eq!(router.route(migration), CompressionMode::Safe);
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
        assert!(mode == CompressionMode::Default || mode == CompressionMode::Safe);
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
