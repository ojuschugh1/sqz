//! Basic compression example.
//!
//! Shows how to create an engine and compress different types of content.
//!
//! Run with: `cargo run --example basic_compress -p sqz-engine`

use sqz_engine::SqzEngine;

fn main() {
    let engine = SqzEngine::new().expect("failed to init engine");

    // Plain text passes through mostly unchanged
    let plain = "hello world, this is a simple string";
    let result = engine.compress(plain).unwrap();
    println!("=== Plain text ===");
    println!("input:  {plain}");
    println!("output: {}", result.data);
    println!("tokens: {} → {}", result.tokens_original, result.tokens_compressed);
    println!("stages: {:?}", result.stages_applied);
    println!();

    // Repeated lines get condensed
    let repeated = "Loading...\nLoading...\nLoading...\nLoading...\nLoading...\nDone.";
    let result = engine.compress(repeated).unwrap();
    println!("=== Repeated lines ===");
    println!("input lines:  {}", repeated.lines().count());
    println!("output lines: {}", result.data.lines().count());
    println!("output:\n{}", result.data);
    println!();

    // compress_or_passthrough never fails — returns input unchanged on error
    let safe_result = engine.compress_or_passthrough("anything goes here");
    println!("=== Passthrough (never fails) ===");
    println!("output: {}", safe_result.data);
    println!("ratio:  {:.2}", safe_result.compression_ratio);
}
