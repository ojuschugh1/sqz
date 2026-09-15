# Context Rot vs Context Compression: Proactive vs Reactive Context Management for AI Coding Agents

"Context rot" is the name that has stuck for a pattern anyone running long agent sessions has seen: the model gets worse before it gets full. Answers reference stale state, edits go to the wrong file, the agent repeats a command it ran ten minutes ago. Nothing has overflowed yet. The context is simply carrying too much that no longer matters, and the useful parts are getting harder to find.

This page is about where that degradation comes from, why the usual remedy (compaction) arrives late, and what it means to manage context before it enters the window rather than after.

## Where the rot comes from

Almost none of it is the conversation. In a coding session the transcript is dominated by tool output: build logs, test runs, `git status`, directory listings, file contents. Three properties of that output cause the damage.

**Volume.** A single `npm install` can be 1,000 tokens of deprecation notices around one line the agent needs. A 2,000-line build log usually matters for six lines. Everything else is dead weight the model has to attend across on every turn.

**Repetition.** The same file read after every edit. `git status` twenty times. The same test suite run until it passes, with the same 40 passing tests listed each time. Repeated content is worse than merely wasteful: near-identical blocks compete for the model's attention and make it easier to pick up the stale copy.

**Persistence.** Once tool output is in the transcript it stays there and is re-sent on every following turn. A verbose result from turn 3 is still being paid for, and still diluting attention, on turn 60.

## Why compaction is late

Every major agent harness has a compaction step: when the context approaches its limit, an LLM summarises the transcript and the session continues from the summary. It is necessary. It is also reactive by construction.

- It triggers on size, not on quality. Rot accumulates from the first verbose tool result; compaction does nothing until the window is nearly full. In between, every turn is paying for and attending across the accumulated waste.
- It costs a model call. Summarising a 150k-token transcript is itself expensive, and it happens at the moment the session is most expensive.
- It is lossy in ways you cannot audit. The summary keeps what the summarising model judged important. The exact error message from an hour ago, the precise line number, the branch name: whether those survive is up to another LLM, and there is no way to get them back if they do not.
- It resets the agent's memory of what it has seen, so the re-reads start again.

Compaction treats the symptom at the last possible moment. The cause is upstream: what was allowed into the context in the first place.

## Pre-injection context compression

The alternative is to intervene at the point where tool output is about to enter the context, and make three decisions there, deterministically, before the model sees anything:

1. **Is this a repeat?** If the same bytes were served earlier in the session and are still in the model's context, send a 13-token reference instead. This removes repetition at the source and does not depend on a model noticing the duplicate.
2. **What in this output carries information?** For a known command, a purpose-built formatter keeps what an agent acts on (the failing test, the assertion, the `file:line`, the changed lines of a diff, the pod that is crashing) and drops what it does not (progress dots, passing tests, deprecation prose, alignment padding). For unknown output, generic stages fold repeated log lines and collapse table padding, and stop there.
3. **Is this something that must not be touched?** Stack traces, secrets, migrations and source code an agent is about to edit pass through untouched. The rule is that a wrong drop costs more than any saving.

This is what sqz does. It runs as a PreToolUse hook (Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Copilot CLI) and as an MCP server for file reads, greps and directory listings, so the compression happens inside the tool call, before the result is appended to the transcript. There is no model in the loop: the transforms are deterministic Rust, so the same output compresses the same way every time, and everything that was compressed can be recovered byte-exact with `sqz expand`.

Because it acts before injection, the effect is cumulative in the right direction. Each turn's transcript is smaller than it would have been, so every following turn is cheaper and less diluted, and compaction is reached later or not at all.

## Proactive and reactive are not exclusive

Pre-injection compression does not replace compaction; it changes what compaction has to work with. A transcript that never took in the 1,000 tokens of `npm warn deprecated` does not need a model to summarise them away. The two also interact: when compaction does happen, sqz's PreCompact hook marks its cached references stale, so the next file read sends the full file rather than pointing at content the summary may have dropped.

And when compaction has already lost something, `sqz recall` searches everything sqz served during the session (a local full-text index, nothing leaves the machine) so the agent can recover the exact error output from an hour ago instead of re-running the command that produced it.

## What to be sceptical of

Any tool in this space, sqz included, should be judged on what it drops, not on its reduction percentage. A compressor that hits 90% by removing the line the agent needed has made the session more expensive, because the agent re-runs the command, retries in circles, or produces a worse answer. sqz publishes a [quality benchmark](quality-benchmark.md) that measures fact retention alongside reduction, fails its own build when a fact goes missing, and tracks regret signals (quick re-runs and reference expands) in normal use so over-compression is visible in `sqz stats` rather than hidden in a headline number.

The honest summary of the measured effect: about 25% average token reduction across real sessions, rising to 90%+ on the repeat reads that dedup catches, with zero loss on error output, secrets and source files by construction. Compaction still happens. It happens later, on a transcript that was never allowed to rot as fast.

## See also

- [How to stop AI coding agents from re-reading the same files](stop-rereading-files.md)
- [MCP context compression](mcp-context-compression.md)
- [Quality benchmark: compression ratio vs information preservation](quality-benchmark.md)
