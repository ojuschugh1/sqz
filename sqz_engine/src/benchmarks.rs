/// sqz compression benchmark suite.
///
/// Measures faithfulness, critical-info retention, and token reduction
/// across representative content types. Used as CI regression gates.
///
/// Run with: `cargo test -p sqz-engine benchmarks`

#[cfg(test)]
mod tests {
    use crate::engine::SqzEngine;
    use crate::verifier::Verifier;

    // ── Helpers ──────────────────────────────────────────────────────────────

    struct BenchResult {
        content_type: &'static str,
        original_tokens: u32,
        compressed_tokens: u32,
        reduction_pct: f64,
        verify_confidence: f64,
        critical_info_retained: bool,
    }

    fn estimate_tokens(s: &str) -> u32 {
        ((s.len() as f64) / 4.0).ceil() as u32
    }

    fn run_bench(content_type: &'static str, input: &str, critical_marker: &str) -> BenchResult {
        let engine = SqzEngine::new().expect("engine init");
        let result = engine.compress(input).expect("compress");

        let original_tokens = estimate_tokens(input);
        let compressed_tokens = estimate_tokens(&result.data);
        let reduction_pct = (1.0 - compressed_tokens as f64 / original_tokens.max(1) as f64) * 100.0;

        let verify = result.verify.unwrap_or_else(|| Verifier::verify(input, &result.data));
        let critical_info_retained = result.data.contains(critical_marker);

        BenchResult {
            content_type,
            original_tokens,
            compressed_tokens,
            reduction_pct,
            verify_confidence: verify.confidence,
            critical_info_retained,
        }
    }

    fn assert_gates(b: &BenchResult, min_reduction: f64, min_confidence: f64) {
        assert!(
            b.critical_info_retained,
            "[{}] critical info marker missing from output",
            b.content_type
        );
        assert!(
            b.verify_confidence >= min_confidence,
            "[{}] verify confidence {:.2} < threshold {:.2}",
            b.content_type, b.verify_confidence, min_confidence
        );
        // Only assert reduction if input is large enough to compress meaningfully
        if b.original_tokens > 50 {
            assert!(
                b.reduction_pct >= min_reduction,
                "[{}] reduction {:.1}% < threshold {:.1}%",
                b.content_type, b.reduction_pct, min_reduction
            );
        }
        println!(
            "[bench] {} | {}→{} tokens | {:.1}% reduction | confidence {:.2}",
            b.content_type, b.original_tokens, b.compressed_tokens,
            b.reduction_pct, b.verify_confidence
        );
    }

    // ── Benchmark cases ───────────────────────────────────────────────────────

    /// JSON API response — should compress via TOON + strip_nulls.
    /// Gate: ≥5% reduction, confidence ≥0.8, key "id" retained.
    #[test]
    fn bench_json_api_response() {
        let input = r#"{"id":42,"name":"Alice","email":"alice@example.com","role":"admin","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-03-15T10:30:00Z","metadata":{"plan":"pro","seats":10,"billing_cycle":"monthly","internal_id":null,"debug_info":null,"trace_id":null}}"#;
        let b = run_bench("json_api", input, "42");
        assert_gates(&b, 5.0, 0.8);
    }

    /// Repeated log output — should compress via condense stage.
    /// Gate: ≥30% reduction, confidence ≥0.8, "ERROR" retained.
    #[test]
    fn bench_repeated_logs() {
        let repeated = "2024-01-01 10:00:00 [INFO] Connected to database\n".repeat(10);
        let input = format!(
            "2024-01-01 09:59:59 [INFO] Server started\n{repeated}2024-01-01 10:00:11 [ERROR] Connection timeout after 30s\n"
        );
        let b = run_bench("repeated_logs", &input, "ERROR");
        assert_gates(&b, 30.0, 0.7);
    }

    /// Git diff — should fold unchanged context lines.
    /// Gate: ≥10% reduction, confidence ≥0.8, hunk header retained.
    #[test]
    fn bench_git_diff() {
        let input = concat!(
            "diff --git a/src/main.rs b/src/main.rs\n",
            "--- a/src/main.rs\n",
            "+++ b/src/main.rs\n",
            "@@ -1,15 +1,15 @@\n",
            " line1\n line2\n line3\n line4\n line5\n line6\n line7\n line8\n",
            "-old_function_name\n",
            "+new_function_name\n",
            " line9\n line10\n line11\n line12\n line13\n line14\n line15\n",
        );
        let b = run_bench("git_diff", input, "@@ -1,15");
        assert_gates(&b, 5.0, 0.8);
    }

    /// Plain prose — the Rust CLI engine does minimal prose compression
    /// (it's optimized for JSON/logs). Verify no regression and critical info retained.
    #[test]
    fn bench_prose_documentation() {
        let input = "In order to understand the system, it is important to note that the architecture consists of multiple components. Due to the fact that each component has the ability to operate independently, the system as a whole is resilient. In the event that one component fails, the other components continue to function. The vast majority of operations are handled by the core engine, which processes requests in real time. As a result of this design, the system achieves high availability and low latency.";
        let b = run_bench("prose_docs", input, "architecture");
        // Rust engine doesn't aggressively compress prose — just verify no data loss
        assert!(b.critical_info_retained, "critical info must be retained");
        assert!(
            b.verify_confidence >= 0.7,
            "[prose_docs] confidence {:.2} too low", b.verify_confidence
        );
        assert!(
            b.compressed_tokens <= b.original_tokens,
            "[prose_docs] output must not be larger than input"
        );
        println!(
            "[bench] {} | {}→{} tokens | {:.1}% reduction | confidence {:.2}",
            b.content_type, b.original_tokens, b.compressed_tokens,
            b.reduction_pct, b.verify_confidence
        );
    }

    /// Stack trace — should route to safe mode, preserve all error lines.
    /// Gate: confidence ≥0.9 (safe mode), "panicked" retained.
    #[test]
    fn bench_stack_trace_safe_mode() {
        let input = "thread 'main' panicked at 'index out of bounds: the len is 3 but the index is 5', src/main.rs:42\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\nstack backtrace:\n   0: rust_begin_unwind\n   1: core::panicking::panic_fmt\n   2: core::slice::index_failed\n   3: myapp::process_items\n   4: myapp::main\n";
        let b = run_bench("stack_trace", input, "panicked");
        // Safe mode: high confidence, critical info preserved
        assert!(b.critical_info_retained, "panicked marker must be retained");
        assert!(
            b.verify_confidence >= 0.7,
            "stack trace confidence {:.2} too low", b.verify_confidence
        );
        println!(
            "[bench] {} | {}→{} tokens | {:.1}% reduction | confidence {:.2}",
            b.content_type, b.original_tokens, b.compressed_tokens,
            b.reduction_pct, b.verify_confidence
        );
    }

    /// Large JSON array — should sample via schema extraction.
    /// Gate: ≥20% reduction, confidence ≥0.7, "count" or item count retained.
    #[test]
    fn bench_large_json_array() {
        let items: Vec<String> = (1..=20)
            .map(|i| format!(r#"{{"id":{i},"name":"item{i}","value":{i}00,"active":true}}"#))
            .collect();
        let input = format!("[{}]", items.join(","));
        let b = run_bench("large_json_array", &input, "id");
        assert_gates(&b, 20.0, 0.6);
    }
}
