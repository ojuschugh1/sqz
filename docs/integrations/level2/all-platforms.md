# Level 2 Platform Setup — All Platforms

All Level 2 platforms follow the same two-step setup: install shell hooks, then add the MCP config.

## Step 1 — Install shell hooks (all platforms)

```sh
sqz init
```

This installs the sqz hook into your shell profile and creates default presets. Supports Bash, Zsh, Fish, and PowerShell.

## Step 2 — Add MCP config

Find your platform below and paste the config into the appropriate file.

---

### Claude Code

File: `.claude/mcp_servers.json` (project) or global Claude Code MCP config

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

File: `~/.codex/config.json`

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

### OpenCode

File: `opencode.json` (project root) or `~/.config/opencode/opencode.json`

```json
{
  "mcp": {
    "sqz": {
      "command": "sqz-mcp",
      "args": ["--transport", "stdio"],
      "env": {}
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
sqz status
```

You should see:
- Shell hook: active
- MCP server: `sqz-mcp` registered

## What you get on all Level 2 platforms

- CLI output compressed before it reaches the LLM context window
- MCP tool responses filtered through the 8-stage compression pipeline
- Semantic tool selection (3-5 relevant tools per task, not the full list)
- Real-time token budget warnings
- Cost tracking with per-tool breakdown
