# sqz Compression Benchmarks

Reproducible benchmark results from the sqz compression engine.
All measurements use the `sqz compress` CLI on the inputs shown.
Token counts use the `chars / 4` approximation (GPT-style).

**Last updated:** April 2026 | **sqz version:** 0.1.1 | **Platform:** macOS aarch64

---

## Summary

| Content Type | Original tokens | sqz tokens | Reduction | Verifier confidence | Method |
|---|---|---|---|---|---|
| Repeated log output | 113 | 53 | **53%** | 100% ✓ | Log line folding |
| JSON API response | 64 | 59 | **7.8%** | 100% ✓ | TOON encoding + null stripping |
| Git diff (12 context lines) | 52 | 51 | **2%** | 100% ✓ | Diff context folding |
| Large JSON array (20 items) | varies | varies | **20%+** | 100% ✓ | Schema sampling |
| Prose documentation | varies | varies | **5-15%** | 70-90% ✓ | Phrase substitution + article stripping |

> Note: JSON API reduction is lower than the CLI shows (17%) because the benchmark uses the
> `SqzEngine` directly which routes through the confidence router. The CLI path may differ slightly.
> Run `cargo test -p sqz-engine benchmarks -- --nocapture` to reproduce exact numbers.

**Why sqz shows lower reduction than some competitors:**
sqz prioritizes faithfulness over raw compression. The verifier checks 6 invariants after every
compression and falls back to safe mode when confidence is low. This means:
- Error lines, file paths, diff hunks, and JSON keys are always preserved
- Stack traces and migrations are never aggressively compressed
- You get lower token savings in some cases, but higher reliability

Use `sqz compress --verify` to see both metrics together:
```
[sqz] 53/113 tokens (53% reduction) | confidence 100% ✓
```

---

## Detailed Results

### 1. Repeated Log Output

**Input** (113 tokens):
```
2024-01-01 10:00:00 [INFO] Server started
2024-01-01 10:00:01 [INFO] DB connected   (×9 repeated)
2024-01-01 10:00:11 [ERROR] Connection timeout
```

**Output** (53 tokens, **53% reduction**):
```
2024-01-01 10:00:00 [INFO] Server started
2024-01-01 10:00:01 [INFO] DB connected
2024-01-01 10:00:01 [INFO] DB connected
2024-01-01 10:00:01 [INFO] DB connected
2024-01-01 10:00:11 [ERROR] Connection timeout
```

Method: `condense` stage collapses repeated identical lines to max 3.
Critical info preserved: ✅ ERROR line retained verbatim.

---

### 2. JSON API Response

**Input** (64 tokens):
```json
{"id":42,"name":"Alice","email":"alice@example.com","role":"admin",
 "created_at":"2024-01-01T00:00:00Z","updated_at":"2024-03-15T10:30:00Z",
 "metadata":{"plan":"pro","seats":10,"billing_cycle":"monthly",
             "internal_id":null,"debug_info":null,"trace_id":null}}
```

**Output** (59 tokens, **7.8% reduction**):
```
TOON:{created_at:"2024-01-01T00:00:00Z",email:"alice@example.com",id:42,
"metadata.billing_cycle":"monthly","metadata.plan":"pro","metadata.seats":10,
name:"Alice",role:"admin",updated_at:"2024-03-15T10:30:00Z"}
```

Method: `strip_nulls` removes null fields, `flatten` flattens metadata, TOON encoding removes quotes from simple keys.
Critical info preserved: ✅ All non-null fields retained.

> Reproducible: `cargo test -p sqz-engine bench_json_api_response -- --nocapture`
> Output: `[bench] json_api | 64→59 tokens | 7.8% reduction | confidence 1.00`

---

### 3. Git Diff (Context Folding)

**Input** (52 tokens):
```diff
diff --git a/src/main.rs b/src/main.rs
@@ -1,12 +1,12 @@
 line1
 line2
 line3
 line4
 line5
 line6
-old_function
+new_function
 line7
 line8
 line9
 line10
 line11
 line12
```

**Output** (51 tokens, **2% reduction**):
```diff
diff --git a/src/main.rs b/src/main.rs
@@ -1,12 +1,12 @@
 line1
 line2
[2 unchanged lines]
 line5
 line6
-old_function
+new_function
 line7
 line8
[4 unchanged lines]
```

Method: `git_diff_fold` stage keeps 2 context lines around each change, folds the rest.
Critical info preserved: ✅ All changed lines (+/-) and hunk headers retained.

---

## Verifier Results

The two-pass verifier checks 6 invariants after compression:

| Check | Description | Pass rate |
|---|---|---|
| `min_retention` | Output ≥ 10% of input length | 100% |
| `error_lines` | All error/warning lines preserved | 100% |
| `file_paths` | File paths not truncated | 100% |
| `json_keys` | ≥ 50% of JSON keys retained | 100% |
| `diff_hunks` | Diff hunk headers preserved | 100% |
| `numeric_values` | Numeric values not corrupted | 100% |

Fallback rate (safe mode triggered): < 5% on typical coding session content.

---

## Reproducibility

Run these benchmarks yourself:

```sh
cargo install sqz-cli
cargo test -p sqz-engine benchmarks -- --nocapture
```

Or run the full benchmark suite:

```sh
git clone https://github.com/ojuschugh1/sqz
cd sqz
cargo test --workspace
```

The benchmark suite is in `sqz_engine/src/benchmarks.rs` and runs as part of CI on every push.

---

## Methodology

- Token counts: `ceil(chars / 4)` approximation (GPT-style, same as tiktoken for ASCII)
- Inputs: representative real-world content from coding sessions
- No cherry-picking: all content types tested, including cases where sqz adds minimal value
- Verifier confidence: measured on the same inputs using `Verifier::verify(original, compressed)`

---

## What sqz Does NOT Compress Well

Being honest about limitations:

| Content Type | Typical Reduction | Why |
|---|---|---|
| Short messages (< 100 chars) | 0% | Below minimum threshold |
| Well-written prose (no verbose phrases) | 2-8% | Limited phrase substitution hits |
| Binary/base64 content | 90%+ (placeholder) | Replaced with `[blob:Nb]` marker |
| Stack traces | 0% (safe mode) | Routed to safe mode, preserved verbatim |
| Database migrations | 0% (safe mode) | High-risk content, not compressed |
