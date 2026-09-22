# sqz: context compression for AI coding agents

sqz compresses tool output before it enters the model's context window. It runs as a hook and an MCP server for Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Codex, Kiro, Zed and Copilot CLI, and as a proxy in front of any other MCP server. Deterministic Rust, no LLM calls, offline, and every compressed result can be recovered byte-exact.

```sh
cargo install sqz-cli sqz-mcp     # or: brew install ojuschugh1/sqz/sqz · npm i -g sqz-cli · pipx install sqz
sqz init                          # installs hooks, MCP server and agent guidance for every client it finds
```

Then work as usual. `sqz stats --breakdown` shows what it saved, per command, from your own sessions. `sqz doctor` verifies the whole chain: binary, database, shell hook, which clients are actually routed through sqz, and whether anything was compressed recently.

Not installed yet? If you already use Claude Code or Kiro, `sqz discover --replay` replays your existing transcripts through the engine and reports what it would have saved — a counterfactual estimate on your own sessions, computed locally.

## Where the tokens actually go

Across 3,003 real compressions sqz saved 178,442 tokens, a 24.7% average reduction. That number is deliberately not the headline, because most of the tools in this space are graded on ratio alone, and a compressor that hits 90% by dropping the line the agent needed has made the session more expensive.

What the data shows instead:

- **Command output dominates.** Build logs, test runs, `git status`, package installs. A 1,200-token `cargo test` with one failure becomes the failure, the assertion and the `file:line`, about 120 tokens. Forty-five commands have purpose-built formatters; unknown output goes through a generic pipeline that folds repeated lines and never touches source code, stack traces or secrets.
- **Identical file re-reads are rare.** Measured on 92 Claude Code sessions by a user who counted: 1 in 542 reads. What repeats is unchanged command output (a 13-token `§ref§`), line ranges of a file already read in full (a `§ref:HASH:L40-80§`), and files re-read after a small edit (a line-level delta).
- **Over-compression is tracked, not hidden.** `sqz stats` reports how often the agent re-ran a command whose output had not changed, or asked for an original back. Those regret signals sit next to the savings so you can discount the number yourself.

## Guides

- [Stop AI coding agents from re-reading the same files](stop-rereading-files.md): what repeats, the three kinds of reference, and the one path sqz cannot see.
- [MCP context compression](mcp-context-compression.md): the `sqz-mcp` tools, the proxy for any MCP server, per-client configuration.
- [Context rot vs context compression](context-rot.md): why compaction arrives late and what proactive context management looks like.
- [Choosing a context compression approach](comparison.md): shell wrappers, session dedup, MCP proxies, retrieval and LLM summarisation compared by approach, plus how to compare tools yourself.

## Measurements

- [Quality benchmark](quality-benchmark.md): compression ratio against information preservation, run as a regression test on every change. Includes the zeros and the bugs it found.
- [Token savings benchmark](benchmark.md): session-level dedup and delta numbers.
- [White paper](whitepaper.md): pre-injection context compression for agentic code generation.
- [Per-formatter reductions](https://github.com/ojuschugh1/sqz/blob/main/BENCHMARKS.md), backed by fixtures.

## Integrations

- [Overview](integrations/README.md): three integration levels and a config snippet for every client.
- [Claude Code](integrations/level2/claude-code.md), [Cursor](integrations/level2/cursor.md), [Codex CLI](integrations/level2/codex.md), [all platforms](integrations/level2/all-platforms.md).
- [VS Code](integrations/level3/vscode-marketplace.md), [JetBrains](integrations/level3/jetbrains-marketplace.md), [Firefox](integrations/level3/firefox-addons.md), [Chrome](integrations/level3/chrome-web-store.md), [API proxy](integrations/level3/api-proxy.md).

## Reference

- [Source and issues](https://github.com/ojuschugh1/sqz) on GitHub. If sqz mangles a command you use daily, the best report is the command and its output; it becomes a fixture in the quality benchmark.
- API docs: [sqz-engine](https://docs.rs/sqz-engine), [sqz-mcp](https://docs.rs/sqz-mcp) on docs.rs, or [this site's copy](api/sqz_engine/index.html).
- [Changelog](https://github.com/ojuschugh1/sqz/blob/main/CHANGELOG.md). Machine-readable index for AI tools: [llms.txt](https://github.com/ojuschugh1/sqz/blob/main/llms.txt).
