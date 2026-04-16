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

sqz compresses command output before it reaches your LLM. Single Rust binary, 100+ commands, zero config.

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

| Command | Before | After | Saved |
|---|---:|---:|---:|
| `git status` (10x) | 3,000 | 450 | -85% |
| `cat` / file reads (20x, with dedup) | 40,000 | 4,800 | -88% |
| `cargo test` / `npm test` (5x) | 25,000 | 2,500 | -90% |
| `git diff` (5x) | 10,000 | 2,800 | -72% |
| `git log` (5x) | 2,500 | 425 | -83% |
| JSON API responses | 4,000 | 1,200 | -70% |
| **30-min session total** | **~84,500** | **~12,175** | **-86%** |

Dedup cache is where the biggest savings come from. Without it, repeated file reads get compressed each time (~60% savings). With it, reads 2+ are 13 tokens.

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
  04-13 │██████████████████████████████│ 2329 saved
  04-14 │                              │ 0 saved
  04-15 │████████████████████████      │ 1894 saved
──────────────────────────────────────────────────
  Total: 835 compressions, 2622 tokens saved
```

## How Compression Works

1. **Per-command formatters** — `git status` → compact summary, `cargo test` → failures only, `docker ps` → name/image/status table
2. **Dedup cache** — SHA-256 content hash, persistent across sessions. Second read = 13-token reference.
3. **JSON pipeline** — strip nulls → flatten → collapse arrays → TOON encoding (lossless compact format)
4. **Safe mode** — stack traces, secrets, migrations detected by entropy analysis and routed through with 0% compression

For the full technical details, see [ARCHITECTURE.md](ARCHITECTURE.md).

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
