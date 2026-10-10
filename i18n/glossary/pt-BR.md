# Glossary: Brazilian Portuguese (Português (Brasil))

Terms that recur in `README.md` and `CONTRIBUTING.md`, and how the Brazilian Portuguese translation writes them. Address the reader as você.

| English | Português (Brasil) | Notes |
|---|---|---|
| AI coding agent | agente de programação com IA | |
| client | cliente | Claude Code, Cursor and the other tools sqz configures. |
| tool output | saída das ferramentas | |
| context window | janela de contexto | |
| token | token | Plural tokens. |
| compress, compression | comprimir, compressão | |
| pre-injection | antes da injeção | "pre-injection context compression" is compressão de contexto antes da injeção. |
| dedup, deduplication | deduplicação | |
| reference | referência | The `§ref:HASH§` marker is marcador de referência. |
| dedup cache | cache de deduplicação | |
| hook | hook | Kept in English. Plural hooks. |
| MCP server | servidor MCP | |
| MCP proxy | proxy MCP | |
| guidance | instruções | Rules files are arquivos de regras. Kiro's "steering" stays in English. |
| integration tier | nível de integração | |
| transparent (tier) | transparente (transparent) | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| advisory (tier) | orientativo (advisory) | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| per-command formatter | formatador por comando | |
| compression pipeline | pipeline de compressão | |
| stage | etapa | A compression stage is etapa de compressão. |
| safe mode | modo seguro | |
| stack trace | stack trace | Kept in English. |
| migration | migração | SQL migrations are migrações SQL. |
| credential line | linha com credenciais | |
| lossless, lossy | sem perdas, com perdas | |
| byte-exact | idêntico byte a byte | |
| session | sessão | |
| transcript | transcrição | |
| compaction | compactação | |
| prompt caching | cache de prompts | |
| counterfactual replay | reprodução contrafactual | |
| estimate | estimativa | |
| out of reach | fora de alcance (out of reach) | A row label in `sqz discover --replay`. |
| regret signals | sinais de arrependimento (regret signals) | A section of `sqz stats`. |
| quick re-run | reexecução rápida (quick re-runs) | A row label in `sqz stats`. |
| ref expand | expansão de referência (ref expands) | A row label in `sqz stats`. |
| tokens saved | tokens economizados (tokens saved) | Printed by `sqz stats` and `sqz gain`. |
| property-based test | teste baseado em propriedades | |
| pull request, issue | pull request, issue | Kept in English, as on GitHub. |
| Contributor License Agreement | Contrato de Licença de Contribuidor (CLA) | |

## Keep in English

- Product, client and service names: sqz, sqz-mcp, sqz-engine, sqz-wasm, Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Kiro, OpenCode, Codex, Zed, Copilot CLI, VS Code, JetBrains, Chrome, Firefox, GitHub, Homebrew, npm, PyPI, crates.io, Discord.
- Protocols, formats and libraries: MCP, CLI, JSON, TOML, TOON, SQLite, FTS5, SHA-256, cl100k, ANSI, PEM, WASM, Rust, proptest.
- Hook event names: PreToolUse, PostToolUse, BeforeTool, preToolUse.
- License names.
- Everything in code: commands, flags, file names, paths, environment variables, config keys and `§ref:HASH§`.
