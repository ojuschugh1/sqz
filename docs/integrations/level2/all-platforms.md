# Level 2 Platform Setup — All Platforms

All Level 2 platforms follow the same two-step setup: run `sqz init`, then add the MCP config where `sqz init` does not.

## Step 1: run `sqz init` (all platforms)

```sh
sqz init
```

This writes a hook or plugin for Claude Code, Gemini CLI, Copilot CLI and OpenCode, agent guidance for Kiro, Codex, Zed, Cursor, Windsurf and Cline, and the MCP entry for Claude Code, Kiro, Codex, Zed and OpenCode. An existing `.gemini/settings.json` gets the Gemini CLI hook merged in; if it has comments or doesn't parse, `sqz init` leaves it alone and prints the entry to add by hand. It also adds the `sqz_run` shell helpers to your shell profile and creates default presets. Goose, Aider and Amp are not configured by `sqz init`.

## Step 2 — Add MCP config

Find your platform below and paste the config into the appropriate file.

---

### Claude Code

File: `~/.claude.json` (`sqz init` writes this entry for you)

```json
{
  "mcpServers": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {}
    }
  }
}
```

---

### Cursor

File: `~/.cursor/mcp.json`

```json
{
  "mcpServers": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {}
    }
  }
}
```

---

### GitHub Copilot (VS Code)

File: `.vscode/mcp.json` (workspace) or `~/.vscode/mcp.json` (global)

```json
{
  "servers": {
    "sqz": {
      "type": "stdio",
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"]
    }
  }
}
```

---

### Windsurf

File: `~/.codeium/windsurf/mcp_config.json`

```json
{
  "mcpServers": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {}
    }
  }
}
```

---

### Gemini CLI

File: `~/.gemini/settings.json`

```json
{
  "mcpServers": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {}
    }
  }
}
```

---

### Codex (OpenAI)

File: `~/.codex/config.toml` (TOML, snake_case key). `sqz init` merges this entry for you and leaves the rest of the file alone.

```toml
[mcp_servers.sqz]
command = "sqz-mcp"
args = ["--transport", "stdio"]
```

See [codex.md](codex.md) for the full setup.

---

### OpenCode

File: `opencode.json` (project root) or `~/.config/opencode/opencode.json`

```json
{
  "mcp": {
    "sqz": {
      "type": "local",
      "command": ["sqz-mcp", "--transport", "stdio"],
      "enabled": true
    }
  }
}
```

---

### Goose

File: `~/.config/goose/config.yaml`

```yaml
extensions:
  sqz:
    type: stdio
    cmd: sqz-mcp
    args:
      - --transport
      - stdio
    enabled: true
```

---

### Aider

Aider uses MCP via a config file. Add to `~/.aider.conf.yml`:

```yaml
mcp_servers:
  sqz:
    command: sqz-mcp
    args:
      - --transport
      - stdio
```

Or pass on the command line:

```sh
aider --mcp-server sqz-mcp --mcp-args "--transport stdio"
```

---

### Amp

File: `~/.config/amp/mcp.json`

```json
{
  "mcpServers": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {}
    }
  }
}
```

---

## Verify your setup

After completing both steps, run:

```sh
sqz doctor
```

It lists each client `sqz init` configured with its tier (transparent or advisory), checks the shell helpers, and reports whether anything was compressed recently.

## What you get on all Level 2 platforms

- CLI output compressed before it reaches the LLM context window: on its own on transparent clients, when the agent pipes it through `sqz compress` on advisory ones
- `sqz_read_file`, `sqz_grep` and `sqz_list_dir`, with repeat results replaced by a short reference
- Per-command token usage with `sqz stats --breakdown`
