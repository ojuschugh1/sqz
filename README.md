<p align="center">
  <pre>
  ███████╗ ██████╗ ███████╗
  ██╔════╝██╔═══██╗╚══███╔╝
  ███████╗██║   ██║  ███╔╝
  ╚════██║██║▄▄ ██║ ███╔╝
  ███████║╚██████╔╝███████╗
  ╚══════╝ ╚══▀▀═╝ ╚══════╝
  </pre>
</p>

<p align="center">
  <strong>Compress LLM context to save tokens and reduce costs</strong>
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
  <a href="#supported-tools">Supported Tools</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

sqz compresses command output before it reaches your LLM. Single Rust binary, zero config.

The real win is dedup: when the same file gets read 5 times in a session, sqz sends it once and returns a 13-token reference for every repeat.

```
Without sqz:                    With sqz:

File read #1:  2,000 tokens     File read #1:  ~800 tokens (compressed)
File read #2:  2,000 tokens     File read #2:  ~13 tokens  (dedup ref)
File read #3:  2,000 tokens     File read #3:  ~13 tokens  (dedup ref)
───────────────────────         ───────────────────────
Total:         6,000 tokens     Total:         ~826 tokens (86% saved)
```

## Token Savings

Single-command compression (measured via `cargo test -p sqz-engine benchmarks`):

| Content | Before | After | Saved |
|---|---:|---:|---:|
| Repeated log lines | 148 | 62 | **58%** |
| Large JSON array | 259 | 142 | **45%** |
| JSON API response | 64 | 53 | **17%** |
| Git diff | 61 | 54 | **12%** |
| Prose/docs | 124 | 121 | **2%** |
| Stack trace (safe mode) | 82 | 82 | **0%** |

Session-level savings (with dedup cache across repeated reads):

| Scenario | Without sqz | With sqz | Saved |
|---|---:|---:|---:|
| Same file read 5x | 10,000 | 826 | **92%** |
| Same JSON response 3x | 192 | 79 | **59%** |
| Test-fix-test cycle (3 runs) | 15,000 | 5,186 | **65%** |

The dedup cache is where the real savings live. Single-command compression ranges from 2-58% depending on content. Repeated reads drop to 13 tokens each.

## Install

```sh
cargo install sqz-cli
```

Then:

```sh
sqz init
```

That's it. Shell hooks installed, AI tool hooks configured.

## How It Works

sqz installs a PreToolUse hook that intercepts bash commands before your AI tool runs them. The output gets compressed transparently — the AI tool never knows.

```
Claude → git status → [sqz hook rewrites] → compressed output (85% smaller)
```

What gets compressed:
- **Shell output** — git, cargo, npm, docker, kubectl, ls, grep, etc.
- **JSON** — strips nulls, compact encoding
- **Logs** — collapses repeated lines
- **Test output** — shows failures only

What doesn't get compressed:
- Stack traces, error messages, secrets — routed to safe mode (0% compression)
- Your prompts and the AI's responses — controlled by the AI tool, not sqz

## Supported Tools

| Tool | Integration | Setup |
|---|---|---|
| Claude Code | PreToolUse hook (transparent) | `sqz init` |
| Cursor | PreToolUse hook (transparent) | `sqz init` |
| Windsurf | PreToolUse hook (transparent) | `sqz init` |
| Cline | PreToolUse hook (transparent) | `sqz init` |
| Gemini CLI | BeforeTool hook (transparent) | `sqz init` |
| OpenCode | TypeScript plugin (transparent) | `sqz init` |
| VS Code | [Extension](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Install from Marketplace |
| JetBrains | [Plugin](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Install from Marketplace |
| Chrome | Browser extension | ChatGPT, Claude.ai, Gemini, Grok, Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | Browser extension | Same sites |

## CLI

```sh
sqz init              # Install hooks
sqz compress <text>   # Compress (or pipe from stdin)
sqz gain              # Show daily token savings
sqz stats             # Cumulative report
sqz discover          # Find missed savings
sqz resume            # Re-inject session context after compaction
sqz hook claude       # Process a PreToolUse hook
sqz proxy --port 8080 # API proxy (compresses full request payloads)
```

## Track Your Savings

```sh
$ sqz gain
sqz token savings (last 7 days)
──────────────────────────────────────────────────
  04-13 │█████                         │ 2329 saved
  04-14 │                              │ 0 saved
  04-15 │██████████████████████████████│ 12954 saved
  04-16 │████████████                  │ 5532 saved
──────────────────────────────────────────────────
  Total: 1178 compressions, 19214 tokens saved
```

## How Compression Works

1. **Per-command formatters** — `git status` → compact summary, `cargo test` → failures only, `docker ps` → name/image/status table
2. **Dedup cache** — SHA-256 content hash, persistent across sessions. Second read = 13-token reference.
3. **JSON pipeline** — strip nulls → flatten → collapse arrays → TOON encoding (lossless compact format)
4. **Safe mode** — stack traces, secrets, migrations detected by entropy analysis and routed through with 0% compression

For the full technical details, see [docs/](docs/).

## Configuration

```toml
# ~/.sqz/presets/default.toml
[preset]
name = "default"
version = "1.0"

[compression.condense]
enabled = true
max_repeated_lines = 3

[compression.strip_nulls]
enabled = true

[budget]
warning_threshold = 0.70
default_window_size = 200000
```

## Privacy

- Zero telemetry — no data transmitted, no crash reports
- Fully offline — works in air-gapped environments
- All processing local

## Development

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
cargo build --release
```

## License

[Elastic License 2.0](LICENSE) (ELv2) — use, fork, modify freely. Two restrictions: no competing hosted service, no removing license notices.

## Links

- [Benchmark: sqz vs rtk](docs/benchmark-vs-rtk.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [Changelog](CHANGELOG.md)
