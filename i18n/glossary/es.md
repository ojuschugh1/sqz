# Glossary: Spanish (Español)

Terms that recur in `README.md` and `CONTRIBUTING.md`, and how the Spanish translation writes them. Use neutral international Spanish and address the reader as tú.

| English | Español | Notes |
|---|---|---|
| AI coding agent | agente de programación con IA | |
| client | cliente | Claude Code, Cursor and the other tools sqz configures. |
| tool output | salida de las herramientas | |
| context window | ventana de contexto | |
| token | token | Plural tokens. |
| compress, compression | comprimir, compresión | |
| pre-injection | previa a la inyección | "pre-injection context compression" is compresión de contexto previa a la inyección. |
| dedup, deduplication | deduplicación | |
| reference | referencia | The `§ref:HASH§` marker is marcador de referencia. |
| dedup cache | caché de deduplicación | |
| hook | hook | Kept in English. Plural hooks. |
| MCP server | servidor MCP | |
| MCP proxy | proxy MCP | |
| guidance | instrucciones | Rules files are archivos de reglas. Kiro's "steering" stays in English. |
| integration tier | nivel de integración | |
| transparent (tier) | transparente (transparent) | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| advisory (tier) | orientativo (advisory) | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| per-command formatter | formateador por comando | |
| compression pipeline | pipeline de compresión | |
| stage | etapa | A compression stage is etapa de compresión. |
| safe mode | modo seguro | |
| stack trace | traza de pila | |
| migration | migración | SQL migrations are migraciones SQL. |
| credential line | línea con credenciales | |
| lossless, lossy | sin pérdida, con pérdida | |
| byte-exact | idéntico byte a byte | |
| session | sesión | |
| transcript | transcripción | |
| compaction | compactación | |
| prompt caching | caché de prompts | |
| counterfactual replay | reproducción contrafactual | |
| estimate | estimación | |
| out of reach | fuera de alcance (out of reach) | A row label in `sqz discover --replay`. |
| regret signals | señales de arrepentimiento (regret signals) | A section of `sqz stats`. |
| quick re-run | reejecución rápida (quick re-runs) | A row label in `sqz stats`. |
| ref expand | expansión de referencia (ref expands) | A row label in `sqz stats`. |
| tokens saved | tokens ahorrados (tokens saved) | Printed by `sqz stats` and `sqz gain`. |
| property-based test | prueba basada en propiedades | |
| pull request, issue | pull request, issue | Kept in English, as on GitHub. |
| Contributor License Agreement | Acuerdo de Licencia de Contribuidor (CLA) | |

## Keep in English

- Product, client and service names: sqz, sqz-mcp, sqz-engine, sqz-wasm, Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Kiro, OpenCode, Codex, Zed, Copilot CLI, VS Code, JetBrains, Chrome, Firefox, GitHub, Homebrew, npm, PyPI, crates.io, Discord.
- Protocols, formats and libraries: MCP, CLI, JSON, TOML, TOON, SQLite, FTS5, SHA-256, cl100k, ANSI, PEM, WASM, Rust, proptest.
- Hook event names: PreToolUse, PostToolUse, BeforeTool, preToolUse.
- License names.
- Everything in code: commands, flags, file names, paths, environment variables, config keys and `§ref:HASH§`.
