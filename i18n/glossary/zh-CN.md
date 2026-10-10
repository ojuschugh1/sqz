# Glossary: Simplified Chinese (简体中文)

Terms that recur in `README.md` and `CONTRIBUTING.md`, and how the Simplified Chinese translation writes them. Put a half-width space between Chinese characters and Latin letters or digits, as in the examples below.

| English | 简体中文 | Notes |
|---|---|---|
| AI coding agent | AI 编程智能体 | |
| client | 客户端 | Claude Code, Cursor and the other tools sqz configures. |
| tool output | 工具输出 | |
| context window | 上下文窗口 | |
| token | token | Kept in English. |
| compress, compression | 压缩 | |
| pre-injection | 注入前 | "pre-injection context compression" is 注入前上下文压缩. |
| dedup, deduplication | 去重 | |
| reference | 引用 | The `§ref:HASH§` marker is 引用标记. |
| dedup cache | 去重缓存 | |
| hook | 钩子 | |
| MCP server | MCP 服务器 | |
| MCP proxy | MCP 代理 | |
| guidance | 指引 | Rules files are 规则文件. Kiro's "steering" stays in English. |
| integration tier | 集成层级 | |
| transparent (tier) | 透明式（transparent） | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| advisory (tier) | 建议式（advisory） | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| per-command formatter | 按命令格式化器 | |
| compression pipeline | 压缩流水线 | |
| stage | 阶段 | A compression stage is 压缩阶段. |
| safe mode | 安全模式 | |
| stack trace | 堆栈跟踪 | |
| migration | 迁移 | SQL migrations are SQL 迁移. |
| credential line | 凭据行 | |
| lossless, lossy | 无损，有损 | |
| byte-exact | 逐字节一致 | |
| session | 会话 | |
| transcript | 会话记录 | |
| compaction | 上下文精简（compaction） | Keeps it apart from sqz's 压缩. |
| prompt caching | 提示缓存 | |
| counterfactual replay | 反事实回放 | |
| estimate | 估算 | |
| out of reach | 无法触及（out of reach） | A row label in `sqz discover --replay`. |
| regret signals | 遗憾信号（regret signals） | A section of `sqz stats`. |
| quick re-run | 快速重跑（quick re-runs） | A row label in `sqz stats`. |
| ref expand | 引用展开（ref expands） | A row label in `sqz stats`. |
| tokens saved | 节省的 token（tokens saved） | Printed by `sqz stats` and `sqz gain`. |
| property-based test | 基于属性的测试 | |
| pull request, issue | pull request，issue | Kept in English, as on GitHub. |
| Contributor License Agreement | 贡献者许可协议（CLA） | |

## Keep in English

- Product, client and service names: sqz, sqz-mcp, sqz-engine, sqz-wasm, Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Kiro, OpenCode, Codex, Zed, Copilot CLI, VS Code, JetBrains, Chrome, Firefox, GitHub, Homebrew, npm, PyPI, crates.io, Discord.
- Protocols, formats and libraries: MCP, CLI, JSON, TOML, TOON, SQLite, FTS5, SHA-256, cl100k, ANSI, PEM, WASM, Rust, proptest.
- Hook event names: PreToolUse, PostToolUse, BeforeTool, preToolUse.
- License names.
- Everything in code: commands, flags, file names, paths, environment variables, config keys and `§ref:HASH§`.
