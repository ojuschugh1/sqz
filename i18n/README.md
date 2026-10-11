# Translations

This folder holds the translations of sqz's README and contributing guide, a glossary per language, and the checker that keeps every translation in step with the English text.

## What is translated

Only `README.md` and `CONTRIBUTING.md`. They are the two pages a new user or contributor reads first, and they change slowly enough for translators to keep up.

Everything else stays in English: the docs site under `docs/`, the changelog, `CLA.md`, `LICENSE-APACHE`, `LICENSE-MIT`, the npm README, sqz's own output, code, comments and commit messages. These change with every release, or they are the text that applies, and a translation that falls behind would mislead readers.

The English files are the reference. Where a translation and the English text disagree, the English text applies.

## Languages

| Language | README | Contributing guide | Glossary |
|---|---|---|---|
| Simplified Chinese (简体中文) | `README.zh-CN.md` | `CONTRIBUTING.zh-CN.md` | [zh-CN](glossary/zh-CN.md) |
| Japanese (日本語) | `README.ja.md` | `CONTRIBUTING.ja.md` | [ja](glossary/ja.md) |
| Korean (한국어) | `README.ko.md` | `CONTRIBUTING.ko.md` | [ko](glossary/ko.md) |
| Spanish (Español) | `README.es.md` | `CONTRIBUTING.es.md` | [es](glossary/es.md) |
| Brazilian Portuguese (Português (Brasil)) | `README.pt-BR.md` | `CONTRIBUTING.pt-BR.md` | [pt-BR](glossary/pt-BR.md) |

## Rules

Translators and reviewers both follow these. The checker enforces the parts a script can see.

One to one:

- Keep the same blocks in the same order. Translate every sentence. Add nothing, drop nothing, and leave out translator notes.
- Tables keep all their rows and lists keep all their items.

Never change:

- Fenced code blocks, including the comments and output inside them.
- Inline code.
- Commands, flags, file names and paths.
- Environment variables, config keys, JSON and URLs.
- Link targets. Relative links are the exception: adjust them to the translation's folder, so `docs/x.md` becomes `../docs/x.md`.
- Product and client names: sqz, sqz-mcp, Claude Code, Cursor, Windsurf, Cline, Gemini CLI, Kiro, OpenCode, Codex, Zed, Copilot CLI, VS Code, JetBrains, MCP and the others in the text.
- Numbers, exactly as written: 24.7%, 12,676, 1.85. No localised digit grouping or decimal separators.

In-page links such as `#install` keep the English fragment. End the translated heading they point to with `<a id="install"></a>` so the link still lands there.

Translate:

- Headings, link text, image alt text, and the prose in table cells.
- Terms that appear in sqz's output follow the glossary for your language, which keeps the English word where readers need to match what sqz prints.

Register:

- Simplified Chinese: technical documentation style, with a half-width space between Chinese characters and Latin letters or digits.
- Japanese: です・ます調.
- Korean: 합니다체.
- Spanish: neutral international Spanish, addressing the reader as tú.
- Brazilian Portuguese: address the reader as você.

Punctuation:

- Outside code, never use an em dash (U+2014), an en dash (U+2013) or a horizontal bar (U+2015). Rephrase with a comma, a colon or parentheses. The English text uses em dashes in places; they are not carried over.
- The katakana prolonged sound mark ー (U+30FC) is a letter, not a dash. Never remove or replace it.
- No emoji outside code. Code blocks keep theirs, such as the `sqz stats` output.

## File layout

A translation is `i18n/<NAME>.<lang>.md`, where `<NAME>.md` is the English file and `<lang>` is a BCP 47 tag. It starts like this:

```
<!-- i18n-source: README.md sha256:<hash> -->
<!-- i18n:languages -->
[English](../README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md) | [Español](README.es.md) | [Português (Brasil)](README.pt-BR.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> One sentence, in the translation's language, saying that this is a translation of the English README.md, with a link to it.
<!-- /i18n:note -->
```

Line 1 is the stamp: the hash of the English text the translation matches. The checker writes it; never edit it by hand. The English files carry the same languages block with links into `i18n/`. The checker leaves both blocks out of every comparison and out of the hash, so editing a switcher never makes a translation stale.

## Updating a translation

1. Run `python3 i18n/check.py`. A stale translation is listed with the English commit it was translated from, `git diff --stat` since that commit, and the headings of the sections that changed.
2. Read the English changes with `git diff <commit> HEAD -- README.md`.
3. Translate the changed parts and nothing else.
4. Run `python3 i18n/check.py --stamp i18n/README.ja.md` (with your file) to record that it now matches the current English text.
5. Run `python3 i18n/check.py` again. It must report no problems.
6. Open a pull request.

A stale translation never fails CI, so English changes can be merged without waiting for translators. A translation that does not match the English structure does fail CI.

## Adding a language

1. Open an issue labelled `translation` first, naming the language and who can review it, so two people do not start the same work.
2. Add `i18n/README.<lang>.md`, `i18n/CONTRIBUTING.<lang>.md` and `i18n/glossary/<lang>.md`.
3. Add the language to every languages block: both English files, `npm/README.md` and every existing translation.
4. Add a row to the table above.
5. Run `python3 i18n/check.py --stamp` on the two new files, then `python3 i18n/check.py`.

## Review by native speakers

Every translation pull request is reviewed by a native speaker of the language who is not its author. The reviewer reads it side by side with the English text and checks that the meaning is the same, that the rules and the glossary are followed, and that it reads naturally. Use GitHub's suggestion feature for wording changes so the author can apply them directly.

The checker covers structure. Meaning is the reviewer's job. To review translations for your language, comment on an issue labelled `translation`.

## The checker

`check.py` needs Python 3.8 or later and nothing else. For each translation it finds the English version named by the stamp and compares the two: block structure, fenced code blocks, inline code, link and image targets (and that relative ones exist), and the numbers in the prose. `python3 i18n/check.py --help` lists the options. Its tests run with `python3 -m unittest discover -s i18n -p 'test_*.py'`, and CI runs both on every pull request that touches these files.
