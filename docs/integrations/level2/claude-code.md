# Claude Code

Claude Code supports both shell hooks (via `sqz init`) and MCP servers.

## Setup

### 1. Install shell hooks

```sh
sqz init
```

This installs the sqz shell hook into your shell profile (Bash, Zsh, Fish, or PowerShell). Shell output is automatically compressed before it reaches Claude Code.

### 2. Add MCP config

Add the following to your Claude Code MCP config (`.claude/mcp_servers.json` in your project, or the global config):

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

### 3. Verify

```sh
sqz status
```

You should see shell hooks active and the MCP server listed.

## What you get

- CLI output compressed before it reaches the context window
- MCP tool responses filtered through the 8-stage compression pipeline
- Real-time token budget tracking
- Semantic tool selection (3-5 relevant tools per task)
