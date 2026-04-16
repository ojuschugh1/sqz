//! JSON compression with TOON encoding.
//!
//! Demonstrates how JSON input gets automatically detected and encoded into
//! TOON (Token-Optimized Object Notation) for 30-60% fewer tokens.
//!
//! Run with: `cargo run --example json_compress -p sqz-engine`

use sqz_engine::{SqzEngine, ToonEncoder};

fn main() {
    let engine = SqzEngine::new().expect("failed to init engine");

    // JSON with null fields and debug info — sqz strips the noise
    let json_input = r#"{
        "id": 42,
        "name": "Alice",
        "email": "alice@example.com",
        "debug_info": null,
        "trace_id": null,
        "metadata": {
            "internal_id": "abc123",
            "created_at": "2025-01-15T10:30:00Z"
        },
        "tags": ["rust", "compression", "llm"]
    }"#;

    let result = engine.compress(json_input).unwrap();

    println!("=== JSON → TOON ===");
    println!("input length:  {} chars", json_input.len());
    println!("output length: {} chars", result.data.len());
    println!("tokens: {} → {} ({:.0}% reduction)",
        result.tokens_original,
        result.tokens_compressed,
        (1.0 - result.compression_ratio) * 100.0,
    );
    println!("stages: {:?}", result.stages_applied);
    println!();
    println!("TOON output:");
    println!("{}", result.data);
    println!();

    // You can decode TOON back to JSON if needed
    if result.data.starts_with("TOON:") {
        let decoded = ToonEncoder.decode(&result.data).unwrap();
        println!("Decoded back to JSON:");
        println!("{}", serde_json::to_string_pretty(&decoded).unwrap());
    }
}
