# Changelog

All notable changes to sqz will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **The README and the contributing guide in five more languages.**
  Both are available in Simplified Chinese, Japanese, Korean, Spanish
  and Brazilian Portuguese under `i18n/`. CI checks each translation
  against the English text and reports any that fall behind.

### Changed

- **sqz is now licensed under MIT OR Apache-2.0, at your option,
  instead of the Elastic License 2.0.** `LICENSE-APACHE` and
  `LICENSE-MIT` replace `LICENSE`, and the crates, the npm and PyPI
  packages, the VS Code extension and the Docker image declare the
  new license. Releases up to and including 2.0.0 keep the Elastic
  License 2.0.

### Fixed

- **OpenCode 2 loads the sqz plugin.** OpenCode 2 starts a plugin
  through the `setup` function on its default export, and the plugin
  `sqz init` wrote had only OpenCode 1's `server`, so every OpenCode 2
  release (2.0.0 through 2.0.26; reported on 2.0.16 to 2.0.21) logged
  "failed to load plugin" and ran every command uncompressed. The
  plugin now carries both: OpenCode 1 calls `server` as before,
  OpenCode 2 calls `setup`, which registers an `execute.before` hook,
  and both go through the same rewrite, so OpenCode 1 runs the same
  commands as with 2.0.0. A command OpenCode 2 starts in the background
  is left as it is, since `sqz compress` holds output until the
  command exits. sqz 1.0.5 through 2.0.0 wrote the OpenCode 1 object
  `{ id, server }`, and 0.6.0 through 1.0.4 only a `SqzPlugin` export,
  which OpenCode 2 refuses too. `sqz doctor` reports a plugin an older
  sqz wrote, and `sqz doctor --fix` and `sqz init` rewrite it in place;
  `--fix` keeps the sqz binary it runs.

## [2.0.0] - 2026-10-10

### Security

- **sqz no longer approves commands on your behalf.** The Claude Code
  hook returned `permissionDecision: "allow"` with every rewrite, and in
  Claude Code an allow skips the permission prompt, so a command like
  `git push --force` or `rm -rf build` ran without the prompt your
  settings would have shown (deny and ask rules still applied). Copilot
  CLI 1.0.66 and later treat its `allow` the same way. The Claude Code
  and Copilot CLI hooks now return only the rewritten input and leave
  the decision to the client. Checked against Claude Code 2.1.286: a
  command that needed approval before still needs it after the rewrite,
  and one your rules allow still runs without a prompt.
- **`sqz hook cursor` and `sqz hook opencode` no longer approve
  commands.** From 0.7.0 through 1.9.0 the Cursor hook answered every
  rewrite with `"permission": "allow"`, which lets Cursor run the
  command without its own approval, and from 0.6.0 through 1.9.0 the
  OpenCode hook answered with `"decision": "approve"`. `sqz init` 0.4.0
  through 0.7.0 wrote `.cursor/hooks.json` running
  `<sqz path> hook cursor` (0.4.0 to 0.6.0 in Claude Code's hook
  format), and later versions left the file in place, so in a project
  set up with 0.7.0 every command the hook rewrote was approved. Cursor
  reads any answer to a permission hook as a decision and takes a new
  command only together with one, so `sqz hook cursor` now prints
  nothing, writes one line to stderr and exits 1, which Cursor treats as
  a failed hook: its normal approval applies and the command runs as
  written, uncompressed. `sqz doctor` reports such a hook in
  `.cursor/hooks.json` or `~/.cursor/hooks.json`, and `sqz doctor --fix`
  and `sqz uninstall` remove it, keeping any other hooks and deleting
  the file when nothing else is left. The OpenCode hook now returns only
  the rewritten command; `sqz init` has never installed it, since
  OpenCode gets the plugin.

### Added

- **Claude Code commands keep your permission rules when rewritten.**
  Claude Code checks its rules against the rewritten command and treats
  a grouped `{ ...; } | sqz` command as too complex to match any of
  them, so the hook now gives each command its own pipe instead:
  `set -o pipefail; cd sub && cargo test 2>&1 | sqz compress --cmd 'cargo test'`.
  `pipefail` keeps each command's exit status and the `&&`/`||` logic,
  and `cd`, `export` and `source` stay outside the pipes, so lists that
  change shell state are compressed too. `sqz init` adds two allow rules
  for the parts the rewrite adds, `Bash(set -o pipefail)` and
  `Bash(sqz compress --cmd *)`; neither runs anything. Until they are
  there the hook leaves commands alone, and `sqz doctor` flags an older
  install (`--fix` adds them and keeps the hook's sqz binary while it
  resolves, a bare `sqz` on `PATH` included). Pipelines, loops and
  heredocs are left as written, except in bypass mode where nothing is
  checked. A list is also left as written, in every mode, when one of
  its piped commands could stop the shell on a failed expansion
  (`${DEST:?}`, `$((1/0))`, and unless `$SHELL` is bash an unquoted glob
  that matches nothing, `=cmd` or `~user`) and another command follows
  after `;`, a newline or `||`: piped on its own the failing command
  would stop only its pipe, and the rest would run. `&&` is fine, since
  `pipefail` stops the list there. On 12,676 real agent commands, 56.3%
  are rewritten in default mode with zsh as `$SHELL` (56.4% with bash;
  1.9.0 rewrote 26.7%, and approved all of them) and 95.4% in bypass
  mode; 4,261 of the rest contain a pipe, where the agent is usually
  filtering the output already.
- **Claude Code PostToolUse hook.** When a command the PreToolUse hook
  left alone exits 0, `sqz hook claude-post` compresses its stdout and
  stderr in place through `updatedToolOutput`, which keeps the Bash
  result's shape. Output Claude Code has already cut to a preview
  (`BASH_MAX_OUTPUT_LENGTH`, 30,000 characters by default), images and
  interrupted commands are left as they are. It always answers with the
  compressed text, never a `§ref` to earlier output, so a second copy of
  the hook can't point the model at output it never saw; the original
  is still stored for `sqz expand`. A repeated output shows in
  `sqz stats` and `sqz gain` as an ordinary compression, not a dedup
  hit or a quick re-run.
- **`sqz-mcp --version`.** It printed nothing and started the stdio
  server, which then waited on stdin; it now prints `sqz-mcp <version>`
  and exits, like `--help`. Flags it doesn't know still start the
  server, as before. 1.9.0 and earlier started the server.

### Changed

- **sqz adds fewer tokens to every model request.** The sqz-mcp tool
  and parameter descriptions went from 4,907 to 1,871 characters, so
  `tools/list` dropped from 8,705 to 5,779 bytes (about 2,177 to 1,445
  tokens by doctor's estimate), and `§ref:HASH§` is explained once, on
  `expand`. The guidance `sqz init` writes for Kiro, Codex, Zed,
  Cursor, Windsurf and Cline is one shared text of under 1 KB; the
  files it goes into went from 13,337 to 5,282 bytes together. It keeps
  the pipe with the absolute binary path, what not to pipe, which MCP
  tools to prefer, how to read and expand `§ref` tokens and `«A1»`
  symbols, and the opt-out variables, and drops the rest, including
  every percentage. Codex and Zed share one `AGENTS.md` block
  (`sqz-guidance`) instead of writing one each.
- **Older sqz guidance is upgraded in place.** `sqz doctor` flags
  `AGENTS.md` and `.rules` blocks, Kiro steering, and Cursor, Windsurf
  and Cline rules written by an older sqz; `--fix` rewrites them
  keeping the binary path they point at, and `sqz init` rewrites them
  with its own. The old `sqz-agents-guidance` and `sqz-zed-guidance`
  blocks become one block where the first of them was. Text outside the
  sqz markers stays as it is, but anything you added inside an old
  block, or inside the text an older sqz wrote at the top of
  `.clinerules`, is replaced, so move it below first. Cline's rules are
  now the same marked block, so your own rules in `.clinerules` survive
  later upgrades and `sqz uninstall`, and the old text in
  `.windsurfrules` is removed without the rules that follow it. Files
  sqz did not write are never touched, and a current copy saved with
  CRLF line endings counts as current. Doctor's overhead report counts
  the shared block once, on a line for each file Codex and Zed read.
- **`sqz doctor` shows the integration tier of every client.**
  Transparent clients (Claude Code, Copilot CLI, Gemini CLI, OpenCode)
  have a hook that routes shell commands through sqz on its own.
  Advisory clients (Kiro, Cursor, Windsurf, Cline, Codex, Zed) can't let
  sqz rewrite tool calls, so sqz saves tokens there only when the agent
  chooses to pipe output through `sqz compress` or call the sqz MCP
  tools; their built-in shell and file tools bypass it. Before this,
  doctor showed both as "configured", which is how "sqz saves nothing in
  my Kiro sessions" looked like a bug instead of a limit.
- **Doctor reports what sqz adds to every model request**: the size of
  the installed sqz-mcp tool definitions and every copy of sqz guidance
  Kiro loads (user and project steering, AGENTS.md blocks).
- **`sqz init` no longer writes project-level Kiro steering when the
  user-level copy exists.** Kiro applies `~/.kiro/steering` to every
  workspace, so the project copy was the same guidance sent twice.
- **The MCP read tools cap what they return.** `sqz_read_file` returned
  up to 4 MB (about a million tokens) by default; it now stops at
  256 KB, cuts on a line boundary, and puts `continue_with_offset` in
  the header. The cap applies after `offset`/`limit`, so a ranged read
  deep into a big file still works, and files over 64 MB are refused
  instead of loaded. `sqz_grep` clips a match line over 400 chars to a
  window around the hit (one minified bundle used to return its whole
  line) and stops at 100 KB of output. `sqz_list_dir` stops at 1000
  entries. Every cap is a tool parameter (`max_bytes`,
  `max_line_chars`, `max_entries`; 0 lifts it) and the header says when
  one cut the output.
- **`sqz discover --replay` estimates what sqz can actually reach.**
  1.9.0 ran every tool output through the full lossy pipeline, so web
  fetches (which sqz can't intercept) and built-in file reads (which the
  sqz read tools return losslessly) made up almost all of its estimate,
  including the README example. Replay now handles each output the way
  sqz could have reached it on that client: shell output through the hook
  path (on Claude Code, split into commands the hooks compress and ones
  they skip), file reads dedup-only, other MCP servers exactly what
  `sqz-mcp proxy` does, and web fetches, edits and sub-agent results
  counted as out of reach. The report gives two totals, saved with no
  help from the agent and saved if the agent uses every route, and says
  how much of the second is the proxy's lossy first pass. Kiro IDE
  sessions (`~/.kiro/sessions/<workspace>/<session>/`, sub-agent runs
  included) are new, and Kiro CLI shell and MCP results, which the 1.9.0
  parser skipped because they are JSON items, are read.

- **The shell hook rewrites pipes, lists, loops and heredocs.** It
  skipped any command containing `|`, `&&`, `;`, `>` or `<<`, which left
  73% of one machine's 12,676 real agent commands uncompressed. A
  quote-aware scanner now pipes the whole command through sqz as one
  group (Gemini CLI, Copilot CLI, and Claude Code in bypass mode; other
  Claude Code sessions get the per-command form above), keeps a leading
  `cd dir &&` outside it so the directory change sticks, and leaves
  alone anything that changes shell state (`export`, `source`, `set`, a
  later `cd`, a variable set in a loop or `if` body), backgrounds a
  job, follows output (`tail -f`, `docker logs -f`, `kubectl get -w`),
  runs a server (`npm run dev`, `rails s`, `docker compose up` without
  `-d`), already runs sqz (a Windows path to `sqz.exe` included) or
  nests more than 64 levels deep. It also leaves a command alone when
  the group could run different commands than the original: when a
  leading `cd dir &&` is followed by `;`, `||` or another line, all of
  which a failed `cd` would skip along with the group, and, with bash
  as `$SHELL`, when more lines follow one with an arithmetic expansion
  or a `${...}` such as `${VAR:?}` or `${VAR^^}`, since bash goes on to
  the next line after some failed expansions. On those commands the
  share rewritten went from 26.7% to 94.9%, and every rewrite parses in
  bash and zsh wherever the original did. Lists and pipelines skip the
  per-command formatters, since their output is partial or comes from
  several commands. `python` and `node` running a script, `-c` code or
  `-m` module are rewritten too; bare REPLs, `-i` and dev servers still
  aren't.
- **`sqz compress` is faster on large and error-heavy output.** The
  checks that make sure no error line is lost searched the whole output
  once per error line, so a 1 MB log where every line is an error took
  1.5 s; they now find every line in one pass, and that log takes
  0.38 s. The tokenizer also ran over the same text up to five times per
  call and now counts each text once. At 1 MB an app log went from 604
  to 404 ms and a `kubectl get` table piped in from 580 to 401 ms.
  Output that now keeps all its data still costs more than in 1.9.0:
  1 MB of JSON 378 to 490 ms, `cargo test` with failures 404 to 531 ms,
  and `kubectl get` through the shell hook 65 to 397 ms. Calls under
  10 KB are unchanged at about 40-45 ms.
- **Safe mode is decided by structure, not by words.** A whole output
  went to safe mode if it mentioned a panic, a password, an api key or
  legal text anywhere, so a `git log` whose commit messages mention a
  panic, a README with a `password: &str` example or an HTML page with a
  privacy policy footer was cut and never stored, and nothing dropped
  from it could be expanded. Safe mode now needs a trace header followed
  by frames (Rust, Python, JVM, JavaScript, Go), DDL at the start of a
  line, a PEM block, or a credential line with a real value; everything
  else gets the normal pipeline, stored and expandable. Python
  tracebacks now pass through whole like the others. Output that looks
  like it holds a secret is still never stored, on every path including
  the MCP `compress` tool.
- **The docs say what sqz measures and does.** The integration tables
  list Cursor, Windsurf, Cline and Kiro as advisory instead of hooked,
  the benchmark tables, quality benchmark and white paper come from
  fresh runs, and the latency claim (p99 under 5 ms up to 100 KB, never
  measured) is replaced by measured numbers: about 40 ms per
  `sqz compress` call up to 10 KB, most of it process start and building
  the tokenizer, 76-86 ms at 100 KB and 330-525 ms at 1 MB, on an Apple
  M4 Pro.
- **The docs site, extension guides and npm README are corrected.**
  Links on the docs site that led nowhere now resolve, the Chrome and
  Firefox guides describe the `wasm-pack --target no-modules` build that
  `background.js` loads (Firefox 140 or later), and the npm README lists
  the same clients and commands as the main one.
- **Large outputs can need more memory than in 1.9.0.** Output that now
  keeps most of its data also feeds more text to the n-gram abbreviation
  pass. Piped into `sqz compress` without `--cmd`, a 10 MB `cargo test`
  log peaks at about 1.05 GB and an error-heavy log at 1.1 GB, against
  353 and 731 MB in 1.9.0. With the command as the label, as the hooks
  pass it for a single command, the `cargo test` log peaks at 128 MB
  against 122 MB in 1.9.0, and the error log, labelled
  `kubectl logs api`, at 1.1 GB against 132 MB, since the 1.9.0
  `kubectl logs` formatter cut it to 4 KB. `SQZ_NO_ABBREV=1` brings the
  unlabelled `cargo test` run down to 250 MB and the labelled error log
  to 274 MB.
- **`sqz init` leaves Claude Code hooks to the user-level settings.**
  When `~/.claude/settings.json` already has the sqz hooks, `sqz init`
  at project scope no longer adds them to `.claude/settings.local.json`
  and says why. `sqz doctor` flags hooks installed at both scopes, and
  `--fix` removes the project copy, keeping its permissions; the file
  is deleted only if nothing else is left.
- **`sqz doctor` checks which sqz binary each hook runs.** It reports
  hook and plugin files whose sqz binary no longer exists (Claude Code,
  Gemini CLI, Copilot CLI, OpenCode), and `--fix` points them at the
  running binary; it never writes a hook whose binary does not exist. It
  also notes hooks that run a different sqz binary than the one running
  doctor, so after an upgrade reinstall sqz first, then run
  `sqz doctor --fix`.
- **`sqz uninstall` and `sqz reset` need an answer.** Without `--yes`,
  an empty answer counted as yes, so with stdin closed or piped from
  nothing they went ahead. When stdin is not a terminal and gives no
  answer they now stop with a message; `y` or `yes` on a pipe still
  works, and a terminal keeps the `[Y/n]` default. 1.9.0 and earlier
  went ahead.

### Fixed

- **Gemini CLI commands kept only their `command` argument.** Gemini's
  scheduler replaces a call's arguments with the hook's `tool_input`
  rather than merging it, so a rewritten command lost `dir_path` and ran
  in the project root. The hook now sends every original argument.
- **A command's own stderr redirect lost to the hook's.** The one-line
  rewrite put `2>&1` after the command, so `cargo build 2> errors.log`
  sent its errors into the pipe and left `errors.log` empty, and
  `2>/dev/null` stopped hiding anything. The redirect now sits on the
  group, where the command's own wins.
- **`sqz uninstall` deleted a project's whole
  `.claude/settings.local.json`.** Claude Code saves your "don't ask
  again" answers in that file, so they went too. It now removes only
  sqz's hooks and rules, and the file only if nothing else is left.
- **A trailing `# comment` broke the rewritten command.** The comment
  ran on over the closing `}` of the group, so the shell hit end of
  file instead of running the command.
- **Background tool calls are no longer piped through sqz.** Claude
  Code's `run_in_background`, Gemini's `is_background` and async calls
  stream their output while they run; through sqz nothing showed until
  the command exited, so a server started in the background looked
  dead.
- **`sqz compress` passes binary output through.** Input that wasn't
  UTF-8 made it print a read error and exit 1, losing the output. It now
  writes the bytes back unchanged and exits with the command's status.
- **The OpenCode plugin keeps the command's exit status.** Its rewrite,
  `cmd 2>&1 | sqz compress`, exits with sqz's status, so a failing
  command reported success. Outside Windows it now uses the exit-status
  trailer the other hooks use; Windows keeps the old form because
  PowerShell and cmd.exe can't run it.
- **`sqz_list_dir` listed two levels by default.** Its schema says
  `max_depth` 1 means the immediate children; it now does.
- **The `compress` MCP tool cut its input to 500 characters.** It
  compressed the serialized `{"text": …}` request instead of the text,
  so the pipeline saw one long JSON string and returned its first 500
  characters as `TOON:{text:"…"}`. It now compresses the `text` field.
- **Entropy truncation no longer drops error lines.** A log without blank
  lines is scored in 10-line windows, and a window holding an `ERROR`
  line could fall below the median entropy and be dropped with the
  routine ones. A segment that mentions an error, panic, traceback,
  failure or exception is now always kept.
- **`sqz-mcp proxy` says how to recover every lossy result.** On first
  pass the pipeline can cut long JSON strings and large arrays, but only
  entropy-truncated results carried the `sqz_expand` hint, so an agent
  shown `... and 5 more items` had no ref to ask for. Every first-pass
  compression now ends with the hint, and the net-win gate counts it.
- **JSON output keeps every record and field.** Arrays over five
  elements were cut to five unless every element had the same keys, so
  a mixed search result (issues next to pull requests, Slack messages
  with and without threads) showed half its entries, and arrays that
  did share keys became a text table that printed nested objects as
  `{6 keys}` (every Jira summary and status) and cut strings at 50
  characters. JSON projection stripped any key starting with `_`,
  `debug`, `internal`, `trace` or `x_` (an Elasticsearch hit's
  `_source`, HAL's `_embedded`, a `traceback`), summarized anything five
  levels down (Notion page titles), dropped `cursor` and `_links` (the
  next page), and kept one timestamp per day, so `closed_at` went when
  an issue closed the day it opened. Arrays are now cut only past 32,000
  characters of compact JSON (`max_chars` in a preset's
  `collapse_arrays`; 0 restores the count-only cap), projection removes
  only `__v`, `etag` and request or correlation ids, and an array of
  objects is written as a lossless TOON table: one header row of keys,
  one line per object, an empty cell for a missing key. On seven MCP
  payloads (GitHub, Jira, Notion, Elasticsearch, MongoDB, Slack) the
  proxy now keeps every id and title, against about half before, and
  saves 36% instead of 67%; most of the 67% was the dropped data. The
  proxy and the `compress` MCP tool also get the verifier's safe-mode
  fallback that `sqz compress` already had. The browser extension's
  WASM source has the same array fix; its bundled build predates it.
- **TOON keeps non-ASCII text as-is.** `é` was written as `\u00e9` and
  an emoji as a 12-character surrogate pair, which costs more tokens
  than the character itself. Control characters are still escaped.
- **Failed commands no longer come back as "ok".** Several per-command
  formatters answered "ok" whenever they didn't find their success line,
  so a failed `npm install` (ERESOLVE) or `pip install` showed the agent
  "ok", `git push` rejected or without an upstream showed "ok" or
  "ok ! [rejected]", a `git pull` that stopped on a merge conflict showed
  "ok", `kubectl apply` reduced its output to "applied: 1 configured"
  and dropped the `Error from server` line, and gradle, dotnet and
  xcodebuild fell back to the last line of the output. Every formatter
  result is now checked: if a line that reports a failure (`error:`,
  `fatal:`, `npm error`, `Error from server`, `CONFLICT`, `panicked at`,
  `! [rejected]` and similar, as whole words) is missing, the formatter
  isn't used and the output goes through the generic pipeline, which
  keeps error lines. Lines that only restate the failure (`Build
  FAILED.`, `could not compile`) may go when the output still says it
  failed. Push and pull now drop only transfer progress, so pull request
  links from `remote:` survive.
- **Formatters no longer replace the data an agent asked for with a
  summary.** `kubectl get -o json` became `List: 2500 items (showing
  30)` with no items, `-o yaml` lost its indentation, `kubectl logs`
  dropped the middle of long logs; `gh pr view --json body,comments`
  kept only number, title, state and author, and `gh pr diff`, `gh pr
  view` and `gh run view` were cut at 30 lines; an aws or gcloud object
  holding an array of more than 30 items became a one-line summary with
  none of them; `ls` showed
  five file names, `find` printed `dir/ (40 files)` without the names,
  and `grep` kept three matches per file. `git log -p` and `--stat`
  kept only commit subjects, `git diff --stat` and `--name-only` kept
  their first line, `git show rev:path` stripped the file's indentation,
  `git branch` showed three branches, and `git diff` dropped rename,
  mode and binary lines. `terraform plan` dropped every attribute
  change (`# forces replacement` included) and `terraform apply` kept
  only `Apply complete!`, losing the outputs. kubectl get and logs, gh,
  aws, gcloud, ls and find now go through the generic pipeline; grep
  output is written in ripgrep's heading layout, each path once with
  every match under it; terraform drops only refresh and progress lines;
  git log keeps commit bodies and passes anything else through; curl
  drops only its progress meter, not blank lines or lines starting with
  `%`; tree keeps every entry outside `node_modules`-style directories.
- **Lists under 16,000 characters are no longer entropy-truncated.**
  Entropy truncation drops the segments below the median, about half of
  any long output, so 60 grep matches came back as 30 and a `find` of
  80 files as 40. Output where most lines are records (`path:line:`
  matches, bare paths, `ls -l` entries, `git log --oneline`, aligned
  tables) keeps every line up to that size.
- **Prose pruning no longer removes words that carry meaning.** The
  built-in trigram table pruned the only completion it knew for a pair,
  so "there are no failing tests" became "there are failing tests",
  "does not exist" became "does not", and "at the same time" became "at
  the time". Those entries are gone.
- **Formatter savings are judged in real tokens.** The formatter path
  counted chars/4 while the pipeline counted BPE tokens, and collapsing
  column padding shrinks chars/4 a lot but tokens hardly at all, so
  whitespace-only reformatting passed the net-win gate and was logged as
  savings. Both paths now use the same token counter.
- **`sqz doctor --fix` repairs log rows written by the pre-1.8.0
  token estimator.** sqz-mcp before 1.8.0 counted a read tool's input
  with chars/4 and its output with the BPE tokenizer, so verbatim
  `sqz_read_file` / `sqz_grep` / `sqz_list_dir` results logged as larger
  than their input; on one machine this pushed the all-time total
  negative, which `sqz stats` rendered as "0 tokens saved". `--fix` now
  raises the input count to the real output count, recording those calls
  as saving nothing, instead of deleting them (the version of this fix
  that never shipped deleted the rows, which also dropped their real
  volume from the totals). The database is backed up first with
  `VACUUM INTO`, which includes writes still in the WAL.
- **Doctor and `--fix` handle stale and duplicate Kiro steering.**
  Steering written by an older sqz is refreshed in place, keeping the
  binary path it points at; a project copy next to the user-level one is
  removed. Files without the sqz marker are never touched.
- **The shell block no longer overrides your own `preexec` or DEBUG
  trap.** The zsh block defined a `preexec()` function and the bash block
  installed `trap … DEBUG`; both only exported a variable nothing read,
  and both replaced any hook of the same kind defined earlier in the rc
  file. `sqz init` and `sqz doctor --fix` rewrite the sqz block in place,
  touching only the lines between its markers.
- **Doctor no longer reports a missing shell block as lost savings.** The
  block only defines the optional `sqz_run` helper and compresses
  nothing on its own.
- **`test_init_exits_zero` ran `sqz init` against the real HOME and this
  repository**, which rewrote the developer's shell rc during `cargo
  test`. It now runs in a throwaway HOME and project.
- **The same input gives the same output.** The `§dict§` header listed
  equal-length patterns in hash-map order, and abbreviations with tied
  savings were picked and numbered (`«A1»`, `«A2»`) in hash-map order,
  so the same output could come back different from one run to the next.
  Ties are now broken on the text itself.
- **The opt-out examples in the guidance never reached sqz.** They put
  the variable before the first command,
  `SQZ_NO_DEDUP=1 git log | sqz compress`, which sets it for `git log`
  only. The guidance and the `CLAUDE.md` block now put it right before
  `sqz compress`.
- **Stack traces, migrations and keys come through whole.** sqz routes
  them to safe mode and said they pass through unchanged, but past about
  500 characters the pipeline still cut them, and since safe-mode output
  is never stored the cut lines could not be expanded: in 1.9.0 a
  40-frame Rust panic lost 40 lines, a SQL migration 26, and a key file
  its `-----END RSA PRIVATE KEY-----` line. The per-command formatters
  ran before the check, so `./gradlew test` turned a 22-line JUnit trace
  into 81 bytes and a build-script panic under `cargo build` lost 42 of
  its 55 lines. Safe-mode output now comes back unchanged apart from
  ANSI codes, and a formatter's summary is used only when every line of
  each paragraph holding a trace, migration, key or credential line is
  still in it.
- **Counts are exact after repeated lines are merged.** Ten identical
  lines came back as one line marked `[×3]`, a folded diff reported 11
  unchanged lines for 18, and `cargo build` after downloading crates
  reported `ok (5 crates compiled)` for 30, because the step that merges
  identical lines ran first. The same step shortened pretty-printed JSON
  arrays (`[0,0,0,0,0,0,1]` came back as `[0,0,0,1]` in safe mode).
  Counts now come from the original lines, and that step leaves JSON
  and diffs alone.
- **`sqz discover` no longer prints reduction figures nobody
  measured.** Its suggestions gave a reduction range per command, none
  of them measured, including `kubectl get`, which no longer has a
  formatter. It now names the commands without numbers.
- **The exit status survives an expansion error.** A rewritten
  `{ cmd; printf trailer; } | sqz` printed no trailer when `cmd`
  stopped the shell on an expansion error, such as `${X:?}`, a zsh
  `=cmd` that isn't found or a zsh `for` loop over a glob that matches
  nothing, so sqz exited 0 and the agent saw success. The command now
  runs in a subshell inside the group, so the trailer always reports
  its status. Gemini CLI, Copilot CLI, Windsurf, the OpenCode plugin
  and Claude Code in bypass mode use this form; it shipped for single
  commands in 1.6.1 through 1.9.0.
- **Negative and wide exit codes in the trailer.** PowerShell reports
  a crashed native command as a negative `$LASTEXITCODE` such as
  -1073741819, and the trailer parser took only digits, so the marker
  stayed in the output and sqz exited 0; a code of 256 also came out
  as 0. sqz now exits with the low byte on Unix (1 when that byte is
  0) and the full code on Windows. Shipped in 1.6.1 through 1.9.0.
- **Copilot CLI's `toolArgs` as a JSON string.** GitHub documents
  `toolArgs` as a string of JSON, and the hook read only an object, so
  it rewrote nothing for that form. It now parses the string and still
  answers with a `modifiedArgs` object. Shipped in 1.5.0 through 1.9.0.
- **`sqz hook opencode` keeps the command's exit status.** It piped
  the command straight into sqz, so a failing command reported success.
  Outside Windows it now uses the same trailer as the other hooks, as
  the OpenCode plugin already does. Shipped in 0.6.0 through 1.9.0.
- **Large non-ASCII output no longer crashes `sqz compress`.** The
  verifier cut its scan at byte 1,048,576, and when that byte fell
  inside a multibyte character sqz panicked and the command's whole
  output was lost. It now cuts on a character boundary. 1.9.0 had the
  same bug.
- **`git log` and `git show` no longer crash on a non-ASCII `commit`
  line.** Their formatters cut the hash by bytes. 1.9.0 had the same
  bug.
- **Long runs of one character no longer stall or crash token
  counting.** tiktoken-rs is quadratic on a long run of one character
  class and overflows its stack at about 1 MiB, so 256 KB of `=` took
  about 14 s and 1 MiB crashed sqz on every path. Runs over 2,048
  bytes are now counted in slices, which can be off by about 1% plus 4
  tokens, and counts for other text are unchanged. 1.9.0 had the same
  problem.
- **A panic during compression no longer loses the output.**
  `sqz compress` prints its input unchanged, keeps the command's exit
  status and prints one line on stderr, and `sqz hook claude-post`
  leaves the result as it was. With `RUST_BACKTRACE` set (and not
  `0`), the stderr line names the file, line and column of the panic.
  In 1.9.0 and earlier a panic lost the output.
- **Token pruning no longer rewrites whole lines.** Once it removed one
  filler word anywhere, it rebuilt every prose-looking line with single
  spaces. That collapsed indentation in unlabeled diffs and markdown,
  turned CRLF into LF and dropped words from error lines. It now
  rewrites only lines that lose a word, keeps their spacing, and leaves
  diff lines, indented lines, lines starting with `-` and error lines
  alone. 1.9.0 had the same bug.
- **Colored traces, DDL, PEM blocks and credential lines are protected
  like plain ones.** The router read raw lines, so ANSI color codes hid
  them: a colored 40-frame backtrace kept 50 of 83 lines and was
  cached. Routing, never-store and protected-line checks now strip ANSI
  first. 1.9.0 had the same bug.
- **`sqz stats` counts what was printed.** The stats line and the
  session database logged the compressed size from before the
  abbreviation legend and expand hint were added, so savings were
  overstated by a median of 19 tokens per affected command. 1.9.0 had
  the same bug.
- **`sqz compress --cmd -j` works.** A label starting with `-` was
  parsed as a flag, so sqz exited 2 with no output. 1.9.0 had the same
  bug.
- **A panic in one request no longer stops sqz-mcp.** A panic while
  handling a request ended the server, and in `sqz-mcp proxy` it left
  a lock poisoned, so the next message ended the proxy too; the
  tokenizer overflowing its stack on a long run of one character was
  one way to get there. Each request is now caught: `compress` returns
  its input unchanged, other tools answer with JSON-RPC error -32603,
  the proxy passes the upstream result on as it came, and the next
  request is served as usual. 1.9.0 had the same crash.
- **A line that is not UTF-8 no longer ends an MCP session.** One
  invalid byte made sqz-mcp exit. In `sqz-mcp proxy` a bad line from
  the client ended the proxy without an answer, and one from the
  upstream lost that response and every later one. The server now
  answers such a line with -32700 and a null id and keeps reading; the
  proxy forwards it with the bad bytes replaced. 1.9.0 had the same
  bug.
- **sqz-mcp answers `ping`.** MCP expects an empty result; it returned
  -32601 method not found. 1.9.0 had the same bug.
- **`sqz-mcp proxy` stops an upstream that ignores end of input.**
  After the client closed stdin, the proxy waited for the upstream to
  close its stdout, so an upstream that kept running kept the proxy
  running, and a later SIGTERM to the proxy left it orphaned. The proxy
  now closes the upstream's stdin, waits up to 2.5 seconds for it to
  exit, then kills it and waits up to another 2.5 seconds while it
  finishes passing on a result it had already read. A response the
  upstream had not sent by then is lost; MCP clients keep stdin open
  while they wait, so this mostly affects scripted one-shot use. A
  SIGTERM to the proxy itself still leaves such an upstream running.
  1.9.0 had the same bug.
- **Deltas number lines from 1 and account for every line.** The
  position of removed lines was 0-based, and the unchanged lines next
  to each change were neither shown nor counted. A delta now shows one
  unchanged line on each side of a change, and its skip counts add up
  to the whole file. This applies to every delta sqz-mcp returns:
  `sqz_read_file`, `sqz_list_dir`, `sqz_grep`, `compress` and the
  proxy. The context lines make a delta longer, so a re-read with
  edits spread through the file now comes back in full. 1.9.0 had the
  same bug.
- **No reference where it costs more than the content.** Empty and
  tiny results (13 tokens or fewer, such as an empty file or a search
  with no matches) came back as a 13-token `§ref:HASH§`, and a ranged
  `sqz_read_file` of an unchanged file could come back as a delta
  against the previous range. Both are now sent in full and still
  stored, so `expand` and `sqz_recall` find them. A ranged re-read of a
  file read in full is still a line-range reference. 1.9.0 had the
  same bug.
- **The `compress` tool says how to get dropped segments back.** When
  it dropped low-information segments from text it stores, nothing in
  the output said how to recover them. The output now ends with
  `[full output: call expand with prefix "<16 hex>"]`, and the tool's
  `sqz:transforms` list names the step. 1.9.0 gave no hint either.
- **`sqz-mcp proxy --no-cache` no longer offers `sqz_expand`.**
  Nothing is stored with `--no-cache`, so every call to it failed. The
  tool is left out, and the instructions note no longer mentions
  `§ref:HASH§`. 1.9.0 had the same bug.
- **`sqz uninstall` no longer deletes a `.gemini/settings.json` sqz
  didn't write.** It removes only sqz's hook and deletes the file only
  when nothing else is left. 1.9.0 and earlier deleted the whole file.
- **`sqz init` adds its Gemini CLI hook to an existing
  `.gemini/settings.json`.** It skipped the file when it existed, and
  doctor's suggested fix then had nothing to do. A file with comments,
  or one that doesn't parse, is left alone, and `sqz init` and
  `sqz doctor` give the entry to add by hand. 1.9.0 and earlier
  skipped the file silently.
- **User hooks that look like sqz's survive.** sqz recognised its own
  Claude Code hooks by substring (`resume`, `hook precompact`,
  `hook claude`), so `sqz init`, `sqz doctor --fix` and `sqz uninstall`
  deleted user hooks such as `/usr/local/bin/on-resume.sh` or
  `~/bin/save-notes --hook precompact`, and replaced ones that wrap sqz,
  such as `/usr/bin/time /opt/sqz hook claude`. An entry is now sqz's
  only when its command is `sqz` or `sqz.exe`, bare or as a path, quoted
  or not, followed by exactly the arguments sqz writes. Since an
  unquoted path may contain spaces, a wrapper that runs sqz by a
  relative path, such as `/usr/bin/time ./sqz hook claude`, still counts
  as sqz's. 1.9.0 and earlier deleted and replaced them.
- **`sqz uninstall` removes the `CLAUDE.md` guidance block and the
  `mcpServers.sqz` entry in `~/.claude.json`.** 1.9.0 and earlier left
  both.
- **An outdated `CLAUDE.md` guidance block is replaced.** Once written,
  the block was never refreshed, so it still described the 1.9.0
  rewrite and the `SQZ_NO_DEDUP=1 git log | sqz compress` placement
  that never reached sqz. `sqz doctor` now reports an old block, and
  `sqz init` and `sqz doctor --fix` replace it in place, keeping the
  binary path it names while that resolves and every byte outside the
  markers; a current block saved with CRLF line endings counts as
  current. 1.9.0 and earlier never refreshed it.
- **Removing the shell rc block keeps every other byte.** Uninstall
  collapsed every run of blank lines in the whole file and dropped its
  final newline. It now removes exactly the sqz block and the blank
  line install added before it. 1.9.0 and earlier had the same bug.
- **JSON configs sqz edits keep their final newline and file mode.**
  1.9.0 and earlier dropped the final newline, gave `opencode.json` one
  it didn't have, and left Claude Code settings at 0600. Key order
  still changes.
- **Hook files older sqz versions wrote are cleaned up.** `sqz doctor`
  reports them, and `sqz doctor --fix` and `sqz uninstall` remove sqz's
  entries from `.windsurf/hooks.json`, `.cline/hooks.json` and Cursor's
  `hooks.json` and the old `.clinerules/hooks/PreToolUse` script,
  deleting a file when nothing else is left in it. Uninstall ends by
  listing the files it keeps, such as the preset and shell completions.
  1.9.0 and earlier left them all.
- **The uninstall plan lists only files that hold sqz entries**, so a
  second `sqz uninstall` says there is nothing to uninstall; 1.9.0 and
  earlier listed every existing config file. The plan also names the
  user-level Kiro steering and `~/.kiro/settings/mcp.json` (when it has
  `mcpServers.sqz`), which uninstall changed without listing them, and
  when they are the only sqz files left uninstall removes them instead
  of saying there is nothing to uninstall.
- **`cargo test` no longer writes to your home directory.** Several
  tests wrote the Copilot CLI hook, the Codex and Zed configs and the
  session database under the real home directory; they now use
  temporary directories. In 1.9.0,
  `test_reinit_is_idempotent_at_project_scope` already wrote the
  Copilot CLI hook there whenever `~/.copilot` existed, pointing it at
  `/usr/bin/sqz`.
- **`pipx install sqz` fetches the right release.** The PyPI wrapper
  downloaded the 0.1.0 binaries whatever its own version, asked for a
  `.tar.gz` on Windows, where releases ship a `.zip`, and used a type
  hint that fails on Python 3.8. It now takes its version from the
  installed package and the archive each release target ships. 1.9.0
  and earlier had all three bugs.
- **The npm package declares its licence as `Elastic-2.0` and ships
  `LICENSE`.** `package.json` said `ELv2`, which is not an SPDX id, and
  the publish jobs did not copy `LICENSE` into the package. 1.9.0 and
  earlier had both.
- **`sqz proxy`, its help and its guide say what it does.** `sqz --help`
  called the command "[Coming soon]"; it now says the local endpoint
  compresses an OpenAI, Anthropic or Google AI request body and sends it
  back without forwarding it. When it started, the endpoint told you to
  set `OPENAI_BASE_URL` or `ANTHROPIC_BASE_URL` to its address, which
  hands your client the compressed request instead of a model response;
  it now says what it does and shows a `curl` example, and a body it
  cannot compress is reported as sent back rather than forwarded. The
  API proxy guide described forwarding, a `proxy.toml` sqz never reads
  and two options that do not exist, `--upstream` and `--dry-run`; it
  now describes the endpoint and its one option, `--port`. 1.9.0 and
  earlier had the same help text and, from 0.5.0, printed the base URL
  advice.

## [1.9.0] — 2026-09-23

### Added

- **`sqz doctor` — deployment validator.** Checks the three layers that
  actually go wrong: installation (binary, database), wiring (shell
  hook, per-client sqz configs at project and user level, client
  detected vs actually routed through sqz), and activity (compressions
  logged recently). The diagnosis is the gap between layers — "Cursor
  is installed but not routed through sqz", "hooks installed but
  nothing compressed in 7 days" — each with the command that fixes it.
  Exit codes: 0 healthy, 1 problems found, 2 sqz itself broken.
- **`sqz discover --replay` — counterfactual replay of your own agent
  sessions.** sqz can't observe sessions before its hook is installed,
  but the transcripts already exist on disk. Replay parses Claude Code
  (`~/.claude/projects`) and Kiro (`~/.kiro/sessions/cli`) transcript
  JSONL, extracts every tool output, replays it through the current
  engine (per-command formatters, dedup, slice refs all fire exactly
  as they would have live), and reports the estimate per source.
  Each session file replays against a fresh throwaway store so
  references never pretend to span sessions, and the report calls
  itself an estimate because that's what it is. Runs locally; nothing
  is uploaded. `--transcripts PATH` points it anywhere.
- **`SQZ_QUIET=1`** silences the informational per-compression stderr
  lines (progress, dedup hits, verifier fallback). Errors still print.
  Replay sets it for itself; available to anyone whose hook setup
  surfaces stderr where it isn't wanted.
- **`sqz stats --share`** renders a five-line share card (date range,
  totals, how much of the saving came from references, regret rate)
  sized for a paste into an issue or post.
- **`SQZ_REF_TTL_SECS`** overrides the 30-minute dedup ref freshness
  window. Useful on clients without a compaction hook; `0` disables
  refs entirely.
- **Documentation site** at <https://ojuschugh1.github.io/sqz/> built
  with mdBook from `docs/`, with the rustdoc API reference under
  `/api/`. Replaces the raw rustdoc tree that previously lived in
  `docs/` and the redirect `docs/index.html`.

### Changed

- Whitepaper (and arXiv source) refreshed to 1.8.0: documents
  near-duplicate deltas, ranged (slice) references, and the MCP proxy
  integration surface; the dedup-granularity limitation now reflects
  what shipped in 1.5.0–1.8.0.

### Fixed

- `sqz compress` preserves the trailing newline when the input ended
  with one, so hook-wrapped commands no longer glue the next prompt
  onto the last output line.

## [1.8.0] — 2026-09-22

### Added

- **Line-range references for ranged re-reads.** A user who counted 92
  of their own Claude Code sessions found that byte-identical repeats
  were rare (0.3% of result characters, 1 in 542 file reads) but 8.5%
  of file reads were a line range of a file already read in full. Exact
  hash dedup misses those, and so did near-duplicate matching (a
  40-line slice of a 500-line file has a Jaccard similarity near 0.1).
  sqz now checks containment: when new output is, on line boundaries,
  a slice of a recent full read the model still has in context, it
  returns `§ref:HASH:L40-80§` instead of the lines. Guards: at least
  160 bytes and two lines, the same freshness and compaction rules as
  plain refs, and the bytes must also appear in what was actually
  served for the full read, so a ranged ref never points at lines a
  lossy stage dropped. Works through the shell hook (`sed -n`, `head`,
  `tail` after `cat`) and the MCP read tool. Logged as its own `slice`
  row in `sqz stats --breakdown` so you can see how often it fires.
- **`sqz_read_file` takes `offset` and `limit`** (1-based lines, same
  meaning as the host's Read tool) so agents can route ranged reads
  through sqz. The header reports `lines=40-80 of 500`.
- **`sqz expand` and the `expand` MCP tool accept ranged tokens**
  (`sqz expand 'a1b2c3d4:L40-80'`) and return just those lines.

### Fixed

- MCP `compress`, `sqz_grep` and `sqz_list_dir` counted the input with
  a bytes/4 estimate and the output with the BPE tokenizer, so verbatim
  results logged as negative savings (about -5% on grep). Both sides
  now use the same tokenizer.

### Changed

- README and docs no longer lead with "the same file read five times".
  Measured, identical file re-reads are rare; the savings come from
  command output, unchanged-output refs, ranged refs and deltas, and
  the docs now say so with the numbers.

## [1.7.0] — 2026-09-22

### Added

- **`sqz recall` — full-text search over session memory.** Everything
  that flows through sqz is now indexed in a local SQLite FTS5 table
  (BM25-ranked, snippet highlighting, binary content skipped). When an
  agent's context is compacted away, `sqz recall "connection refused"`
  finds the original output and prints the `sqz expand` ref to recover
  it byte-exact. Exposed to agents as the `sqz_recall` MCP tool. The
  index respects eviction: cache clears and entry deletion drop the
  matching rows.
- **Generic table compactor.** Aligned-column output from commands
  without a dedicated formatter (`ps aux`, `netstat`, database CLIs)
  collapses its alignment padding to two-space separators. The detector
  is deliberately strict — 5+ rows, zero leading whitespace, and
  full-height separator bands required — so code, YAML, diffs, and
  JSON never match.
- **New formatters: xcodebuild, adb logcat, dotnet.** Apple builds
  collapse to diagnostics + verdict (compiler invocations and
  environment noise dropped), logcat keeps errors/fatals plus capped
  warnings with consecutive-duplicate folding, and dotnet build/test
  keep MSBuild diagnostics (deduplicated across target frameworks) and
  failing-test blocks.
- **`sqz discover` ranks your weakest-compressing commands** from real
  usage history (high token volume, low reduction) so you can see
  which formatter would pay off next — and file an issue for it.
- **Global Kiro integration** — `sqz init --global` now installs Kiro at
  the user level: steering at `~/.kiro/steering/sqz.md` and the MCP
  server in `~/.kiro/settings/mcp.json`, both of which Kiro applies to
  every workspace. Previously the Kiro integration was workspace-only,
  so compression silently never ran in projects where `sqz init` hadn't
  been executed. Honors `KIRO_HOME`. `sqz uninstall` removes both when
  sqz-authored; existing servers in the user MCP config are preserved
  on merge.
- **Per-project database** (#41, requested by @bmad4ever) — set
  `SQZ_DB_PATH` to point sqz at any SQLite file (e.g.
  `$PWD/.sqz/sessions.db`) instead of the global `~/.sqz/sessions.db`.
  Honored by every surface: shell hook, CLI commands, vizit, and the MCP
  server. The parent directory is created if missing (0700); pre-existing
  directories are never chmodded.
- **Quality benchmark as a regression test.** `cargo test -p sqz-cli
  --test quality_bench` pushes 13 realistic command outputs through the
  real binary and fails if any agent-critical fact (failing test name,
  assertion, `file:line`, changed diff lines, crashing pod, vulnerability
  count) goes missing, if a stack trace or secret is altered, if a
  secret is persisted, or if compressed content cannot be recovered
  byte-exact with `sqz expand`. Results and methodology in
  [docs/quality-benchmark.md](docs/quality-benchmark.md).
- **Docs:** [MCP context compression](docs/mcp-context-compression.md),
  [stop AI agents re-reading the same files](docs/stop-rereading-files.md),
  [context rot vs context compression](docs/context-rot.md),
  [choosing a compression approach](docs/comparison.md), a
  [Codex CLI guide](docs/integrations/level2/codex.md), a
  [docs index](docs/README.md) and a root `llms.txt`.

### Fixed

Over-compression found by the quality benchmark on its first run:

- **`npm install`** dropped the `N vulnerabilities (...)` line and the
  `npm audit fix` hint: an early return on the "added N packages" line
  skipped everything after it.
- **`docker ps`** mis-parsed rows whenever COMMAND or CREATED contained
  spaces (`"uvicorn app:app"`, `2 hours ago`), putting status and ports
  in the wrong cells. Rows are now sliced at the header's column
  offsets, and PORTS is kept.
- **`git status`** dropped the branch name and the ahead/behind line.
  Both are kept; only "up to date" is elided.
- **`pytest`** kept only the short-summary line, losing the failure's
  `file:line`, the failing source line and the `1 failed, 41 passed`
  totals. New dedicated formatter keeps all three.
- **`tsc`** only recognised the piped `file(line,col)` layout; with
  `--pretty` output it fell through to the generic pipeline, which dropped
  the `Found N errors` summary. Both layouts are parsed now.
- **Source files read through the hook** (`cat src/x.py`) lost imports
  and the `class` line to entropy truncation. The pipeline now detects
  source code and skips every line-dropping stage; savings on file reads
  come from dedup, as intended.
- **Inflated savings on the pipeline path.** `sqz stats` compared a BPE
  token count for the input with a bytes/4 estimate for the output, so
  unchanged passthroughs were recorded as 15-37% reductions and the
  stats header was printed on them. Both sides now use the same
  tokenizer, and the 16-token net-win gate applies to the formatter path
  too, so a formatter that trims a few blank lines no longer costs the
  agent a 12-token header.
- The safe-mode notice on stderr is silent unless `SQZ_VERBOSE` is set:
  in the hook path every stderr line lands in the agent's context, and a
  passthrough has nothing to report.
- The `sqz_read_file` / `sqz_grep` / `sqz_list_dir` tool descriptions
  claimed signature extraction and line folding that the lossless
  file-read path (#32) never does. They now describe what happens.

### Changed

- Package descriptions and keywords on crates.io, npm and PyPI now say
  what sqz is for: pre-injection context compression and session
  deduplication for AI coding agents.

## [1.6.1] — 2026-08-16

### Fixed

- **Hook rewrites no longer mask command exit status** (#39, reported by
  @Piotr1215) — `cmd 2>&1 | sqz compress` exits with sqz's status, so
  every hooked command reported success to the agent even when it failed.
  The rewrite now sends the command's real status through the pipe as a
  `__SQZ_EXIT_<n>__` trailer that sqz strips and re-raises as its own
  exit code. Plain POSIX (works in bash/zsh/dash/ash/ksh, unlike
  `pipefail` or `PIPESTATUS`), with a `$LASTEXITCODE`-based form for
  PowerShell tool calls. The `sqz_run`/`sqz_sudo` shell functions and the
  fish/PowerShell variants get the same treatment. Old-style pipes
  without a trailer behave as before.

## [1.6.0] — 2026-08-16

### Added

- **MCP compression proxy** — `sqz-mcp proxy -- <upstream command>` wraps any
  stdio MCP server and compresses its tool results through the full sqz
  pipeline (dedup refs, safe-mode routing, error results verbatim, 16-token
  net-win gate). Verbose tool descriptions are compacted on `tools/list`
  (`--no-desc` to keep them), and an injected `sqz_expand` tool recovers any
  original byte-exact. Works with every MCP client.
- **CI hook for Copilot coding agent** — `sqz init --ci` writes
  `.github/hooks/sqz.json`, which bootstraps sqz via install.sh inside the
  cloud agent sandbox on session start and compresses bash output there,
  same as local runs. `sqz uninstall` removes it when sqz-authored.
- **Regret signals** — `sqz stats` now reports quick re-runs (byte-identical
  output re-produced within 2 minutes of a compressed serving; window
  configurable via `SQZ_REGRET_WINDOW_SECS`) and ref expands (`§ref§`
  tokens recovered via `sqz expand` or the MCP `sqz_expand` tool), with
  per-command attribution. Both are proxies for "compression dropped
  something the agent needed" — a self-check on compression quality.
- **Spill refs on truncation** — when entropy truncation drops segments
  from a compressed serving, the output now ends with a recovery hint
  (`[full output: sqz expand <prefix>]` in the shell hook,
  `[full output: call sqz_expand with ref "<prefix>"]` in the MCP proxy).
  The untruncated original is already in the cache, so truncation becomes
  a preview instead of a loss. Hints are only added when the original was
  actually stored.
- **Lazy tool descriptions** — `sqz-mcp proxy --lazy-tools` shortens every
  upstream tool description to one sentence (~160 chars) and injects an
  `sqz_tool_help` tool that returns the full original documentation on
  demand. Schemas are untouched. Aimed at servers whose `tools/list` alone
  costs 10-40k tokens.
- **Cache-aware cost estimate** — `sqz stats --cost` estimates dollars
  saved under a prompt-cached billing model (cache write ×1.25 once, cache
  read ×0.10 per re-read turn) instead of the misleading tokens-×-list-price
  math. Assumptions are printed and overridable (`--price-in`,
  `--reread-turns`, `--cache-write-mult`, `--cache-read-mult`). Motivated
  by arXiv:2607.12161 ("Token Reduction Is Not Cost Reduction").
  `sqz stats --json` gains `regretReruns`, `regretExpands`, and (with
  `--cost`) `estimatedCostSavedUsd` fields.

## [1.5.0] — 2026-08-16

### Added
- **Log-template compaction** — consecutive log lines identical except for
  timestamps, durations, or percentages collapse to the first line plus an
  explicit `[×N similar lines; only timestamps/durations vary]` marker.
  Deliberately identifier-safe: lines containing hex ids, UUIDs, filenames,
  or plain integers never collapse.
- **MinHash fuzzy dedup** — the LSH index now feeds delta encoding, so a
  near-duplicate of any content stored earlier in the session resolves as
  a compact delta, not just duplicates within the 10 most recent entries.
  Closes the whitepaper's sub-file dedup future-work item at whole-content
  granularity.
- **Identifier factsheet** — when entropy truncation drops segments, exact
  identifiers from the dropped region (SHAs, UUIDs, ticket codes, versions)
  ride along in a capped `[ids in omitted segments: ...]` line.
- **GitHub Copilot CLI support** — 10th integration. `sqz init` installs a
  `preToolUse` hook at `~/.copilot/hooks/sqz.json` (honors `COPILOT_HOME`);
  Copilot's documented `modifiedArgs` makes it a full command-rewrite host
  like Claude Code and Gemini. `copilot` joins `--only`/`--skip`.
- **BPE benchmark verification** — the formatter benchmark fixtures are
  re-measured with the real `cl100k_base` tokenizer on every test run;
  current divergence from the chars/4 unit is 1.7 points, and CI fails if
  it ever reaches 10.

### Changed
- **Net-win gate** — generic-path compressions that save fewer than 16
  tokens now pass the original through verbatim instead of trading markers
  for negligible savings. Repeat content still resolves to dedup refs.
- **Benign-aware truncation** — output with no error markers gets 2x the
  entropy-truncation threshold, so clean builds and passing test runs stay
  verbatim.
- **Kiro integration rebuilt** — the old hook file used a schema Kiro never
  recognized (and Kiro hooks can't rewrite tool input regardless). `sqz
  init` now installs an always-on steering file plus MCP registration in
  `.kiro/settings/mcp.json`, and cleans up the legacy hook file.
- **Windsurf** — rules now install to the documented `.windsurf/rules/`
  directory; a legacy sqz-authored `.windsurfrules` is migrated
  automatically.
- **Cline** — `.clinerules`-as-directory projects now get
  `.clinerules/sqz.md` instead of being silently skipped.

### Fixed
- **Claude Code `updatedInput` field loss** — the hook rewrite replaced the
  whole tool input with just the command, silently dropping
  `run_in_background`, `timeout`, and `description`. Sibling fields are now
  carried over. (Windsurf hook path fixed identically.)
- **Uninstall over-deletion** — `sqz uninstall` no longer deletes a
  user-authored `.clinerules` file it never wrote, and no longer deletes
  `AGENTS.md` through Zed's placeholder path.

## [1.4.0] — 2026-08-15

### Added
- **`sqz pin`** — pinned cache entries that bypass the 30-minute TTL
  freshness check, so `§ref:HASH§` tokens to stable knowledge-base content
  (project docs, system prompts, a wiki) stay dedup-able forever and across
  agents sharing the session store. Subcommands: `add` (file/stdin/literal),
  `mark`/`remove` (by hash or short prefix), `list`, `clear`.
- **Zed editor support** (#38) — `sqz init` now configures the Zed Agent:
  guidance goes into the instruction file Zed actually reads (`.rules` when
  present, else `AGENTS.md`) and `sqz-mcp` is registered under
  `context_servers` in Zed's settings.json, exposing the compress/expand
  tools and the cached file-read tools — especially useful with local
  models. JSONC settings files are never rewritten; sqz prints the snippet
  for manual paste instead. `zed` joins `--only`/`--skip`.
- **`--no-abbrev` / `SQZ_NO_ABBREV=1`** (#29, thanks @zacwolfe) — opt out of
  n-gram phrase abbreviation. The abbreviator keeps only the first
  occurrence of a repeated phrase; when a SHA/path/URL lives inside it,
  later references became `«A1»` and copy-paste broke. Mirrors the
  `--no-cache`/`SQZ_NO_DEDUP` convention; documented in all generated
  agent rules.

### Fixed
- **Panic on multi-byte UTF-8 input** (#34) — byte-offset slicing landed
  mid-character on densely non-ASCII text (Cyrillic, CJK, Arabic, emoji)
  and killed sqz mid-pipeline, losing the command's output entirely. All
  ten affected sites now go through a boundary-safe `text_boundary` module;
  property tests cover arbitrary strings and cut points.
- **MCP file-read tools are now lossless** (#32, thanks @pm7y) —
  `sqz_read_file`/`sqz_grep`/`sqz_list_dir` previously ran the full
  pipeline including entropy truncation, silently dropping roughly half of
  any non-JSON file over 500 bytes. File reads are now byte-faithful apart
  from ANSI stripping; token savings on that path come from the dedup
  `§ref` cache on repeat reads. The general `compress` tool stays
  aggressive.
- **`git status` parser on non-English locales** (#30, thanks
  @nandanadileep) — the short-format fallback matched any line with `M` in
  the first two characters, corrupting localized headers ("Modifiche…" →
  `M ifiche…`) and dropping every file. Porcelain lines are now parsed
  strictly, long-format section headers are matched against git's own
  translations (it/de/fr/es/pt_PT, verified against git's po files), and
  anything unrecognized passes through raw instead of emitting a wrong
  summary.
- **Dashboard SSE connection leak** (#36) — interrupted `/events` clients
  left sockets in CLOSE_WAIT and eventually made the dashboard unresponsive
  until restart. The server now handles each connection on its own thread
  and detects client disconnects the moment the FIN arrives.
- **Stale Claude Code hook configs now heal on re-init** (#26) — the
  PowerShell fix in 1.3.0 only helped fresh installs; `sqz init` skipped
  existing project settings, so pre-fix Windows installs kept the
  `Bash`-only matcher forever. Re-running `sqz init` now upserts sqz's
  entries in place (matcher upgraded, binary path refreshed) while
  preserving the user's own settings.
- **CI test flakiness** (thanks @pm7y, @nandanadileep) — cli_proxy tests
  no longer race on the shared `~/.sqz/sessions.db` under parallel
  `cargo test`, and ref-freshness timing tests are deterministic instead
  of wall-clock-sensitive.

## [1.3.0] — 2026-06-21

### Added

- **`sqz dashboard` live metrics** — background thread polls the session
  store every 5s and pushes token totals, cache hit/miss counters, per-tool
  and per-command breakdowns, and session history through the SSE endpoint.
  `cmd_dashboard` uses `dirs-next` for cross-platform home dir lookup.
- **Expanded per-command formatters** — from ~12 commands to 40+ across 9
  ecosystems (git, Rust, JS/TS, Python, Go, Cloud, Containers, JVM, System).
  Purpose-built compression now covers the vast majority of commands seen in
  AI coding sessions.
- **New command support** — gh (GitHub CLI with JSON parsing), ruff, mypy, pip,
  go test `-json` event stream parser, go build/vet, golangci-lint, grep/rg/ag
  (group by file), tree (collapse noise directories), curl/wget (strip progress),
  aws, terraform plan/apply/init, gcloud, gradle build/test, maven.
- **Ruby ecosystem** — rspec (JSON + text), rubocop, rake (test tasks),
  bundle (install/exec unwrap).
- **pnpm/yarn/bun support** — package manager detection and formatting for
  install, audit, outdated commands.
- **ANSI pre-processing** — formatters now strip ANSI escape codes and progress
  bars before parsing, improving reliability on colored terminal output.
- **Truncation caps** — large outputs (>30 errors, >15 warnings) are capped
  with "...+N more" summaries to prevent unbounded context consumption.
- **Property tests** — 7 proptest properties ensure formatters never panic,
  never expand output, and behave deterministically across arbitrary inputs
  including binary data and unicode.
- **MCP-path compression logging** — MCP tool calls (compress, sqz_read_file,
  sqz_list_dir, sqz_grep) now log their savings to the session store, so
  `sqz stats` and `sqz gain` reflect MCP usage, not just the shell-hook path.
- **Reproducible benchmark fixtures** — `cmd_formatter_bench.rs` provides the
  test inputs behind every BENCHMARKS.md number. Run
  `cargo test -p sqz-engine cmd_formatter_bench -- --nocapture` to regenerate.

### Changed

- **Modular cmd_formatters architecture** — `cmd_formatters.rs` split into a
  directory of 18 files organized by ecosystem. Each ecosystem module is
  independently testable and extensible.
- **Cargo formatter rewrite** — block-based build parser (skips Compiling/
  Downloading noise, groups errors with source context), multi-suite test
  result aggregation, proper clippy formatter with error blocks and
  warning-by-rule grouping with source locations.
- **Git formatter improvements** — diff now includes a file-level summary
  header (file count, +/- stats), new sub-commands: show, stash, remote,
  fetch.
- **npm formatter improvements** — audit (JSON + human-readable), outdated
  package listing with truncation.

### Fixed

- **Shell hook drops subcommand from `--cmd`** — the Bash hook passed only
  the base name (`extract_base_command("git status")` → `"git"`), so every
  subcommand-routed formatter (git/cargo/go/jvm/rake) silently fell through
  to generic compression in the live hook. Now passes the full command.
  Bug predates all formatter work.
- **PowerShell tool not recognized on Windows Claude Code** (issue #26) —
  Claude Code 2.1.126+ on Windows exposes a `PowerShell` tool. The hook's
  is_shell allowlist only matched Bash/Shell variants, so PowerShell calls
  passed through unchanged. The installed matcher was also hard-coded to
  `Bash`. Now recognises `PowerShell`/`pwsh` and uses `Bash|PowerShell`
  regex matcher in `.claude/settings.local.json` and
  `~/.claude/settings.json`.
- **vizit fails to compile on Windows MSVC** (PR #25) — three `libc::STDOUT_FILENO`
  / `STDIN_FILENO` calls were not gated behind `cfg(unix)`. On Windows the
  dashboard now prints a single plain-text snapshot and exits, skipping the
  unsupported raw-mode path.
- **vizit non-TTY snapshot truncated to footer** — `strip_ansi` only handled
  CSI codes ending in `m`, so the cursor-home (`\x1b[H`) and hide-cursor
  (`\x1b[?25l`) prefix from `render_frame` made the function eat everything
  up to the first `m` byte further down. Now treats any byte in `0x40..=0x7e`
  as a CSI final byte.
- **`sqz gain` reported "No compression data yet" with stats present** —
  `cmd_gain` couldn't distinguish "no data at all" from "no data in the
  selected time window". Now checks `stats.total_compressions` to give the
  correct message in each case.

### Chore / Docs

- `.kiro/` added to `.gitignore` (PR #27) — Kiro hooks generated by
  `sqz init` contain machine-specific paths and shouldn't be committed.
- `docs/integrations/level3/api-proxy.md`: status flipped from
  "Not yet implemented" to "Implemented (since v0.5.0)" (PR #28).
- README: added `sqz reset` to the CLI command listing.

## [1.2.0] — 2026-06-02

### Added

- **`sqz reset` command** (issue #17) — clear the dedup cache, compression
  stats, or both. Supports `--cache-only`, `--stats-only`, `--project .`
  for scoped resets. Gives users control when switching models or starting
  fresh on a project where stale `§ref:HASH§` tokens confuse the agent.

### Fixed

- **Windows: `sqz init` not idempotent** (issue #21) — re-running `sqz init
  --global` appended duplicate hook entries instead of replacing them. Root
  cause: the upsert sentinel was `"sqz hook claude"` which doesn't match
  `"sqz.exe hook claude"` on Windows (`.exe` breaks the substring match).
  Fixed by using subcommand-only sentinels (`"hook claude"`, etc.).
- **Windows: hook command fails under Git Bash** (issue #20) — Claude Code
  runs hooks through Git Bash where `\` is an escape character. The stored
  path `C:\Users\...\sqz.exe` collapsed to `C:Users...sqz.exe`. Fixed by
  normalizing all backslashes to forward slashes (`C:/Users/.../sqz.exe`)
  which works in Git Bash, cmd.exe, and PowerShell.
- **OpenCode: shell operators corrupt commands** (issue #22) — the OpenCode
  plugin was missing the `hasShellOperators` guard that the Claude Code path
  already had. Commands with heredocs, pipes, redirects, and compound
  operators (`&&`, `||`) got `2>&1 | sqz compress` appended, breaking the
  command structure. Fixed in both the generated TS plugin and the Rust hook
  processor.
- **CI: Hardened homebrew dispatch** (issue #15) — removed dead
  `update-homebrew` job and stale `homebrew/sqz.rb`. Dispatch now skips
  gracefully if `HOMEBREW_TAP_TOKEN` is unset and surfaces the actual API
  error on failure.

## [1.1.1] — 2026-05-10

### Added

- **Kiro IDE and CLI integration** — `sqz init` now configures Kiro with
  PreToolUse hooks and MCP server registration. Supports both Kiro IDE
  (`.kiro/settings/mcp.json`) and Kiro CLI hook format.
- **Proptest regression seeds** — added seeds for vizit and api_proxy to
  prevent flaky test regressions.

### Changed

- README: Updated stats output examples and CLI commands section.
- CLAUDE.md added to `.gitignore` (generated per-project, not committed).

## [1.1.0] — 2026-05-09

### Added

- **`sqz vizit`** — live terminal dashboard showing real-time compression
  stats, cache hits, and session activity in a TUI interface.
- **Colorful terminal output** — `sqz stats` and `sqz gain` now use colored
  output with progress bars and visual indicators.
- **Per-command breakdown** — stats now show compression savings broken down
  by command type (git, cargo, npm, etc.).
- **Adaptive compression pressure** — automatically adjusts aggressiveness
  based on observed savings patterns.
- **Homebrew tap** — `brew install ojuschugh1/tap/sqz` now works. CI
  dispatches to the homebrew-sqz tap on release.
- **Featured badge** — linking to NextGen Tech Insider article.

### Fixed

- **VS Code extension wired to real stats** (issue #12) — the extension
  now reads from the actual sqz session store instead of showing zeros.
- **npm auto-publish on release** — the publish workflow now triggers
  correctly and binary wrappers are improved.

### Changed

- README: Added cargo install instructions for both `sqz-cli` and `sqz-mcp`.

## [1.0.9] — 2026-04-23

### Fixed

- **MCP tools now use dedup cache** (issue #12 follow-up) — all 4 MCP tool
  handlers (`compress`, `sqz_read_file`, `sqz_grep`, `sqz_list_dir`) were
  calling `engine.compress()` directly, bypassing `CacheManager`. The dedup
  cache — sqz's headline 92% savings feature — never fired through the MCP
  path. Now routed through `compress_with_cache()`. Second read of the same
  content returns a 13-token `§ref:HASH§` instead of re-compressing.
- **Test isolation** — `make_server()` in sqz-mcp tests now uses a per-tempdir
  SQLite store so cache state doesn't bleed between tests.

### Added

- **`sqz stats --project` / `sqz gain --project`** — filter stats and gain
  charts to the current project directory.
- **`sqz print-opencode-plugin`** — dump the generated OpenCode plugin to
  stdout for manual install without running `sqz init`.

## [1.0.8] — 2026-04-23

### Fixed

- **JSONC trailing commas broke OpenCode config merge** (issue #6) —
  `strip_jsonc_comments` removed `//` and `/* */` comments but left
  trailing commas intact. Trailing commas are valid JSONC but invalid
  JSON, so `serde_json` failed to parse configs like
  `{ "mcp": { "dart": { ... }, }, }`. The error was silently swallowed,
  so `sqz init` printed "OpenCode hook installed" while the `.jsonc`
  file was never modified. Fix: added a string-aware second pass that
  strips commas before `]` and `}`.
- **Codex integration test env-var races** — replaced
  `std::env::set_var("CODEX_HOME")` with home-dir-injectable `_at`
  variants (same pattern as `claude_md_integration`), eliminating flaky
  failures in `remove_codex_mcp_config` and `prop_compressed_output_is_valid_json`.

## [1.0.7] — 2026-04-22

### Fixed

- **Claude Code utilization gap** (issue #12 follow-up) — sqz-mcp was
  connected but Claude Code never called its tools because the MCP server
  only offered `compress` while the PreToolUse hook already compressed
  Bash output. Added guidance in the tool description so Claude knows
  when to prefer the MCP tool over the built-in Read/Grep.
- **OpenCode plugin always regenerated on init** — `sqz init` now
  overwrites `~/.config/opencode/plugins/sqz.ts` every time to pick up
  fixes (V1 shape, `--cmd` rewrite, etc.) without requiring manual
  deletion. Also adds `enabled: true` to the MCP entry in opencode.json.
- **Codex integration test env-var races** — eliminated `std::env::set_var`
  in codex tests; each test now uses isolated temp directories with
  `CODEX_HOME` override instead of mutating the global `HOME`.

## [1.0.6] — 2026-04-22

### Fixed

- **MCP: `notifications/initialized` caused Claude Code to mark server as
  failed** (issue #12) — the server was returning an error response to
  JSON-RPC notifications (messages without an `id`). Per the spec,
  notifications are one-way and must produce no output. Claude Code saw
  the unexpected error and marked sqz-mcp as failed. Fix: silently ignore
  all notifications. 2 regression tests added.

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
  env, statusLine, unrelated hooks). Following Anthropic's official scope
  table (Managed > Local > Project > User).
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
