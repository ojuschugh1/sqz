# How to Stop AI Coding Agents From Re-reading the Same Files

Watch a coding agent work on a real task and count the file reads. It reads `auth.py` to understand it, edits three lines, reads it again to check the edit, runs the tests, reads it a third time when a test fails. Every one of those reads costs the full file in input tokens, and then costs it again on every following turn, because the whole transcript is re-sent to the model each time.

Every one of those reads adds to the transcript that is re-sent on every following turn. Multiply by the twenty files an agent touches in an afternoon and file reads are one of the largest line items on the bill. The question is what, exactly, repeats, because that decides what a tool can do about it.

## What actually repeats

The intuitive picture is "the same file read five times". Measured, that is not what happens.

A user counted 92 Claude Code sessions from their own logs (14 days, 11,149 tool results over 200 characters, counters reset at each compaction, edit and write confirmations excluded). Byte-identical repeats were 81 of 11,149 results, about 0.3% of result characters. Among file reads specifically, 1 in 542 was an identical repeat. What did show up: 49 of 578 file reads (8.5%) were a line range of a file already read in full earlier in the same session.

sqz's own database agrees. In the stats behind the numbers on the README, dedup hits were short repeated shell output (`git status` with nothing changed, a build re-run, a listing) averaging 57 to 180 tokens each. Across 29 MCP file reads there were zero exact-repeat hits.

So identical re-reads of whole files are rare, and a tool that only catches those is solving a small problem. Three things do repeat:

1. **Unchanged command output.** The agent re-runs `git status`, `ls`, a test suite, and gets byte-identical output. Common, small per hit, adds up.
2. **Line ranges of a file it already has.** It read the file in full, then reads lines 40-80 to look at one function. 8.5% of reads in the measurement above. Exact-match dedup misses this, and so does similarity matching: a 40-line slice of a 500-line file has a Jaccard similarity around 0.1 against the whole file, so it never even becomes a candidate.
3. **The file after a small edit.** Most of the bytes are the same, a few lines differ.

## Why agents re-read

Not because they are badly written. An agent has no memory outside its context window, so after an edit it cannot know the file looks the way it intended without reading it back. It reads a range because it wants to look at one function without paying for the whole file again, which is the right instinct; it just cannot know the range is already in its context. Re-reading is the correct behaviour given what the agent can see. The waste is in how the read is served, not in the decision to read.

So the fix has to happen below the agent: in the layer that serves tool output.

## What sqz does about it

sqz keeps a SHA-256 index of every piece of tool output it has served, with the original bytes stored locally. Each of the three repeat patterns gets its own answer:

**Identical output: a reference.** When the same bytes come through again, the agent gets a 13-token `§ref:e03d5588…§` instead of the content. This is where the repeated `git status` and the re-run test suite go.

**A line range of something the model has: a ranged reference.** When the new output is, byte for byte and on line boundaries, a slice of a recent full read, the agent gets `§ref:e03d5588…:L40-80§`: lines 40-80 of content it already has. This works for `sed -n '40,80p' auth.py` and `head -50 auth.py` through the shell hook, and for `sqz_read_file` with `offset` and `limit` through MCP. Two guards: the slice has to be at least 160 bytes and two lines (below that the token costs about as much as the text), and the same bytes must appear in what was actually served for the full read, so a ranged reference never points at lines a lossy stage dropped.

```
$ sed -n '41,80p' auth.py            # after `cat auth.py` earlier in the session
[sqz] slice ref: lines 41-80 of 200 already in context
§ref:e03d55889cad8a3a:L41-80§
```

**The file after a small edit: a delta.** Through `sqz_read_file`, sqz detects the near-duplicate (SimHash, then a line-level diff) and returns only what changed. Measured on a 7,140-byte Python file with two lines appended: the re-read came back as 203 characters.

```
[sqz_read_file path=src/auth.py size=7182]
§delta:e03d55889cad8a3a§
 @@ skip 238 unchanged lines @@
+def handler_new(request):
+    return None
```

None of these is a summary. Each points at bytes the model already has in its context, and the guidance `sqz init` writes for the agent (CLAUDE.md, AGENTS.md, Kiro steering) tells it exactly what the tokens mean and how to expand them if it needs the bytes again.

Two more details make this safe rather than clever:

**Freshness.** A reference is only served if the content was last seen within the last 30 minutes and after the most recent context compaction. Claude Code's PreCompact hook calls `sqz hook precompact`, which marks every cached reference stale; the next read sends the full file again because the model's context no longer has it. If sqz cannot tell, it sends the file.

**Recovery.** `sqz expand e03d5588` on the CLI, or the `expand` MCP tool, returns the original byte-exact. `sqz expand 'e03d5588:L41-80'` returns just those lines. Nothing is ever unrecoverable.

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

The `dedup` row is identical repeats and the `slice` row is ranged re-reads, so you can see for yourself which kind of repetition your sessions actually contain. Underneath, `sqz stats` prints two regret signals: quick re-runs of byte-identical output (the agent asked again within two minutes, which usually means the first serving was missing something) and ref expands (a recovery round trip). Both should be small. If one command dominates the re-run list, its formatter is dropping something an agent needs; open an issue with the command and sqz will get a fixture for it in the [quality benchmark](quality-benchmark.md).

```sh
sqz gain
```

shows the saved tokens per day as a bar chart, so you can see the effect the day you install it.

## What this does not fix

- Reads through a client's built-in Read tool, for the reason above. The instruction file steers the agent to `sqz_read_file`, including for ranged reads; it cannot force it. The 8.5% figure above was measured on built-in Read calls, which is exactly the path sqz never sees unless the agent is redirected.
- The first read. sqz compresses command output where it safely can (see [how compression works](../README.md#how-compression-works)), but a source file read is served in full by design, because an agent that reads a file is usually about to edit it. The saving on files is the repeat, not the first read.
- The transcript re-send. Every provider re-sends the full conversation each turn; sqz makes what enters the transcript smaller, it cannot change how the provider bills it. Prompt caching helps with the re-send and stacks with sqz: `sqz stats --cost` models the saving under a cached billing model, which is the conservative case.

## Setup

```sh
cargo install sqz-cli sqz-mcp     # or brew / npm / pipx, see the README
sqz init
```

`sqz init` installs the hook, registers the MCP server and writes the agent guidance for every client it finds. Everything is local: the dedup index lives in `~/.sqz/sessions.db` (or wherever `SQZ_DB_PATH` points), no telemetry, no network.
