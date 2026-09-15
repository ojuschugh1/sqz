# How to Stop AI Coding Agents From Re-reading the Same Files

Watch a coding agent work on a real task and count the file reads. It reads `auth.py` to understand it, edits three lines, reads it again to check the edit, runs the tests, reads it a third time when a test fails. Every one of those reads costs the full file in input tokens, and then costs it again on every following turn, because the whole transcript is re-sent to the model each time.

For a 2,000-token file read three times, that is 6,000 tokens of tool output, not counting the re-sends. Multiply by the twenty files an agent touches in an afternoon and this is usually the largest single line item on the bill. It is also the easiest to remove, because the second and third reads carry almost no new information.

## Why agents re-read

Not because they are badly written. An agent has no memory outside its context window, so after an edit it cannot know the file looks the way it intended without reading it back. After a context compaction, it may not know it ever read the file at all. Re-reading is the correct behaviour given what the agent can see. The waste is in how the read is served, not in the decision to read.

So the fix has to happen below the agent: in the layer that serves tool output.

## What sqz does about it

sqz keeps a SHA-256 index of every piece of tool output it has served in the session. When the same bytes come through again, the agent gets a 13-token reference instead of the content:

```
Without sqz:                    With sqz:
File read #1:  2,000 tokens     File read #1:  ~2,000 tokens (served once)
File read #2:  2,000 tokens     File read #2:  ~13 tokens  (§ref:e03d5588…§)
File read #3:  2,000 tokens     File read #3:  ~13 tokens
Total:         6,000 tokens     Total:         ~2,026 tokens
```

The reference is not a summary. It is a pointer to bytes the model already has in its context from the first read, and the guidance `sqz init` writes for the agent (CLAUDE.md, AGENTS.md, Kiro steering) tells it exactly that: a `§ref:HASH§` means content it has already seen in this session, and how to expand it if it needs the bytes again.

Three details make this safe rather than clever:

**Freshness.** A reference is only served if the content was last seen within the last 30 minutes and after the most recent context compaction. Claude Code's PreCompact hook calls `sqz hook precompact`, which marks every cached reference stale; the next read sends the full file again because the model's context no longer has it. If sqz cannot tell, it sends the file.

**Deltas for edited files.** The common case is not an identical re-read but a re-read after a small edit. Through the `sqz_read_file` MCP tool, sqz detects the near-duplicate (SimHash, then a line-level diff) and returns only what changed. Measured on a 7,140-byte Python file with two lines appended: the re-read came back as 203 characters.

```
[sqz_read_file path=src/auth.py size=7182]
§delta:e03d55889cad8a3a§
 @@ skip 238 unchanged lines @@
+def handler_new(request):
+    return None
```

**Recovery.** Every reference and delta points at bytes sqz has stored locally. `sqz expand e03d5588` on the CLI, or the `expand` MCP tool, returns the original byte-exact. Nothing is ever unrecoverable.

## The catch: which reads sqz can see

This matters and most write-ups skip it.

sqz intercepts file reads in two places:

1. **Shell commands.** The PreToolUse hook that `sqz init` installs for Claude Code, Cursor, Windsurf, Cline, Gemini CLI and Copilot CLI rewrites every Bash call to pipe through `sqz compress`. So `cat src/auth.py`, `head -50 config.yaml`, `sed -n '10,40p' main.rs` are all deduplicated.
2. **The `sqz_read_file` MCP tool.** Registered by `sqz init` alongside the hook.

What sqz cannot intercept is the client's **built-in Read tool**. Claude Code's `Read`, Cursor's file viewer and the equivalents in other clients do not go through Bash, and PostToolUse hooks can observe but not modify their output ([anthropics/claude-code#4544](https://github.com/anthropics/claude-code/issues/4544)). No shell-level tool can compress those reads.

This is why `sqz init` also writes guidance into the agent's instructions (CLAUDE.md, AGENTS.md, Kiro steering) telling it to prefer `sqz_read_file` for anything over about 2 KB or anything it may read twice, and to keep using the built-in Read for tiny config files and byte-exact needs like lockfiles. The agent's file reads move to the tool that can dedupe them. In practice agents follow this well; when they do not, `sqz stats` shows it as a smaller-than-expected dedup share and you can strengthen the instruction.

## Checking it works

```sh
sqz stats --breakdown
```

The `dedup` row is the repeat reads. Underneath, `sqz stats` prints two regret signals: quick re-runs of byte-identical output (the agent asked again within two minutes, which usually means the first serving was missing something) and ref expands (a recovery round trip). Both should be small. If one command dominates the re-run list, its formatter is dropping something an agent needs; open an issue with the command and sqz will get a fixture for it in the [quality benchmark](quality-benchmark.md).

```sh
sqz gain
```

shows the saved tokens per day as a bar chart, so you can see the effect the day you install it.

## What this does not fix

- Reads through a client's built-in Read tool, for the reason above. The instruction file steers the agent to `sqz_read_file`; it cannot force it.
- The first read. sqz compresses command output where it safely can (see [how compression works](../README.md#how-compression-works)), but a source file read is served in full by design, because an agent that reads a file is usually about to edit it. The saving on files is the repeat, not the first read.
- The transcript re-send. Every provider re-sends the full conversation each turn; sqz makes what enters the transcript smaller, it cannot change how the provider bills it. Prompt caching helps with the re-send and stacks with sqz: `sqz stats --cost` models the saving under a cached billing model, which is the conservative case.

## Setup

```sh
cargo install sqz-cli sqz-mcp     # or brew / npm / pipx, see the README
sqz init
```

`sqz init` installs the hook, registers the MCP server and writes the agent guidance for every client it finds. Everything is local: the dedup index lives in `~/.sqz/sessions.db` (or wherever `SQZ_DB_PATH` points), no telemetry, no network.
