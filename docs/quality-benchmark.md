# Context Compression Quality Benchmark: Compression Ratio vs Information Preservation

Token reduction is easy to inflate. Drop the one line the agent needed and the headline number goes up while the agent re-runs the command, retries in circles, or ships a worse answer. This benchmark measures both sides of that trade for sqz, on the exact code path the shell hook uses, and it runs as a regression test on every change.

Reproduce it:

```sh
cargo test -p sqz-cli --test quality_bench -- --nocapture
```

Source: [`sqz/tests/quality_bench.rs`](../sqz/tests/quality_bench.rs). The fixtures, facts and gates are all in that one file.

## What is measured

Each fixture is realistic command output paired with a list of facts an agent would need from it: the failing test name, the assertion, the `file:line`, the changed lines of a diff, the pod in `CrashLoopBackOff`, the vulnerability count. The fixture is pushed through the real `sqz` binary exactly as the hook does (`sqz compress --cmd <cmd>` on stdin, isolated database via `SQZ_DB_PATH`), and five things are recorded:

| Metric | How |
|---|---|
| Token reduction | `ceil(bytes / 4)` on input vs everything the agent sees: compressed stdout plus sqz's own stderr lines. The `[sqz] N/M tokens` header is counted against sqz. |
| Critical-fact recall | Every fact must appear verbatim in the compressed stdout. One missed fact fails the test. |
| Noise dropped | Lines the fixture expects compression to remove (progress dots, deprecation chatter, passing tests). Reported, not gated. |
| Verbatim preservation | Stack traces and secrets must come out byte-identical to the input. |
| Byte-exact recovery | A second run of the same output must return a `§ref:HASH§` dedup token, and `sqz expand HASH` must return the original bytes. Sensitive content must instead never be persisted. |

Facts are chosen for what an agent needs to act, not for what sqz happens to keep. When the two disagreed during development, the formatter was fixed, not the fixture (see the bug list below).

## Results

sqz `main` after 1.6.1, September 2026, default settings (dedup on, abbreviation on).

| Fixture | Command | Tokens in | Tokens out | Reduction | Facts kept | Noise dropped | Recovery |
|---|---|---:|---:|---:|---:|---:|---|
| cargo test failure | `cargo test` | 303 | 123 | 59.4% | 5/5 | 2/3 | byte-exact |
| pytest failure | `pytest` | 311 | 71 | 77.2% | 5/5 | 3/3 | byte-exact |
| tsc errors | `tsc --noEmit` | 126 | 92 | 27.0% | 6/6 | 2/2 | byte-exact |
| git diff | `git diff` | 148 | 148 | 0.0% | 7/7 | 0/1 | byte-exact |
| git status | `git status` | 155 | 82 | 47.1% | 6/6 | 3/3 | byte-exact |
| npm install | `npm install` | 161 | 39 | 75.8% | 4/4 | 2/2 | byte-exact |
| kubectl get pods | `kubectl get pods` | 183 | 110 | 39.9% | 10/10 | n/a | byte-exact |
| docker ps | `docker ps` | 193 | 79 | 59.1% | 6/6 | 1/1 | byte-exact |
| JSON API response | `curl .../orders/ord_8812` | 129 | 127 | 1.6% | 11/11 | 1/1 | byte-exact |
| app log with repeats | `tail -n 200 app.log` | 1025 | 135 | 86.8% | 3/3 | 2/2 | byte-exact |
| source file read | `cat src/billing/refund.py` | 221 | 221 | 0.0% | 5/5 | n/a | byte-exact |
| python traceback | `python app.py` | 82 | 82 | 0.0% | 4/4 | n/a | byte-exact |
| dotenv with secrets | `cat .env` | 78 | 78 | 0.0% | 4/4 | n/a | passthrough, never cached |
| **Aggregate** | | **3,115** | **1,387** | **55.5%** | **76/76** | | |

Regression gates enforced by the test: 100% fact recall on every fixture, byte-identical output for the traceback and secrets fixtures, byte-exact recovery for everything else, no persistence of the secrets fixture, and at least 40% aggregate reduction across the compressible fixtures.

### Reading the zeros

The 0.0% rows are deliberate and are the point of the benchmark.

- **git diff.** Every changed line is a fact. sqz's diff formatter found nothing safe to remove, and the generic pipeline agreed, so the output passed through untouched with no stats header. An earlier build reported this same fixture as a 15% reduction because it compared a BPE token count for the input against a bytes/4 estimate for the output. That mismatch inflated every pipeline-path number in `sqz stats` and is fixed.
- **source file read.** An agent that runs `cat` on a file is usually about to edit it. The pipeline now detects source code and skips every line-dropping stage. Before this, the entropy truncator scored the imports and the `class` line as "low information" and removed them. The saving on file reads comes from dedup instead: the second read is 13 tokens.
- **python traceback, dotenv.** High-risk content is routed to safe mode and passed through unchanged. Secrets are additionally never written to the cache, which the test proves by checking that a repeat run does not produce a dedup reference.

The **JSON API** row is honest too: minified JSON has almost nothing to squeeze, and stripping the `null` fields roughly pays for the header line.

## Regret: the metric that catches over-compression in the field

A fixture can only test facts someone thought of. For everything else, sqz tracks two signals in normal use and prints them in `sqz stats`:

- **Quick re-runs.** The agent produced byte-identical output within 120 seconds of last seeing it (`SQZ_REGRET_WINDOW_SECS` to tune, `0` to disable). The repeat bought no new information, which usually means the compressed first serving was missing something. Grouped by command, so the formatter at fault is visible.
- **Ref expands.** A `§ref:HASH§` token was expanded back to its original bytes. A recovery round trip.

These are reported next to the savings total as a rate, not subtracted from it. The token cost of the re-run itself is small and misleading: the second serving is identical, so it hits the dedup cache and goes out as a 13-token reference. The real cost is the wasted agent turn, which sqz cannot see from tool output alone. Any dollar figure for it would be an invented multiplier, so `sqz stats --cost` models direct token cost only and says so.

## What this benchmark does not measure

- **Agent task success.** Whether a model given the compressed output completes the task as often as with the raw output. This needs a live model, a task set and many runs, and it is the number that matters most. Fact recall is the proxy used here because it is deterministic and cheap enough to run on every commit.
- **Real tokenizer counts.** The table uses bytes/4 so the numbers match what `sqz` prints in its header. `cargo test -p sqz-engine cmd_formatter_bench` re-measures the formatter fixtures with the cl100k BPE tokenizer and checks the two estimates agree within 10 points.
- **Session-level effects.** Dedup across a whole session (the 13-token repeat) is where most real savings come from. That is measured separately in [benchmark.md](benchmark.md) and in the `sqz stats` figures on the README, which come from real usage, not fixtures.
- **Abbreviation losses.** N-gram abbreviation replaces repeated phrases with `«A1»` symbols plus a legend. A fact inside a repeated phrase still counts as kept if it appears in the legend. Use `--no-abbrev` or `SQZ_NO_ABBREV=1` if an agent struggles with the symbols.

## Bugs this benchmark found on its first run

Listed because they are the kind of loss that a ratio-only benchmark hides, and because they are now regression-tested.

| Command | What was lost | Fix |
|---|---|---|
| `npm install` | The `3 vulnerabilities (1 moderate, 2 high)` line and the `npm audit fix` hint. An early `return` on the "added N packages" line skipped everything after it. | Collect all summary lines. |
| `docker ps` | Columns mis-parsed whenever COMMAND or CREATED contained spaces (`"uvicorn app:app"`, `2 hours ago`), so status and ports landed in the wrong cells. | Slice rows at the header's column offsets instead of splitting on whitespace. |
| `git status` | The branch name and ahead/behind line. | Keep the leading branch and tracking lines; drop only "up to date". |
| `pytest` | The `file:line` of the failure, the failing source line and the `1 failed, 41 passed` totals. Only the short-summary line survived. | Dedicated pytest formatter that keeps totals, `>` and `E` lines and the location. |
| `tsc` | Everything, when tsc used its `--pretty` format: the formatter only knew the piped `file(line,col)` layout, so output fell through to the generic pipeline, which dropped `Found 2 errors`. | Parse both formats, keep tsc's own summary line. |
| `cat file.py` | Imports and the `class` declaration, scored as low-information by the entropy truncator. | Source-code detection disables line-dropping stages. |
| any passthrough | `sqz stats` recorded savings on output that was returned unchanged, because input and output were counted with different estimators. | Count both with the same tokenizer; apply the 16-token net-win gate on the formatter path too. |

## Adding a fixture

Add a `fx(...)` entry to `fixtures()` in [`sqz/tests/quality_bench.rs`](../sqz/tests/quality_bench.rs) with the raw output, the facts an agent would need, and optional noise lines. If a fact goes missing, the test fails with the fixture name and the missing strings. If you use a command daily and sqz mangles it, that fixture plus the failing test is the best issue report you can file.
