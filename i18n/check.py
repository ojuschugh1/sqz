import argparse
import collections
import difflib
import hashlib
import os
import posixpath
import re
import subprocess
import sys
import urllib.parse

SOURCES = ("README.md", "CONTRIBUTING.md")
FOLDER = "i18n"
TRANSLATION_RE = re.compile(r"^(README|CONTRIBUTING)\.([A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*)\.md$")
STAMP_RE = re.compile(r"^<!-- i18n-source: (\S+) sha256:([0-9a-f]{64}) -->$")
MARKERS = {
    "languages": ("<!-- i18n:languages -->", "<!-- /i18n:languages -->"),
    "note": ("<!-- i18n:note -->", "<!-- /i18n:note -->"),
}
FENCE_RE = re.compile(r"^( *)(`{3,}|~{3,})(.*)$")
HEADING_RE = re.compile(r"^ {0,3}(#{1,6})(?:[ \t]|$)")
LIST_RE = re.compile(r"^( *)(?:[-*+]|\d{1,9}[.)])(?:[ \t]|$)")
RULE_RE = re.compile(r"^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$")
TABLE_SEP_RE = re.compile(r"^\s*\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*)*\|?\s*$")
HTML_RE = re.compile(r"^ {0,3}<[A-Za-z/!]")
NUMBER_RE = re.compile(r"(?<![A-Za-z0-9_.,])\d+(?:[.,]\d+)*")
SCHEME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
TICKS_RE = re.compile(r"`+")
CODE_TAG_RE = re.compile(r"<code>(.*?)</code>", re.S)
AUTOLINK_RE = re.compile(r"<((?:https?|ftp|mailto):[^\s<>]+)>")
ATTR_RE = re.compile(r"\b(?:href|src)\s*=\s*(?:\"([^\"]*)\"|'([^']*)')", re.I)
MD_LINK_RE = re.compile(
    r"\]\(\s*(<[^>\n]*>|(?:[^\s()]|\([^\s()]*\))*)(?:\s+(?:\"[^\"]*\"|'[^']*'|\([^)]*\)))?\s*\)"
)
REF_DEF_RE = re.compile(r"^ {0,3}\[[^\]]+\]:\s*(\S+)", re.M)
BARE_URL_RE = re.compile(r"https?://[^\s<>()\[\]\"'`]+")
TAG_RE = re.compile(r"<[^>]*>")

USAGE = """\
Check the translations in i18n/ against the English README.md and CONTRIBUTING.md.

A translation is i18n/<NAME>.<lang>.md, where <NAME>.md is one of the English
sources at the repository root and <lang> is a BCP 47 tag (ja, zh-CN, pt-BR).
Line 1 of every translation is its stamp:

  <!-- i18n-source: README.md sha256:<hash of the English text it translates> -->

With no FILE, every translation is checked, and so is the language switcher
(<!-- i18n:languages --> ... <!-- /i18n:languages -->) in every source that has
translations and in every translation. With FILE arguments only those
translations are checked and switchers are skipped.

The checker finds the English version a translation was made from by its stamp
and compares the two: block structure, fenced code blocks, inline code, link
and image targets, and the numbers in the prose. A translation whose stamp is
not the current English text is stale: that is a warning, with the English
changes since then, unless --strict is given.
"""

EPILOG = """\
examples:
  python3 i18n/check.py
  python3 i18n/check.py i18n/README.ja.md
  python3 i18n/check.py --stamp i18n/README.ja.md i18n/CONTRIBUTING.ja.md

exit status:
  0  no problems (warnings allowed)
  1  structure problems, or stale translations with --strict
  2  usage error
"""


class UsageError(Exception):
    pass


def numbered(text):
    return [(i + 1, t) for i, t in enumerate(text.replace("\r\n", "\n").split("\n"))]


def fence_open(text):
    m = FENCE_RE.match(text)
    if not m:
        return None
    if m.group(2)[0] == "`" and "`" in m.group(3):
        return None
    return m.group(2)[0], len(m.group(2)), len(m.group(1))


def fence_closes(text, opener):
    s = text.strip()
    return len(s) >= opener[1] and s == opener[0] * len(s)


def strip_marked(lines, names):
    out = []
    problems = []
    opener = None
    i = 0
    while i < len(lines):
        n, t = lines[i]
        if opener is not None:
            if fence_closes(t, opener):
                opener = None
            out.append((n, t))
            i += 1
            continue
        name = None
        for key in names:
            if t.strip() == MARKERS[key][0]:
                name = key
        if name is None:
            opener = fence_open(t)
            out.append((n, t))
            i += 1
            continue
        end = None
        for j in range(i + 1, len(lines)):
            if lines[j][1].strip() == MARKERS[name][1]:
                end = j
                break
        if end is None:
            problems.append((n, "%s has no closing %s" % (MARKERS[name][0], MARKERS[name][1])))
            out.extend(lines[i:])
            break
        i = end + 1
        if not out or not out[-1][1].strip():
            while i < len(lines) and not lines[i][1].strip():
                i += 1
    return out, problems


def source_hash(text):
    kept, _ = strip_marked(numbered(text), ("languages",))
    return hashlib.sha256("\n".join(t for _, t in kept).encode("utf-8")).hexdigest()


def first_kind(text):
    if RULE_RE.match(text):
        return "rule"
    if LIST_RE.match(text):
        return "list"
    if text.lstrip().startswith(">"):
        return "quote"
    if text.lstrip().startswith("|"):
        return "table"
    if HTML_RE.match(text):
        return "html"
    return "paragraph"


def classify(block):
    prose = [t for seg in block["segments"] for _, t in seg]
    first = prose[0]
    if len(prose) == 1 and RULE_RE.match(first):
        return ("rule",)
    if LIST_RE.match(first):
        indents = [len(LIST_RE.match(t).group(1)) for t in prose if LIST_RE.match(t)]
        levels = sorted(set(indents))
        return ("list", tuple(levels.index(x) for x in indents))
    if first.lstrip().startswith(">"):
        return ("quote",)
    if len(prose) >= 2 and first.lstrip().startswith("|") and TABLE_SEP_RE.match(prose[1]):
        return ("table", sum(1 for t in prose if t.lstrip().startswith("|")))
    if HTML_RE.match(first):
        return ("html",)
    return ("paragraph",)


def describe(kind):
    if kind[0] == "heading":
        return "heading level %d" % kind[1]
    if kind[0] == "list":
        return "list with %d items at indent levels %s" % (len(kind[1]), ",".join(str(x) for x in kind[1]))
    if kind[0] == "table":
        return "table with %d rows" % kind[1]
    if kind[0] == "fence":
        return "fenced code block"
    if kind[0] == "html":
        return "HTML block"
    return kind[0]


def parse(lines):
    blocks = []
    fences = []
    cur = None
    i = 0
    while i < len(lines):
        n, t = lines[i]
        opener = fence_open(t)
        if opener is not None:
            j = i + 1
            while j < len(lines) and not fence_closes(lines[j][1], opener):
                j += 1
            fences.append((n, "\n".join(x for _, x in lines[i:j + 1])))
            if cur is not None and opener[2] > 0:
                cur["segments"].append([])
            else:
                blocks.append({"line": n, "kind": ("fence",), "segments": []})
                cur = None
            i = j + 1
            continue
        if not t.strip():
            cur = None
        elif HEADING_RE.match(t):
            level = len(HEADING_RE.match(t).group(1))
            blocks.append({"line": n, "kind": ("heading", level), "segments": [[(n, t)]]})
            cur = None
        else:
            if cur is not None and cur["first"] == "paragraph" and LIST_RE.match(t) and len(LIST_RE.match(t).group(1)) <= 3:
                cur = None
            if cur is None:
                cur = {"line": n, "first": first_kind(t), "segments": [[]]}
                blocks.append(cur)
            cur["segments"][-1].append((n, t))
        i += 1
    for block in blocks:
        if "kind" not in block:
            block["kind"] = classify(block)
    return blocks, fences


def tidy_span(content):
    content = re.sub(r"[ \t]*\n[ \t]*", " ", content)
    if len(content) > 1 and content.startswith(" ") and content.endswith(" ") and content.strip():
        content = content[1:-1]
    return content


def code_spans(text):
    spans = []
    rest = []
    pos = 0
    i = 0
    while True:
        m = TICKS_RE.search(text, i)
        if not m:
            break
        close = re.compile(r"(?<!`)" + m.group(0) + r"(?!`)").search(text, m.end())
        if not close:
            i = m.end()
            continue
        spans.append(tidy_span(text[m.end():close.start()]))
        rest.append(text[pos:m.start()])
        pos = close.end()
        i = close.end()
    rest.append(text[pos:])
    return spans, " ".join(rest)


def split_prose(text):
    spans, rest = code_spans(text)
    targets = []

    def code_tag(m):
        spans.append(tidy_span(m.group(1)))
        return " "

    def grab(value):
        value = value.strip()
        if value.startswith("<") and value.endswith(">"):
            value = value[1:-1].strip()
        if value:
            targets.append(value)
        return " "

    def bare(m):
        value = m.group(0).rstrip(".,;:!?")
        grab(value)
        return " " + m.group(0)[len(value):]

    rest = CODE_TAG_RE.sub(code_tag, rest)
    rest = AUTOLINK_RE.sub(lambda m: grab(m.group(1)), rest)
    rest = ATTR_RE.sub(lambda m: grab(m.group(1) if m.group(1) is not None else m.group(2)), rest)
    rest = MD_LINK_RE.sub(lambda m: "]" + grab(m.group(1)), rest)
    rest = REF_DEF_RE.sub(lambda m: grab(m.group(1)), rest)
    rest = BARE_URL_RE.sub(bare, rest)
    return spans, targets, TAG_RE.sub(" ", rest)


def resolve(target, folder):
    if SCHEME_RE.match(target) or target.startswith("//"):
        return target, None
    path, sep, fragment = target.partition("#")
    if not path:
        return target, None
    path_only = path.split("?", 1)[0]
    query = path[len(path_only):]
    if path_only.startswith("/"):
        resolved = posixpath.normpath(path_only.lstrip("/"))
    else:
        resolved = posixpath.normpath(posixpath.join(folder, path_only))
    return resolved + query + sep + fragment, resolved


def analyse(lines, folder):
    blocks, fences = parse(lines)
    spans = []
    links = []
    numbers = []
    for block in blocks:
        for seg in block["segments"]:
            if not seg:
                continue
            line = seg[0][0]
            found, targets, prose = split_prose("\n".join(t for _, t in seg))
            spans.extend((s, line) for s in found)
            for target in targets:
                key, resolved = resolve(target, folder)
                links.append((key, resolved, target, line))
            numbers.extend((x, line) for x in NUMBER_RE.findall(prose))
    return {"blocks": blocks, "fences": fences, "spans": spans, "links": links, "numbers": numbers}


def check_blocks(src, tr, src_name):
    out = []
    a = [b["kind"] for b in src["blocks"]]
    b = [x["kind"] for x in tr["blocks"]]
    last = tr["blocks"][-1]["line"] if tr["blocks"] else 1

    def tr_line(j):
        return tr["blocks"][j]["line"] if j < len(tr["blocks"]) else last

    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        for k in range(max(i2 - i1, j2 - j1)):
            i = i1 + k
            j = j1 + k
            if i < i2 and j < j2:
                out.append(("error", tr_line(j), "%s where %s line %d has a %s" % (
                    describe(b[j]), src_name, src["blocks"][i]["line"], describe(a[i]))))
            elif i < i2:
                out.append(("error", tr_line(j1), "missing %s (%s line %d)" % (
                    describe(a[i]), src_name, src["blocks"][i]["line"])))
            else:
                out.append(("error", tr_line(j), "extra %s, not in %s" % (describe(b[j]), src_name)))
    return out


def check_fences(src, tr, src_name):
    out = []
    a = [text for _, text in src["fences"]]
    b = [text for _, text in tr["fences"]]
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        for k in range(max(i2 - i1, j2 - j1)):
            i = i1 + k
            j = j1 + k
            if i < i2 and j < j2:
                out.append(("error", tr["fences"][j][0], "code block differs from the one at %s line %d" % (
                    src_name, src["fences"][i][0])))
            elif i < i2:
                line = tr["fences"][j1][0] if j1 < len(b) else (tr["fences"][-1][0] if b else 1)
                out.append(("error", line, "code block from %s line %d is missing or out of order" % (
                    src_name, src["fences"][i][0])))
            else:
                out.append(("error", tr["fences"][j][0], "code block is not in %s" % src_name))
    return out


def compare_counts(src_items, tr_items, what, src_name):
    out = []
    a = collections.Counter(x for x, _ in src_items)
    b = collections.Counter(x for x, _ in tr_items)
    first_src = {}
    for x, line in src_items:
        first_src.setdefault(x, line)
    first_tr = {}
    for x, line in tr_items:
        first_tr.setdefault(x, line)
    for x in a:
        if b[x] < a[x]:
            out.append(("error", first_tr.get(x, 1), "%s %s appears %d times, %s has it %d times (first at line %d)" % (
                what, x, b[x], src_name, a[x], first_src[x])))
    for x in b:
        if b[x] > a[x]:
            out.append(("error", first_tr[x], "%s %s appears %d times, %s has it %d times" % (what, x, b[x], src_name, a[x])))
    return out


def check_inline_code(src, tr, src_name):
    return compare_counts(
        [("`%s`" % s, line) for s, line in src["spans"]],
        [("`%s`" % s, line) for s, line in tr["spans"]],
        "inline code", src_name)


def check_links(src, tr, src_name):
    return compare_counts(
        [(key, line) for key, _, _, line in src["links"]],
        [(key, line) for key, _, _, line in tr["links"]],
        "link target", src_name)


def check_link_files(tr, root, level):
    out = []
    for key, resolved, target, line in tr["links"]:
        if resolved is None:
            continue
        path = urllib.parse.unquote(resolved)
        if path == ".." or path.startswith("../") or not os.path.exists(os.path.join(root, path)):
            out.append((level, line, "link target %s does not exist (resolves to %s)" % (target, resolved)))
    return out


def check_numbers(src, tr, src_name):
    out = []
    a = collections.Counter(x for x, _ in src["numbers"])
    b = collections.Counter(x for x, _ in tr["numbers"])
    first_src = {}
    for x, line in src["numbers"]:
        first_src.setdefault(x, line)
    first_tr = {}
    for x, line in tr["numbers"]:
        first_tr.setdefault(x, line)
    for x in a:
        if b[x] < a[x]:
            out.append(("error", first_tr.get(x, 1), "number %s appears %d times, %s has it %d times (first at line %d)" % (
                x, b[x], src_name, a[x], first_src[x])))
    for x in b:
        if b[x] > a[x]:
            out.append(("warning", first_tr[x], "number %s appears %d times, %s has it %d times" % (x, b[x], src_name, a[x])))
    return out


def structure_checks(src, tr, src_name, root, stale):
    out = []
    out.extend(check_blocks(src, tr, src_name))
    out.extend(check_fences(src, tr, src_name))
    out.extend(check_inline_code(src, tr, src_name))
    out.extend(check_links(src, tr, src_name))
    out.extend(check_link_files(tr, root, "warning" if stale else "error"))
    out.extend(check_numbers(src, tr, src_name))
    return out


class Repo:
    def __init__(self, root):
        self.root = root
        self.versions = {}

    def git(self, *args):
        result = subprocess.run(["git"] + list(args), cwd=self.root, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return result.returncode, result.stdout.decode("utf-8", "replace")

    def read(self, rel):
        with open(os.path.join(self.root, rel), "rb") as f:
            return f.read().decode("utf-8", "replace")

    def find_version(self, name, digest):
        if name not in self.versions:
            code, out = self.git("log", "--format=%H", "--", name)
            self.versions[name] = {"commits": out.split() if code == 0 else [], "done": []}
        entry = self.versions[name]
        for commit, h, text in entry["done"]:
            if h == digest:
                return commit, text
        while len(entry["done"]) < len(entry["commits"]):
            commit = entry["commits"][len(entry["done"])]
            code, text = self.git("show", "%s:%s" % (commit, name))
            h = source_hash(text) if code == 0 else None
            entry["done"].append((commit, h, text))
            if h == digest:
                return commit, text
        return None


def heading_for(blocks, line):
    title = None
    for block in blocks:
        if block["kind"][0] != "heading":
            continue
        if block["line"] > line:
            break
        title = block["segments"][0][0][1].strip()
    return title or "(before the first heading)"


def changed_sections(old_lines, new_lines):
    blocks, _ = parse(new_lines)
    old = [t for _, t in old_lines]
    new = [t for _, t in new_lines]
    titles = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, old, new, autojunk=False).get_opcodes():
        if tag == "equal" or not new_lines:
            continue
        if j1 == j2:
            positions = [max(j1 - 1, 0)]
        else:
            positions = range(j1, j2)
        for p in positions:
            title = heading_for(blocks, new_lines[min(p, len(new_lines) - 1)][0])
            if title not in titles:
                titles.append(title)
    return titles


def translation_parts(rel):
    folder, base = posixpath.split(rel)
    if folder != FOLDER:
        return None
    m = TRANSLATION_RE.match(base)
    if not m:
        return None
    return m.group(1) + ".md", m.group(2)


def list_translations(root):
    found = []
    folder = os.path.join(root, FOLDER)
    if not os.path.isdir(folder):
        return found
    for base in sorted(os.listdir(folder)):
        rel = posixpath.join(FOLDER, base)
        parts = translation_parts(rel)
        if parts and os.path.isfile(os.path.join(root, parts[0])) and os.path.isfile(os.path.join(root, rel)):
            found.append(rel)
    return found


def check_translation(repo, rel):
    findings = []
    name, _ = translation_parts(rel)
    lines = numbered(repo.read(rel))
    current = repo.read(name)
    current_hash = source_hash(current)
    m = STAMP_RE.match(lines[0][1]) if lines else None
    base = current
    stale = None
    if m is None or m.group(1) != name:
        findings.append((rel, "error", 1, "line 1 must be <!-- i18n-source: %s sha256:<hash> -->, run check.py --stamp %s" % (name, rel)))
    elif m.group(2) != current_hash:
        found = repo.find_version(name, m.group(2))
        if found:
            commit, base = found
            stale = commit
        else:
            findings.append((rel, "warning", 1, "the stamp matches no committed version of %s, comparing against the current %s" % (name, name)))
            stale = ""
    body = lines[1:] if lines and lines[0][1].startswith("<!-- i18n-source:") else lines
    tr_lines, problems = strip_marked(body, ("languages", "note"))
    for line, msg in problems:
        findings.append((rel, "error", line, msg))
    src_lines, problems = strip_marked(numbered(base), ("languages",))
    for line, msg in problems:
        findings.append((name, "error", line, msg))
    src = analyse(src_lines, "")
    tr = analyse(tr_lines, FOLDER)
    for level, line, msg in structure_checks(src, tr, name, repo.root, stale is not None):
        findings.append((rel, level, line, msg))
    if stale is not None:
        findings.append((rel, "stale", 1, stale_message(repo, name, stale, src_lines, current)))
    return findings


def stale_message(repo, name, commit, old_lines, current):
    new_lines, _ = strip_marked(numbered(current), ("languages",))
    details = []
    if commit:
        details.append("stale: translated from %s at commit %s, the English text has changed since" % (name, commit[:12]))
        code, out = repo.git("diff", "--stat", commit, "HEAD", "--", name)
        details.extend("  " + x for x in out.rstrip("\n").split("\n") if x.strip())
    else:
        details.append("stale: the stamp is not the current hash of %s" % name)
    details.append("  changed sections: " + ", ".join(changed_sections(old_lines, new_lines)))
    return "\n".join(details)


def switcher_targets(repo, rel):
    lines = numbered(repo.read(rel))
    start = None
    for i, (n, t) in enumerate(lines):
        if start is None and t.strip() == MARKERS["languages"][0]:
            start = i
        elif start is not None and t.strip() == MARKERS["languages"][1]:
            block = lines[start + 1:i]
            targets = []
            for seg_line, text in block:
                _, found, _ = split_prose(text)
                targets.extend((x, seg_line) for x in found)
            return lines[start][0], targets
    return None, None


def check_switchers(repo, translations):
    findings = []
    by_source = collections.OrderedDict()
    for rel in translations:
        name, lang = translation_parts(rel)
        by_source.setdefault(name, []).append((rel, lang))
    for name, items in by_source.items():
        langs = {"en": name}
        for rel, lang in items:
            langs[lang] = rel
        for rel, own in [(name, "en")] + items:
            line, targets = switcher_targets(repo, rel)
            if line is None:
                findings.append((rel, "error", 1, "no %s ... %s block listing the languages of %s" % (
                    MARKERS["languages"][0], MARKERS["languages"][1], name)))
                continue
            folder = posixpath.dirname(rel)
            listed = set()
            for target, tline in targets:
                key, resolved = resolve(target, folder)
                if resolved is None:
                    continue
                if not os.path.isfile(os.path.join(repo.root, urllib.parse.unquote(resolved))):
                    findings.append((rel, "error", tline, "languages link %s does not exist (resolves to %s)" % (target, resolved)))
                    continue
                if resolved == name:
                    listed.add("en")
                parts = translation_parts(resolved)
                if parts and parts[0] == name:
                    listed.add(parts[1])
            for lang in sorted(langs):
                if lang != own and lang not in listed:
                    findings.append((rel, "error", line, "languages block does not link %s (%s)" % (lang, langs[lang])))
    return findings


def git_root():
    try:
        result = subprocess.run(["git", "rev-parse", "--show-toplevel"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    except OSError:
        raise UsageError("git is not available")
    if result.returncode != 0:
        raise UsageError("not inside a git repository")
    return os.path.realpath(result.stdout.decode("utf-8").strip())


def relative_translation(root, path):
    if not os.path.isfile(path):
        raise UsageError("%s: no such file" % path)
    rel = os.path.relpath(os.path.realpath(path), root).replace(os.sep, "/")
    parts = translation_parts(rel)
    if parts is None or not os.path.isfile(os.path.join(root, parts[0])):
        raise UsageError("%s is not a translation (expected %s/<NAME>.<lang>.md for %s)" % (path, FOLDER, " or ".join(SOURCES)))
    return rel


def stamp(repo, rel):
    name, _ = translation_parts(rel)
    line = "<!-- i18n-source: %s sha256:%s -->" % (name, source_hash(repo.read(name)))
    path = os.path.join(repo.root, rel)
    with open(path, "rb") as f:
        text = f.read().decode("utf-8")
    newline = "\r\n" if "\r\n" in text else "\n"
    first, sep, rest = text.partition("\n")
    if first.rstrip("\r").startswith("<!-- i18n-source:"):
        text = line + newline + rest if sep else line + newline
    else:
        text = line + newline + text
    with open(path, "wb") as f:
        f.write(text.encode("utf-8"))
    print("%s: stamped %s" % (rel, line))


def escape_data(text):
    return text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def escape_property(text):
    return escape_data(text).replace(":", "%3A").replace(",", "%2C")


def report(findings, github):
    for path, level, line, msg in findings:
        shown = "warning" if level == "stale" else level
        head, _, tail = msg.partition("\n")
        print("%s:%d: %s: %s" % (path, line, shown, head))
        for extra in tail.split("\n") if tail else []:
            print("    " + extra)
        if github:
            print("::%s file=%s,line=%d::%s" % (shown, escape_property(path), line, escape_data(msg)))


def summary(translations, findings):
    rows = ["## Translations", ""]
    if not translations:
        rows.append("No translations found.")
        return "\n".join(rows) + "\n"
    rows.extend(["| File | Source | Status |", "|---|---|---|"])
    for rel in translations:
        mine = [f for f in findings if f[0] == rel]
        errors = sum(1 for f in mine if f[1] == "error")
        stale = any(f[1] == "stale" for f in mine)
        status = []
        if errors:
            status.append("%d problems" % errors)
        if stale:
            status.append("stale")
        rows.append("| `%s` | `%s` | %s |" % (rel, translation_parts(rel)[0], ", ".join(status) or "ok"))
    others = sum(1 for f in findings if f[0] not in translations and f[1] == "error")
    if others:
        rows.extend(["", "%d problems in the English sources." % others])
    return "\n".join(rows) + "\n"


def run(args):
    root = git_root()
    repo = Repo(root)
    if args.stamp:
        rels = [relative_translation(root, f) for f in args.files]
        for rel in rels:
            stamp(repo, rel)
        return 0
    if args.files:
        translations = [relative_translation(root, f) for f in args.files]
    else:
        translations = list_translations(root)
    findings = []
    for rel in translations:
        findings.extend(check_translation(repo, rel))
    if not args.files:
        findings.extend(check_switchers(repo, translations))
    report(findings, args.github)
    errors = sum(1 for f in findings if f[1] == "error")
    stale = sum(1 for f in findings if f[1] == "stale")
    warnings = sum(1 for f in findings if f[1] == "warning")
    print("checked %d translations: %d problems, %d stale, %d warnings" % (len(translations), errors, stale, warnings))
    if args.github and os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write(summary(translations, findings))
    if errors or (args.strict and stale):
        return 1
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="check.py",
        description=USAGE,
        epilog=EPILOG,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("files", nargs="*", metavar="FILE", help="translations to check (default: all of them)")
    parser.add_argument("--stamp", action="store_true", help="write the current English source hash into line 1 of each FILE")
    parser.add_argument("--strict", action="store_true", help="exit 1 when a translation is stale")
    parser.add_argument("--github", action="store_true", help="also print GitHub Actions annotations and write $GITHUB_STEP_SUMMARY")
    args = parser.parse_args(argv)
    if args.stamp and not args.files:
        parser.error("--stamp needs at least one FILE")
    try:
        return run(args)
    except UsageError as e:
        print("check.py: error: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
