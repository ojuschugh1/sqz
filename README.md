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
  <sub>
    <strong>Real session stats:</strong>
    3,003 compressions ·
    <strong>178,442 tokens saved</strong> ·
    24.7% avg reduction · up to
    <strong>92%</strong> with dedup
  </sub>
</p>

<p align="center">
  <a href="https://thenextgentechinsider.com/pulse/sqz-tool-cuts-llm-token-use-by-92-for-file-heavy-ai-tasks"><img src="https://img.shields.io/badge/%231_Featured-NextGen_Tech_Insider-ff6600?style=for-the-badge&logo=newspaper&logoColor=white" alt="Featured"></a>
</p>

<p align="center">
  <a href="https://crates.io/crates/sqz-cli"><img src="https://img.shields.io/crates/v/sqz-cli?logo=rust&logoColor=white&label=crates.io&color=e6522c" alt="Crates.io"></a>
  <a href="https://www.npmjs.com/package/sqz-cli"><img src="https://img.shields.io/npm/v/sqz-cli?logo=npm&logoColor=white&label=npm&color=cb3837" alt="npm"></a>
  <a href="https://pypi.org/project/sqz/"><img src="https://img.shields.io/pypi/v/sqz?logo=python&logoColor=white&label=PyPI&color=3775a9" alt="PyPI"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz"><img src="https://img.shields.io/badge/VS%20Code-Marketplace-007acc?logo=visual-studio-code&logoColor=white" alt="VS Code"></a>
  <a href="https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/"><img src="https://img.shields.io/badge/Firefox-Add--on-ff7139?logo=firefox-browser&logoColor=white" alt="Firefox"></a>
  <a href="https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/"><img src="https://img.shields.io/badge/JetBrains-Plugin-000000?logo=jetbrains&logoColor=white" alt="JetBrains"></a>
  <a href="https://discord.gg/j8EEyH5dSB"><img src="https://img.shields.io/discord/1493251029075235076?logo=discord&logoColor=white&label=Discord&color=5865F2" alt="Discord"></a>
  <a href="https://github.com/ojuschugh1/homebrew-sqz"><img src="https://img.shields.io/badge/Homebrew-tap-FBB040?logo=homebrew&logoColor=white" alt="Homebrew"></a>
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

> **24.7%** average reduction across 3,003 real compressions ·
> **92%** saved on repeated file reads ·
> **86%** on shell/git output ·
> **13-token** refs for cached content

One developer's week, measured from actual `sqz gain` output:

```
$ sqz gain
sqz token savings (last 7 days)
──────────────────────────────────────────────────
  04-13 │                              │   2,329 saved
  04-14 │                              │       0 saved
  04-15 │███                           │  12,954 saved
  04-16 │██                            │   9,223 saved
  04-17 │████                          │  14,752 saved
  04-18 │██████████████████████████████│ 105,569 saved
  04-19 │████████                      │  30,882 saved
  04-20 │█                             │   4,334 saved
──────────────────────────────────────────────────
  Total: 3,003 compressions, 178,442 tokens saved (24.7% avg reduction)
```

### Per-command compression

Single-command compression (measured via `cargo test -p sqz-engine benchmarks`):

| Content | Before | After | Saved |
|---|---:|---:|---:|
| Repeated log lines | 148 | 62 | **58%** |
| Large JSON array | 259 | 142 | **45%** |
| JSON API response | 64 | 53 | **17%** |
| Git diff | 61 | 54 | **12%** |
| Prose/docs | 124 | 121 | **2%** |
| Stack trace (safe mode) | 82 | 82 | **0%** |

### Session-level with dedup

Where the real savings live — the cache sends each file once, repeats cost 13 tokens:

| Scenario | Without sqz | With sqz | Saved |
|---|---:|---:|---:|
| Same file read 5× | 10,000 | 826 | **92%** |
| Same JSON response 3× | 192 | 79 | **59%** |
| Test-fix-test cycle (3 runs) | 15,000 | 5,186 | **65%** |

Single-command compression ranges from 2–58% depending on content. Repeated reads drop to 13 tokens each. Your mileage will vary with how repetitive your tool calls are — agentic sessions with many file re-reads see the biggest wins.

## Install

**Prebuilt binaries** (no compiler required — works on every platform):

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.ps1 | iex

# Any platform via npm
npm install -g sqz-cli

# macOS / Linux via Homebrew
brew tap ojuschugh1/sqz
brew install sqz
```

**Build from source via Cargo:**

```sh
cargo install sqz-cli sqz-mcp
```

`sqz-cli` provides the `sqz` binary; `sqz-mcp` provides the MCP server. `sqz-engine` is a library dependency — it compiles automatically and does not need to be installed separately.

**Build from source** (`cargo install sqz-cli`) works too, but needs a C toolchain:

- Linux: `build-essential` (apt) or equivalent
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- **Windows: Visual Studio Build Tools with the "Desktop development with C++" workload.** Without these, `cargo install` fails with `linker link.exe not found`. If you don't already have them, use the PowerShell or npm install above instead.

Then initialize:

```sh
sqz init --global     # hooks apply to every project on this machine
# or
sqz init              # hooks apply to just this project (.claude/settings.local.json)
```

`--global` writes to `~/.claude/settings.json` (the user scope per the
[Anthropic scope table](https://docs.claude.com/en/docs/claude-code/settings)),
so the sqz hook fires in every Claude Code session on this machine. This is
the common case on first install. Your existing `permissions`, `env`,
`statusLine`, and unrelated hooks in `~/.claude/settings.json` are
preserved — sqz merges its entries rather than overwriting.

Plain `sqz init` (project scope) is useful when you want sqz active only
inside one repo.

**Only using one agent?** Pass `--only` (or `--skip`) to limit which
configs are written:

```sh
sqz init --only opencode              # just OpenCode, nothing else
sqz init --only opencode,codex        # OpenCode and Codex
sqz init --skip cursor,windsurf       # everything except Cursor and Windsurf
```

Accepted names: `claude`, `cursor`, `windsurf`, `cline`, `gemini`,
`kiro`, `opencode`, `codex`. Aliases (`claude-code`, `gemini-cli`, `roo`,
`kiro-cli`) also work. `--only` and `--skip` can't be combined.

### Manual installation (preserve comments in your config)

`sqz init` round-trips your config file through a JSON parser to merge
the sqz entry, which drops any comments in your `opencode.jsonc` (and
the analogous JSON-with-comments files other tools accept). If you've
commented your config carefully and want to keep them, install by hand
instead.

**OpenCode** — two steps:

1. Drop the plugin file in place. `sqz` prints the generated TS to
   stdout so you don't have to hand-write the path-escaping logic:

   ```sh
   mkdir -p ~/.config/opencode/plugins
   sqz print-opencode-plugin > ~/.config/opencode/plugins/sqz.ts
   ```

2. Add the MCP entry to your existing `opencode.jsonc` yourself.
   Append this block inside the top-level `mcp` object (create the
   `mcp` object if it doesn't exist):

   ```jsonc
   "sqz": {
     "type": "local",
     "command": ["sqz-mcp", "--transport", "stdio"],
     "enabled": true
   }
   ```

Comments in the rest of your file stay put. OpenCode auto-discovers
the plugin file; no `plugin` array entry needed (adding one causes
double-loading, see issue #10).

**Other tools** — Claude Code, Cursor, Windsurf, Cline, Gemini CLI,
and Codex use plain JSON configs without comment support, so the
automated path is non-destructive there. Use `sqz init --only <tool>`
for those.

That's it. Shell hooks installed, AI tool hooks configured.

## How It Works

<p align="center">
  <img src="assets/sqz-architecture.png" alt="sqz system architecture" width="700" />
</p>

sqz installs a PreToolUse hook that intercepts bash commands before your AI tool runs them. The output gets compressed transparently — the AI tool never knows.

```
Claude → git status → [sqz hook rewrites] → compressed output (85% smaller)
```

What gets compressed:
- **Shell output** — 40+ per-command formatters (git, cargo, npm/pnpm/yarn, pytest, ruff, go test, docker, kubectl, aws, terraform, gradle, gh, grep/rg, tree, curl, and more)
- **JSON** — strips nulls, compact encoding, TOON format
- **Logs** — collapses repeated lines
- **Test output** — shows failures only (state-machine parsers for Rust, Go, Python, JS, JVM)

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
| Kiro | PreToolUse hook (transparent) | `sqz init` |
| OpenCode | TypeScript plugin (transparent) | `sqz init` |
| VS Code | [Extension](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Install from Marketplace |
| JetBrains | [Plugin](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Install from Marketplace |
| Chrome | Browser extension | ChatGPT, Claude.ai, Gemini, Grok, Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | Browser extension | Same sites |

## CLI

```sh
sqz init --global             # Install hooks for every project on this machine
sqz init                      # Install hooks for just this project
sqz init --only kiro          # Only configure Kiro (skip the rest)
sqz init --only opencode      # Only configure OpenCode (skip the rest)
sqz init --skip cursor        # Configure every agent except Cursor
sqz compress <text>           # Compress (or pipe from stdin)
sqz compress --no-cache       # Compress without dedup (always full output)
sqz expand <ref>              # Recover original content from a §ref:HASH§ token
sqz compact                   # Evict stale context to free tokens
sqz reset                     # Clear dedup cache or compression stats
sqz gain                      # Show daily token savings (bar chart)
sqz gain --project .          # Per-project daily gains
sqz gain --days 30            # Last 30 days
sqz stats                     # Cumulative compression report
sqz stats --breakdown         # Per-command token usage breakdown
sqz stats --project .         # Stats for current project only
sqz stats --project list      # List all tracked projects
sqz discover                  # Find missed savings
sqz resume                    # Re-inject session context after compaction
sqz vizit                     # Live terminal dashboard (like htop for AI agents)
sqz hook claude               # Process a PreToolUse hook (Claude Code)
sqz hook kiro                 # Process a PreToolUse hook (Kiro)
sqz print-opencode-plugin     # Print OpenCode plugin TS for manual install
sqz proxy --port 8080         # API proxy (compresses full request payloads)
```

### Dedup Escape Hatch

When sqz sees the same content twice, it returns a compact `§ref:HASH§` token
instead of the full text. Most models handle this fine, but some (e.g., GLM 5.1)
can't parse the ref format and loop. Four ways to work around this:

```sh
# 1. Recover original content from a ref
sqz expand a1b2c3d4              # prefix match
sqz expand '§ref:a1b2c3d4§'     # paste the whole token

# 2. Compress without dedup (per-invocation)
echo "..." | sqz compress --no-cache

# 3. Disable dedup globally (env var)
export SQZ_NO_DEDUP=1

# 4. MCP passthrough tool (returns input byte-exact, zero transforms)
# Available via tools/list when sqz-mcp is running
```

## Track Your Own Savings

Run `sqz gain` in your shell any time to see your own daily breakdown (see the
Token Savings section above for what the output looks like), and `sqz stats`
for the full cumulative report:

```sh
$ sqz stats
  📊 sqz compression stats
  ──────────────────────────────────────────────────

  178,442  tokens saved
  ↓  24.7% average reduction

  Compressions           3,003
  Tokens in              721,840
  Tokens out             543,398
  Tokens saved           178,442
  Avg reduction          24.7%

  🗄️  Cache
  ──────────────────────────────────────────────────
  Entries                43
  Size                   39.1 KB
```

Add `--breakdown` to see exactly which commands consume the most tokens:

```sh
$ sqz stats --breakdown

  🔍 Top Token Consumers
  ──────────────────────────────────────────────────────────────────────
  command               calls  tokens in        out    saved
  ──────────────────────────────────────────────────────────────────────
  dedup                   249      45541       3237      93%
  stdin                    51      30851      24289      21%
  auto                    132      18288       7740      58%
  echo                     17       1050        558      47%
  ls -la                    8        948        948       0%
  cargo build               7        170        145      15%
  git status                4         56          8      86%
  ──────────────────────────────────────────────────────────────────────
```
<img width="3456" height="1918" alt="image" src="https://github.com/user-attachments/assets/d308bc60-81f3-4935-9728-b30534e5b5e5" />


**Per-project filtering:**

```sh
sqz stats --project .           # stats for current project only
sqz stats --project list        # list all tracked projects
sqz gain --project .            # daily gains for current project
sqz gain --days 30              # last 30 days instead of 7
sqz gain --days 30 --project .  # combine both
```

Stats are stored locally in SQLite under `~/.sqz/sessions.db` — nothing leaves your machine.

## How Compression Works

1. **Per-command formatters** — 40+ commands across 9 ecosystems get purpose-built compression:

   | Ecosystem | Commands |
   |---|---|
   | Git | status, log, diff, show, stash, remote, fetch, push, pull, commit |
   | Rust | cargo build/test/clippy/check/nextest |
   | JavaScript | npm/pnpm/yarn/bun install/test/audit/outdated, tsc, eslint, vitest |
   | Python | pytest, ruff, mypy, pip |
   | Go | go test (incl. `-json` stream), go build, go vet, golangci-lint |
   | Cloud | aws, terraform plan/apply/init, gcloud |
   | Containers | docker/podman ps/images/build, kubectl get/describe/logs/apply |
   | JVM | gradle build/test, maven |
   | System | grep/rg, tree, find/fd, ls, curl/wget |
   | GitHub | gh pr/issue/run (JSON + table) |

   Unknown commands fall through to the generic compression pipeline — no output is ever left uncompressed.

2. **Structural summaries** — code files compressed to imports + function signatures + call graph (~70% reduction). The model sees the architecture, not implementation noise.
3. **Dedup cache** — SHA-256 content hash, persistent across sessions. Second read = 13-token reference.
4. **JSON pipeline** — strip nulls → project out debug fields → flatten → collapse arrays → TOON encoding (lossless compact format)
5. **Safe mode** — stack traces, secrets, migrations detected by entropy analysis and routed through with 0% compression

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

- [White Paper: Pre-Injection Context Compression](docs/whitepaper.md)
- [Benchmark: sqz vs rtk](docs/benchmark-vs-rtk.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [Changelog](CHANGELOG.md)

## Star History

<a href="https://star-history.com/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>
