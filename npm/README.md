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
  <a href="https://github.com/ojuschugh1/sqz/blob/main/CHANGELOG.md">Changelog</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

sqz compresses command output before it reaches your LLM. Single Rust binary, zero config.

Per-command formatters keep what an agent acts on (the failing test, the assertion, the `file:line`) and drop what it does not. Anything the model already has in context comes back as a short reference instead of the content. Source code passes through unchanged, and so do stack traces, migrations and credential lines.

```
Without sqz:                              With sqz:

cat auth.py (200 lines):   1,300 tokens    1,300 tokens  (source is served in full)
cargo test (1 failure):      366 tokens      185 tokens  (failure, assertion, file:line)
sed -n '41,80p' auth.py:     260 tokens       35 tokens  (§ref:…:L41-80§ to the read above)
git status, unchanged:       166 tokens       37 tokens  (§ref:…§)
─────────────────────────────────────      ─────────────
Total:                     2,092 tokens    1,557 tokens  (26% saved, nothing dropped)
```

## Token Savings

> **24.7%** average reduction across 3,003 real compressions ·
> **13-token** refs for output the model already has ·
> **100%** of agent-critical facts kept in the [quality benchmark](https://github.com/ojuschugh1/sqz/blob/main/docs/quality-benchmark.md) ·
> **0%** on source code, and on stack traces and credential lines

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
| Repeated log lines | 148 | 39 | **74%** |
| Large JSON array | 259 | 111 | **57%** |
| JSON API response | 64 | 53 | **17%** |
| Git diff | 61 | 54 | **11%** |
| Prose/docs | 124 | 122 | **2%** |
| Stack trace (safe mode) | 82 | 82 | **0%** |

### Session-level with dedup

Anything the model already has in its context comes back as a reference. Measured with the release binary against a throwaway database, using the fixtures in `sqz/tests/quality_bench.rs` and `demo/`, in cl100k tokens of everything the agent sees (the reference plus sqz's one-line note on stderr):

| Repeat | Without sqz | With sqz | Saved |
|---|---:|---:|---:|
| Unchanged `git status` run again | 166 | 37 | **78%** |
| Lines 41-80 of a 200-line file read earlier | 260 | 35 | **87%** |
| Same `cargo test` output, nothing changed | 366 | 39 | **89%** |

Single-command compression in the first table above ranges from 0% (stack trace) to 74% (repeated log lines) depending on content. Your mileage will vary with how repetitive your tool calls are: sessions with noisy command output and repeated commands see the biggest wins.

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
The entries are its hooks and two allow rules, `Bash(set -o pipefail)` and
`Bash(sqz compress --cmd *)`, which cover only the parts its rewrite adds.
sqz does not approve commands; your own permission rules still decide.
When Claude Code asks before a rewritten command, or denies it, its message
quotes the rewritten command, such as `set -o pipefail; git tag v0 2>&1` for
`git tag v0`.

Plain `sqz init` (project scope) is useful when you want sqz active only
inside one repo. If `~/.claude/settings.json` already has the sqz hooks, it
leaves them out of the project file, since they already run in every project.

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

**Other tools**: Claude Code and Gemini CLI use plain JSON configs
without comment support, Cursor, Windsurf and Cline get markdown rules
files, and Codex gets a TOML entry in `~/.codex/config.toml` that keeps
your comments, so the automated path is non-destructive there. Use
`sqz init --only <tool>` for those. An existing `.gemini/settings.json`
gets sqz's hook merged in; if it has comments or doesn't parse, `sqz init`
leaves it alone and prints the entry to add by hand.

Run `sqz doctor` afterwards to see what was configured for each client
and which integration tier it is on. After upgrading sqz, run
`sqz doctor --fix` once: it brings hooks and guidance an older version
wrote up to date and points hooks whose sqz binary is gone at the running
one. `sqz uninstall` removes only sqz's own entries and ends by listing the
files it kept.

## How It Works

On transparent clients (Claude Code, Gemini CLI, Copilot CLI, OpenCode) sqz installs a hook that rewrites shell commands so their output goes through sqz before it reaches the AI tool. On advisory clients the agent is told to pipe output through `sqz compress` or use the sqz MCP tools.

```
Claude → git status → [sqz hook rewrites] → compressed output
```

What gets compressed:
- **Shell output**: git, cargo, npm, docker ps, kubectl describe/apply, grep, etc.
- **JSON**: strips nulls and empty values, cuts strings over 500 characters, compact encoding
- **Logs** — collapses repeated lines
- **Test output** — shows failures only

What doesn't get compressed:
- Source code: served in full on first read
- Stack traces, SQL migrations, PEM blocks and credential lines: routed to safe mode and passed through unchanged at any size, and never cached; on the shell hook a per-command formatter may still summarize the rest of a test run around them
- Your prompts and the AI's responses — controlled by the AI tool, not sqz

## Supported Tools

| Tool | Integration | Setup |
|---|---|---|
| Claude Code | PreToolUse + PostToolUse hooks + MCP server (transparent) | `sqz init` |
| Cursor | `.cursor/rules/sqz.mdc` guidance (advisory); MCP server added by hand | `sqz init` |
| Windsurf | `.windsurf/rules/sqz.md` guidance (advisory); MCP server added by hand | `sqz init` |
| Cline | `.clinerules` guidance (advisory); MCP server added by hand | `sqz init` |
| Gemini CLI | BeforeTool hook (transparent) | `sqz init` |
| Kiro | Steering + MCP server (advisory) | `sqz init` |
| OpenCode | TypeScript plugin + MCP server (transparent) | `sqz init` |
| Codex CLI | AGENTS.md guidance + MCP server (advisory) | `sqz init` |
| Zed | AGENTS.md guidance + MCP server (advisory) | `sqz init` |
| Copilot CLI | preToolUse hook (transparent), when `~/.copilot` exists | `sqz init` |
| Copilot coding agent (CI) | Repo-level hook, self-bootstraps in the cloud sandbox | `sqz init --ci` + commit |
| Any CI pipeline | [setup-sqz action](https://github.com/ojuschugh1/setup-sqz): compress logs before an LLM step | `uses: ojuschugh1/setup-sqz@v1` |
| Any MCP server | `sqz-mcp proxy` wraps it, compresses its tool results | see [the MCP proxy](https://github.com/ojuschugh1/sqz#compress-any-mcp-server) |
| VS Code | [Extension](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Install from Marketplace |
| JetBrains | [Plugin](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Install from Marketplace |
| Chrome | Browser extension | ChatGPT, Claude.ai, Gemini, Grok, Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | Browser extension | Same sites |

Transparent clients have a hook that routes shell commands through sqz on
its own. On advisory clients sqz saves tokens only when the agent pipes
output through `sqz compress` or calls the sqz MCP tools; the client's
built-in shell and file tools bypass it. `sqz doctor` shows each client's
tier.

## CLI

```sh
sqz init --global             # Install hooks for every project on this machine
sqz init                      # Install hooks for just this project
sqz init --only kiro          # Only configure Kiro (skip the rest)
sqz init --only opencode      # Only configure OpenCode (skip the rest)
sqz init --skip cursor        # Configure every agent except Cursor
sqz init --ci                 # Also write .github/hooks/sqz.json for Copilot agent CI runs
sqz compress <text>           # Compress (or pipe from stdin)
sqz compress --no-cache       # Compress without dedup (always full output)
sqz expand <ref>              # Recover original content from a §ref:HASH§ token
sqz compact                   # Evict stale context to free tokens
sqz reset                     # Clear dedup cache or compression stats
sqz gain                      # Show daily token savings (bar chart)
sqz gain --project .          # Per-project daily gains
sqz gain --days 30            # Last 30 days
sqz stats                     # Cumulative compression report (includes regret signals)
sqz stats --cost              # Estimated $ saved under a prompt-cached billing model
sqz stats --breakdown         # Per-command token usage breakdown
sqz stats --project .         # Stats for current project only
sqz stats --project list      # List all tracked projects
sqz stats --share             # Five-line share card sized for a paste into an issue or post
sqz doctor                    # Verify sqz is installed, wired into your clients, and active
sqz doctor --fix              # Apply the fixes doctor prints (run once after upgrading)
sqz uninstall                 # Remove sqz's hooks, guidance and MCP entries, keep the rest
sqz discover                  # Find missed savings (incl. your weakest-compressing commands)
sqz discover --replay         # Counterfactual replay of your agent transcripts (Claude Code, Kiro)
sqz recall "auth timeout"     # Full-text search everything sqz has compressed
sqz resume                    # Re-inject session context after compaction
sqz vizit                     # Live terminal dashboard (like htop for AI agents)
sqz hook claude               # Process a PreToolUse hook (Claude Code)
sqz hook claude-post          # Process a PostToolUse hook (Claude Code)
sqz hook kiro                 # Legacy; Kiro now uses steering + MCP (sqz init)
sqz print-opencode-plugin     # Print OpenCode plugin TS for manual install
sqz proxy --port 8080         # Local endpoint: compresses an API request body and sends it back, no forwarding
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

1. **Per-command formatters** — `git status` → compact summary, `cargo test` → failures only, `docker ps` → name/image/status table
2. **Structural summaries**: the engine has an API that reduces a code file to imports, function signatures and a call graph. The hook and MCP paths do not use it; they serve source in full.
3. **Dedup cache** — SHA-256 content hash, persistent across sessions. Second read = 13-token reference.
4. **JSON pipeline**: strip nulls → drop bookkeeping fields (`__v`, `etag`, request and correlation ids) → flatten → collapse arrays → TOON encoding (lossless compact format). Fields whose value is an empty string, array or object are dropped at any depth, and strings over 500 characters are cut to their first 500 characters followed by `...`.
5. **Safe mode**: stack traces (a trace header followed by its frames), SQL DDL at the start of a line, PEM blocks and credential lines (`key=value` or `key: value` with a secret-looking key) skip the pipeline and pass through unchanged at any size, with only ANSI codes stripped, and are never cached. On the shell hook a per-command formatter still runs first. Its summary is used only when it keeps every line of each paragraph that holds one of them unchanged, so it may summarize the rest of a test run around a trace; otherwise the whole output passes through. Output that only mentions a password, key, trace or migration outside those structures is compressed as usual but is not cached either, so anything a lossy stage drops from it cannot be expanded.

For the full technical details, see [docs/](https://github.com/ojuschugh1/sqz/tree/main/docs).

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

[Elastic License 2.0](https://github.com/ojuschugh1/sqz/blob/main/LICENSE) (ELv2): use, fork, modify freely. Two restrictions: no competing hosted service, no removing license notices.

## Links

- [Token Savings Benchmark](https://github.com/ojuschugh1/sqz/blob/main/docs/benchmark.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [Changelog](https://github.com/ojuschugh1/sqz/blob/main/CHANGELOG.md)

## Star History

<a href="https://star-history.dera.page/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="Star History Chart" src="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>
