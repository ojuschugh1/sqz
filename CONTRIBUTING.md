# Contributing to sqz

Thanks for your interest in contributing. Here's how to get started.

## Setup

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
```

If all tests pass, you're ready.

## Building

The workspace has four crates:

| Crate | What it is |
|---|---|
| `sqz_engine` | Core compression library — all the logic lives here |
| `sqz` | CLI binary (`sqz compress`, `sqz init`, etc.) |
| `sqz-mcp` | MCP server (stdio/SSE transport) |
| `sqz-wasm` | WASM build for browser extensions |

Build everything:

```sh
cargo build --workspace
```

Build a single crate:

```sh
cargo build -p sqz-engine
```

Generate docs:

```sh
cargo doc --workspace --no-deps --open
```

## Running Tests

Run all tests across all crates:

```sh
cargo test --workspace
```

Run tests for a single crate:

```sh
cargo test -p sqz-engine
```

Run a specific test by name:

```sh
cargo test -p sqz-engine test_compress_json_applies_toon
```

Run only the property-based tests (proptest):

```sh
cargo test -p sqz-engine prop_
```

## Code Style

- Run `cargo fmt` before committing — standard Rust formatting, no custom rules
- Run `cargo clippy --workspace` — no warnings allowed
- Match the patterns you see in existing code. If the file uses `thiserror`, use `thiserror`. If it uses `serde`, use `serde`. Don't introduce new libraries for things we already handle
- Doc comments on all public items. Keep them short and practical — say what the function does, not why it's amazing
- Use `///` doc comments with code examples where it helps. Examples must compile (`cargo test --doc`)

## Adding a New Compression Stage

1. Create your stage struct in `sqz_engine/src/stages.rs` (or a new file if it's complex)
2. Implement the `CompressionStage` trait:
   ```rust
   impl CompressionStage for MyStage {
       fn name(&self) -> &str { "my_stage" }
       fn priority(&self) -> u32 { 50 } // lower = runs earlier
       fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
           // mutate content.raw in place
           Ok(())
       }
   }
   ```
3. Add it to the pipeline in `sqz_engine/src/pipeline.rs` — both in `new()` and `reload_preset()`
4. Add config fields in `sqz_engine/src/preset.rs` (struct + TOML deserialization)
5. Wire up the config in `stage_config_from_preset()` in `pipeline.rs`
6. Write property-based tests (proptest) that validate the stage's invariants
7. Update the default preset in `Preset::default()` if the stage should be on by default

## Adding a New CLI Command

1. Add a variant to the `Command` enum in `sqz/src/main.rs`
2. Add the match arm in `main()` that dispatches to your `cmd_*` function
3. Write the `cmd_*` function — follow the pattern of existing commands
4. Use `require_engine()` to get an `SqzEngine` instance
5. Print user-facing output to stdout, diagnostics to stderr (prefixed with `[sqz]`)

## Architecture Rules

- All compression logic lives in `sqz_engine`. The CLI, MCP server, and WASM crate are thin adapters — they should not contain compression logic
- No network requests from the core engine
- No telemetry, no analytics, no crash reports
- Property-based tests (proptest) for all correctness properties
- `cargo check --workspace` must pass before committing

## PR Process

1. Fork the repo and create a branch from `main`
2. Make your changes
3. Run the full check:
   ```sh
   cargo fmt --check
   cargo clippy --workspace
   cargo test --workspace
   cargo doc --workspace --no-deps
   ```
4. Write a clear commit message explaining what and why
5. Submit a pull request against `main`
6. Address review feedback

## Reporting Issues

- Use GitHub Issues
- Include: OS, sqz version (`sqz --version`), steps to reproduce, expected vs actual behavior
- For test failures: include the full `cargo test` output

## CLA

By submitting a pull request, you agree to the [Contributor License Agreement](CLA.md).
