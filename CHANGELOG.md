# Changelog

All notable changes to sqz will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.0] — 2026-04-19

## [0.7.0] — 2026-04-18

### Added

- **Structural summary extraction** — code files compressed to imports + function
  signatures + call graph (~70% reduction). The model sees the architecture, not
  implementation noise.

### Fixed

- **MCP `initialize` capability (issue #3)** — changed `"tools": {}` to
  `"tools": {"listChanged": false}` per MCP 2024-11-05 spec. OpenCode and other
  compliant clients were interpreting the empty object as "no tools" and skipping
  `tools/list`. Regression test added.
- **MCP `tools/list` outputSchema (issue #5)** — all 8 tools declared
  `outputSchema: {type: "string"}` which violates the MCP spec (root type must be
  `"object"` when present). OpenCode rejected all tools during discovery. Fix:
  dropped outputSchema entirely since all tools return plain text via
  `content[{type:"text"}]`, not structured content. Two regression tests added.
- **Windows path escaping in hook configs (issue #2)** — `std::env::current_exe()`
  returns backslash paths on Windows. These were interpolated raw into JSON/TS
  string literals, producing invalid JSON. Added `json_escape_string_value()` helper
  implementing RFC 8259 escaping. Markdown rules files (Windsurf/Cline) keep raw
  paths for copy-paste readability. 7 new tests.
- **Hook format corrections** — matched hook JSON output to official docs for Claude
  Code (`hookSpecificOutput.updatedInput`), Cursor (flat `permission` + `updated_input`,
  `"version": 1`, matcher `"Shell"`), Gemini CLI (`decision` + `hookSpecificOutput.tool_input`),
  and Windsurf (`agent_action_name` + `tool_info.command_line`). Windsurf/Cline
  downgraded to prompt-level `.windsurfrules`/`.clinerules` guidance since they
  don't support command rewriting via hooks.
- **Word abbreviation removed from CLI and WASM paths** — the n-gram abbreviator
  was mangling directory names and filenames in `ls -l` output. Removed from the
  shell hook compression path and browser extension.
- **RLE false-positive on `ls -l` output** — the pattern-run compressor was
  collapsing filenames that happened to share prefixes. Fixed.
- **GitDiffFoldStage false-positive on `ls -l`** — the diff folder was triggering
  on lines starting with `d` (directory entries). Fixed.
- **`sqz init` now asks for confirmation** before modifying existing files.
- **Audit findings addressed** — H-1, M-1, M-2, M-6, M-9, M-12, L-13 from
  external security audit.

### Changed

- Benchmark doc corrected: edited file re-reads use delta encoding (~60-75 tokens),
  not dedup refs (13 tokens). Session totals updated accordingly.
- npm README synced with root README.

### Testing

- 1010 tests total (up from 947 in 0.6.0), 0 new failures
- 1 pre-existing flaky proptest in `api_proxy` (SQLite temp file race, unrelated)

### Also in this release

- **PreCompact hook** — invalidates dedup refs before context compaction so stale
  references don't survive into the next context window.
- **Dedup freshness persistence** — dedup hit tracking now persists across sqz
  processes via SQLite, so `sqz stats` reflects real savings.
- **Dedup stats logging** — dedup hits are now logged so `sqz stats` shows them.
- **Preservation-token verifier** — catches silent identifier rewrites during
  compression (e.g., function names mangled by abbreviation).
- **Cursor downgraded to rules-based guidance** — Cursor cannot rewrite commands
  via hooks; switched to `.cursorrules` prompt-level guidance.
- **Windows install docs** — pointed Windows users at prebuilt binary paths.

## [0.6.0] — 2026-04-17

### Added

- **OpenCode plugin support** — transparent compression for OpenCode via a TypeScript plugin
  (`~/.config/opencode/plugins/sqz.ts`). Unlike other tools that use JSON hook configs,
  OpenCode requires a TS factory function. `sqz init` installs the plugin, creates
  `opencode.json` with MCP config, and handles idempotent re-runs. New `sqz hook opencode`
  subcommand routes to the OpenCode-specific hook processor which handles OpenCode's
  `tool + args` field format (vs `toolName + toolCall` used by Claude Code / Cursor).
  15 new tests covering plugin generation, install, config update, and hook processing.

- **Schema-Aware JSON Projection** — `project_json()` strips API responses to only the
  fields the agent needs, going beyond null removal to eliminate entire irrelevant keys.
  Configurable via field allowlist or deny list. Particularly effective on large API
  responses (GitHub issues, REST payloads) where agents need 3-5 fields out of 50+.

- **`sqz compact` command** — proactive context eviction. The agent can call `sqz compact`
  to summarize and evict stale session context before the window fills, rather than waiting
  for reactive compaction. Supports `--strategy` (keep_recent, keep_relevant, keep_errors)
  and `--retain-minutes` flags.

### Changed

- `generate_hook_configs()` now includes OpenCode in the returned config list
- `install_tool_hooks()` also installs the OpenCode TypeScript plugin (user-level)
- README: OpenCode added to the supported tools table
- `cmd_hook()` in CLI now dispatches `"opencode"` to `process_opencode_hook()` instead
  of the generic `process_hook()`

### Testing

- 947 tests total (up from 800 in 0.5.0), 0 failures
- 15 new OpenCode plugin tests
- 1 pre-existing flaky proptest in `api_proxy` (SQLite temp file race, unrelated)

## [0.5.0] — 2026-04-16

### Added

#### Novel Features (no competitor has these)
- **Compression Transparency Protocol** — structured annotations (`[sqz: 847→312 tokens | stripped: 12 nulls | confidence: 0.97 ✓]`) that tell the LLM exactly what was compressed, so it can decide whether to re-read content in full
- **Compression Regret Tracker** — learns from compression mistakes per-file. When the LLM re-reads dedup'd content or the verifier triggers a fallback, aggressiveness is reduced for that file. Successful compressions slowly recover aggressiveness. Produces per-file profiles and regret reports
- **Compression Cascades** — multi-level degradation as content ages out of relevance: Fresh (full compressed) → Aging (signatures + changed lines) → Old (file name + public API count) → Ancient (one-line reference). Configurable turn thresholds. sqz controls what's lost, not the LLM's unpredictable compaction

#### Advanced Compression Algorithms
- **MinHash + LSH** — locality-sensitive hashing for O(1) near-duplicate detection in the cache, replacing linear scans
- **Parse Tree Compressor** — tree-sitter-based code compression that collapses low-entropy AST subtrees while preserving high-entropy (information-dense) nodes
- **AST Delta Encoding** — tree-sitter-powered semantic diffs that produce compact change descriptions instead of line-level diffs
- **KV Cache Optimizer** — preserves attention sink tokens (first N tokens) and prompt cache boundaries during compression for better LLM comprehension
- **Adaptive Semantic Tree** — builds a priority-scored tree from document structure and prunes to a token budget, with optional query-aware relevance boosting

#### API Proxy
- `sqz proxy --port 8080` — HTTP proxy that intercepts full LLM API request payloads (OpenAI, Anthropic, Google formats) and compresses them before forwarding. Tracks per-request compression stats

### Changed
- README rewritten — honest benchmark numbers, separated measured (single-command) from session-level (with dedup) savings tables
- Benchmark table now matches actual `cargo test -p sqz-engine benchmarks` output exactly

### Fixed
- Removed unused imports from `regret_tracker` and `cascade_compressor`
- Confidence router no longer false-positives on git logs containing words like "password" or "migration" in commit messages

### Testing
- 800 tests (796 unit + 4 doc tests), 0 failures
- Property-based tests cover all new modules

## [0.1.0] — 2026-04-11

### Added

#### Phase 1 — Core Engine + CLI Proxy
- Rust workspace with 4 crates: `sqz_engine`, `sqz`, `sqz-mcp`, `sqz-wasm`
- Core data model types and enums (`Content`, `Session`, `Preset` with `PresetHeader`, `CacheResult`, etc.; `SessionState` / `PresetMeta` kept as compatibility aliases)
- TOON encoder/decoder — lossless JSON compression with ASCII-safe output
- 8-stage compression pipeline (keep_fields, strip_fields, condense, strip_nulls, flatten, truncate_strings, collapse_arrays, custom_transforms)
- TOML preset parser with validation and hot-reload
- SQLite FTS5 session store with full-text search
- SHA-256 file cache with LRU eviction and cross-session persistence
- Immutable correction log with compaction protection
- Cost calculator with per-tool USD breakdown and cache discount awareness
- Budget tracker with multi-agent support and predictive warnings
- Pin/unpin content protection from compaction
- Tree-sitter AST parser for 18 programming languages
- Prompt cache detector for Anthropic (90%) and OpenAI (50%) boundaries
- Model router with complexity-based local/remote routing
- Terse mode system prompt injection (3 levels)
- CTX format serializer/deserializer for cross-model session portability
- Plugin API (Rust trait + WASM interface) with priority-ordered pipeline insertion
- SqzEngine facade wiring all modules together
- CLI binary with shell hooks (Bash, Zsh, Fish, PowerShell)
- CLI commands: init, compress, export, import, status, cost
- 100+ CLI compression patterns
- Cross-compilation configs for 5 platforms (Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64)
- Distribution: cargo, brew, npm, pip, curl script, Docker, GitHub Releases

#### Phase 2 — MCP Server
- MCP server with stdio and SSE transports
- Tool selector with Jaccard similarity matching
- Preset hot-reload via file watcher (<2s)
- JSON-RPC 2.0 handler (initialize, tools/list, tools/call)
- Platform integration configs for 15 Level 1 + Level 2 platforms
- npm and pip distribution wrappers
- Homebrew formula

#### Phase 3 — Browser Extension (WASM)
- WASM build target with self-contained TOON encoder
- Chrome extension manifest v3
- Content scripts for 5 web UIs (ChatGPT, Claude.ai, Gemini, Grok, Perplexity)
- Compression preview banner for content > 500 tokens
- Settings popup with stats display

#### Phase 4 — IDE Native Extensions
- VS Code extension with CLI bridge, status bar widget, 7 commands
- JetBrains plugin with CLI bridge, status bar widget, 5 actions
- Image-to-semantic-description compression (95%+ reduction)
- Level 3 platform publishing guides (VS Code Marketplace, JetBrains Marketplace, Chrome Web Store, API proxy)

#### Testing
- 753 tests across all crates
- 81 property-based correctness properties via proptest
- Property tests cover: TOON round-trip, token reduction, ASCII safety, cache dedup/invalidation/LRU/persistence, budget invariants, pin round-trips, CTX round-trip, preset round-trip, plugin priority, tool selection cardinality, model routing, terse mode injection, prompt cache preservation, cross-tokenizer determinism, and more
