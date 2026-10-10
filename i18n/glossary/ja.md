# Glossary: Japanese (日本語)

Terms that recur in `README.md` and `CONTRIBUTING.md`, and how the Japanese translation writes them. Write in です・ます調.

| English | 日本語 | Notes |
|---|---|---|
| AI coding agent | AIコーディングエージェント | |
| client | クライアント | Claude Code, Cursor and the other tools sqz configures. |
| tool output | ツールの出力 | |
| context window | コンテキストウィンドウ | |
| token | トークン | |
| compress, compression | 圧縮する、圧縮 | |
| pre-injection | 注入前の | "pre-injection context compression" is 注入前のコンテキスト圧縮. |
| dedup, deduplication | 重複排除 | |
| reference | 参照 | The `§ref:HASH§` marker is 参照マーカー. |
| dedup cache | 重複排除キャッシュ | |
| hook | フック | |
| MCP server | MCPサーバー | |
| MCP proxy | MCPプロキシ | |
| guidance | ガイダンス | Rules files are ルールファイル. Kiro's "steering" stays in English. |
| integration tier | 統合レベル | |
| transparent (tier) | 透過型（transparent） | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| advisory (tier) | 助言型（advisory） | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| per-command formatter | コマンド別フォーマッター | |
| compression pipeline | 圧縮パイプライン | |
| stage | ステージ | A compression stage is 圧縮ステージ. |
| safe mode | セーフモード | |
| stack trace | スタックトレース | |
| migration | マイグレーション | SQL migrations are SQLマイグレーション. |
| credential line | 認証情報の行 | |
| lossless, lossy | 可逆、非可逆 | |
| byte-exact | バイト単位で同一 | |
| session | セッション | |
| transcript | トランスクリプト | |
| compaction | コンパクション | |
| prompt caching | プロンプトキャッシュ | |
| counterfactual replay | 反実仮想リプレイ | |
| estimate | 推定値 | |
| out of reach | 対象外（out of reach） | A row label in `sqz discover --replay`. |
| regret signals | リグレットシグナル（regret signals） | A section of `sqz stats`. |
| quick re-run | 短時間での再実行（quick re-runs） | A row label in `sqz stats`. |
| ref expand | 参照の展開（ref expands） | A row label in `sqz stats`. |
| tokens saved | 節約したトークン（tokens saved） | Printed by `sqz stats` and `sqz gain`. |
| property-based test | プロパティベーステスト | |
| pull request, issue | プルリクエスト、Issue | |
| Contributor License Agreement | コントリビューターライセンス契約（CLA） | |

## Keep in English

- Product, client and service names: sqz, sqz-mcp, sqz-engine, sqz-wasm, Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Kiro, OpenCode, Codex, Zed, Copilot CLI, VS Code, JetBrains, Chrome, Firefox, GitHub, Homebrew, npm, PyPI, crates.io, Discord.
- Protocols, formats and libraries: MCP, CLI, JSON, TOML, TOON, SQLite, FTS5, SHA-256, cl100k, ANSI, PEM, WASM, Rust, proptest.
- Hook event names: PreToolUse, PostToolUse, BeforeTool, preToolUse.
- License names.
- Everything in code: commands, flags, file names, paths, environment variables, config keys and `§ref:HASH§`.
