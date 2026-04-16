//! Dedup cache demonstration.
//!
//! Shows how the CacheManager returns a tiny reference token (~13 tokens)
//! on the second read of identical content, instead of re-compressing.
//!
//! Run with: `cargo run --example cache_dedup -p sqz-engine`

use std::path::Path;

use sqz_engine::cache_manager::{CacheManager, CacheResult};
use sqz_engine::pipeline::CompressionPipeline;
use sqz_engine::preset::Preset;
use sqz_engine::session_store::SessionStore;

fn main() {
    // Set up a temp store and cache manager
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let store_path = dir.path().join("cache_demo.db");
    let store = SessionStore::open_or_create(&store_path).expect("failed to open store");
    let cache = CacheManager::new(store, 512 * 1024 * 1024); // 512 MB limit

    let preset = Preset::default();
    let pipeline = CompressionPipeline::new(&preset);

    let content = br#"{"users": [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]}"#;
    let path = Path::new("api_response.json");

    // First read: cache miss — full compression
    let first = cache.get_or_compress(path, content, &pipeline).unwrap();
    match &first {
        CacheResult::Fresh { output } => {
            println!("First read: MISS (fresh compression)");
            println!("  output: {}", output.data);
            println!("  tokens: {}", output.tokens_compressed);
        }
        _ => println!("First read: unexpected result"),
    }

    println!();

    // Second read: cache hit — returns a ~13-token reference
    let second = cache.get_or_compress(path, content, &pipeline).unwrap();
    match &second {
        CacheResult::Dedup { inline_ref, token_cost } => {
            println!("Second read: HIT (dedup reference)");
            println!("  ref: {inline_ref}");
            println!("  tokens: {token_cost}");
        }
        _ => println!("Second read: unexpected result"),
    }

    println!();

    // Modified content: cache miss again
    let modified = br#"{"users": [{"id": 1, "name": "Alice"}, {"id": 3, "name": "Carol"}]}"#;
    let third = cache.get_or_compress(path, modified, &pipeline).unwrap();
    match &third {
        CacheResult::Fresh { output } => {
            println!("Modified content: MISS (re-compressed)");
            println!("  tokens: {}", output.tokens_compressed);
        }
        CacheResult::Delta { delta_text, token_cost, similarity } => {
            println!("Modified content: DELTA (near-duplicate)");
            println!("  similarity: {similarity:.2}");
            println!("  delta tokens: {token_cost}");
            println!("  delta: {delta_text}");
        }
        _ => println!("Modified content: unexpected result"),
    }
}
