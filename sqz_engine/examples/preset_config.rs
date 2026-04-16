//! Preset configuration example.
//!
//! Shows how to parse a TOML preset, create an engine with it, and compress
//! content using custom settings.
//!
//! Run with: `cargo run --example preset_config -p sqz-engine`

use sqz_engine::preset::{Preset, PresetParser};
use sqz_engine::SqzEngine;

fn main() {
    // Define a custom preset as TOML
    let toml = r#"
[preset]
name = "code-review"
version = "1.0"
description = "Tuned for code review workflows — keeps error context, strips noise"

[compression]
stages = ["condense", "strip_nulls", "truncate_strings"]

[compression.condense]
enabled = true
max_repeated_lines = 2

[compression.strip_nulls]
enabled = true

[compression.truncate_strings]
enabled = true
max_length = 300

[tool_selection]
max_tools = 5
similarity_threshold = 0.7

[budget]
warning_threshold = 0.70
ceiling_threshold = 0.85
default_window_size = 128000

[terse_mode]
enabled = false
level = "moderate"

[model]
family = "anthropic"
primary = "claude-sonnet-4-20250514"
complexity_threshold = 0.4
"#;

    // Parse and validate the preset
    let preset = PresetParser::parse(toml).expect("invalid preset TOML");
    println!("Loaded preset: {} v{}", preset.preset.name, preset.preset.version);
    println!("Description: {}", preset.preset.description);
    println!("Window size: {} tokens", preset.budget.default_window_size);
    println!();

    // Create an engine with this preset
    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("demo.db");
    let engine = SqzEngine::with_preset_and_store(preset.clone(), &store_path)
        .expect("failed to create engine");

    // Compress some content
    let input = r#"{"status": "error", "message": "connection refused", "debug_info": null, "trace_id": null}"#;
    let result = engine.compress(input).unwrap();
    println!("Input:  {input}");
    println!("Output: {}", result.data);
    println!("Tokens: {} → {}", result.tokens_original, result.tokens_compressed);
    println!();

    // You can also serialize a preset back to TOML
    let round_tripped = PresetParser::to_toml(&preset).unwrap();
    println!("Serialized back to TOML:");
    println!("{round_tripped}");

    // Validation catches bad values
    let bad_toml = r#"
[preset]
name = ""
version = "1.0"

[compression]
stages = []

[tool_selection]
max_tools = 5
similarity_threshold = 0.7

[budget]
warning_threshold = 0.70
ceiling_threshold = 0.85
default_window_size = 200000

[terse_mode]
enabled = false
level = "moderate"

[model]
family = "anthropic"
complexity_threshold = 0.4
"#;

    match PresetParser::parse(bad_toml) {
        Ok(_) => println!("Unexpected success"),
        Err(e) => println!("Validation caught the error: {e}"),
    }
}
