# sqz Compression Benchmarks

Reproducible benchmark results from the sqz compression engine.
The numbers come from `cargo test` over `SqzEngine::compress`
(`sqz_engine/src/benchmarks.rs`) and over the per-command formatters
(`format_command`, `sqz_engine/src/cmd_formatter_bench.rs`), not from the
`sqz` CLI or the shell hook.
Token counts use the `chars / 4` approximation (GPT-style).

**Verified under a real tokenizer:** the formatter fixtures are also
re-measured with the real `cl100k_base` BPE tokenizer on every test run
(`cargo test -p sqz-engine bench_reductions_hold_under_real_bpe -- --nocapture`).
Latest run: chars/4 reports 63.7% aggregate reduction, cl100k reports
74.0%, so on these fixtures chars/4 understates the reduction by 10.4
points. The test fails CI if chars/4 ever overstates the cl100k reduction
by 10 points or more.

**Last updated:** October 2026 | **sqz version:** main after 1.9.0 | **Platform:** macOS aarch64 (Apple M4 Pro)

---

## Summary

| Content Type | Original tokens | sqz tokens | Reduction | Verifier confidence | Method |
|---|---|---|---|---|---|
| Repeated log output | 148 | 39 | **73.6%** | 100% ✓ | Repeated lines condensed, then folded into one line |
| JSON API response | 64 | 53 | **17.2%** | 100% ✓ | Nulls stripped, `metadata` flattened, TOON encoding |
| Git diff (15 context lines) | 61 | 54 | **11.5%** | 86% ⚠ | Diff context folding |
| Large JSON array (20 items) | 259 | 111 | **57.1%** | 100% ✓ | TOON table: one header row, one line per item |
| Prose documentation | 124 | 122 | **1.6%** | 100% ✓ | Token pruning removed two words |
| Stack trace | 82 | 82 | **0.0%** | 100% ✓ | Routed to safe mode, passed through unchanged |

> Run `cargo test -p sqz-engine benchmarks -- --nocapture` to reproduce exact numbers.

**Verifier and safe mode:**
After every compression the verifier runs 7 checks (table below) and turns them into a
confidence score. Below 0.6, sqz re-compresses the input with the safe preset. That re-run
skips the configured lossy JSON stages but still runs the generic stages (JSON projection,
TOON encoding, and entropy truncation on long text), so it does not keep everything; on the
shell hook the original is cached, and when segments were dropped a
`[full output: sqz expand ...]` line points at it. Stack traces, SQL migrations, PEM blocks
and credential lines are routed to safe mode up front and pass through unchanged at any
size; they are never cached. On the shell hook a per-command formatter may still summarize
the rest of a test run around them, but only when it keeps every line of their paragraphs.
Output that only mentions a credential, trace or migration is compressed as usual but is
not cached either, so what it loses cannot be expanded.

Use `sqz compress --mode default --verify` to see both metrics together (repeated log input
below; the CLI counts cl100k tokens, not chars/4):
```
[sqz] 62/230 tokens (73% reduction) | confidence 100% ✓
```

---

## Detailed Results

### 1. Repeated Log Output

**Input** (148 tokens):
```
2024-01-01 09:59:59 [INFO] Server started
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:00 [INFO] Connected to database
2024-01-01 10:00:11 [ERROR] Connection timeout after 30s
```

**Output** (39 tokens, **73.6% reduction**):
```
2024-01-01 09:59:59 [INFO] Server started
2024-01-01 10:00:00 [INFO] Connected to database [×10]
2024-01-01 10:00:11 [ERROR] Connection timeout after 30s
```

Method: `condense` cuts the run of identical lines to 3, then `rle` folds them into one line marked `[×10]`, the number of times the line appeared in the input.
Critical info preserved: ✅ ERROR line retained verbatim.

---

### 2. JSON API Response

**Input** (64 tokens):
```json
{"id":42,"name":"Alice","email":"alice@example.com","role":"admin","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-03-15T10:30:00Z","metadata":{"plan":"pro","seats":10,"billing_cycle":"monthly","internal_id":null,"debug_info":null,"trace_id":null}}
```

**Output** (53 tokens, **17.2% reduction**):
```
TOON:{created_at:"2024-01-01T00:00:00Z",email:"alice@example.com",id:42,"metadata.billing_cycle":"monthly","metadata.plan":"pro","metadata.seats":10,name:"Alice",role:"admin",updated_at:"2024-03-15T10:30:00Z"}
```

Method: `strip_nulls` removes null fields, `flatten` flattens metadata, TOON encoding removes quotes from simple keys.
Critical info preserved: ✅ All non-null fields retained.

> Reproducible: `cargo test -p sqz-engine bench_json_api_response -- --nocapture`
> Output: `[bench] json_api | 64→53 tokens | 17.2% reduction | confidence 1.00`

---

### 3. Git Diff (Context Folding)

**Input** (61 tokens):
```diff
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,15 +1,15 @@
 line1
 line2
 line3
 line4
 line5
 line6
 line7
 line8
-old_function_name
+new_function_name
 line9
 line10
 line11
 line12
 line13
 line14
 line15
```

**Output** (54 tokens, **11.5% reduction**):
```diff
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,15 +1,15 @@
 line1
 line2
[4 unchanged lines]
 line7
 line8
-old_function_name
+new_function_name
 line9
 line10
[5 unchanged lines]
```

Method: `git_diff_fold` keeps the first two lines of the hunk and two context lines on each side of the change, and folds the other runs into `[N unchanged lines]`. Verifier confidence is 0.86 (6 of 7 checks passed), above the 0.6 fallback threshold, so this output is used.
Critical info preserved: ✅ All changed lines (+/-), file headers and the hunk header retained.

---

### 4. Per-Command Formatters

Command-specific formatters run before the general compression pipeline and produce
high savings for known commands. Every number below is measured by a fixture test
in `sqz_engine/src/cmd_formatter_bench.rs`; the test inputs are the fixtures behind
these figures, and each row is gated so the numbers can't silently drift:

| Command | Input (tokens) | Output (tokens) | Reduction | Method |
|---|---:|---:|---:|---|
| `npm install` (200 packages) | 1,186 | 13 | **99%** | Summary line only |
| `cargo test` (15 pass, 3 suites) | 171 | 8 | **95%** | Skip pass lines, aggregate suites |
| `cargo build` (success, 30 crates) | 233 | 23 | **90%** | Skip Compiling noise, status line |
| `terraform plan` (2 resources) | 325 | 121 | **63%** | Drop refresh and progress lines |
| `cargo clippy` (5 warnings) | 163 | 70 | **57%** | Group by rule + location |
| `git status` (verbose, 10 files) | 102 | 56 | **45%** | Branch line + compact staged/modified/untracked |
| `grep` (100 matches, 5 files) | 1,607 | 1,085 | **32%** | ripgrep heading layout: each path once, every match under it |

The savings are largest on the verbose, repetitive
output (package lists, passing-test runs) and smaller on output that
is already compact (a short `git status`, a handful of clippy warnings with source
context). The formatters cover git, cargo, npm/pnpm/yarn/bun, tsc, eslint/biome, pytest,
ruff, mypy, pip, go, golangci-lint, gradle, maven, xcodebuild, adb logcat, dotnet, rspec,
rubocop, rake/rails, bundle, docker/podman (ps, images), kubectl (describe, apply),
terraform, grep/rg/ag, tree and curl/wget (dispatch in `sqz_engine/src/cmd_formatters/mod.rs`).
Other commands fall through to the general compression pipeline (0% to 73.6% on the
engine benchmark inputs above, depending on content type).

Reproduce: `cargo test -p sqz-engine cmd_formatter_bench -- --nocapture`

---

## Verifier Results

The two-pass verifier checks 7 invariants after compression (`sqz_engine/src/verifier.rs`):

| Check | Description |
|---|---|
| `min_retention` | Output is at least 10% of the input length |
| `error_lines` | Lines with `error:`, `warning:`, `fatal:`, `panic:` or `exception:` are kept |
| `file_paths` | The first 20 path-like lines (under 200 characters) are kept |
| `json_keys` | At least 50% of top-level JSON keys are kept |
| `diff_hunks` | Diff hunk headers (`@@`) are kept |
| `numeric_values` | The first 10 numbers of two or more characters are kept |
| `preservation` | At least 85% of paths, URLs, backticked code, environment variables and version strings are kept |

Confidence is the share of checks that passed, capped at 0.5 when `preservation` fails.
Below 0.6, sqz falls back to the safe preset (see above).

---

## Reproducibility

Run these benchmarks yourself:

```sh
git clone https://github.com/ojuschugh1/sqz
cd sqz
cargo test -p sqz-engine benchmarks -- --nocapture
cargo test -p sqz-engine cmd_formatter_bench -- --nocapture
```

Or run the full benchmark suite:

```sh
cargo test --workspace
```

The benchmark suite is in `sqz_engine/src/benchmarks.rs` and runs as part of CI on every push.

---

## Methodology

- Token counts: `ceil(chars / 4)` approximation (GPT-style)
- Inputs: representative real-world content from coding sessions
- No cherry-picking: all content types tested, including cases where sqz adds minimal value
- Verifier confidence: measured on the same inputs using `Verifier::verify(original, compressed)`

---

## What sqz Does NOT Compress Well

Being honest about limitations:

| Content Type | Typical Reduction | Why |
|---|---|---|
| Short messages (< 100 chars) | 0% | Below minimum threshold |
| Prose | 1.6% on the benchmark paragraph | Token pruning removes a few filler words; no other stage changes it |
| Stack traces | 0% (safe mode) | Routed to safe mode and passed through unchanged at any size |
| Database migrations | 0% (safe mode) | Same safe-mode routing as stack traces |
