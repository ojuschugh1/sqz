# Glossary: Korean (한국어)

Terms that recur in `README.md` and `CONTRIBUTING.md`, and how the Korean translation writes them. Write in 합니다체.

| English | 한국어 | Notes |
|---|---|---|
| AI coding agent | AI 코딩 에이전트 | |
| client | 클라이언트 | Claude Code, Cursor and the other tools sqz configures. |
| tool output | 도구 출력 | |
| context window | 컨텍스트 윈도우 | |
| token | 토큰 | |
| compress, compression | 압축하다, 압축 | |
| pre-injection | 주입 전 | "pre-injection context compression" is 주입 전 컨텍스트 압축. |
| dedup, deduplication | 중복 제거 | |
| reference | 참조 | The `§ref:HASH§` marker is 참조 마커. |
| dedup cache | 중복 제거 캐시 | |
| hook | 훅 | |
| MCP server | MCP 서버 | |
| MCP proxy | MCP 프록시 | |
| guidance | 지침 | Rules files are 규칙 파일. Kiro's "steering" stays in English. |
| integration tier | 통합 등급 | |
| transparent (tier) | 투명형(transparent) | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| advisory (tier) | 권고형(advisory) | Printed by `sqz doctor` and `sqz discover --replay`. Keep the English word in parentheses. |
| per-command formatter | 명령별 포매터 | |
| compression pipeline | 압축 파이프라인 | |
| stage | 단계 | A compression stage is 압축 단계. |
| safe mode | 안전 모드 | |
| stack trace | 스택 트레이스 | |
| migration | 마이그레이션 | SQL migrations are SQL 마이그레이션. |
| credential line | 자격 증명 줄 | |
| lossless, lossy | 무손실, 손실 | |
| byte-exact | 바이트 단위로 동일한 | |
| session | 세션 | |
| transcript | 대화 기록 | |
| compaction | 컴팩션 | |
| prompt caching | 프롬프트 캐싱 | |
| counterfactual replay | 반사실적 재생 | |
| estimate | 추정치 | |
| out of reach | 범위 밖(out of reach) | A row label in `sqz discover --replay`. |
| regret signals | 후회 신호(regret signals) | A section of `sqz stats`. |
| quick re-run | 빠른 재실행(quick re-runs) | A row label in `sqz stats`. |
| ref expand | 참조 확장(ref expands) | A row label in `sqz stats`. |
| tokens saved | 절약한 토큰(tokens saved) | Printed by `sqz stats` and `sqz gain`. |
| property-based test | 속성 기반 테스트 | |
| pull request, issue | 풀 리퀘스트, 이슈 | |
| Contributor License Agreement | 기여자 라이선스 계약(CLA) | |

## Keep in English

- Product, client and service names: sqz, sqz-mcp, sqz-engine, sqz-wasm, Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Kiro, OpenCode, Codex, Zed, Copilot CLI, VS Code, JetBrains, Chrome, Firefox, GitHub, Homebrew, npm, PyPI, crates.io, Discord.
- Protocols, formats and libraries: MCP, CLI, JSON, TOML, TOON, SQLite, FTS5, SHA-256, cl100k, ANSI, PEM, WASM, Rust, proptest.
- Hook event names: PreToolUse, PostToolUse, BeforeTool, preToolUse.
- License names.
- Everything in code: commands, flags, file names, paths, environment variables, config keys and `§ref:HASH§`.
