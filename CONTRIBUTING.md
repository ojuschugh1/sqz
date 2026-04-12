# Contributing to sqz

Thanks for your interest in contributing. Here's how to get started.

## Setup

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
```

If all 260+ tests pass, you're ready.

## Development Workflow

1. Create a branch from `main`
2. Make your changes
3. Run `cargo test --workspace` — all tests must pass
4. Run `cargo clippy --workspace` — no warnings
5. Submit a pull request

## Code Style

- Follow standard Rust formatting (`cargo fmt`)
- Add tests for new functionality — property-based tests (proptest) preferred for core logic
- Keep modules focused — one responsibility per file
- Use `thiserror` for error types, `serde` for serialization
- Document public APIs with doc comments

## Architecture Rules

- All compression logic lives in `sqz_engine` — integration surfaces are thin adapters
- No network requests from the core engine (Requirement 23)
- No telemetry, no analytics, no crash reports (Requirement 23)
- Property-based tests for all correctness properties in the design doc
- `cargo check --workspace` must pass before committing

## Adding a New Compression Stage

1. Implement the `CompressionStage` trait in `sqz_engine/src/stages.rs`
2. Add it to the pipeline in `sqz_engine/src/pipeline.rs`
3. Add preset config fields in `sqz_engine/src/preset.rs`
4. Write property tests validating the stage's behavior
5. Update the default preset

## Adding a New Platform Integration

1. Add a config snippet in `docs/integrations/`
2. Add the platform to the compatibility matrix in the README
3. If the platform supports hooks, add hook config generation to `sqz init --agent <platform>`

## CLA

By submitting a pull request, you agree to the [Contributor License Agreement](CLA.md). This allows us to offer sqz under additional license terms (including commercial licenses) in the future.

## Reporting Issues

- Use GitHub Issues
- Include: OS, sqz version (`sqz --version`), steps to reproduce, expected vs actual behavior
- For test failures: include the full `cargo test` output
