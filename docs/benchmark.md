# sqz Token Savings Benchmark

Session-level token counts for five short scenarios, run through the same code path the shell hook uses. Measured on sqz main after 1.9.0, October 2026; the commands to repeat the run are at the end.

## Methodology

- Each call is `sqz compress --cmd "<command>"` with the command's output on stdin, which is what the shell hook runs.
- Release binary, a fresh throwaway database per scenario (`SQZ_DB_PATH`), default settings (dedup and abbreviation on).
- Inputs: `demo/auth.py` (200 lines, 6,100 bytes), `demo/cargo-test.txt` (31 lines, 1,210 bytes, one failing test), and the `git status` and `JSON API response` fixtures from `sqz/tests/quality_bench.rs`.
- Token counts are `cl100k_base` BPE counts of the raw output and of everything the agent sees: stdout plus sqz's own stderr line (`[sqz] ...`).

## Results

| Scenario | Call | Raw | Agent sees | What the agent gets |
|---|---|---:|---:|---|
| Read a file, then a line range of it | `cat auth.py` | 1,300 | 1,300 | the file, unchanged |
| | `sed -n '41,80p' auth.py` | 260 | 35 | `§ref:…:L41-80§` and a `slice ref` line |
| | total | 1,560 | 1,335 | 14.4% saved |
| `cargo test` twice | 1st | 366 | 185 | the failing test, its assertion and `file:line`, the result line |
| | 2nd, unchanged | 366 | 39 | `§ref:…§` and a `dedup hit` line |
| | total | 732 | 224 | 69.4% saved |
| `git status` twice | 1st | 166 | 97 | branch, ahead count, staged, modified and untracked files |
| | 2nd, unchanged | 166 | 37 | `§ref:…§` and a `dedup hit` line |
| | total | 332 | 134 | 59.6% saved |
| JSON API response twice | 1st | 165 | 168 | TOON encoding, 3 tokens more than the raw output (see below) |
| | 2nd, unchanged | 165 | 42 | `§ref:…§` and a `dedup hit` line |
| | total | 330 | 210 | 36.4% saved |
| Read a file, edit 3 lines, read it again | 1st | 1,300 | 1,300 | the file, unchanged |
| | 2nd, edited | 1,312 | 1,312 | the whole edited file again |
| | total | 2,612 | 2,612 | 0% saved |
| **Session total** | | **5,566** | **4,515** | **18.9% saved** |

## What the numbers show

- **Repeats carry the savings.** An unchanged repeat costs 37-42 tokens. The reference itself is 12-15 tokens depending on the hash digits; the rest is sqz's stderr line saying it was a dedup hit.
- **Source is not compressed on first read.** The 200-line file is served byte for byte. An agent that reads a file is usually about to edit it, so sqz drops nothing from source code; a line range of it read later becomes a slice reference.
- **Edited files on the shell path.** The shell hook path knows exact repeats and line ranges, not near-duplicates, so a file re-read after an edit is sent in full. The sqz MCP tools (`sqz_read_file` and the others) and `sqz-mcp proxy` go through the cache manager, which can answer a near-duplicate with a line-level delta; that path was not measured here.
- **Small outputs can grow.** On the JSON response, null stripping, flattening and TOON encoding cut stdout from 165 to 138 tokens, enough to pass the 16-token net-win gate, but the stderr header that repeats the full `curl` URL costs about 30 tokens, so the first call shows the agent 3 tokens more than the raw output.

**Note:** These dedup savings apply when files are read through Bash commands (`cat`, `head`, etc.) that the hook routes through sqz. Claude Code's built-in Read tool does not go through Bash, and sqz does not rewrite its results. For Bash, sqz's PostToolUse hook replaces the output of commands the PreToolUse hook left alone. The MCP server (`sqz-mcp`) provides compressed file reading as an alternative tool that Claude can use instead of the built-in Read.

## Reproduce

```sh
git clone https://github.com/ojuschugh1/sqz
cd sqz
cargo build --release -p sqz-cli

# Scenario 1: a file, then a line range of it
export SQZ_DB_PATH="$(mktemp -d)/session.db"
target/release/sqz compress --cmd "cat auth.py" < demo/auth.py
sed -n '41,80p' demo/auth.py | target/release/sqz compress --cmd "sed -n '41,80p' auth.py"

# Scenario 2: the same cargo test output twice, then the original back from the reference
export SQZ_DB_PATH="$(mktemp -d)/session.db"
target/release/sqz compress --cmd "cargo test" < demo/cargo-test.txt
target/release/sqz compress --cmd "cargo test" < demo/cargo-test.txt
target/release/sqz expand 94e6bcf79dfd592d
```

The second `cargo test` call prints `§ref:94e6bcf79dfd592d§`, and `sqz expand` returns the 1,210 bytes of `demo/cargo-test.txt` unchanged. For scenarios 3 and 4, pipe the `git status` and `JSON API response` strings from `sqz/tests/quality_bench.rs` the same way, with `--cmd "git status"` and `--cmd "curl https://api.example.com/v1/orders/ord_8812"`, each with a new `SQZ_DB_PATH`. The token counts above are `cl100k_base` counts of stdout plus stderr; the `[sqz] N/M tokens` line sqz prints on stderr uses the same tokenizer for its own numbers.

To see what sqz saves in your own sessions, run `sqz gain --days 7` or `sqz stats --breakdown`.
