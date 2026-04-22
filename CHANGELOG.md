# Changelog

All notable changes to sqz will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.5] — 2026-04-22

### Added

- **`sqz init --only <agents>`** — comma-separated list of agents to install.
  Only the named tools get configured; all others are skipped. Accepts:
  claude, cursor, windsurf, cline, gemini, opencode, codex. Aliases like
  `claude-code`, `gemini-cli`, `roo` (= cline) are also recognised.
  Requested in issue #11 (@shochdoerfer).
- **`sqz init --skip <agents>`** — exclude list (complementary to `--only`).
  Cannot be combined with `--only`.
- **`sqz compress --cmd <name>`** — pass the base command label as a CLI
  argument instead of the `SQZ_CMD=` env var. Shell-neutral: works in
  PowerShell, cmd.exe, and POSIX shells.

### Fixed

- **OpenCode plugin shows as `file:///...` instead of "sqz"** — migrated
  the TS plugin to OpenCode's V1 shape with `export default { id: "sqz",
  server: factory }`. Modern OpenCode reads the `id` field and displays
  "sqz" in the plugin list. Legacy OpenCode still loads via the named
  export fallback. Both paths fire exactly once (identity dedup).
- **Windows: `SQZ_CMD=cmd` CommandNotFoundException** (issue #10) — the
  rewritten command used sh-style inline env-var syntax which fails in
  PowerShell and cmd.exe. Changed all 3 emission sites (tool_hooks.rs,
  opencode_plugin.rs TS, opencode_plugin.rs Rust) to use `--cmd NAME`
  instead. `SQZ_CMD=` still recognised as legacy fallback.
- **OpenCode duplicate plugin load** (issue #10) — `sqz init` registered
  the plugin both via `opencode.json` (`"plugin": ["sqz"]`) and via
  `~/.config/opencode/plugins/sqz.ts`. OpenCode loaded both. Fix: stop
  writing the `plugin` array entry; rely on the auto-discovered .ts file.
  Legacy entries cleaned up on re-run.
- **npm install: nested tarball layout** — handle pre-v1.0 release tarballs
  that contain a wrapper directory instead of a flat binary.

### Testing

- 1099 tests total, 0 failures

## [1.0.4] — 2026-04-21

### Fixed

- **CI: `rm: sqz: is a directory`** — the packaging step copied the binary
  to `./sqz` which collided with the `sqz/` crate directory. Now stages
  binaries in a temp directory before creating tarballs.

## [1.0.3] — 2026-04-21

### Fixed

- **CI: Exclude sqz-wasm from native builds** — `sqz-wasm` depends on
  `wasm-bindgen` which can't compile for non-wasm targets. The release
  workflow now uses `--bin sqz --bin sqz-mcp` (single command, two binaries)
  instead of building the full workspace. Benchmarks use `--exclude sqz-wasm`.
  Docs use `--exclude sqz-wasm`. This was the root cause of v1.0.0, v1.0.1,
  and v1.0.2 build failures on Linux and macOS.

## [1.0.2] — 2026-04-21

### Fixed

- **CI: Release workflow builds all targets again** — v1.0.0 and v1.0.1
  failed to build Linux and macOS binaries because the workflow used
  `--bin sqz` + `--bin sqz-mcp` as separate commands. Changed to build
  the full workspace in one pass. Packaging step now gracefully skips
  missing binaries instead of failing the entire job.

## [1.0.1] — 2026-04-21

### Added

- **`sqz expand <ref>`** — CLI command to recover original content from a
  `§ref:HASH§` dedup token. Accepts hash prefixes or the full `§ref:...§`
  token. Returns exact original bytes from the cache. Exit codes distinguish
  hit (0), no-match (1), ambiguous (1), and error (2).
- **`sqz compress --no-cache`** — per-invocation opt-out from dedup. The
  compression pipeline still runs but the 13-token shortcut never fires.
- **`SQZ_NO_DEDUP=1` env var** — same effect as `--no-cache`, settable once
  in shell config for models that can't handle `§ref:...§` tokens.
- **MCP `passthrough` tool** — returns input byte-exact with zero transforms.
  Agents that need raw data can call this instead of `compress`.
- **MCP `expand` tool** — MCP equivalent of `sqz expand`. Agents can resolve
  dedup refs without shelling out.
- **Original bytes stored in cache** — new `original` BLOB column on
  `cache_entries` so `expand` returns true uncompressed content, not the
  compressed version. Additive migration; pre-migration rows return
  compressed-only with a note.
- **Escape hatch docs in rules files** — Cursor, Windsurf, Cline, and Codex
  AGENTS.md templates now include the four escape paths so agents discover
  them without human intervention.

### Fixed

- Agents that can't parse `§ref:HASH§` tokens (e.g., GLM 5.1 on Synthetic)
  now have four independent ways to bypass dedup, breaking the 500-tiny-call
  loop reported by SquireNed.

## [1.0.0] — 2026-04-21

## [0.10.0] — 2026-04-21

### Added

- **`sqz init --global` / `-g`** — installs Claude Code hooks to user-scope
  `~/.claude/settings.json` so compression works across all projects without
  per-repo setup. Merges with existing user settings (preserves permissions,
  env, statusLine, unrelated hooks). Following RTK's model and Anthropic's
  official scope table (Managed > Local > Project > User).
- **Native OpenAI Codex integration** — `sqz init` now configures Codex via
  `~/.codex/config.toml` MCP server entry.
- **Release workflow ships sqz-mcp** — both `sqz` and `sqz-mcp` binaries are
  now built and packaged for all 5 platforms. npm/pip/curl installers updated
  to install both (sqz-mcp is optional — soft failure if tarball missing).

### Fixed

- **npm install silent failure** — the postinstall script expected sqz-mcp
  tarballs that weren't in the release. Now handles missing sqz-mcp gracefully
  and rejects tarballs that unpack as directories instead of binaries.
- **`sqz init` project-scope was invisible across projects** — hooks written to
  `.claude/settings.local.json` only applied inside that one repo. `--global`
  is now the recommended first-install path (documented in README).
- **OpenCode plugin double-wrap** — `SQZ_CMD=SQZ_CMD=ddev ...` runaway prefix
  from issue #5 follow-up. Added `isAlreadyWrapped()` guard checking for
  `SQZ_CMD=`, `sqz compress`, pipe-to-sqz, and bare sqz invocations.
- **OpenCode plugin env-var base extraction** — `FOO=bar make test` now picks
  `make` as the base command, not `FOO=bar`.
- **MCP `tools/list` outputSchema** (issue #5) — dropped invalid
  `outputSchema: {type: "string"}` from all tools. OpenCode's validator
  requires `type: "object"` when present; our tools return plain text so
  outputSchema is now omitted entirely.

### Changed

- `sqz uninstall` now also cleans up user-scope Claude Code settings
  (`~/.claude/settings.json`), removing only sqz entries and preserving
  everything else.
- README updated: `--global` is the recommended install path, Star History
  chart added.

### Testing

- 1062 tests total, 0 failures
- 8 new tests for global install: fresh install, merge semantics, idempotency,
  stale-hook upgrade, uninstall preserves user config, uninstall deletes
  sqz-only files, no-op on missing, refuses corrupted JSON

## [0.9.0] — 2026-04-20

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
