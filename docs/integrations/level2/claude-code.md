# Claude Code

Claude Code is a transparent client: `sqz init` installs hooks that route shell commands through sqz on their own, and registers the sqz MCP server.

## Setup

### 1. Run `sqz init`

```sh
sqz init            # this project: .claude/settings.local.json
sqz init --global   # every project: ~/.claude/settings.json
```

The settings file gets these hooks:

- PreToolUse (Bash and PowerShell): `sqz hook claude` rewrites the command so its output is piped through `sqz compress`.
- PostToolUse (Bash): `sqz hook claude-post` compresses the stdout and stderr of a successful command the PreToolUse hook left alone, through `updatedToolOutput`. Output Claude Code already cut to a preview (`BASH_MAX_OUTPUT_LENGTH`), images and interrupted commands are left as they are.
- PreCompact: `sqz hook precompact` marks cached references stale when Claude Code compacts.
- SessionStart (after compaction): `sqz resume` re-injects the session context.

It also adds two allow rules to `permissions.allow`, `Bash(set -o pipefail)` and `Bash(sqz compress --cmd *)`, which cover only the parts the rewrite adds. The hook does not approve commands: Claude Code's own permission rules still decide whether a command runs or asks first. When Claude Code asks, or denies a command, its message quotes the rewritten command, such as `set -o pipefail; git tag v0 2>&1` for `git tag v0`. Until both rules are there the hook leaves commands alone. Outside bypass mode, pipelines, loops and heredocs are left as written.

Your existing settings are kept; sqz merges its entries.

### 2. MCP server

`sqz init` adds the server to `~/.claude.json` (user scope, with or without `--global`):

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

It also writes a guidance block to `CLAUDE.md` in the project.

### 3. Verify

```sh
sqz doctor
```

Claude Code should show as configured, transparent. Doctor also flags an older install without the allow rules (`sqz doctor --fix` adds them).

## What you get

- Shell output compressed before it reaches the context window
- Repeated output replaced by a short `§ref:HASH§` reference, re-sent in full after compaction
- `sqz_read_file`, `sqz_grep` and `sqz_list_dir` for reads the hook cannot see: Claude Code's built-in Read tool does not go through Bash, and sqz does not rewrite its results
