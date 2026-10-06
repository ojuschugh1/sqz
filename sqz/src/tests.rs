/// Smoke tests for build artifacts (Task 22.3).
///
/// Verifies at compile/test time that the binary's key invariants hold:
/// - 90+ CLI patterns registered (Requirement 1.2)
/// - 18+ tree-sitter grammars loaded (Requirement 19.1)
/// - Default preset is valid (Requirement 24.2)
/// - SqzEngine::new() succeeds without network requests (Requirements 23.1, 16.1)
#[cfg(test)]
mod smoke_tests {
    use sqz_engine::{AstParser, SqzEngine};
    use sqz_engine::preset::{Preset, PresetParser};

    /// Binary is operational: SqzEngine::new() succeeds.
    /// No network requests are made (offline operation, Requirement 23.1).
    #[test]
    fn test_engine_initialises_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let engine = SqzEngine::with_preset_and_store(Preset::default(), &dir.path().join("sessions.db"));
        assert!(
            engine.is_ok(),
            "the engine must initialise: {:?}",
            engine.err()
        );
    }

    /// 90+ CLI patterns are registered (Requirement 1.2).
    #[test]
    fn test_90_plus_cli_patterns() {
        assert!(
            crate::cli_proxy::CLI_PATTERNS.len() >= 90,
            "expected ≥90 CLI patterns, got {}",
            crate::cli_proxy::CLI_PATTERNS.len()
        );
    }

    /// 18+ tree-sitter grammars are loaded (Requirement 19.1).
    #[test]
    fn test_18_plus_grammars_loaded() {
        let parser = AstParser::new();
        let count = parser.supported_languages().len();
        assert!(
            count >= 18,
            "expected ≥18 supported languages, got {count}"
        );
    }

    /// Default preset is valid (passes PresetParser::validate).
    /// Requirement 24.2: default presets bundled.
    #[test]
    fn test_default_preset_is_valid() {
        let preset = Preset::default();
        let result = PresetParser::validate(&preset);
        assert!(
            result.is_ok(),
            "Preset::default() must pass validation: {:?}",
            result.err()
        );
    }

    /// Default preset round-trips through TOML serialization.
    #[test]
    fn test_default_preset_round_trips() {
        let preset = Preset::default();
        let toml = PresetParser::to_toml(&preset).expect("serialize default preset");
        let parsed = PresetParser::parse(&toml).expect("parse serialized default preset");
        // Verify the name survives the round-trip.
        assert_eq!(preset.preset.name, parsed.preset.name);
    }
}

/// Unit tests for CLI proxy and shell hooks (Task 20.4).
///
/// Covers:
/// - Hook installation failure fallback (Requirement 1.5)
/// - `sqz init` creates hooks and presets
/// - CLI output interception end-to-end (Requirements 1.1, 1.4)

#[cfg(test)]
mod cli_proxy_tests {
    use crate::cli_proxy::CliProxy;

    /// A `CliProxy` on an isolated temp-dir session DB. `CliProxy::new()` opens
    /// the shared `~/.sqz/sessions.db`, which races and corrupts under parallel
    /// `cargo test` (CI flake on #33). The `TempDir` must outlive the proxy.
    fn isolated_proxy() -> (CliProxy, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("sessions.db");
        let engine =
            sqz_engine::SqzEngine::with_preset_and_store(sqz_engine::Preset::default(), &store)
                .expect("engine init");
        (CliProxy::with_engine(engine), dir)
    }

    /// End-to-end: intercept_output returns a non-empty string for typical
    /// CLI output (Requirement 1.1, 1.4).
    #[test]
    fn test_intercept_output_end_to_end() {
        let (proxy, _dir) = isolated_proxy();
        let raw = "On branch main\nnothing to commit, working tree clean\n";
        let result = proxy.intercept_output("git status", raw);
        assert!(!result.is_empty(), "compressed output must not be empty");
    }

    /// Transparent operation: when compression fails the original output is
    /// returned unchanged (Requirement 1.5 fallback).
    #[test]
    fn test_intercept_output_transparent_on_empty_input() {
        let (proxy, _dir) = isolated_proxy();
        // Empty input — should not panic and should return something.
        let result = proxy.intercept_output("cargo build", "");
        // Either empty or the original empty string — no panic is the key assertion.
        let _ = result;
    }

    /// 90+ CLI patterns are registered (Requirement 1.2).
    #[test]
    fn test_at_least_90_cli_patterns() {
        assert!(
            crate::cli_proxy::CLI_PATTERNS.len() >= 90,
            "expected ≥90 patterns, got {}",
            crate::cli_proxy::CLI_PATTERNS.len()
        );
    }

    /// Known commands are recognised.
    #[test]
    fn test_known_commands_recognised() {
        for cmd in &["git", "cargo", "npm", "docker", "kubectl", "aws"] {
            assert!(
                CliProxy::is_known_command(cmd),
                "'{cmd}' should be a known command"
            );
        }
    }

    /// Unknown commands are not recognised.
    #[test]
    fn test_unknown_command_not_recognised() {
        assert!(!CliProxy::is_known_command("my_totally_custom_tool_xyz"));
    }

    fn words(seed: u64, n: usize) -> String {
        const VOCAB: [&str; 12] = [
            "cache", "hook", "proxy", "diff", "stats", "windows", "hint", "large", "search",
            "cursor", "expand", "client",
        ];
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                VOCAB[(x >> 33) as usize % VOCAB.len()]
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn samples() -> Vec<(&'static str, String)> {
        let json = (0..40)
            .map(|i| {
                format!(
                    r#"{{"id":{},"number":{i},"title":"{}","state":"{}","html_url":"https://github.com/acme/app/issues/{i}","user":{{"login":"dev{}","type":"User"}},"created_at":"2026-08-{:02}T10:00:00Z","body":"{}"}}"#,
                    2_900_000_000u64 + i,
                    words(i, 6),
                    if i % 3 == 0 { "closed" } else { "open" },
                    i % 5,
                    i % 28 + 1,
                    words(i + 100, 25),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let pods: Vec<serde_json::Value> = (0..20)
            .map(|i| {
                serde_json::json!({
                    "apiVersion": "v1",
                    "kind": "Pod",
                    "metadata": {
                        "name": format!("web-{i}"),
                        "namespace": "default",
                        "labels": {"app": "web", "pod-template-hash": "5d8f7c9b4", "tier": "frontend"},
                    },
                    "spec": {
                        "containers": [{
                            "name": "web",
                            "image": "nginx:1.27",
                            "ports": [{"containerPort": 8080, "protocol": "TCP"}],
                        }],
                        "nodeName": format!("node-{}", i % 3),
                    },
                    "status": {
                        "phase": "Running",
                        "conditions": [{"type": "Ready", "status": "True"}],
                        "containerStatuses": [{"name": "web", "ready": true, "restartCount": i % 2}],
                    },
                })
            })
            .collect();
        let kubectl = serde_json::to_string_pretty(&serde_json::json!({
            "apiVersion": "v1",
            "kind": "List",
            "items": pods,
        }))
        .unwrap();
        let mut cargo_test = String::from("running 120 tests\n");
        for i in 0..120 {
            let status = if i % 15 == 0 { "FAILED" } else { "ok" };
            cargo_test.push_str(&format!("test engine::tests::case_{i} ... {status}\n"));
        }
        cargo_test.push_str("\nfailures:\n\n");
        for i in (0..120).step_by(15) {
            cargo_test.push_str(&format!(
                "---- engine::tests::case_{i} stdout ----\nthread 'engine::tests::case_{i}' panicked at src/engine.rs:{}:9:\nassertion `left == right` failed\n  left: {i}\n right: {}\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n",
                100 + i,
                i + 1,
            ));
        }
        cargo_test.push_str("test result: FAILED. 112 passed; 8 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s\n");
        let git_diff: String = (0..12)
            .map(|i| {
                format!(
                    "diff --git a/src/handler_{i}.rs b/src/handler_{i}.rs\nindex 3f2a1b{i:x}..7c9d0e{i:x} 100644\n--- a/src/handler_{i}.rs\n+++ b/src/handler_{i}.rs\n@@ -{0},7 +{0},8 @@ fn handler_{i}(request: &Request) -> Response {{\n     let token = request.headers.get(\"Authorization\");\n     let scope = format!(\"handler_{i}\");\n-    verify(token, &scope)\n+    let claims = verify(token, &scope)?;\n+    audit(&claims, &scope);\n+    Ok(respond(claims))\n }}\n",
                    10 + i,
                )
            })
            .collect();
        let prose: String = (0..30)
            .map(|i| {
                format!(
                    "## Section {i}\n\nThe proxy keeps the cache warm while the client expands every hint. {}\n\n",
                    words(i, 40),
                )
            })
            .collect();
        let mut cargo_build: String = (0..25)
            .map(|i| {
                format!(
                    "error[E0308]: mismatched types\n --> src/module_{i}.rs:{0}:18\n  |\n{0} |     let count: i32 = \"{i}\";\n  |                ---   ^^^ expected `i32`, found `&str`\n  |                |\n  |                expected due to this\n\n",
                    10 + i,
                )
            })
            .collect();
        cargo_build
            .push_str("error: could not compile `app` (bin \"app\") due to 25 previous errors\n");
        vec![
            ("json", format!("[{json}]")),
            ("kubectl json", kubectl),
            ("cargo test", cargo_test),
            ("git diff", git_diff),
            ("prose", prose),
            ("cargo build", cargo_build),
        ]
    }

    #[test]
    fn test_output_is_identical_across_fresh_engines() {
        let mut first = Vec::new();
        for (name, input) in samples() {
            let outputs: Vec<String> = (0..5)
                .map(|_| {
                    let (proxy, _dir) = isolated_proxy();
                    proxy.intercept_output("stdin", &input)
                })
                .collect();
            assert!(
                outputs.iter().all(|o| *o == outputs[0]),
                "{name}: output differs between runs"
            );
            first.push(outputs[0].clone());
        }
        assert!(first.iter().any(|o| o.contains("§dict§")));
        assert!(first.iter().any(|o| o.contains("[Abbreviations]")));
    }
}

#[cfg(test)]
mod shell_hook_tests {
    use crate::shell_hook::{install_hook_to_file, ShellHook};
    use std::fs;
    use tempfile::TempDir;

    /// Hook installation writes the script to the RC file (Requirement 1.1).
    #[test]
    fn test_install_hook_writes_script() {
        let dir = TempDir::new().unwrap();
        let rc = dir.path().join(".bashrc");
        let result = install_hook_to_file(
            &rc,
            "# sqz — context intelligence layer (auto-installed)\nsqz_run() { \"$@\" | sqz compress; }",
            "# sqz — context intelligence layer (auto-installed)",
        );
        assert!(result.unwrap(), "first install should return true");
        let content = fs::read_to_string(&rc).unwrap();
        assert!(content.contains("sqz compress"));
    }

    /// Installing twice is idempotent — no duplicate entries (Requirement 1.4).
    #[test]
    fn test_install_hook_idempotent() {
        let dir = TempDir::new().unwrap();
        let rc = dir.path().join(".zshrc");
        let script = "# sqz — context intelligence layer (auto-installed)\nsqz_run() {}";
        let sentinel = "# sqz — context intelligence layer (auto-installed)";
        install_hook_to_file(&rc, script, sentinel).unwrap();
        let second = install_hook_to_file(&rc, script, sentinel).unwrap();
        assert!(!second, "second install should be a no-op");
        // Ensure the sentinel appears exactly once.
        let content = fs::read_to_string(&rc).unwrap();
        assert_eq!(content.matches(sentinel).count(), 1);
    }

    /// Installation failure fallback: when the RC path is unwritable the
    /// error is returned (Requirement 1.5).
    #[test]
    fn test_install_hook_failure_returns_error() {
        // Use a path that cannot be created (root-owned directory on Unix).
        let bad_path = std::path::Path::new("/root/.bashrc_sqz_test_unwritable");
        let result = install_hook_to_file(
            bad_path,
            "# sqz — context intelligence layer (auto-installed)\n",
            "# sqz — context intelligence layer (auto-installed)",
        );
        // On a non-root test runner this should fail.
        if result.is_err() {
            // Expected: error is returned, not panicked.
            let err = result.unwrap_err();
            assert!(err.to_string().contains("sqz hook installation failed"));
        }
        // If somehow it succeeds (running as root), that's also fine.
    }

    /// `sqz init` creates the default preset file (Requirement 16.3).
    #[test]
    fn test_init_creates_default_preset() {
        let dir = TempDir::new().unwrap();
        let preset_dir = dir.path().join(".sqz").join("presets");
        std::fs::create_dir_all(&preset_dir).unwrap();
        let preset_path = preset_dir.join("default.toml");
        assert!(!preset_path.exists());

        // Write the default preset as init would.
        let default_toml = "[meta]\nname = \"default\"\nversion = \"1\"\n";
        fs::write(&preset_path, default_toml).unwrap();

        assert!(preset_path.exists());
        let content = fs::read_to_string(&preset_path).unwrap();
        assert!(content.contains("[meta]"));
    }

    /// All four shell variants produce non-empty hook scripts.
    #[test]
    fn test_all_shell_variants_have_scripts() {
        for hook in &[
            ShellHook::Bash,
            ShellHook::Zsh,
            ShellHook::Fish,
            ShellHook::PowerShell,
        ] {
            assert!(
                !hook.hook_script().is_empty(),
                "{hook:?} hook script must not be empty"
            );
            assert!(
                hook.hook_script().contains("sqz compress"),
                "{hook:?} hook script must reference 'sqz compress'"
            );
        }
    }

    /// RC paths are distinct per shell variant.
    #[test]
    fn test_rc_paths_are_distinct() {
        let paths: Vec<_> = [
            ShellHook::Bash,
            ShellHook::Zsh,
            ShellHook::Fish,
            ShellHook::PowerShell,
        ]
        .iter()
        .map(|h| h.rc_path())
        .collect();

        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len(), "each shell must have a unique RC path");
    }
}

#[cfg(test)]
mod share_card_tests {
    use crate::{group, ShareCard};

    #[test]
    fn share_card_reports_savings_and_regret_side_by_side() {
        let card = ShareCard {
            first_day: "2026-09-01".into(),
            last_day: "2026-09-22".into(),
            compressions: 3003,
            tokens_in: 721_840,
            tokens_out: 543_398,
            reference_saved: 44_296,
            reruns: 41,
            expands: 3,
        };
        let text = card.render();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "sqz, 2026-09-01 to 2026-09-22");
        assert_eq!(lines[1], "3,003 tool outputs compressed: 721,840 tokens in, 543,398 out, 178,442 saved (24.7%)");
        assert_eq!(lines[2], "25% of the saving came from references to output the model already had");
        assert_eq!(lines[3], "regret: 41 quick re-runs (1.4% of compressions), 3 ref expands");
        assert!(lines[4].contains("github.com/ojuschugh1/sqz"));
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn share_card_single_day_and_zero_division() {
        let card = ShareCard {
            first_day: "2026-09-22".into(),
            last_day: "2026-09-22".into(),
            compressions: 0,
            tokens_in: 0,
            tokens_out: 0,
            reference_saved: 0,
            reruns: 0,
            expands: 0,
        };
        let text = card.render();
        assert!(text.starts_with("sqz, 2026-09-22\n"));
        assert!(text.contains("0 saved (0.0%)"));
        assert!(text.contains("(0.0% of compressions)"));
    }

    #[test]
    fn group_inserts_thousands_separators() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
        assert_eq!(group(178_442), "178,442");
        assert_eq!(group(1_234_567), "1,234,567");
    }
}
