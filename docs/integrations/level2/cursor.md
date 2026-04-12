# Cursor

Cursor supports MCP servers and benefits from sqz shell hooks for CLI output compression.

## Setup

### 1. Install shell hooks

```sh
sqz init
```

### 2. Add MCP config

Open Cursor settings → MCP, or edit `~/.cursor/mcp.json`:

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

### 3. Restart Cursor

Cursor picks up MCP server changes on restart.

## What you get

- Compressed CLI output in Cursor's terminal context
- MCP tool responses filtered through the compression pipeline
- Token budget warnings in real time
