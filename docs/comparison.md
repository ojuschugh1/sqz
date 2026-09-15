# Choosing a Context Compression Approach for AI Coding Agents

There are now several tools that promise to cut what coding agents spend on tokens, and they are not interchangeable. They intercept at different points, make different bets on what is safe to drop, and fail in different ways. This page compares the approaches rather than the products, because the products change monthly and a name-based table would be stale and self-serving within a quarter. Where sqz sits in each category is stated plainly, along with what it does not do.

If you want numbers, the last section describes how to run any tool through the same fixtures sqz uses to grade itself.

## The five approaches

### 1. Shell-command wrappers and hook rewriters

**How it works.** The tool rewrites the agent's shell commands (through a PreToolUse hook, a shell function, or a wrapper binary) so output is piped through a compressor before the agent's harness captures it. Per-command formatters know what `cargo test`, `git status` or `kubectl get pods` look like and keep the parts that matter.

**Strengths.** Transparent: the agent does not know it is there. Deterministic and cheap: no model call, sub-millisecond latency. Highest reduction on verbose, repetitive output (build logs, passing tests, package installs). Precise about what it keeps because each formatter is written for one command's structure.

**Limits.** Only sees shell output. File reads that go through a client's built-in Read tool never pass through Bash and are invisible to it ([anthropics/claude-code#4544](https://github.com/anthropics/claude-code/issues/4544)). Coverage is per command: unknown commands fall back to generic heuristics, which is where over-compression risk lives. Quality depends entirely on how strict the formatters are about dropping content.

**Where sqz sits.** This is sqz's PreToolUse hook: 45+ formatters, a generic pipeline that folds repeated log lines and table padding but refuses to touch source code, stack traces or secrets, and a 16-token net-win gate so nothing is ever made larger. Its exit-status trailer protocol preserves the command's real exit code through the pipe, which a naive `cmd | compress` loses.

### 2. Session-level deduplication

**How it works.** Content-hash everything served to the agent. When the same bytes come back (a file re-read after an edit, `git status` for the twentieth time, the same failing test output), serve a short reference to the earlier copy instead of the content. Near-duplicates can be served as a line-level delta.

**Strengths.** Attacks the single largest waste in real sessions: repetition. Lossless in the strict sense, since the reference points at bytes already in the model's context. Works on any content, not just known commands. A repeat read costs ~13 tokens regardless of file size, so savings scale with how much the agent re-reads, which is a lot.

**Limits.** The reference is only valid while the original is still in the model's context. After a compaction, or after enough time, the tool must know to send the full content again or the model is pointed at something it no longer has. Some models handle reference tokens poorly and need an opt-out. Requires state (a local database) that survives across the short-lived hook processes.

**Where sqz sits.** Dedup is the core of sqz's session model: SHA-256 index in a local SQLite database, 30-minute freshness window, invalidation on the harness's PreCompact hook, line-level deltas for edited files through the MCP read tool, and `sqz expand` to recover any reference byte-exact. `--no-cache` and `SQZ_NO_DEDUP=1` exist for models that cannot parse the tokens.

### 3. MCP-layer compressors and proxies

**How it works.** Sit between the client and MCP servers as an MCP server yourself: either offer compressed replacements for expensive tools (file read, grep, list) or wrap an upstream server and compress its `tools/call` results and `tools/list` descriptions in transit.

**Strengths.** Reaches traffic the shell hook cannot: file reads through MCP tools, third-party server results, and the tool-description bloat that can cost 10-40k tokens per session before any work starts. Client-agnostic, because every MCP client just sees a normal server.

**Limits.** The agent has to choose the compressed tool over the built-in one; that is done with instructions (CLAUDE.md, AGENTS.md, steering), which the agent follows but cannot be forced to. Proxying adds a process hop. Compressing another server's results generically is riskier than compressing a known command, so the safe-content rules matter more here.

**Where sqz sits.** `sqz-mcp` provides `sqz_read_file`, `sqz_grep` and `sqz_list_dir` (lossless, dedup-backed) plus `sqz-mcp proxy` for wrapping any stdio server, with description compaction, `--lazy-tools` one-sentence descriptions backed by an on-demand help tool, and an injected `sqz_expand` for recovery. Details in [MCP context compression](mcp-context-compression.md).

### 4. Retrieval and indexing layers

**How it works.** Index the repository (symbols, embeddings, file summaries) and serve the agent snippets or a repo map in place of full files, so it reads less in the first place.

**Strengths.** Reduces the first read, which the approaches above cannot. Good for orientation in large unfamiliar codebases. Complementary to compression: less enters the context, and what enters can still be compressed.

**Limits.** Returning a summary or snippet in place of a file is a judgement call, and a wrong one means the agent edits without seeing the code it is changing. Requires an index that stays in sync with the working tree. Often runs as a separate daemon with its own cost and failure modes. On code the model has effectively memorised (popular libraries), an index adds tokens without adding information.

**Where sqz sits.** sqz does not do repository indexing. Its structural-summary API (imports plus signatures plus call graph) exists in the engine crate for consumers that want it, but the hook and MCP paths deliberately serve full source on first read. `sqz recall` indexes what sqz has already served, not the repo, so an agent can find an old error message after compaction without re-running the command. A retrieval layer and sqz stack cleanly.

### 5. LLM-based summarisation and task-aware pruning

**How it works.** Send tool output (or the whole transcript) to a model and ask it to keep what is relevant to the current task. This is also what built-in compaction does when the context fills up.

**Strengths.** Can reason about relevance in a way rules cannot. Handles novel output formats. Can achieve the highest reduction percentages on prose-heavy content.

**Limits.** Costs a model call per compression, which can exceed the tokens saved on small outputs. Non-deterministic: the same output compresses differently each run, so behaviour is hard to test and regressions are hard to catch. Lossy in ways that cannot be audited or reversed; the exact line number, the branch name, the precise error string survive only if the summarising model decided they mattered. Adds latency to every tool call. Requires network access and a provider.

**Where sqz sits.** sqz makes zero LLM calls, by design. Every transform is deterministic Rust, runs offline, and is reversible. That rules out the highest headline reductions on prose, and sqz accepts that trade: its measured average across real sessions is about 25%, with 90%+ on the repeats dedup catches, and 0% by construction on error output, secrets and source files. The [quality benchmark](quality-benchmark.md) exists because a compressor should be graded on what it drops, not on its percentage.

## Summary

| Approach | Reaches | Deterministic | Reversible | Main risk |
|---|---|---|---|---|
| Shell wrapper / hook | Bash output | yes | with a cache | formatter drops something needed |
| Session dedup | anything repeated | yes | yes | stale reference after compaction |
| MCP compressor / proxy | file reads, third-party tools, tool descriptions | yes | with a cache | agent ignores the compressed tool |
| Retrieval / indexing | the first read | usually | n/a | snippet served where full file was needed |
| LLM summarisation | anything | no | no | silent loss of exact details, per-call cost |

sqz combines the first three. It does not do the fourth (a retrieval layer stacks with it) and deliberately not the fifth.

## How to compare tools yourself

Compression ratio alone rewards the tool that drops the most. Measure these instead, on the same inputs, for every tool:

1. **Critical-fact recall.** For each realistic output, list the facts an agent needs (failing test name, assertion text, `file:line`, changed diff lines, the crashing pod, the vulnerability count). Check each survives verbatim. One missing fact is a failure, whatever the ratio.
2. **Agent-visible tokens.** Count everything the agent sees, including the tool's own status lines. A header that costs 12 tokens on a 20-token saving is a loss.
3. **Untouchable content.** Stack traces, secrets, source code, diffs. These should come out byte-identical.
4. **Recovery.** Can the original be recovered exactly after compression? With what command?
5. **Regret in real use.** Does the tool track how often the agent re-runs a command whose output had not changed, or asks for the original back? That rate is the field measurement of over-compression.
6. **The built-in Read tool.** Test a session where the agent reads files through the client's native tool, not Bash. Most shell-level tools report a number that excludes this path entirely.

sqz's fixtures for points 1-4 are plain text in [`sqz/tests/quality_bench.rs`](../sqz/tests/quality_bench.rs); pipe them through any other tool and compare. Results for sqz, including the zeros, are in the [quality benchmark](quality-benchmark.md). Session-level dedup numbers are in [benchmark.md](benchmark.md).
