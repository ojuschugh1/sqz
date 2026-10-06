# sqz Platform Integrations

sqz integrates with AI coding tools at three levels, depending on how deeply the platform supports MCP and shell hooks.

## Integration Levels

### Level 1 — MCP Config Only

These platforms support MCP natively. Just add the `sqz-mcp` server to their config file.

| Platform | Config file |
|---|---|
| Continue | `~/.continue/config.json` |
| Zed | `~/.config/zed/settings.json` |

See [`level1/continue.json`](./level1/continue.json) and [`level1/zed.json`](./level1/zed.json) for ready-to-paste config snippets.

### Level 2: `sqz init` + MCP

These platforms use the CLI proxy (`sqz compress`) and the MCP server. On transparent clients (Claude Code, Gemini CLI, Copilot CLI, OpenCode) `sqz init` installs a hook or plugin that routes shell commands through sqz on its own. On advisory clients (Kiro, Codex, Zed, Cursor, Windsurf, Cline) it writes agent guidance (steering, `AGENTS.md` or a rules file), and sqz saves tokens only when the agent pipes output through `sqz compress` or calls the sqz MCP tools. `sqz init` registers the MCP server for Claude Code, Kiro, Codex, Zed and OpenCode; for the other clients add the MCP config by hand. Goose, Aider and Amp are not configured by `sqz init`; they take the MCP config only.

**Setup (all Level 2 platforms):**

```sh
# Step 1: write hooks, guidance and MCP entries for the clients sqz init supports
sqz init

# Step 2: add MCP config where sqz init does not (see platform-specific guide below)
```

Platforms: Claude Code, Cursor, Copilot, Windsurf, Gemini CLI, Codex, OpenCode, Goose, Aider, Amp

`sqz doctor` shows what was configured for each client and its tier.

See [Claude Code](./level2/claude-code.md), [Cursor](./level2/cursor.md), [Codex CLI](./level2/codex.md) and [all platforms](./level2/all-platforms.md) for platform-specific guides.

### Level 3 — Native / Deep Integration

Native extensions for VS Code, JetBrains, Chrome (5 web UIs), and API proxy mode.

| Platform | Guide |
|---|---|
| VS Code Marketplace | [`level3/vscode-marketplace.md`](./level3/vscode-marketplace.md) |
| JetBrains Marketplace | [`level3/jetbrains-marketplace.md`](./level3/jetbrains-marketplace.md) |
| Chrome Web Store | [`level3/chrome-web-store.md`](./level3/chrome-web-store.md) |
| Firefox Add-ons | [`level3/firefox-addons.md`](./level3/firefox-addons.md) |
| API proxy (OpenAI, Anthropic, Google AI) | [`level3/api-proxy.md`](./level3/api-proxy.md) |

The guides in the table cover publishing to each store; [`level3/api-proxy.md`](./level3/api-proxy.md) covers proxy configuration.

---

## MCP Server Config (all platforms)

The `sqz-mcp` binary is the MCP server. The config is the same across all platforms:

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

Make sure `sqz-mcp` is on your `PATH` (it's installed alongside `sqz`).
