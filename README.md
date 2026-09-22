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
  <strong>Pre-injection context compression and session deduplication for AI coding agents</strong>
</p>

<p align="center">
  <sub>
    <strong>Real session stats:</strong>
    3,003 compressions ·
    <strong>178,442 tokens saved</strong> ·
    24.7% avg reduction · up to
    <strong>92%</strong> on output the model already has
  </sub>
</p>

<p align="center">
  <a href="https://thenextgentechinsider.com/pulse/sqz-tool-cuts-llm-token-use-by-92-for-file-heavy-ai-tasks"><img src="https://img.shields.io/badge/%231_Featured-NextGen_Tech_Insider-ff6600?style=for-the-badge&logo=newspaper&logoColor=white" alt="Featured"></a>
  <a href="https://hysenlabs.com/en/projects/ojuschugh1-sqz"><img src="https://img.shields.io/badge/Featured-Hysen_Labs-1f6feb?style=for-the-badge&logo=readme&logoColor=white" alt="Featured on Hysen Labs"></a>
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
  <a href="https://ojuschugh1.github.io/sqz/">Docs</a> ·
  <a href="#install">Install</a> ·
  <a href="#how-it-works">How It Works</a> ·
  <a href="#supported-tools">Supported Tools</a> ·
  <a href="#compress-any-mcp-server">MCP Proxy</a> ·
  <a href="docs/quality-benchmark.md">Benchmark</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

AI coding agents spend most of their input tokens on tool output: build logs, test runs, `git status`, the same file read again after every edit. All of it goes into the context window raw, and it is re-sent on every turn after that.

sqz compresses tool output before it enters the context. Per-command formatters keep what an agent acts on (the failing test, the assertion, the `file:line`) and drop what it does not. Anything the model already has in context comes back as a short reference instead of the content. Source code, stack traces and secrets pass through untouched.

```sh
cargo install sqz-cli sqz-mcp     # or: brew install ojuschugh1/sqz/sqz · npm i -g sqz-cli · pipx install sqz
sqz init                          # hooks + MCP server + agent guidance for every client it finds
```

<p align="center">
  <img src="assets/demo.gif" alt="sqz: a 200-line file is served in full, a re-read of lines 41-80 becomes a line-range reference, cargo test output collapses to the failure, and sqz stats shows both" width="800">
</p>

<sub>Real output from the release binary (<code>assets/demo.tape</code>). Regenerate with <code>vhs assets/demo.tape</code>.</sub>

```
Without sqz:                              With sqz:

cat auth.py (500 lines):   4,000 tokens    4,000 tokens  (source is served in full)
cargo test (1 failure):    1,200 tokens      120 tokens  (failure, assertion, file:line)
sed -n '40,80p' auth.py:     330 tokens       18 tokens  (§ref:…:L40-80§ to the read above)
git status, unchanged:       160 tokens       13 tokens  (§ref:…§)
─────────────────────────────────────      ─────────────
Total:                     5,690 tokens    4,151 tokens  (27% saved, nothing dropped)
```

Single Rust binary, deterministic, no LLM calls, works offline. Every compressed result can be recovered byte-exact with `sqz expand`.

> [!NOTE]
> **Name disambiguation:** this repo, [ojuschugh1/sqz](https://github.com/ojuschugh1/sqz), is an independent project and is not affiliated with any other similarly named or working tool or other compression projects that shorten "squeeze". If you installed `sqz` / `sqz-cli` / `sqz-mcp` from crates.io, npm, PyPI, or Homebrew, it comes from this repository.

## Token Savings

> **24.7%** average reduction across 3,003 real compressions ·
> **13-token** refs for output the model already has ·
> **100%** of agent-critical facts kept in the [quality benchmark](docs/quality-benchmark.md) ·
> **0%** by construction on source code, stack traces and secrets

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

Anything the model already has in its context comes back as a reference. Measured with the release binary against a throwaway database, using the fixtures in `sqz/tests/quality_bench.rs` and `demo/`:

| Repeat | Without sqz | With sqz | Saved |
|---|---:|---:|---:|
| Unchanged `git status` run again | 155 | 13 | **92%** |
| Lines 41-80 of a 200-line file read earlier | 305 | 16 | **95%** |
| Same `cargo test` output, nothing changed | 303 | 13 | **96%** |

A word on what actually repeats. A user who counted 92 of their own Claude Code sessions found identical whole-file re-reads in 1 of 542 reads, and line-range re-reads in 8.5% of them. So sqz does not lead with "the same file read five times": the repeats that happen in practice are unchanged command output, line ranges of a file already read, and files re-read after a small edit, and each has its own reference type ([details](docs/stop-rereading-files.md)). Sessions with noisy command output and repeated commands see the biggest wins.

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

After installing, `sqz doctor` checks the whole chain — binary, database,
shell hook, which clients are detected vs actually routed through sqz, and
whether anything was compressed recently — and prints the fix for each gap
it finds.

### See what sqz would have saved, before installing any hook

If you already use Claude Code or Kiro, your transcripts are on disk. Replay
them through the sqz engine and get a counterfactual estimate on your own
sessions — computed locally, nothing uploaded:

```sh
$ sqz discover --replay
sqz discover — counterfactual replay (last 7 days)
────────────────────────────────────────────────────────

  Sessions replayed:   24
  Tool outputs:        43
  Tokens (original):   116,598
  Tokens (compressed):  92,560

  Estimated avoidable: 24,038 tokens (20.6%)

  Largest opportunities:

    source                tokens in    avoidable
    web_fetch                 58,102       12,288
    read                      37,085       10,486
```

It scans `~/.claude/projects` and `~/.kiro/sessions/cli`, or point it
anywhere with `--transcripts PATH`. Each session replays against a fresh
throwaway cache, so dedup references never pretend to span sessions, and the
report says "estimate" because that's what it is: these outputs were not
compressed at the time.

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
- **Shell output** — 45+ per-command formatters (git, cargo, npm/pnpm/yarn, pytest, ruff, go test, docker, kubectl, aws, terraform, gradle, xcodebuild, adb logcat, dotnet, gh, grep/rg, tree, curl, and more)
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
| Kiro | Steering + MCP server | `sqz init` |
| OpenCode | TypeScript plugin (transparent) | `sqz init` |
| Codex CLI | AGENTS.md guidance + MCP server | `sqz init` |
| Zed | AGENTS.md guidance + MCP server | `sqz init` |
| Copilot CLI | preToolUse hook (transparent) | `sqz init` |
| Copilot coding agent (CI) | Repo-level hook, self-bootstraps in the cloud sandbox | `sqz init --ci` + commit |
| Any MCP server | `sqz-mcp proxy` wraps it, compresses its tool results | see below |
| VS Code | [Extension](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Install from Marketplace |
| JetBrains | [Plugin](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Install from Marketplace |
| Chrome | Browser extension | ChatGPT, Claude.ai, Gemini, Grok, Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | Browser extension | Same sites |

## Compress Any MCP Server

`sqz-mcp proxy` sits between your agent and any stdio MCP server and compresses
what flows back: tool results go through the full sqz pipeline (dedup refs on
repeats, safe-mode for stack traces and secrets, error results untouched), and
verbose tool descriptions get compacted so `tools/list` stops eating your
context window. Everything stays reversible — the proxy injects an `sqz_expand`
tool so the agent can recover any original byte-exact.

Wrap a server by prefixing its command in your MCP config:

```json
{
  "mcpServers": {
    "github": {
      "command": "sqz-mcp",
      "args": ["proxy", "--", "npx", "-y", "@modelcontextprotocol/server-github"]
    }
  }
}
```

Flags: `--lazy-tools` shortens every tool description to one sentence and
injects an `sqz_tool_help` tool that serves the full original docs on demand
(some MCP servers spend 10-40k tokens on `tools/list` alone); `--no-desc`
keeps descriptions verbatim; `--no-cache` disables dedup refs. Works with
every MCP client (Claude Code, Cursor, Windsurf, Zed, Codex, Kiro, ...)
because the client just sees a normal MCP server. Full guide, including the
server's own `sqz_read_file` / `sqz_grep` / `sqz_list_dir` tools and per-client
config: [MCP context compression](docs/mcp-context-compression.md).

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
sqz discover                  # Find missed savings (incl. your weakest-compressing commands)
sqz discover --replay         # Counterfactual replay of your agent transcripts (Claude Code, Kiro)
sqz recall "auth timeout"     # Full-text search everything sqz has compressed
sqz resume                    # Re-inject session context after compaction
sqz vizit                     # Live terminal dashboard (like htop for AI agents)
sqz hook claude               # Process a PreToolUse hook (Claude Code)
sqz hook kiro                 # Legacy; Kiro now uses steering + MCP (sqz init)
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

# 3b. Or just shorten the ref freshness window (seconds, default 1800).
# Useful on clients without a compaction hook; 0 disables refs.
export SQZ_REF_TTL_SECS=300

# 4. MCP passthrough tool (returns input byte-exact, zero transforms)
# Available via tools/list when sqz-mcp is running
```

### Recall: search everything sqz has seen

Everything that flows through sqz is indexed locally (SQLite FTS5, on your
machine, nothing leaves it). When your agent compacts its context and loses
that error message from an hour ago, search for it instead of re-running
the command:

```sh
$ sqz recall "connection refused"
  1. [git · 2h ago]  … curl: (7) Failed to connect: >>connection refused<< on port 5432 …
     full content: sqz expand 3f9a1c05e2d84b17
```

Agents get the same thing as the `sqz_recall` MCP tool.

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

### Honest accounting: cost and regret

Token reduction is not automatically billed-cost reduction — with provider
prompt caching, most context re-transmits at a ~90% discount, and compression
that drops something the agent needed costs extra turns instead
([arXiv:2607.12161](https://arxiv.org/abs/2607.12161)). sqz measures both
sides instead of hand-waving:

- `sqz stats --cost` estimates dollars saved under a prompt-cached billing
  model (cache write ×1.25 once, cache read ×0.10 per subsequent turn), with
  every assumption printed and overridable (`--price-in`, `--reread-turns`,
  `--cache-write-mult`, `--cache-read-mult`). This is the conservative
  estimate: without caching the same savings would bill at full input price
  on every turn.
- `sqz stats` reports regret signals: quick re-runs (the agent re-produced
  byte-identical output within 2 minutes — the repeat bought nothing) and
  ref expands (`§ref§` tokens recovered to original bytes). Both are proxies
  for "compression dropped something the model needed." If one command
  keeps showing up, its formatter needs work — file an issue.

sqz's design already avoids the failure modes that study measured:
compression is deterministic and query-agnostic (never invalidates provider
prompt caches), only new tool output is touched (history is never rewritten),
piped or redirected commands are never intercepted, and the 16-token net-win
gate skips compressions that wouldn't pay for their own markers.
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

1. **Per-command formatters** — 45+ commands across 12 ecosystems get purpose-built compression:

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
   | Mobile | xcodebuild (build + test), adb logcat |
   | .NET | dotnet build/publish/test |
   | System | grep/rg, tree, find/fd, ls, curl/wget |
   | GitHub | gh pr/issue/run (JSON + table) |

   Unknown commands fall through to the generic compression pipeline — no output is ever left uncompressed.

2. **Table compactor** — aligned-column output from tools without a dedicated formatter (`ps aux`, `netstat`, database CLIs) collapses its padding runs to two-space separators. The detector is strict — indented lines, code, YAML, and JSON never match.
3. **Structural summaries** — code files compressed to imports + function signatures + call graph (~70% reduction). The model sees the architecture, not implementation noise.
4. **Dedup cache** — SHA-256 content hash, persistent across sessions. Byte-identical output the model already has = 13-token reference. A line range of a file the model already has in full (`sed -n '40,80p'`, `sqz_read_file` with `offset`/`limit`) = `§ref:HASH:L40-80§`. A re-read after a small edit = only the changed lines. References are only served while the original is still in context (30-minute window, tunable via `SQZ_REF_TTL_SECS`, reset on compaction).
5. **JSON pipeline** — strip nulls → project out debug fields → flatten → collapse arrays → TOON encoding (lossless compact format)
6. **Safe mode** — stack traces, secrets, migrations detected by entropy analysis and routed through with 0% compression

Measured quality, including what each stage drops and what it never touches: [quality benchmark](docs/quality-benchmark.md). For the full technical details, see [docs/](docs/README.md).

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

### Per-project database

Everything sqz persists (stats, dedup cache, sessions) lives in one SQLite
file, `~/.sqz/sessions.db` by default. Set `SQZ_DB_PATH` to keep it
per-project instead:

```sh
# e.g. in the project's .envrc (direnv)
export SQZ_DB_PATH="$PWD/.sqz/sessions.db"
```

Every surface honors it — the shell hook, `sqz stats`/`gain`/`expand`, and
the MCP server (set it under `env` in your MCP config). Prefer absolute
paths: relative values resolve against whatever directory the process runs
from. The parent directory is created if missing (mode 0700, like `~/.sqz`).

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

- [Documentation index](docs/README.md)
- [How to stop AI coding agents from re-reading the same files](docs/stop-rereading-files.md)
- [MCP context compression](docs/mcp-context-compression.md)
- [Quality benchmark: compression ratio vs information preservation](docs/quality-benchmark.md)
- [Context rot vs context compression](docs/context-rot.md)
- [Choosing a context compression approach](docs/comparison.md)
- [White Paper: Pre-Injection Context Compression](docs/whitepaper.md)
- [Token Savings Benchmark](docs/benchmark.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [Changelog](CHANGELOG.md)
- MCP Registry name: `mcp-name: io.github.ojuschugh1/sqz`

## Star History

<a href="https://star-history.dera.page/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="Star History Chart" src="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>

## Contributors

Thanks to everyone who has contributed code, fixes, and ideas to sqz:

<a href="https://github.com/ojuschugh1/sqz/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=ojuschugh1/sqz" alt="sqz contributors" />
</a>

And to everyone who filed the detailed bug reports behind our fixes. Precise repros make this project better with every release. Want to join them? PRs are reviewed fast: see the [open issues](https://github.com/ojuschugh1/sqz/issues) to get started.

<sub>Contributor grid made with [contrib.rocks](https://contrib.rocks).</sub>
