# MCP Context Compression for AI Coding Agents

Every MCP tool result an agent receives is billed as input tokens, on every turn, for the rest of the session. A verbose `tools/list` alone can cost 10-40k tokens before the agent has done anything. `sqz-mcp` is a compression layer for that traffic: an MCP server with compressed file, grep and directory tools, and a proxy that wraps any other MCP server and compresses what comes back. No LLM calls, deterministic, offline, and every compressed result is recoverable byte-exact.

Install (`sqz init` registers it with every client it finds):

```sh
cargo install sqz-cli sqz-mcp      # or: brew install ojuschugh1/sqz/sqz, npm i -g sqz-cli, pipx install sqz
sqz init
```

Registry name: `io.github.ojuschugh1/sqz` on the [official MCP Registry](https://registry.modelcontextprotocol.io). API docs: [docs.rs/sqz-mcp](https://docs.rs/sqz-mcp).

## Two ways to use it

### 1. As an MCP server: compressed replacements for the expensive tools

Agents burn most of their context on three operations: reading files, grepping, and listing directories. `sqz-mcp` exposes drop-in versions of each, plus tools for compressing, recovering and searching content:

| Tool | What it does | Why it saves tokens |
|---|---|---|
| `sqz_read_file` | Read a file. Content is returned faithfully (every line, identifier and line number; only ANSI codes stripped). | A repeat read of unchanged content returns a 13-token `§ref:HASH§` reference instead of the file. Agents re-read the same file after every edit; this is where the savings are. |
| `sqz_grep` | Literal or regex search under a path, `path:lineno:text` output, capped at `max_matches` (default 200). | The cap stops one popular term from flooding the context. Repeat searches dedupe. |
| `sqz_list_dir` | Directory listing that skips `.git`, `node_modules`, `target`, `dist`, `build`, `vendor`, `__pycache__`. | No permission/owner/date columns, no bulk directories, repeat listings dedupe. |
| `compress` | Run arbitrary text through the full pipeline (per-command formatters, log folding, JSON compaction, dedup). | For output the agent already holds and wants to shrink before re-sending. |
| `passthrough` | Return text unchanged. | Escape hatch for models that cannot parse `§ref§` tokens. |
| `expand` | Resolve a `§ref:HASH§` token (or bare hex prefix) to the original bytes. | Makes every compression reversible. |
| `sqz_recall` | Full-text search (SQLite FTS5) over everything sqz has compressed in past sessions. | Recovers context lost to compaction without re-running commands. |

The read, grep and list tools are lossless by design ([issue #32](https://github.com/ojuschugh1/sqz/issues/32)): a file read that silently dropped lines would break edits. Their token savings come from dedup and from not fetching what the agent does not need, not from summarising.

The tool descriptions themselves are kept short and tell the agent when to prefer the built-in tool instead (tiny config files, byte-exact lockfile reads).

### 2. As a proxy: compress any other MCP server

`sqz-mcp proxy` sits between the client and any stdio MCP server. Prefix the server's command in your MCP config and nothing else changes; the client sees a normal MCP server.

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

On the way back to the client the proxy:

- compresses `tools/call` text results through the full sqz pipeline: dedup `§ref` on repeats, per-command formatters, safe mode for stack traces and secrets, a 16-token net-win gate so small results are never made worse. Originals are persisted, so any result is recoverable.
- compacts `tools/list` descriptions (whitespace collapsed, capped at 400 characters, input schemas untouched). With `--lazy-tools` the cap drops to one sentence (160 characters) and an injected `sqz_tool_help` tool serves the full original documentation for any tool on demand.
- appends an `sqz_expand` tool to the upstream tool list and answers calls to it locally, so the agent can recover any compressed or referenced result byte-exact.
- adds a short note to the `initialize` instructions explaining what `§ref:HASH§` means.

Error results, non-text content, notifications, and every client-to-server message pass through verbatim.

Flags: `--lazy-tools` (one-sentence descriptions plus `sqz_tool_help`), `--no-desc` (keep descriptions verbatim), `--no-cache` (disable dedup refs). Run `sqz-mcp proxy --help`.

## Per-client configuration

`sqz init` writes these for you. Shown here for manual setups and for checking what it did.

Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Copilot CLI, OpenCode and Kiro all take the same JSON shape (Kiro at the user level in `~/.kiro/settings/mcp.json`, applied to every workspace):

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

Codex CLI uses TOML in `~/.codex/config.toml`:

```toml
[mcp_servers.sqz]
command = "sqz-mcp"
args = ["--transport", "stdio"]
```

Zed uses `context_servers` in `settings.json`; Continue uses `~/.continue/config.json`. Ready-to-paste snippets for every client are in [integrations/](integrations/).

Server flags: `--transport stdio|sse` (default stdio), `--port PORT` (SSE, default 3000), `--preset-dir DIR`. Set `SQZ_DB_PATH` in the server's `env` block to keep the dedup cache and stats per project instead of in `~/.sqz/sessions.db`.

## How it relates to the shell hook

For Claude Code, Cursor, Windsurf, Cline, Gemini CLI and Copilot CLI, `sqz init` also installs a PreToolUse hook that pipes Bash output through `sqz compress`. The hook covers shell commands; the MCP server covers file reads, greps and listings, which the hook cannot see because those go through the client's built-in tools rather than Bash. Use both. Kiro, Codex and Zed have no hook mechanism, so there `sqz init` installs steering or AGENTS.md guidance that tells the agent to pipe long commands through `sqz compress` and to prefer the `sqz_*` MCP tools.

## Guarantees

- **Zero LLM calls.** Every transform is deterministic Rust. Same input, same output, no model in the loop, works offline.
- **Recoverable.** Every compressed or deduplicated result can be expanded back to the original bytes with `expand` (server) or `sqz_expand` (proxy).
- **Never worse.** A 16-token net-win gate returns the original when compression would not pay for its own header.
- **Safe mode.** Stack traces, secrets and migration-like content pass through untouched, and secrets are never written to the cache.
- **No telemetry.** The dedup cache and stats live in a local SQLite file you own.

Measured quality, including what compression drops and what it never touches, is in the [quality benchmark](quality-benchmark.md).
