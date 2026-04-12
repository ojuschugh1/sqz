# Kiro

Kiro supports MCP servers via Powers and benefits from sqz shell hooks for CLI output compression.

## Setup

### 1. Install shell hooks

```sh
sqz init
```

### 2. Add MCP config

Edit `.kiro/settings/mcp.json` in your workspace (or the global Kiro MCP config):

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

### 3. Reload Kiro

Kiro picks up MCP config changes automatically or on window reload.

## What you get

- CLI output compressed before it reaches the Kiro agent context
- MCP tool responses filtered through the 8-stage compression pipeline
- Semantic tool selection reduces tool noise in the context window
- Token budget tracking across agent tasks
