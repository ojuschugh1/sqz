<p align="center">
  <pre>
  ███████╗ ██████╗ ███████╗
  ██╔════╝██╔═══██╗╚══███╔╝
  ███████╗██║   ██║  ███╔╝
  ╚════██║██║▄▄ ██║ ███╔╝
  ███████║╚██████╔╝███████╗
  ╚══════╝ ╚══▀▀═╝ ╚══════╝
  The Context Intelligence Layer
  </pre>
</p>

<p align="center">
  <strong>Compress LLM context to save tokens and reduce costs</strong> — Shell Hook + MCP Server + Browser Extension + IDE Extensions
</p>

<p align="center">
  <em>sqz: Compress what is safe, preserve what is critical.</em>
</p>

<p align="center">
  Single Rust binary · Zero telemetry · 753 tests · 81 property-based correctness proofs
</p>

<p align="center">
  <a href="https://crates.io/crates/sqz-cli"><img src="https://img.shields.io/crates/v/sqz-cli?logo=rust&logoColor=white&label=crates.io&color=e6522c" alt="Crates.io"></a>
  <a href="https://www.npmjs.com/package/sqz-cli"><img src="https://img.shields.io/npm/v/sqz-cli?logo=npm&logoColor=white&label=npm&color=cb3837" alt="npm"></a>
  <a href="https://pypi.org/project/sqz/"><img src="https://img.shields.io/pypi/v/sqz?logo=python&logoColor=white&label=PyPI&color=3775a9" alt="PyPI"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz"><img src="https://img.shields.io/badge/VS%20Code-Marketplace-007acc?logo=visual-studio-code&logoColor=white" alt="VS Code"></a>
  <a href="https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/"><img src="https://img.shields.io/badge/Firefox-Add--on-ff7139?logo=firefox-browser&logoColor=white" alt="Firefox"></a>
  <a href="https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/"><img src="https://img.shields.io/badge/JetBrains-Plugin-000000?logo=jetbrains&logoColor=white" alt="JetBrains"></a>
  <a href="https://discord.gg/j8EEyH5dSB"><img src="https://img.shields.io/discord/1493251029075235076?logo=discord&logoColor=white&label=Discord&color=5865F2" alt="Discord"></a>
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#how-it-works">How It Works</a> ·
  <a href="#features">Features</a> ·
  <a href="#platforms">Platforms</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

## The Problem

AI coding tools waste tokens. Every file read sends the full content — even if the LLM saw it 30 seconds ago. Every `git status` sends raw output. Every API response dumps uncompressed JSON. You're paying for tokens that carry zero signal.

## The Solution

sqz sits between your AI tool and the LLM, compressing everything before it reaches the model. Two layers work together:

**Noise reduction** — a multi-stage compression pipeline strips nulls from JSON, collapses repeated log lines, folds unchanged diff context, encodes JSON arrays as tables, abbreviates common words, and applies run-length encoding to repetitive output. This is the core — it cleans up noisy tool output before it hits the context window.

**Deduplication** — a compaction-aware SHA-256 cache returns a 13-token reference for repeated content. When a file changes by a few lines, delta encoding sends only the diff. A turn-counter heuristic detects when refs may have gone stale (the original content was compacted out of the LLM's context) and automatically re-sends the full compressed content instead of a dangling reference.

```
Without sqz:                              With sqz:

File read #1:  2,000 tokens               File read #1:  ~800 tokens (compressed)
File read #2:  2,000 tokens               File read #2:  ~13 tokens  (dedup ref)
File read #3:  2,000 tokens               File read #3:  ~13 tokens  (dedup ref)
─────────────────────────                  ─────────────────────────
Total:         6,000 tokens               Total:         ~826 tokens (86% saved)
```

No workflow changes. Install once, save on every API call.

## Token Savings

sqz saves tokens in two ways: compression (removing noise from content) and deduplication (replacing repeated reads with 13-token references). The dedup cache is where the biggest savings happen in real sessions.

### Where sqz shines

| Scenario | Savings | Why |
|---|---:|---|
| Repeated file reads (5x) | **86%** | Dedup cache: 13-token ref after first read |
| JSON API responses with nulls | **7–56%** | Strip nulls + TOON encoding (varies by null density) |
| Repeated log lines | **58%** | Condense + RLE collapses duplicates |
| Large JSON arrays | **45%** | Tabular encoding for uniform arrays, collapse for mixed |
| Git diffs | **11%** | Fold unchanged context lines |
| Prose / documentation | **2–20%** | Token pruning + word abbreviation + entropy truncation |

### Where sqz intentionally preserves content

| Scenario | Savings | Why |
|---|---:|---|
| Stack traces | **0%** | Error content is critical — safe mode preserves it |
| Test output | **0%** | Pass/fail signals must not be altered |
| Short git output | **0%** | Already compact, nothing to strip |

This is by design. sqz's confidence router detects high-risk content (errors, test results, diffs) and routes it through safe mode to avoid dropping signal. A tool that claims 89% compression on `cargo test` output is either lying or deleting your error messages.

### Benchmark suite

Command: `cargo test -p sqz-engine benchmarks -- --nocapture`

For a full session-level comparison with rtk, see [docs/benchmark-vs-rtk.md](docs/benchmark-vs-rtk.md).

| Case | Before | After | Saved |
|---|---:|---:|---:|
| repeated_logs | 148 | 62 | **58.1%** |
| json_api | 64 | 59 | **7.8%** |
| git_diff | 61 | 54 | **11.5%** |
| large_json_array | 259 | 142 | **45.2%** |
| stack_trace (safe mode) | 82 | 82 | **0.0%** |
| prose_docs | 124 | 121 | **2.4%** |

### Track your savings

```sh
sqz gain          # ASCII chart of daily token savings
sqz stats         # Cumulative compression report
```

## Install

```sh
# Confirmed working:
cargo install sqz-cli

# Coming soon (scaffolded, not yet live):
# curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh
# brew install sqz
# npm install -g sqz-cli
```

> All install channels point to [github.com/ojuschugh1/sqz](https://github.com/ojuschugh1/sqz).

Then:

```sh
sqz init
```

That's it. Shell hooks installed, default presets created, ready to go.

## How It Works

sqz operates at four integration levels simultaneously:

### 1. Shell Hook (CLI Proxy)

Intercepts command output from 100+ CLI tools (git, cargo, npm, docker, kubectl, aws, etc.) and compresses it before the LLM sees it. Includes session-level n-gram abbreviation for recurring phrases and word abbreviation for common long words.

```sh
# Before: git log sends ~800 tokens of raw output
# After: sqz compresses to ~150 tokens, same information
```

### 2. MCP Server

A compiled Rust binary (not Node.js) that serves as an MCP server with intelligent tool selection (TF-IDF + cosine similarity), preset hot-reload, and the full compression pipeline.

```json
{
  "mcpServers": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"]
    }
  }
}
```

### 3. Browser Extension

Chrome and Firefox extensions for ChatGPT, Claude.ai, Gemini, Grok, and Perplexity. Compresses pasted content client-side via a lightweight WASM engine (TOON encoding + whitespace normalization + phrase substitution). The full pipeline runs in the CLI/MCP — the browser uses a fast subset optimized for paste-time latency. Zero network requests.

### 4. IDE Extensions

Native [VS Code](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) and JetBrains extensions that intercept file reads at the editor level, with AST-aware compression for 18 languages and a status bar showing token budget.

## Features

### Compression Pipeline
- **10 registered stages** — ansi_strip, keep_fields, strip_fields, condense, git_diff_fold, strip_nulls, flatten, truncate_strings, collapse_arrays, custom_transforms
- **6 post-stage processors** — RLE (run-length encoding), sliding window dedup, entropy-weighted truncation, self-information token pruning, dictionary compression, TOON encoding
- **Word abbreviation** — 100+ common long words abbreviated at the output layer (implementation→impl, configuration→config, authentication→auth, etc.)
- **Tabular encoding** — uniform JSON arrays (objects with identical keys) encoded as compact header + rows instead of repeated objects
- **TOON encoding** — lossless JSON compression producing compact ASCII-safe output (reduction varies by structure, 4–30% typical)
- **Tree-sitter AST** — structural code extraction for 4 languages natively (Rust, Python, JavaScript, Bash) + 14 via regex fallback (TypeScript, Go, Java, C, C++, Ruby, JSON, HTML, CSS, C#, Kotlin, Swift, TOML, YAML)
- **Image compression** — screenshots → semantic DOM descriptions
- **ANSI auto-strip** — removes color codes before compression

### Caching & Deduplication
- **SHA-256 content cache** — on a miss, content is compressed and stored; on a hit, the engine returns a compact inline reference (~13 tokens). LRU eviction, persisted across sessions.
- **Compaction-aware dedup** — a turn-counter heuristic tracks when each ref was last sent. After 20 turns (configurable), refs are considered stale and the full compressed content is re-sent instead of a dangling reference. `notify_compaction()` explicitly invalidates all refs when the harness signals a context reset.
- **Delta encoding** — near-duplicate content (similarity > 0.6) produces a compact line-level diff instead of re-sending the full file. SimHash fingerprinting enables O(1) candidate detection before falling back to LCS comparison.
- **N-gram abbreviation** — session-level phrase frequency tracking replaces recurring multi-word phrases with short symbols + legend.
- **SQLite FTS5 session store** — cross-session memory with full-text search
- **Correction log** — immutable append-only log that survives compaction
- **CTX format** — portable session graph across Claude, GPT, and Gemini

### Intelligence
- **Confidence routing** — entropy analysis + pattern detection routes high-risk content (stack traces, secrets, migrations) to safe mode automatically
- **TF-IDF + cosine tool selection** — exposes 3–5 relevant tools per task via TF-IDF weighted semantic matching (falls back to Jaccard for short queries)
- **Prompt cache awareness** — preserves Anthropic 90% and OpenAI 50% cache boundaries
- **Model routing** — routes simple tasks to cheaper local models based on complexity scoring
- **Terse mode** — system prompt injection for concise LLM responses (3 levels)
- **Predictive budget warnings** — alerts at 70% and 85% thresholds
- **Compression quality metrics** — Shannon entropy-based efficiency measurement with quality grades (Excellent/Good/Fair/Poor) and headroom reporting

### Cost & Analytics
- **Real-time USD tracking** — per-tool breakdown with cache discount impact
- **Multi-agent budgets** — per-agent allocation with isolation and enforcement
- **Session cost summaries** — total tokens, USD, cache savings, compression savings

### Extensibility
- **TOML presets** — hot-reload within 2 seconds, community-driven ecosystem
- **Plugin API** — Rust trait + WASM interface for custom compression strategies
- **100+ CLI patterns** — git, cargo, npm, docker, kubectl, aws, and more

### Privacy
- **Zero telemetry** — no data transmitted, no crash reports, no analytics
- **Fully offline** — works in air-gapped environments after install
- **Local only** — all processing happens on your machine

## Platforms

sqz integrates with AI coding tools across 3 levels:

### Level 1 — MCP Config Only
Continue · Zed

### Level 2 — Shell Hook + MCP
Claude Code · Cursor · Copilot · Windsurf · Gemini CLI · Codex · OpenCode · Goose · Aider · Amp

### Level 3 — Native / Deep
[VS Code](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) · JetBrains · Chrome (ChatGPT, Claude.ai, Gemini, Grok, Perplexity)

See [docs/integrations/](docs/integrations/) for platform-specific setup guides.

## CLI Commands

```sh
sqz init              # Install shell hooks + default presets
sqz compress <text>   # Compress text (or pipe from stdin)
sqz compress --verify # Compress with confidence score
sqz compress --mode safe|aggressive  # Force compression mode
sqz stats             # Cumulative compression report
sqz gain              # ASCII chart of daily token savings
sqz gain --days 30    # Last 30 days
sqz analyze <file>    # Per-block Shannon entropy analysis
sqz export <session>  # Export session to .ctx format
sqz import <file>     # Import a .ctx file
sqz status            # Show token budget and usage
sqz cost <session>    # Show USD cost breakdown
```

## Configuration

sqz uses TOML presets with hot-reload. The `[preset]` table maps to the Rust `PresetHeader` type (`name`, `version`, optional `description`).

```toml
[preset]
name = "default"
version = "1.0"

[compression]
stages = ["keep_fields", "strip_fields", "condense", "strip_nulls",
          "flatten", "truncate_strings", "collapse_arrays", "custom_transforms"]

[compression.condense]
enabled = true
max_repeated_lines = 3

[compression.strip_nulls]
enabled = true

[budget]
warning_threshold = 0.70
ceiling_threshold = 0.85
default_window_size = 200000

[terse_mode]
enabled = true
level = "moderate"

[model]
family = "anthropic"
primary = "claude-sonnet-4-20250514"
complexity_threshold = 0.4
```

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                Integration Surfaces                  │
│  CLI Binary  │  MCP Server  │  Browser  │  IDE Ext  │
└──────┬───────┴──────┬───────┴─────┬─────┴─────┬─────┘
       │              │             │            │
       └──────────────┴─────────────┴────────────┘
                          │
       ┌──────────────────┴──────────────────┐
       │         sqz_engine (Rust core)       │
       │         50 modules · ~30K lines      │
       │                                      │
       │  Compression Pipeline (16 stages)    │
       │  TOON Encoder (lossless JSON)        │
       │  AST Parser (tree-sitter, 18 langs)  │
       │  Cache Manager (SHA-256 + SimHash)   │
       │  Delta Encoder (LCS + SimHash)       │
       │  Session Store (SQLite FTS5)         │
       │  Budget Tracker (multi-agent)        │
       │  Cost Calculator (real-time USD)     │
       │  Tool Selector (TF-IDF + cosine)     │
       │  Confidence Router (entropy-based)   │
       │  Prompt Cache Detector               │
       │  Model Router (complexity routing)   │
       │  Token Pruner (self-information)     │
       │  Entropy Truncator (rate-distortion) │
       │  RLE Compressor + Sliding Window     │
       │  Dict Compressor (JSON fields)       │
       │  BPE Compressor (vocabulary)         │
       │  SimHash (LSH fingerprinting)        │
       │  Compression Quality (Shannon bound) │
       │  N-gram Abbreviator (session-level)  │
       │  Correction Log (append-only)        │
       │  Plugin API (Rust + WASM)            │
       └─────────────────────────────────────┘
```

## Distribution

| Channel | Command | Status |
|---|---|---|
| Cargo | `cargo install sqz-cli` | Live |
| Homebrew | `brew install sqz` | Coming soon |
| npm | `npm install -g sqz-cli` / `npx sqz-cli` | Coming soon |
| curl | `curl -fsSL .../install.sh \| sh` | Coming soon |
| Docker | `docker run sqz` | Coming soon |
| GitHub Releases | Pre-built binaries for Linux, macOS, Windows | Coming soon |

## Development

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace    # 753 tests
cargo build --release     # optimized binary
```

### Rust API names (`sqz_engine`)

Prefer the primary type names below; the second name in each row is a `type` alias kept for compatibility.

| Primary | Alias |
| --- | --- |
| `Session` | `SessionState` |
| `Turn` | `ConversationTurn` |
| `PinnedSegment` | `PinEntry` |
| `KvFact` | `Learning` |
| `WindowUsage` | `BudgetState` |
| `ToolCall` | `ToolUsageRecord` |
| `EditRecord` | `CorrectionEntry` |
| `EditHistory` | `CorrectionLog` |
| `PresetHeader` | `PresetMeta` |

**File cache:** `CacheManager` returns `CacheResult::Dedup` (compact inline reference, ~13 tokens), `CacheResult::Delta` (near-duplicate diff), or `CacheResult::Fresh` (newly compressed payload). Stale refs (older than 20 turns) automatically return `Fresh` to avoid dangling references after context compaction.

**Defensive API:** `SqzEngine::compress_or_passthrough()` guarantees any input produces a `CompressedContent` output — never returns an error. On internal failure, returns the original input unchanged.

**Sandbox:** `SandboxResult` uses `status_code`, `was_truncated`, and `was_indexed` (stdout-only data enters the context window).

### Project Structure

```
sqz_engine/     Core Rust library (50 modules, all compression logic)
sqz/            CLI binary (shell hooks, commands)
sqz-mcp/        MCP server binary (stdio/SSE transport)
sqz-wasm/       WASM target for browser extension
extension/      Chrome extension (content scripts, popup)
vscode-extension/   VS Code extension (TypeScript)
jetbrains-plugin/   JetBrains plugin (Kotlin)
docs/           Integration guides and documentation
```

### Testing

The test suite includes 753 tests with 81 property-based correctness properties validated via proptest:

- TOON round-trip fidelity
- Compression preserves semantically significant content
- ASCII-safe output across all inputs
- File cache — deduplication, staleness detection, and invalidation
- Compaction-aware ref tracking (stale refs re-send content)
- Delta encoding similarity bounds
- SimHash hamming distance symmetry and bounds
- Budget token count invariants
- Pin/unpin compaction round-trips
- CTX format round-trip serialization
- Plugin priority ordering
- Tool selection cardinality bounds (TF-IDF + Jaccard)
- Cross-tokenizer determinism
- RLE and sliding window dedup bounds
- Entropy truncation segment accounting
- BPE merge savings non-negativity
- Zipf's law vocabulary pruning preservation

## Contributing

We welcome contributions. By submitting a pull request, you agree to the [Contributor License Agreement](CLA.md).

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow.

## License

Licensed under [Elastic License 2.0](LICENSE) (ELv2). You can use, fork, modify, and distribute sqz freely. Two restrictions: you cannot offer it as a competing hosted/managed service, and you cannot remove licensing notices.

We chose ELv2 over MIT because MIT permits repackaging the code as a competing closed-source SaaS — ELv2 prevents that while keeping the source available to everyone.
