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
  Single Rust binary · Zero telemetry · 549 tests · 57 property-based correctness proofs
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#how-it-works">How It Works</a> ·
  <a href="#features">Features</a> ·
  <a href="#platforms">Platforms</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

## The Problem

AI coding tools waste 60-90% of your context window on noise. Every file read sends the full content. Every `git status` sends raw output. Every API response dumps uncompressed JSON. You're paying for tokens that carry zero signal.

## The Solution

sqz sits between your AI tool and the LLM, compressing everything before it reaches the model. No workflow changes. Install once, save on every API call.

```
Without sqz:                              With sqz:

LLM ──"read auth.ts"──▶ Editor ──▶ File   LLM ──"read auth.ts"──▶ sqz ──▶ File
  ▲                                │         ▲                      │       │
  │    ~2,000 tokens (full file)   │         │   ~13 tokens         │ cache │
  └────────────────────────────────┘         └──── (compressed) ────┴───────┘
```

## Token Savings

Compression ratios depend on content type. These are measured results from the sqz engine:

| Content Type | Typical Reduction | Method |
|---|---|---|
| JSON (large arrays) | 60-80% | Schema sampling + minification |
| Log output | 40-50% | Repeated line folding |
| Code (with comments) | 25-35% | Comment removal + whitespace |
| Prose / documentation | 15-30% | Sentence pruning + filler removal |
| Base64 blobs | 90%+ | Placeholder substitution |
| Cached file re-reads | ~99% | SHA-256 content-addressed cache |

Actual savings vary by input. The browser extension's 16-pass squeeze engine achieves 15-30% on typical prose. The Rust engine's TOON encoding achieves 4-30% on JSON depending on structure.

## Install

```sh
# Pick one:
curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh
cargo install sqz-cli
brew install sqz
npm install -g sqz
pip install sqz
```

Then:

```sh
sqz init
```

That's it. Shell hooks installed, default presets created, ready to go.

## How It Works

sqz operates at four integration levels simultaneously:

### 1. Shell Hook (CLI Proxy)

Intercepts command output from 100+ CLI tools (git, cargo, npm, docker, kubectl, aws, etc.) and compresses it before the LLM sees it.

```sh
# Before: git log sends ~800 tokens of raw output
# After: sqz compresses to ~150 tokens, same information
```

### 2. MCP Server

A compiled Rust binary (not Node.js) that serves as an MCP server with intelligent tool selection, preset hot-reload, and an 8-stage compression pipeline.

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

Chrome extension for ChatGPT, Claude.ai, Gemini, Grok, and Perplexity. Compresses pasted content client-side via WASM. Zero network requests.

### 4. IDE Extensions

Native VS Code and JetBrains extensions that intercept file reads at the editor level, with AST-aware compression for 18 languages and a status bar showing token budget.

## Features

### Compression Engine
- **8-stage pipeline** — keep_fields, strip_fields, condense, strip_nulls, flatten, truncate_strings, collapse_arrays, custom_transforms
- **TOON encoding** — lossless JSON compression producing compact ASCII-safe output (reduction varies by structure, 4-30% typical)
- **Tree-sitter AST** — structural code extraction for 4 languages natively (Rust, Python, JavaScript, Bash) + 14 via regex fallback (TypeScript, Go, Java, C, C++, Ruby, JSON, HTML, CSS, C#, Kotlin, Swift, TOML, YAML)
- **Image compression** — screenshots → semantic DOM descriptions
- **ANSI auto-strip** — removes color codes before compression

### Caching & Memory
- **SHA-256 file cache** — re-reads cost ~13 tokens, LRU eviction, persisted across sessions
- **SQLite FTS5 session store** — cross-session memory with full-text search
- **Correction log** — immutable append-only log that survives compaction
- **CTX format** — portable session state across Claude, GPT, and Gemini

### Intelligence
- **Prompt cache awareness** — preserves Anthropic 90% and OpenAI 50% cache boundaries
- **Dynamic tool selection** — exposes 3-5 relevant tools per task via semantic matching
- **Model routing** — routes simple tasks to cheaper local models
- **Terse mode** — system prompt injection for concise LLM responses (3 levels)
- **Predictive budget warnings** — alerts at 70% and 85% thresholds

### Cost & Analytics
- **Real-time USD tracking** — per-tool breakdown with cache discount impact
- **Multi-agent budgets** — per-agent allocation with isolation and enforcement
- **Session cost summaries** — total tokens, USD, cache savings, compression savings

### Extensibility
- **TOML presets** — hot-reload within 2 seconds, community-driven ecosystem
- **Plugin API** — Rust trait + WASM interface for custom compression strategies
- **150 CLI patterns** — git, cargo, npm, docker, kubectl, aws, and more

### Privacy
- **Zero telemetry** — no data transmitted, no crash reports, no analytics
- **Fully offline** — works in air-gapped environments after install
- **Local only** — all processing happens on your machine

## Platforms

sqz integrates with AI coding tools across 3 levels:

### Level 1 — MCP Config Only
Continue · Zed · Amazon Q Developer

### Level 2 — Shell Hook + MCP
Claude Code · Cursor · Copilot · Windsurf · Kiro · Cline · Gemini CLI · Codex · OpenCode · Goose · Aider · Amp

### Level 3 — Native / Deep
VS Code · JetBrains · Chrome (ChatGPT, Claude.ai, Gemini, Grok, Perplexity) · API Proxy (OpenAI, Anthropic, Google AI)

See [docs/integrations/](docs/integrations/) for platform-specific setup guides.

## CLI Commands

```sh
sqz init              # Install shell hooks + default presets
sqz compress <text>   # Compress text (or pipe from stdin)
sqz export <session>  # Export session to .ctx format
sqz import <file>     # Import a .ctx file
sqz status            # Show token budget and usage
sqz cost <session>    # Show USD cost breakdown
```

## Configuration

sqz uses TOML presets with hot-reload:

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
       │                                      │
       │  Compression Pipeline (8 stages)     │
       │  TOON Encoder (lossless JSON)        │
       │  AST Parser (tree-sitter + regex, 18 langs)  │
       │  Cache Manager (SHA-256 dedup)       │
       │  Session Store (SQLite FTS5)         │
       │  Budget Tracker (multi-agent)        │
       │  Cost Calculator (real-time USD)     │
       │  Tool Selector (semantic matching)   │
       │  Prompt Cache Detector               │
       │  Model Router (complexity routing)   │
       │  Correction Log (append-only)        │
       │  Plugin API (Rust + WASM)            │
       └─────────────────────────────────────┘
```

## Distribution

| Channel | Command |
|---|---|
| Cargo | `cargo install sqz-cli` |
| Homebrew | `brew install sqz` |
| npm | `npm install -g sqz` / `npx sqz` |
| pip | `pip install sqz` |
| curl | `curl -fsSL .../install.sh \| sh` |
| Docker | `docker run sqz` |
| GitHub Releases | Pre-built binaries for Linux, macOS, Windows |

## Development

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace    # 549 tests
cargo build --release     # optimized binary
```

### Project Structure

```
sqz_engine/     Core Rust library (all compression logic)
sqz/            CLI binary (shell hooks, commands)
sqz-mcp/        MCP server binary (stdio/SSE transport)
sqz-wasm/       WASM target for browser extension
extension/      Chrome extension (content scripts, popup)
vscode-extension/   VS Code extension (TypeScript)
jetbrains-plugin/   JetBrains plugin (Kotlin)
docs/           Integration guides and documentation
```

### Testing

The test suite includes 549 tests with 57 property-based correctness properties validated via proptest:

- TOON round-trip fidelity
- Compression preserves semantically significant content
- ASCII-safe output across all inputs
- Cache deduplication and invalidation
- Budget token count invariants
- Pin/unpin compaction round-trips
- CTX format round-trip serialization
- Plugin priority ordering
- Tool selection cardinality bounds
- Cross-tokenizer determinism

## Contributing

We welcome contributions. By submitting a pull request, you agree to the [Contributor License Agreement](CLA.md).

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow.

## License

Licensed under [Elastic License 2.0](LICENSE) (ELv2). You can use, fork, modify, and distribute sqz freely. Two restrictions: you cannot offer it as a competing hosted/managed service, and you cannot remove licensing notices.

We chose ELv2 over MIT because MIT permits repackaging the code as a competing closed-source SaaS — ELv2 prevents that while keeping the source available to everyone.
