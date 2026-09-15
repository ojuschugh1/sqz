# Codex CLI

OpenAI's Codex CLI has no per-tool-call hook (the only documented event hook, `notify`, fires at the end of a turn, after the model has already seen the tool output), so sqz cannot rewrite its shell commands transparently the way it does for Claude Code or Cursor. Two things Codex does support reliably carry the integration instead:

1. **`AGENTS.md`** at the project root, read at session start. `sqz init` writes a fenced block that tells Codex to pipe long-running commands through `sqz compress`, when not to, and how to recover from a `§ref:HASH§` token.
2. **MCP servers** in `~/.codex/config.toml` under `[mcp_servers.<id>]`. `sqz init` merges an `sqz` entry so Codex gets `compress`, `passthrough`, `expand`, `sqz_recall`, `sqz_read_file`, `sqz_grep` and `sqz_list_dir`.

## Setup

```sh
cargo install sqz-cli sqz-mcp     # or brew / npm / pipx, see the README
cd your-project
sqz init --only codex
```

`sqz init` with no `--only` does the same as part of setting up every tool it detects.

What it writes:

- `./AGENTS.md`: appends (or creates) a block between `<!-- BEGIN sqz-agents-guidance -->` and `<!-- END sqz-agents-guidance -->` markers. Anything else in the file is left alone, and re-running `sqz init` replaces only that block.
- `~/.codex/config.toml` (or `$CODEX_HOME/config.toml` if set): adds

  ```toml
  [mcp_servers.sqz]
  command = "sqz-mcp"
  args = ["--transport", "stdio"]
  ```

  The key is `mcp_servers` (snake_case). `mcpServers` is the JSON convention used by other clients and Codex ignores it. If an `[mcp_servers.sqz]` entry with a `command` already exists, sqz does not overwrite it. Other servers in the file are untouched.

Verify:

```sh
sqz status
```

Then start Codex in the project and ask it to run a noisy command (`npm install`, `cargo test`). You should see `[sqz] N/M tokens (X% reduction)` in the tool output, and repeated file reads coming back as `§ref:…§` references.

## What you get

- Shell output compressed when Codex follows the AGENTS.md guidance and pipes through `sqz compress` (per-command formatters for 45+ commands, dedup references on repeats, safe mode for stack traces and secrets).
- Compressed file reads, greps and directory listings through the `sqz_*` MCP tools, with 13-token references on repeat reads and line-level deltas after small edits.
- `sqz_recall` for recovering exact tool output after Codex compacts its context.
- `expand` / `passthrough` as escape hatches, and `SQZ_NO_DEDUP=1` / `SQZ_NO_ABBREV=1` as per-command opt-outs, all documented in the block Codex reads.

## Limits

Because the shell side depends on guidance rather than a hook, compression is only as consistent as Codex's adherence to AGENTS.md. In practice it follows the block well for the commands listed in it. If `sqz stats --breakdown` shows a command you run often with no savings, add it to the examples in the AGENTS.md block, or use the MCP tools for that operation instead.

Codex's `features.codex_hooks` flag is documented as experimental and off by default. sqz will target it once the `hooks.json` schema is stable.

## Uninstall

```sh
sqz uninstall
```

Removes the AGENTS.md block (leaving the rest of the file) and the `[mcp_servers.sqz]` table (leaving other servers).
