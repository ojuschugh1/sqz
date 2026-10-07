# Cursor

Cursor is an advisory client: it can't let sqz rewrite tool calls, so sqz saves tokens there only when the agent pipes output through `sqz compress` or calls the sqz MCP tools. Cursor's built-in terminal and file tools bypass sqz.

## Setup

### 1. Write the rules file

```sh
sqz init
```

This writes `.cursor/rules/sqz.mdc` (`alwaysApply: true`), which tells the agent to pipe long command output through `sqz compress` and to prefer the sqz MCP tools.

### 2. Add MCP config

`sqz init` does not register the MCP server for Cursor. Open Cursor settings → MCP, or edit `~/.cursor/mcp.json`:

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

Cursor picks up MCP server changes on restart. `sqz doctor` shows Cursor as configured, advisory.

## Hooks from older sqz versions

`sqz init` 0.6.0 and 0.7.0 also wrote `.cursor/hooks.json`, with a hook that runs `sqz hook cursor`. From 0.7.0 through 1.9.0, `sqz hook cursor` answered every rewrite with an approval, which lets Cursor run the command without asking. It now prints nothing and exits 1, which Cursor treats as a failed hook: Cursor's own approval applies and the command runs as written, uncompressed. `sqz doctor` reports such a hook in `.cursor/hooks.json` or `~/.cursor/hooks.json`, and `sqz doctor --fix` and `sqz uninstall` remove it, keeping any other hooks in the file and deleting the file when nothing else is left.

## What you get

- CLI output compressed when the agent pipes it through `sqz compress`
- `sqz_read_file`, `sqz_grep` and `sqz_list_dir`, with repeat results replaced by a short reference
