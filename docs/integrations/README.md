# sqz Platform Integrations

sqz integrates with AI coding tools at three levels, depending on how deeply the platform supports MCP and shell hooks.

## Integration Levels

### Level 1 — MCP Config Only

These platforms support MCP natively. Just add the `sqz-mcp` server to their config file.

| Platform | Config file |
|---|---|
| Continue | `~/.continue/config.json` |
| Zed | `~/.config/zed/settings.json` |

See [`level1/`](./level1/) for ready-to-paste config snippets.

### Level 2 — Shell Hook + MCP

These platforms benefit from both the CLI proxy (shell hook) and the MCP server. Run `sqz init` first to install the shell hooks, then add the MCP config.

**Setup (all Level 2 platforms):**

```sh
# Step 1: install shell hooks
sqz init

# Step 2: add MCP config (see platform-specific guide below)
```

Platforms: Claude Code, Cursor, Copilot, Windsurf, Gemini CLI, Codex, OpenCode, Goose, Aider, Amp

See [`level2/`](./level2/) for platform-specific guides.

### Level 3 — Native / Deep Integration

Native extensions for VS Code, JetBrains, Chrome (5 web UIs), and API proxy mode.

| Platform | Guide |
|---|---|
| VS Code Marketplace | [`level3/vscode-marketplace.md`](./level3/vscode-marketplace.md) |
| JetBrains Marketplace | [`level3/jetbrains-marketplace.md`](./level3/jetbrains-marketplace.md) |
| Chrome Web Store | [`level3/chrome-web-store.md`](./level3/chrome-web-store.md) |
| Firefox Add-ons | [`level3/firefox-addons.md`](./level3/firefox-addons.md) |
| API proxy (OpenAI, Anthropic, Google AI) | [`level3/api-proxy.md`](./level3/api-proxy.md) |

See [`level3/`](./level3/) for publishing guides and proxy configuration.

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
