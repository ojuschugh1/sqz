# sqz Token Savings Benchmark

A reproducible comparison of session-level token consumption between sqz and a competing execution-wrapping CLI compressor ("the wrapper" below) across realistic AI coding workflows.

## Methodology

Both tools were tested against the same command sequences on a medium-sized Rust/TypeScript project (~50 files, ~15k LOC). Each scenario simulates a real 30-minute AI coding session pattern.

All numbers are token counts estimated via `cl100k_base` (GPT-4 tokenizer). "Raw" means uncompressed command output. Lower is better.

## Scenario 1: Iterative File Reading (dedup + delta test)

The LLM reads `src/auth.rs` (2,000 tokens raw), edits 3 lines, reads it again, edits 2 more lines, reads it a third time. Then reads `src/db.rs` (1,500 tokens) which `auth.rs` imports.

| Step | Raw | wrapper | sqz |
|---|---:|---:|---:|
| Read auth.rs (1st) | 2,000 | 800 | 800 |
| Read auth.rs (2nd, 3 lines edited) | 2,000 | 800 | 75 (delta) |
| Read auth.rs (3rd, 2 more lines edited) | 2,000 | 800 | 60 (delta) |
| Read db.rs (1st) | 1,500 | 600 | 13 (pre-cached) |
| **Total** | **7,500** | **3,000** | **948** |
| **Savings vs raw** | — | 60% | **87%** |

sqz wins because of three features the wrapper doesn't have:
- **Delta encoding**: when a file changes by a few lines, sqz sends only the diff (~60-75 tokens) instead of re-compressing the full file (800 tokens). SimHash fingerprinting detects the near-duplicate in O(1), then LCS computes the line-level diff.
- **Dedup cache**: if the same content is read again without changes, sqz returns a 13-token reference. Delta encoding handles the near-miss case where content changed slightly.
- **Predictive pre-cache**: when auth.rs was read, sqz parsed its `use` imports and pre-cached db.rs. When the LLM reads db.rs, it's an instant dedup hit (13 tokens).

**Note:** These dedup savings apply when files are read through Bash commands (`cat`, `head`, etc.) intercepted by the PreToolUse hook. Claude Code's built-in Read tool bypasses shell hooks — neither tool can compress its output. PostToolUse hooks can view but not modify tool output ([confirmed limitation](https://github.com/anthropics/claude-code/issues/4544)). The MCP server (`sqz-mcp`) provides compressed file reading as an alternative tool that Claude can use instead of the built-in Read.

## Scenario 2: Test-Fix-Test Cycle

Run `cargo test` (15 tests across 3 suites, 2 failing), fix the code, run `cargo test` again (all passing).

| Step | Raw | wrapper | sqz |
|---|---:|---:|---:|
| cargo test (2 failures) | 5,000 | 500 | 500 |
| cargo test (all pass) | 5,000 | 500 | 375 |
| **Total** | **10,000** | **1,000** | **875** |
| **Savings vs raw** | — | 90% | **91%** |

Near-parity. Both tools skip passing tests and Compiling/Downloading noise, showing only failures with context. sqz's multi-suite aggregation produces a more compact success summary ("cargo test: 15 passed (3 suites)") and its block-based failure parser groups error context tightly.

## Scenario 3: Git Workflow

`git status`, `git diff` (50-line diff), `git add .`, `git commit -m "fix: auth"`, `git push`, `git log -5`.

| Step | Raw | wrapper | sqz |
|---|---:|---:|---:|
| git status | 300 | 60 | 45 |
| git diff | 1,200 | 300 | 280 |
| git add . | 50 | 5 | 3 (→ "ok") |
| git commit | 200 | 15 | 12 |
| git push | 150 | 10 | 8 (→ "ok main") |
| git log -5 | 500 | 100 | 85 |
| **Total** | **2,400** | **490** | **433** |
| **Savings vs raw** | — | 80% | **82%** |

Comparable. Both have git-specific formatters. sqz's `git status` formatter groups by staged/modified/untracked with counts. the wrapper's is similar.

## Scenario 4: JSON API Response Processing

Fetch a JSON API response (180 fields, 40 null values, nested metadata), process it twice.

| Step | Raw | wrapper | sqz |
|---|---:|---:|---:|
| API response (1st) | 4,000 | 1,600 | 1,200 |
| API response (2nd, identical) | 4,000 | 1,600 | 13 (dedup) |
| **Total** | **8,000** | **3,200** | **1,213** |
| **Savings vs raw** | — | 60% | **85%** |

sqz wins on first read (strip_nulls + TOON encoding vs the wrapper's field filtering) and dominates on second read (dedup cache).

## Scenario 5: Build Error Investigation

`cargo build` fails with 3 errors across 2 files. The LLM reads both files, fixes them, rebuilds.

| Step | Raw | wrapper | sqz |
|---|---:|---:|---:|
| cargo build (3 errors) | 3,000 | 600 | 450 |
| Read file1.rs | 2,000 | 800 | 800 |
| Read file2.rs | 1,500 | 600 | 600 |
| cargo build (success) | 500 | 50 | 30 (→ "ok (N crates compiled)") |
| Read file1.rs (verify, after fix) | 2,000 | 800 | 70 (delta) |
| **Total** | **9,000** | **2,850** | **1,950** |
| **Savings vs raw** | — | 68% | **78%** |

sqz's block-based build parser matches the wrapper's quality: it skips all Compiling/Downloading/Finished noise, groups errors with source context (up to 15 lines per error block), and produces a summary header ("cargo build: 3 errors, 0 warnings (12 crates)"). The gap comes from delta encoding on the verification re-read and the more compact success message.

## Scenario 6: Full 30-Minute Session (Combined)

Aggregate of all scenarios above, plus 10 additional `ls` calls, 5 `grep` calls, and 3 `docker ps` calls.


| Category | Raw | wrapper | sqz |
|---|---:|---:|---:|
| File reads (with repeats) | 7,500 | 3,000 | 948 |
| Test cycles | 10,000 | 1,000 | 875 |
| Git workflow | 2,400 | 490 | 433 |
| JSON API (with repeats) | 8,000 | 3,200 | 1,213 |
| Build errors + fix | 9,000 | 2,850 | 1,950 |
| ls/grep/docker (misc) | 5,000 | 1,200 | 1,050 |
| **Session total** | **41,900** | **11,740** | **6,469** |
| **Savings vs raw** | — | **72%** | **85%** |
| **Savings vs wrapper** | — | — | **45%** |

## Where the Wrapper Wins

- **Test runner formatting**: the wrapper's per-command formatters for `pytest`, `rspec`, `go test`, `vitest`, `playwright` are mature and deeply tuned per-runner. sqz has dedicated parsers for each (including `go test -json` event streams), but the wrapper's have more edge-case coverage from longer production use.
- **Execution wrapping**: the wrapper owns command execution and can inject `--json` flags (e.g., `go test -json`, `ruff --output-format=json`) to get structured output. sqz is post-hoc only — it parses whatever output the command produced, handling both JSON and human-readable formats but unable to request structured output.
- **Ruby ecosystem maturity**: the wrapper's `rspec`, `rubocop`, `rake`, and `bundler` formatters have longer production mileage; sqz's are newer.
- **Community**: it has more users, more contributors, more battle-tested edge cases.
- **Hook integrations**: it ships tested, documented hook scripts for 14 AI tools. sqz has verified hook configs for Claude Code, Cursor, Windsurf, Gemini CLI, Cline, Kiro, and OpenCode.

## Where sqz Wins

- **Session-level dedup**: The single biggest differentiator. the wrapper compresses each command output independently. sqz maintains a compaction-aware dedup cache across the entire session. Repeated file reads, identical API responses, and re-run commands return a 13-token reference instead of re-compressing. A turn-counter heuristic detects when refs may have gone stale (content compacted out of the LLM's context) and automatically re-sends the full compressed content. This is where the 45% gap comes from.
- **Delta encoding**: When a file changes by a few lines, sqz sends only the diff (SimHash fingerprinting for O(1) candidate detection, then LCS for the actual diff). the wrapper re-compresses the entire file.
- **Predictive pre-caching**: When sqz reads a file, it parses imports and pre-caches dependencies. When the LLM reads those files next, it's an instant dedup hit. the wrapper has no concept of file relationships.
- **Cross-command context refs**: When an error message references a file that's already in the dedup cache, sqz annotates it with `[in context]` so the LLM knows it doesn't need to re-read the file. the wrapper treats each command as isolated.
- **16-stage compression pipeline**: Beyond the basic formatters, sqz applies RLE, sliding window dedup, entropy-weighted truncation, self-information token pruning, dictionary compression, tabular array encoding, word abbreviation, and n-gram abbreviation. the wrapper has per-command formatters but no general-purpose compression pipeline.
- **TextRank extractive compression**: For long prose content, sqz uses graph-based sentence ranking (PageRank algorithm) to keep the most important sentences and drop the rest. the wrapper has no prose compression.
- **Session continuity**: `sqz resume` generates a session guide from the previous session's state. When the LLM restarts, it gets a 200-token summary instead of losing all context. the wrapper is stateless across sessions.
- **TOON encoding**: Lossless JSON compression format (4-30% reduction) with proven round-trip fidelity. the wrapper strips fields but doesn't have a compact encoding.
- **Compression quality metrics**: Shannon entropy-based efficiency measurement tells you how close sqz is to the theoretical compression optimum. the wrapper has no quality metrics.
- **Browser extension**: sqz has a WASM-powered Chrome/Firefox extension for ChatGPT/Claude.ai/Gemini/Grok/Perplexity with content classification, null stripping, condense, and in-memory dedup. the wrapper has nothing for browser-based AI.
- **IDE extensions**: sqz has VS Code and JetBrains plugins; the wrapper doesn't.
- **Zero telemetry**: sqz collects nothing; the wrapper collects anonymous daily metrics by default.

## The Core Difference

The wrapper compresses commands. sqz compresses sessions.

The wrapper treats each command output as an isolated compression problem. It's excellent at that — mature formatters, broad CLI coverage, fast.

sqz matches the wrapper's per-command formatter breadth (40+ commands across 9 ecosystems) while maintaining its session-level advantages: what files have been read, what their dependencies are, what content is already in the context window, and whether refs have gone stale due to compaction. This session awareness — combined with a 16-stage compression pipeline, delta encoding, SimHash fingerprinting, TextRank extractive compression, and compaction-aware dedup — is what produces the 46% improvement in realistic workflows where the same content appears multiple times.

For a single `cargo test` call, both tools now produce similarly compact output. Over a 30-minute coding session with iterative file reads, test-fix cycles, and repeated API calls, the gap compounds.

## Reproduce These Numbers

```sh
cargo install sqz-cli

# Run the benchmark suite
cargo test -p sqz-engine benchmarks -- --nocapture

# Track your own session savings
sqz gain --days 7
```

The benchmark suite is in `sqz_engine/src/benchmarks.rs`. Each scenario is a deterministic test with fixed input data — no network calls, no randomness, fully reproducible.
