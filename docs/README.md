# sqz documentation

sqz is pre-injection context compression and session deduplication for AI coding agents: it compresses tool output before it enters the model's context window, deterministically, with no LLM calls, and keeps everything recoverable byte-exact.

## Start here

- [README](../README.md): install, supported tools, CLI reference, how compression works.
- [Stop AI coding agents from re-reading the same files](stop-rereading-files.md): the 13-token dedup reference, deltas for edited files, and the one path sqz cannot see.
- [MCP context compression](mcp-context-compression.md): the `sqz-mcp` server tools, the proxy that compresses any other MCP server, per-client configuration.

## Measurements

- [Quality benchmark: compression ratio vs information preservation](quality-benchmark.md): what compression keeps, what it drops, what it never touches, as a regression test.
- [Token savings benchmark](benchmark.md): session-level dedup and delta numbers.
- [BENCHMARKS.md](../BENCHMARKS.md): per-formatter reductions backed by fixtures.

## Background

- [Context rot vs context compression](context-rot.md): why compaction arrives late and what proactive context management looks like.
- [Choosing a context compression approach](comparison.md): shell wrappers, session dedup, MCP proxies, retrieval layers and LLM summarisation compared by approach, plus how to compare tools yourself.
- [White paper: pre-injection context compression](whitepaper.md).

## Integrations

- [Integration guide](integrations/README.md): three integration levels and a config snippet for every client.
- Level 2 (hook or guidance plus MCP): [Claude Code](integrations/level2/claude-code.md), [Cursor](integrations/level2/cursor.md), [Codex CLI](integrations/level2/codex.md), [all platforms](integrations/level2/all-platforms.md).
- Level 3 (marketplaces and browser): [VS Code](integrations/level3/vscode-marketplace.md), [JetBrains](integrations/level3/jetbrains-marketplace.md), [Firefox](integrations/level3/firefox-addons.md), [Chrome](integrations/level3/chrome-web-store.md), [API proxy](integrations/level3/api-proxy.md).

## Reference

- API docs: [docs.rs/sqz-engine](https://docs.rs/sqz-engine), [docs.rs/sqz-mcp](https://docs.rs/sqz-mcp).
- Machine-readable index for AI tools: [llms.txt](../llms.txt).
- [Changelog](../CHANGELOG.md).
