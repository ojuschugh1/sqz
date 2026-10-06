//! Just enough POSIX shell parsing for the PreToolUse rewrite to decide
//! whether a command can run as `{ cmd; } | sqz compress`.
//!
//! A pipeline element runs in a subshell, so a builtin that changes shell
//! state (cd, export, source, set, ...) stops affecting the session once
//! the command is grouped, and a background job keeps the pipe open until
//! it exits. [`plan`] refuses those and anything it can't follow, and
//! accepts the rest: pipes, `&&`/`||`/`;` lists, loops, redirects,
//! heredocs and comments. A leading `cd DIR &&` stays outside the group.

/// Why a command is left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Skip {
    /// Syntax the scanner doesn't follow, or that the shell would reject.
    Unparsed,
    /// A background job would hold the pipe open.
    Background,
    /// A builtin whose effect has to outlive the command.
    StateChange,
    /// An editor, pager, REPL, follower or server.
    LongRunning,
    /// The command already runs or pipes through sqz.
    AlreadySqz,
    /// Nothing to compress once a leading `cd` is set aside.
    Empty,
}

/// A command that can be piped through sqz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan<'a> {
    /// A leading run of `cd DIR &&` (or `;`), kept outside the group so the
    /// directory change still applies afterwards. Usually empty.
    pub prefix: &'a str,
    /// What runs inside the group.
    pub body: &'a str,
    /// `body` is one command with no list, pipe, newline, comment or
    /// heredoc, so it can take the one-line form and its formatter.
    pub simple: bool,
}

/// Builtins whose effect is lost, or changes meaning, inside a subshell.
const STATE_BUILTINS: &[&str] = &[
    "cd", "pushd", "popd", "export", "unset", "source", ".", "alias", "unalias", "set",
    "shopt", "setopt", "unsetopt", "ulimit", "umask", "trap", "eval", "exec", "exit",
    "return", "logout", "declare", "typeset", "local", "readonly", "let", "hash", "rehash",
    "enable", "bind", "complete", "autoload", "zmodload", "wait", "fg", "bg", "jobs",
    "disown", "suspend", "history", "fc",
];

/// Decide whether, and how, `command` can be piped through sqz.
pub fn plan(command: &str) -> Result<Plan<'_>, Skip> {
    let command = command.trim_end();
    let mut scan = Scanner::new(command);
    scan.list(false)?;
    if !scan.pending.is_empty() {
        return Err(Skip::Unparsed);
    }
    let segs = segments(command, &scan.toks)?;
    if segs.iter().filter_map(|s| s.cmd.as_deref()).any(|c| is_sqz(basename(c))) {
        return Err(Skip::AlreadySqz);
    }

    let mut hoisted = 0;
    let mut prefix_end = 0;
    for seg in &segs {
        let leading_cd = seg.cmd.as_deref() == Some("cd")
            && !seg.assign
            && seg.kw == 0
            && matches!(seg.sep, Some(Sep::And | Sep::Semi | Sep::Newline));
        if !leading_cd {
            break;
        }
        hoisted += 1;
        prefix_end = seg.end;
    }
    let rest = &segs[hoisted..];
    if rest.is_empty() {
        return Err(Skip::Empty);
    }
    for seg in rest {
        match seg.cmd.as_deref() {
            Some(cmd) => {
                if STATE_BUILTINS.contains(&cmd)
                    || (cmd == "kill" && seg.args.iter().any(|a| a.starts_with('%')))
                {
                    return Err(Skip::StateChange);
                }
                if long_running(basename(cmd), &seg.args) {
                    return Err(Skip::LongRunning);
                }
            }
            // A bare assignment sets a shell variable.
            None if seg.assign && seg.kw == 0 && !seg.subshell => return Err(Skip::StateChange),
            None => {}
        }
    }
    if watch_text(command) {
        return Err(Skip::LongRunning);
    }

    let simple = rest.len() == 1
        && rest[0].sep.is_none()
        && rest[0].kw == 0
        && !scan.comment
        && !scan.heredoc;
    Ok(Plan {
        prefix: &command[..prefix_end],
        body: &command[prefix_end..],
        simple,
    })
}

/// One command of a [`flat`] list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatCmd {
    /// Byte offset just past the command's last token.
    pub end: usize,
    /// The stats and formatter label, set when this command's output
    /// should go through sqz.
    pub label: Option<String>,
    /// The command doesn't place stderr itself, so `2>&1` goes before the pipe.
    pub merge_stderr: bool,
}

/// Commands too quiet to be worth a pipe.
const QUIET: &[&str] = &[
    "true", "false", ":", "sleep", "mkdir", "rmdir", "touch", "rm", "mv", "cp", "ln", "chmod",
    "chown", "chgrp", "echo", "printf", "test", "[", "kill", "read", "clear", "unlink",
    "truncate",
];

/// Split `command` into simple commands joined by `&&`, `||`, `;` or
/// newlines, marking the ones whose output can go through sqz on their
/// own. Nothing is grouped, so builtins keep running in the shell itself
/// and a permission checker sees the user's commands as written.
///
/// `None` for a pipeline, subshell, group, loop, conditional, heredoc or
/// background job, or when no command is worth piping.
pub fn flat(command: &str) -> Option<Vec<FlatCmd>> {
    let command = command.trim_end();
    let mut scan = Scanner::new(command);
    scan.list(false).ok()?;
    if !scan.pending.is_empty() || scan.heredoc {
        return None;
    }
    let segs = segments(command, &scan.toks).ok()?;
    if segs.iter().any(|s| s.sep == Some(Sep::Pipe) || s.subshell || s.kw > 0) {
        return None;
    }
    if segs.iter().filter_map(|s| s.cmd.as_deref()).any(|c| is_sqz(basename(c))) {
        return None;
    }
    let braces = command.contains(['{', '}']);
    let cmds: Vec<FlatCmd> = segs
        .iter()
        .map(|seg| {
            let label = seg.cmd.as_deref().and_then(|cmd| {
                let base = basename(cmd);
                let text = &command[seg.first.unwrap_or(0)..seg.last];
                let skip = STATE_BUILTINS.contains(&cmd)
                    || QUIET.contains(&base)
                    || seg.out
                    || long_running(base, &seg.args)
                    || watch_text(text);
                if skip {
                    return None;
                }
                flat_label(command, &seg.words, braces)
            });
            FlatCmd { end: seg.last, label, merge_stderr: !seg.err }
        })
        .collect();
    cmds.iter().any(|c| c.label.is_some()).then_some(cmds)
}

/// The command word and the plain words after it, up to the first one with
/// quotes, expansions or globs. With braces elsewhere in the command the
/// label stays one unquoted word: some permission checkers treat a brace
/// and a quote in the same command as possible obfuscation.
fn flat_label(command: &str, words: &[(usize, usize)], braces: bool) -> Option<String> {
    let plain = |w: &str| {
        !w.is_empty()
            && w.bytes().all(|c| c.is_ascii_alphanumeric() || b"_./:@+,=-".contains(&c))
    };
    let mut label = String::new();
    for &(start, end) in words {
        let word = &command[start..end];
        if !plain(word) || label.len() + word.len() + 1 > 100 {
            break;
        }
        if !label.is_empty() {
            if braces {
                break;
            }
            label.push(' ');
        }
        label.push_str(word);
    }
    (!label.is_empty()).then_some(label)
}

/// Watch and dev-server spellings matched on the whole command.
fn watch_text(cmd: &str) -> bool {
    cmd.contains("--watch")
        || cmd.contains("run dev")
        || cmd.contains("run start")
        || cmd.contains("run serve")
}

/// A command's name with its directory and any `.exe` suffix removed.
/// Either path separator counts, and the suffix in any case.
pub fn basename(cmd: &str) -> &str {
    let base = cmd.rsplit(['/', '\\']).next().unwrap_or(cmd);
    match base.len().checked_sub(4) {
        Some(at) if base.get(at..).is_some_and(|e| e.eq_ignore_ascii_case(".exe")) => &base[..at],
        _ => base,
    }
}

fn is_sqz(base: &str) -> bool {
    matches!(base, "sqz" | "sqz-mcp")
}

/// Editors, pagers, REPLs, followers and servers: commands whose output has
/// to reach the agent while they run, or that never end.
fn long_running(base: &str, args: &[String]) -> bool {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let short = |c: char| {
        args.iter()
            .any(|a| a.len() > 1 && a.starts_with('-') && !a.starts_with("--") && a[1..].contains(c))
    };
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect();
    let sub = words.first().copied();
    match base {
        "vim" | "vi" | "nvim" | "nano" | "emacs" | "pico" | "less" | "more" | "most" | "top"
        | "htop" | "btop" | "ssh" | "mosh" | "tmux" | "screen" | "watch" | "yes" | "stern"
        | "ngrok" | "nodemon" | "uvicorn" | "gunicorn" | "hypercorn" | "http-server"
        | "live-server" => true,
        "python" | "python2" | "python3" | "node" | "irb" | "ghci" | "psql" | "mysql"
        | "sqlite3" | "mongo" | "mongosh" | "redis-cli" => !finite_runtime(base, args, &words),
        "tail" => short('f') || short('F') || has("--follow"),
        "journalctl" => short('f') || has("--follow"),
        "kubectl" | "oc" => match sub {
            Some("logs") => short('f') || has("--follow"),
            Some("get") => short('w') || has("--watch"),
            Some("port-forward" | "proxy" | "attach") => true,
            _ => false,
        },
        "docker" | "podman" | "nerdctl" | "docker-compose" => {
            let words = if sub == Some("compose") { &words[1..] } else { &words[..] };
            match words.first().copied() {
                Some("logs") => short('f') || has("--follow"),
                Some("events" | "attach" | "watch") => true,
                Some("stats") => !has("--no-stream"),
                Some("up") => !(short('d') || has("--detach") || has("--wait")),
                _ => false,
            }
        }
        "ping" | "ping6" => !short('c'),
        "npm" | "pnpm" | "yarn" | "bun" => {
            let script = if sub == Some("run") { words.get(1).copied() } else { sub };
            matches!(script, Some("dev" | "start" | "serve" | "watch" | "preview"))
        }
        "vite" => !matches!(sub, Some("build" | "optimize")),
        "next" | "nuxt" | "astro" | "remix" => matches!(sub, Some("dev" | "start" | "preview")),
        "flask" => sub == Some("run"),
        "rails" => matches!(sub, Some("server" | "s" | "console" | "c")),
        "php" => has("-S"),
        "hugo" | "mdbook" | "mkdocs" | "jekyll" | "zola" => {
            matches!(sub, Some("server" | "serve" | "watch"))
        }
        "cargo" => sub == Some("watch"),
        "tsc" | "jest" | "vitest" | "webpack" | "rollup" | "esbuild" | "parcel" | "babel"
        | "sass" | "tailwindcss" => short('w') || has("--watch"),
        _ => false,
    }
}

/// A runtime or database client invoked to do one thing and exit, rather
/// than a REPL, a server or a script that may be either.
fn finite_runtime(base: &str, args: &[String], words: &[&str]) -> bool {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("-i") || has("--interactive") {
        return false;
    }
    match base {
        "python" | "python2" | "python3" => {
            if let Some(module) = args.iter().position(|a| a == "-m").and_then(|i| args.get(i + 1)) {
                return !matches!(
                    module.as_str(),
                    "http.server" | "uvicorn" | "gunicorn" | "flask" | "IPython" | "ipython"
                        | "pdb" | "code" | "asyncio" | "idlelib"
                );
            }
            if words.first().is_some_and(|w| w.ends_with("manage.py")) {
                return !matches!(
                    words.get(1).copied(),
                    None | Some("runserver" | "runserver_plus" | "shell" | "shell_plus" | "dbshell")
                );
            }
            // A script, `-c` code or `-` (stdin, usually a heredoc); bare
            // `python` is the REPL.
            !words.is_empty() || has("-c") || has("-") || has("--version") || has("-V")
                || has("-h") || has("--help")
        }
        "node" => {
            has("-") || has("-e") || has("--eval") || has("-p") || has("--print") || has("--test")
                || has("-v") || has("--version") || has("-c") || has("--check")
        }
        "psql" => {
            has("-c") || has("-f") || has("--command") || has("--file") || has("-l") || has("--list")
        }
        "mysql" => has("-e") || has("--execute"),
        "sqlite3" => words.len() >= 2,
        "mongo" | "mongosh" => has("--eval"),
        _ => false,
    }
}

// ── Lexer ────────────────────────────────────────────────────────────────────

type Res<T> = Result<T, Skip>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    Semi,
    And,
    Or,
    Pipe,
    Newline,
}

#[derive(Debug)]
enum Tok {
    Word { start: usize, end: usize, plain: String },
    Sep { kind: Sep, end: usize },
    /// `out`: stdout goes somewhere other than the pipe. `err`: the
    /// command already says where stderr goes.
    Redirect { start: usize, end: usize, out: bool, err: bool },
    Subshell,
}

/// Nesting of lists and expansions past which a command is left alone, so
/// the recursive scan stays well inside the stack.
const MAX_NESTING: usize = 64;

struct Scanner<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    toks: Vec<Tok>,
    /// Heredocs whose bodies start after the next unquoted newline:
    /// (delimiter, strip leading tabs).
    pending: Vec<(String, bool)>,
    comment: bool,
    heredoc: bool,
    nesting: usize,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str) -> Self {
        Scanner {
            s,
            b: s.as_bytes(),
            i: 0,
            toks: Vec::new(),
            pending: Vec::new(),
            comment: false,
            heredoc: false,
            nesting: 0,
        }
    }

    fn nest(&mut self) -> Res<()> {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            return Err(Skip::Unparsed);
        }
        Ok(())
    }

    fn peek(&self, k: usize) -> Option<u8> {
        self.b.get(self.i + k).copied()
    }

    /// Step over one whole char, optionally keeping it.
    fn bump(&mut self, keep: Option<&mut String>) {
        if let Some(ch) = self.s[self.i..].chars().next() {
            if let Some(plain) = keep {
                plain.push(ch);
            }
            self.i += ch.len_utf8();
        }
    }

    fn blanks(&mut self) {
        loop {
            match (self.peek(0), self.peek(1)) {
                (Some(b' ' | b'\t'), _) => self.i += 1,
                (Some(b'\\'), Some(b'\n')) => self.i += 2,
                _ => return,
            }
        }
    }

    fn sep(&mut self, kind: Sep, nested: bool) {
        if !nested {
            self.toks.push(Tok::Sep { kind, end: self.i });
        }
    }

    /// Scan a command list. The top level keeps its tokens; a nested list
    /// (`( )`, `$( )`, `<( )`) ends at its `)` and only gets the syntax and
    /// background checks.
    fn list(&mut self, nested: bool) -> Res<()> {
        if nested {
            self.nest()?;
        }
        loop {
            self.blanks();
            let Some(c) = self.peek(0) else {
                return if nested { Err(Skip::Unparsed) } else { Ok(()) };
            };
            match c {
                b'\n' => {
                    self.i += 1;
                    self.sep(Sep::Newline, nested);
                    self.heredoc_bodies()?;
                }
                b'#' => {
                    self.comment = true;
                    while self.peek(0).is_some_and(|c| c != b'\n') {
                        self.i += 1;
                    }
                }
                b';' => {
                    if matches!(self.peek(1), Some(b';' | b'&')) {
                        return Err(Skip::Unparsed);
                    }
                    self.i += 1;
                    self.sep(Sep::Semi, nested);
                }
                b'&' => match self.peek(1) {
                    Some(b'&') => {
                        self.i += 2;
                        self.sep(Sep::And, nested);
                    }
                    Some(b'>') => self.redirect(nested, self.i, None)?,
                    _ => return Err(Skip::Background),
                },
                b'|' => {
                    let (kind, len) = match self.peek(1) {
                        Some(b'|') => (Sep::Or, 2),
                        Some(b'&') => (Sep::Pipe, 2),
                        _ => (Sep::Pipe, 1),
                    };
                    self.i += len;
                    self.sep(kind, nested);
                }
                b'(' => {
                    if self.peek(1) == Some(b'(') {
                        return Err(Skip::Unparsed);
                    }
                    self.i += 1;
                    self.list(true)?;
                    if !nested {
                        self.toks.push(Tok::Subshell);
                    }
                }
                b')' => {
                    if nested {
                        self.i += 1;
                        self.nesting -= 1;
                        return Ok(());
                    }
                    return Err(Skip::Unparsed);
                }
                b'<' | b'>' if self.peek(1) != Some(b'(') => self.redirect(nested, self.i, None)?,
                _ => {
                    let digits = self.b[self.i..].iter().take_while(|c| c.is_ascii_digit()).count();
                    let op = self.b.get(self.i + digits).copied();
                    let after = self.b.get(self.i + digits + 1).copied();
                    if digits > 0 && matches!(op, Some(b'<' | b'>')) && after != Some(b'(') {
                        let start = self.i;
                        let fd = self.s[start..start + digits].parse().ok();
                        self.i += digits;
                        self.redirect(nested, start, fd)?;
                    } else {
                        self.word(nested)?;
                    }
                }
            }
        }
    }

    fn word(&mut self, nested: bool) -> Res<()> {
        let (start, plain) = self.word_text()?;
        if nested {
            // `case` patterns end in an unbalanced `)`.
            if plain == "case" {
                return Err(Skip::Unparsed);
            }
        } else {
            self.toks.push(Tok::Word { start, end: self.i, plain });
        }
        Ok(())
    }

    /// One word, returned with quotes and escapes removed. Expansions are
    /// skipped over and leave a `$` or `` ` `` in their place.
    fn word_text(&mut self) -> Res<(usize, String)> {
        let start = self.i;
        let mut plain = String::new();
        while let Some(c) = self.peek(0) {
            match c {
                b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'(' | b')' => break,
                b'<' | b'>' => {
                    if self.peek(1) != Some(b'(') {
                        break;
                    }
                    self.i += 2;
                    self.list(true)?;
                    plain.push_str("<()");
                }
                b'\\' => match self.peek(1) {
                    None => return Err(Skip::Unparsed),
                    Some(b'\n') => self.i += 2,
                    Some(_) => {
                        self.i += 1;
                        self.bump(Some(&mut plain));
                    }
                },
                b'\'' => {
                    let close = self.s[self.i + 1..].find('\'').ok_or(Skip::Unparsed)?;
                    plain.push_str(&self.s[self.i + 1..self.i + 1 + close]);
                    self.i += close + 2;
                }
                b'"' => self.double_quoted(&mut plain)?,
                b'`' => {
                    self.backtick()?;
                    plain.push('`');
                }
                b'$' => self.dollar(&mut plain, false)?,
                _ => self.bump(Some(&mut plain)),
            }
        }
        if self.i == start {
            return Err(Skip::Unparsed);
        }
        Ok((start, plain))
    }

    fn double_quoted(&mut self, plain: &mut String) -> Res<()> {
        self.i += 1;
        loop {
            match self.peek(0) {
                None => return Err(Skip::Unparsed),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(());
                }
                Some(b'\\') => match self.peek(1) {
                    None => return Err(Skip::Unparsed),
                    Some(n @ (b'$' | b'`' | b'"' | b'\\')) => {
                        plain.push(n as char);
                        self.i += 2;
                    }
                    Some(b'\n') => self.i += 2,
                    Some(_) => {
                        plain.push('\\');
                        self.i += 1;
                    }
                },
                Some(b'`') => {
                    self.backtick()?;
                    plain.push('`');
                }
                Some(b'$') => self.dollar(plain, true)?,
                Some(_) => self.bump(Some(plain)),
            }
        }
    }

    /// `$` and what follows it: `$(...)`, `$((...))`, `${...}`, `$'...'`.
    fn dollar(&mut self, plain: &mut String, quoted: bool) -> Res<()> {
        plain.push('$');
        match (self.peek(1), self.peek(2)) {
            (Some(b'('), Some(b'(')) => {
                self.i += 3;
                self.closing(b'(', b')', 2)
            }
            (Some(b'('), _) => {
                self.i += 2;
                self.list(true)
            }
            (Some(b'{'), _) => {
                self.i += 2;
                self.closing(b'{', b'}', 1)
            }
            (Some(b'\''), _) if !quoted => {
                self.i += 2;
                loop {
                    match self.peek(0) {
                        None => return Err(Skip::Unparsed),
                        Some(b'\'') => {
                            self.i += 1;
                            return Ok(());
                        }
                        Some(b'\\') if self.peek(1).is_some() => {
                            self.i += 1;
                            self.bump(None);
                        }
                        Some(_) => self.bump(None),
                    }
                }
            }
            _ => {
                self.i += 1;
                Ok(())
            }
        }
    }

    /// Skip to the `close` that balances `depth` opens already consumed, for
    /// `$(( ))` and `${ }`.
    fn closing(&mut self, open: u8, close: u8, mut depth: usize) -> Res<()> {
        self.nest()?;
        let mut scratch = String::new();
        loop {
            match self.peek(0) {
                None => return Err(Skip::Unparsed),
                Some(b'\\') => {
                    if self.peek(1).is_none() {
                        return Err(Skip::Unparsed);
                    }
                    self.i += 1;
                    self.bump(None);
                }
                Some(b'\'') => {
                    let end = self.s[self.i + 1..].find('\'').ok_or(Skip::Unparsed)?;
                    self.i += end + 2;
                }
                Some(b'"') => self.double_quoted(&mut scratch)?,
                Some(b'`') => self.backtick()?,
                Some(b'$') if self.peek(1) == Some(b'(') && self.peek(2) != Some(b'(') => {
                    self.i += 2;
                    self.list(true)?;
                }
                Some(c) if c == open => {
                    depth += 1;
                    self.i += 1;
                }
                Some(c) if c == close => {
                    depth -= 1;
                    self.i += 1;
                    if depth == 0 {
                        self.nesting -= 1;
                        return Ok(());
                    }
                }
                Some(_) => self.bump(None),
            }
        }
    }

    fn backtick(&mut self) -> Res<()> {
        self.i += 1;
        loop {
            match self.peek(0) {
                None => return Err(Skip::Unparsed),
                Some(b'`') => {
                    self.i += 1;
                    return Ok(());
                }
                Some(b'\\') if self.peek(1).is_some() => {
                    self.i += 1;
                    self.bump(None);
                }
                Some(_) => self.bump(None),
            }
        }
    }

    /// A redirection operator and its target word. `start` is where the
    /// redirect began, including an fd number; `fd` is that number.
    fn redirect(&mut self, nested: bool, start: usize, fd: Option<u32>) -> Res<()> {
        let (op, heredoc) = match (self.peek(0), self.peek(1), self.peek(2)) {
            (Some(b'<'), Some(b'<'), Some(b'<')) => ("<<<", None),
            (Some(b'<'), Some(b'<'), Some(b'-')) => ("<<-", Some(true)),
            (Some(b'<'), Some(b'<'), _) => ("<<", Some(false)),
            (Some(b'<'), Some(b'>'), _) => ("<>", None),
            (Some(b'<'), Some(b'&'), _) => ("<&", None),
            (Some(b'>'), Some(b'>'), _) => (">>", None),
            (Some(b'>'), Some(b'|'), _) => (">|", None),
            (Some(b'>'), Some(b'&'), _) => (">&", None),
            (Some(b'&'), Some(b'>'), Some(b'>')) => ("&>>", None),
            (Some(b'&'), Some(b'>'), _) => ("&>", None),
            (Some(b'<'), _, _) => ("<", None),
            (Some(b'>'), _, _) => (">", None),
            _ => return Err(Skip::Unparsed),
        };
        self.i += op.len();
        while matches!(self.peek(0), Some(b' ' | b'\t')) {
            self.i += 1;
        }
        if matches!(
            self.peek(0),
            None | Some(b'\n' | b';' | b'&' | b'|' | b'(' | b')' | b'<' | b'>')
        ) {
            return Err(Skip::Unparsed);
        }
        let (_, target) = self.word_text()?;
        if let Some(strip_tabs) = heredoc {
            self.heredoc = true;
            self.pending.push((target.clone(), strip_tabs));
        }
        let to_fd = !target.is_empty() && target.bytes().all(|c| c.is_ascii_digit());
        let (out, err) = match (op, fd) {
            ("&>" | "&>>", _) => (true, true),
            (">" | ">>" | ">|", None | Some(1)) => (true, false),
            (">" | ">>" | ">|" | ">&", Some(2)) => (false, true),
            // `>&2` and `>&-` move stdout; bash's `>& file` moves both.
            (">&", None | Some(1)) if target == "1" => (false, false),
            (">&", None | Some(1)) => (true, !to_fd && target != "-"),
            ("<>", Some(1)) => (true, false),
            ("<>", Some(2)) => (false, true),
            _ => (false, false),
        };
        if !nested {
            self.toks.push(Tok::Redirect { start, end: self.i, out, err });
        }
        Ok(())
    }

    /// Consume the bodies of the heredocs started on the line just ended.
    fn heredoc_bodies(&mut self) -> Res<()> {
        for (delim, strip_tabs) in std::mem::take(&mut self.pending) {
            loop {
                if self.i >= self.b.len() {
                    return Err(Skip::Unparsed);
                }
                let end = self.s[self.i..].find('\n').map_or(self.b.len(), |k| self.i + k);
                let line = &self.s[self.i..end];
                let line = if strip_tabs { line.trim_start_matches('\t') } else { line };
                let done = line == delim;
                self.i = (end + 1).min(self.b.len());
                if done {
                    break;
                }
            }
        }
        Ok(())
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// One simple command between list or pipe separators.
#[derive(Debug, Default)]
struct Seg {
    /// The command name, quotes removed.
    cmd: Option<String>,
    args: Vec<String>,
    assign: bool,
    /// Reserved words seen (`if`, `then`, `do`, `!`, `time`, ...).
    kw: usize,
    subshell: bool,
    tokens: usize,
    /// The separator after this command and the offset just past it.
    sep: Option<Sep>,
    end: usize,
    /// Byte range from the first token to the end of the last one.
    first: Option<usize>,
    last: usize,
    /// Source ranges of the command word and the words after it.
    words: Vec<(usize, usize)>,
    /// A redirect sends stdout away from the pipe / already places stderr.
    out: bool,
    err: bool,
}

impl Seg {
    fn span(&mut self, start: usize, end: usize) {
        self.first.get_or_insert(start);
        self.last = end;
    }
}

fn segments(command: &str, toks: &[Tok]) -> Res<Vec<Seg>> {
    let mut segs = vec![Seg::default()];
    let mut at_cmd = true;
    // After `command`/`builtin`, the next non-option word is the command.
    let mut wrapper = false;
    for tok in toks {
        match tok {
            Tok::Sep { kind, end } => {
                let seg = segs.last_mut().expect("never empty");
                if seg.tokens == 0 {
                    if *kind == Sep::Newline {
                        continue;
                    }
                    return Err(Skip::Unparsed);
                }
                seg.sep = Some(*kind);
                seg.end = *end;
                segs.push(Seg::default());
                at_cmd = true;
                wrapper = false;
            }
            Tok::Redirect { start, end, out, err } => {
                let seg = segs.last_mut().expect("never empty");
                seg.tokens += 1;
                seg.out |= *out;
                seg.err |= *err;
                seg.span(*start, *end);
            }
            Tok::Subshell => {
                let seg = segs.last_mut().expect("never empty");
                // Only reserved words may come before `( )`: anything else
                // is a function definition or an array assignment.
                if seg.tokens != seg.kw {
                    return Err(Skip::Unparsed);
                }
                seg.tokens += 1;
                seg.subshell = true;
                at_cmd = false;
            }
            Tok::Word { start, end, plain } => {
                let seg = segs.last_mut().expect("never empty");
                seg.tokens += 1;
                seg.span(*start, *end);
                if !at_cmd {
                    seg.args.push(plain.clone());
                    seg.words.push((*start, *end));
                    continue;
                }
                if is_assignment(&command[*start..]) {
                    seg.assign = true;
                    continue;
                }
                match plain.as_str() {
                    "if" | "then" | "elif" | "else" | "do" | "while" | "until" | "!" | "time" => {
                        seg.kw += 1;
                        continue;
                    }
                    "fi" | "done" | "for" | "select" | "[[" => {
                        seg.kw += 1;
                        at_cmd = false;
                        continue;
                    }
                    "case" | "esac" | "function" | "coproc" | "{" | "}" => {
                        return Err(Skip::Unparsed)
                    }
                    _ => {}
                }
                if wrapper && plain.starts_with('-') {
                    continue;
                }
                if !wrapper && (plain == "command" || plain == "builtin") {
                    wrapper = true;
                    continue;
                }
                // A command named by an expansion could be any builtin.
                if (plain.contains('$') || plain.contains('`')) && !plain.contains('/') {
                    return Err(Skip::Unparsed);
                }
                seg.cmd = Some(plain.clone());
                seg.words.push((*start, *end));
                at_cmd = false;
                wrapper = false;
            }
        }
    }
    if segs.last().is_some_and(|s| s.tokens == 0) {
        segs.pop();
        // A list can't end waiting for its next command.
        if segs.last().is_some_and(|s| matches!(s.sep, Some(Sep::And | Sep::Or | Sep::Pipe))) {
            return Err(Skip::Unparsed);
        }
    }
    Ok(segs)
}

/// `NAME=` or `NAME+=` at the start of a word.
fn is_assignment(raw: &str) -> bool {
    let b = raw.as_bytes();
    if !b.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
        return false;
    }
    let name = b.iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
    match b.get(name) {
        Some(b'=') => true,
        Some(b'+') => b.get(name + 1) == Some(&b'='),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple(cmd: &str) {
        let p = plan(cmd).unwrap_or_else(|e| panic!("{cmd:?}: {e:?}"));
        assert!(p.simple && p.prefix.is_empty(), "{cmd:?}: {p:?}");
        assert_eq!(p.body, cmd.trim_end());
    }

    fn grouped(cmd: &str) -> Plan<'_> {
        let p = plan(cmd).unwrap_or_else(|e| panic!("{cmd:?}: {e:?}"));
        assert!(!p.simple, "{cmd:?}: {p:?}");
        p
    }

    fn skipped(cmd: &str, why: Skip) {
        assert_eq!(plan(cmd), Err(why), "{cmd:?}");
    }

    #[test]
    fn plain_commands_stay_simple() {
        for cmd in [
            "git status",
            "  git status  \n",
            "cargo test 2>&1",
            "ls -la > out.txt",
            "make &> build.log",
            "grep -rn \"a|b;c&&d\" src",
            "echo 'semi;colon' \"pipe|d\" naïve",
            "echo \"say \\\"hi\\\"\"",
            "git log $(git merge-base HEAD main)..HEAD",
            "echo \"$(date) && later\"",
            "FOO=bar make test",
            "diff <(sort a.txt) <(sort b.txt)",
            "echo ${path#./} a#b",
            "echo $((1 + 2 & 3))",
            "(cd sub && make)",
            "jq '.items[] | select(.ok)' data.json",
            "find . -name '*.tmp' -exec rm {} \\;",
            "grep -w foo bar.txt",
            "curl -s -w '%{http_code}' https://example.com",
            "git commit -m \"first line\n\nbody\"",
            "python3 -m pytest -q",
            "PYTHONPATH=src python3 scripts/report.py --all",
            "python manage.py test",
            "psql -c 'select 1'",
            "docker compose up -d",
            "ping -c 3 example.com",
            "tail -n 50 app.log",
        ] {
            simple(cmd);
        }
    }

    #[test]
    fn lists_and_pipelines_are_grouped() {
        for cmd in [
            "git log --oneline | head -20",
            "cargo build && cargo test",
            "make || echo failed",
            "make; make test",
            "cargo test 2>&1 | tail -30",
            "for f in *.rs; do wc -l \"$f\"; done",
            "if [ -f x ]; then cat x; else echo none; fi",
            "while read -r l; do echo \"$l\"; done < list.txt",
            "ls # list files",
            "git status\ngit diff --stat",
            "python3 - <<'EOF'\nprint('a|b; c &')\nEOF",
            "cat <<-EOF | sort\n\tb\n\ta\n\tEOF",
            "cat <<EOF\none\nEOF\necho after",
            "[[ -n \"$X\" && -f y ]] && cat y",
            "time cargo build",
            "! grep -q foo bar.txt",
            "echo done;",
            "make |& tee build.log",
        ] {
            let p = grouped(cmd);
            assert!(p.prefix.is_empty(), "{cmd:?}: {p:?}");
        }
    }

    #[test]
    fn leading_cd_stays_outside_the_group() {
        let p = plan("cd sqz_engine && cargo test").unwrap();
        assert_eq!((p.prefix, p.body, p.simple), ("cd sqz_engine &&", " cargo test", true));
        let p = plan("cd \"my dir\"; cd sub && git log | head").unwrap();
        assert_eq!(p.prefix, "cd \"my dir\"; cd sub &&");
        assert!(!p.simple);
        let p = plan("cd /tmp\nls -la").unwrap();
        assert_eq!((p.prefix, p.body, p.simple), ("cd /tmp\n", "ls -la", true));
        let p = plan("builtin cd /tmp && ls").unwrap();
        assert_eq!(p.prefix, "builtin cd /tmp &&");
        skipped("cd a;", Skip::Empty);
        skipped("cd /tmp", Skip::StateChange);
        skipped("cd a || exit 1", Skip::StateChange);
    }

    #[test]
    fn state_changes_are_left_alone() {
        for cmd in [
            "export CI=1 && make",
            "make && cd build",
            "source .venv/bin/activate && pytest",
            ". ./env.sh; make",
            "X=1; echo $X",
            "set -e; make",
            "make || exit 1",
            "alias ll='ls -l'",
            "pushd src && make && popd",
            "trap 'echo bye' EXIT; make",
            "ulimit -n 4096 && make",
            "for d in a b; do cd $d; done",
            "kill %1",
        ] {
            skipped(cmd, Skip::StateChange);
        }
    }

    #[test]
    fn background_jobs_are_left_alone() {
        for cmd in ["npm run build &", "sleep 5 & wait", "(server &)", "cmd >/dev/null 2>&1 &", "echo $(sleep 1 &)"] {
            skipped(cmd, Skip::Background);
        }
    }

    #[test]
    fn long_running_commands_are_left_alone() {
        for cmd in [
            "tail -f app.log",
            "tail -n 100 -F app.log",
            "journalctl -fu nginx",
            "kubectl logs -f pod/web",
            "kubectl get pods -w",
            "docker compose logs --follow api",
            "docker compose up",
            "docker stats",
            "npm run dev",
            "pnpm dev",
            "yarn start",
            "vim notes.txt",
            "git log | less",
            "python3",
            "python3 -i run.py",
            "python3 -m http.server 8000",
            "python manage.py runserver",
            "watch -n 1 ls",
            "ping example.com",
            "tsc -w",
            "cargo test -- --watch",
            "uvicorn app:main --reload",
        ] {
            skipped(cmd, Skip::LongRunning);
        }
    }

    #[test]
    fn commands_already_through_sqz() {
        for cmd in [
            "git status 2>&1 | sqz compress",
            "cargo test | /usr/local/bin/sqz compress --cmd cargo",
            "sqz stats",
            "cd x && sqz gain",
            "\"C:\\Users\\me\\.cargo\\bin\\sqz.exe\" stats",
            "git log | 'C:\\tools\\sqz.EXE' compress",
            "/c/Users/me/.cargo/bin/sqz.Exe gain",
        ] {
            skipped(cmd, Skip::AlreadySqz);
        }
    }

    #[test]
    fn basenames_on_either_separator() {
        assert_eq!(basename("C:\\Users\\me\\sqz.exe"), "sqz");
        assert_eq!(basename("/usr/bin/sqz"), "sqz");
        assert_eq!(basename("tools\\Cargo.EXE"), "Cargo");
        assert_eq!(basename("exe"), "exe");
        assert_eq!(basename("naïve.exe"), "naïve");
        assert_eq!(basename("éabc"), "éabc");
    }

    #[test]
    fn unparsed_syntax_is_left_alone() {
        for cmd in [
            "echo 'unterminated",
            "echo \"unterminated",
            "echo $(unterminated",
            "echo `unterminated",
            "make &&",
            "make |",
            "; ls",
            "ls ;; ls",
            "case $x in a) echo a;; esac",
            "f() { echo hi; }",
            "{ echo a; echo b; }",
            "cat <<EOF\nno terminator",
            "cat <<EOF",
            "cat >",
            "echo trailing \\",
            "((i++))",
            "$CMD --flag",
            "arr=(a b c)",
            "ls )",
        ] {
            skipped(cmd, Skip::Unparsed);
        }
    }

    #[test]
    fn deep_nesting_is_left_alone() {
        let nest = |open: &str, close: &str, n: usize| {
            format!("echo {}x{}", open.repeat(n), close.repeat(n))
        };
        for (open, close) in [("$(", ")"), ("$(echo ", ")"), ("\"${a:-", "}\""), ("( ", " )")] {
            let deep = nest(open, close, 100_000);
            assert_eq!(plan(&deep), Err(Skip::Unparsed), "{open}");
            assert_eq!(flat(&deep), None, "{open}");
        }
        simple(&nest("$(echo ", ")", 3));
        simple(&nest("\"${a:-", "}\"", 3));
    }

    #[test]
    fn heredoc_bodies_are_not_scanned_as_commands() {
        // Operators, quotes and `cd` inside the body are data.
        let p = grouped("cat <<'EOF' > notes.md\ncd away && rm -rf / &\nit's \"fine\" | ok\nEOF");
        assert!(p.body.ends_with("EOF"));
        // The terminator must be alone on its line.
        skipped("cat <<EOF\n  EOF\n", Skip::Unparsed);
        grouped("cat <<-EOF\n\tbody\n\tEOF");
    }

    /// `(text up to each command's end, its label, merge_stderr)` for a flat list.
    fn flat_parts(cmd: &str) -> Vec<(&str, Option<String>, bool)> {
        let parts = flat(cmd).unwrap_or_else(|| panic!("{cmd:?} should be flat"));
        parts.into_iter().map(|p| (&cmd[..p.end], p.label, p.merge_stderr)).collect()
    }

    #[test]
    fn flat_lists_mark_the_commands_worth_piping() {
        assert_eq!(flat_parts("git status"), [("git status", Some("git status".into()), true)]);
        assert_eq!(
            flat_parts("cd sub && cargo test --release; ls -la"),
            [
                ("cd sub", None, true),
                ("cd sub && cargo test --release", Some("cargo test --release".into()), true),
                ("cd sub && cargo test --release; ls -la", Some("ls -la".into()), true),
            ]
        );
        // Builtins, assignments and quiet commands stay unpiped, in the shell.
        let parts = flat_parts("export CI=1\nsource env.sh && mkdir -p out && X=1 make");
        let labels: Vec<_> = parts.iter().map(|p| p.1.as_deref()).collect();
        assert_eq!(labels, [None, None, None, Some("make")]);
        // A trailing comment stays after the inserted pipe.
        assert_eq!(flat_parts("ls # list"), [("ls", Some("ls".into()), true)]);
    }

    #[test]
    fn flat_respects_the_commands_own_redirects() {
        // stdout already goes elsewhere: nothing to pipe.
        assert_eq!(flat("make > build.log 2>&1"), None);
        assert_eq!(flat("make &> build.log"), None);
        assert_eq!(flat("echo hi >&2"), None);
        // stderr already placed: pipe, but don't add a second `2>&1`.
        assert_eq!(flat_parts("cargo test 2>&1")[0].2, false);
        assert_eq!(flat_parts("cargo build 2>/dev/null")[0].2, false);
        assert_eq!(flat_parts("sort < in.txt")[0].2, true);
        let parts = flat_parts("make > log; make test");
        assert_eq!(parts[0].1, None);
        assert_eq!(parts[1].1.as_deref(), Some("make test"));
    }

    #[test]
    fn flat_labels_stop_at_quotes_and_expansions() {
        assert_eq!(flat_parts("git commit -m \"fix it\"")[0].1.as_deref(), Some("git commit -m"));
        assert_eq!(flat_parts("cargo test $ARGS")[0].1.as_deref(), Some("cargo test"));
        assert_eq!(flat_parts("RUST_LOG=debug cargo run --bin x")[0].1.as_deref(), Some("cargo run --bin x"));
        // With a brace anywhere, the label stays one word so the rewrite
        // never pairs a brace with a quote.
        assert_eq!(flat_parts("find . -exec grep foo {} \\;")[0].1.as_deref(), Some("find"));
        assert_eq!(flat_parts("ls ${DIR}")[0].1.as_deref(), Some("ls"));
        // Nothing plain to name the command by: leave it unpiped.
        assert_eq!(flat("\"$TOOL\"/bin/run"), None);
    }

    #[test]
    fn flat_refuses_what_needs_grouping() {
        for cmd in [
            "git log | head -20",
            "make |& tee log",
            "(cd sub && make)",
            "for f in *.rs; do wc -l \"$f\"; done",
            "if [ -f x ]; then cat x; fi",
            "time cargo build",
            "! grep -q x y",
            "cat <<EOF\nhi\nEOF",
            "make &",
            "git status 2>&1 | sqz compress",
            "cd x && sqz stats",
            "echo 'unterminated",
            // Only builtins, quiet or long-running commands: nothing to pipe.
            "cd sub",
            "export A=1 && cd b",
            "mkdir -p x && touch x/y",
            "tail -f app.log",
            "cd web && npm run dev",
        ] {
            assert_eq!(flat(cmd), None, "{cmd:?}");
        }
    }

    #[test]
    fn assignment_detection() {
        assert!(is_assignment("FOO=1"));
        assert!(is_assignment("_x+=a"));
        assert!(!is_assignment("1X=2"));
        assert!(!is_assignment("--flag=1"));
        assert!(!is_assignment("\"A\"=b"));
    }
}
