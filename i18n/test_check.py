import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

CHECK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "check.py")

SOURCE_SWITCHER = """<!-- i18n:languages -->
[English](README.md) | [日本語](i18n/README.ja.md)
<!-- /i18n:languages -->

"""

SOURCE_BODY = """# Demo

Demo compresses tool output before it reaches the agent.

## Install

Run the installer, it takes 42 seconds on a laptop.

```sh
cargo install demo
```

Then run `demo init` in your project. See the [guide](docs/guide.md).

- first item
- second item
  - nested item

| Name | Value |
|---|---|
| ratio | 24.7% |

## Usage

Pipe output through `demo compress`.

A plain paragraph with nothing special in it.
"""

TRANSLATION_HEAD = """<!-- i18n:languages -->
[English](../README.md) | [日本語](README.ja.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> この文書は英語版 README.md の翻訳です。
<!-- /i18n:note -->

"""

TRANSLATION_BODY = """# Demo

Demo は、ツールの出力がエージェントに届く前に圧縮します。

## インストール

インストーラーを実行します。ノートパソコンで約42秒かかります。

```sh
cargo install demo
```

プロジェクトで `demo init` を実行します。[ガイド](../docs/guide.md)を参照してください。

- 最初の項目
- 二番目の項目
  - 入れ子の項目

| 名前 | 値 |
|---|---|
| 比率 | 24.7% |

## 使い方

`demo compress` に出力をパイプします。

特別なものを含まない普通の段落です。
"""

CONTRIBUTING = """# Contributing

Run `cargo test` before you open a pull request.
"""


def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


class CheckTest(unittest.TestCase):
    def setUp(self):
        self.dir = os.path.realpath(tempfile.mkdtemp(prefix="i18n-check-"))
        self.addCleanup(shutil.rmtree, self.dir)
        self.env = dict(os.environ)
        self.env.pop("GITHUB_STEP_SUMMARY", None)
        self.env.update({
            "GIT_AUTHOR_NAME": "Test",
            "GIT_AUTHOR_EMAIL": "test@example.com",
            "GIT_COMMITTER_NAME": "Test",
            "GIT_COMMITTER_EMAIL": "test@example.com",
        })
        self.git("init", "-q")
        self.write("README.md", SOURCE_SWITCHER + SOURCE_BODY)
        self.write("CONTRIBUTING.md", CONTRIBUTING)
        self.write("docs/guide.md", "# Guide\n")
        self.write("docs/other.md", "# Other\n")
        self.write("i18n/README.ja.md", self.stamped(TRANSLATION_HEAD + TRANSLATION_BODY))
        self.commit("first")

    def git(self, *args):
        result = subprocess.run(
            ["git", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"] + list(args),
            cwd=self.dir, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.assertEqual(result.returncode, 0, result.stdout.decode())
        return result.stdout.decode()

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD").strip()

    def write(self, rel, text):
        path = os.path.join(self.dir, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8", newline="") as f:
            f.write(text)

    def read(self, rel):
        with open(os.path.join(self.dir, rel), encoding="utf-8", newline="") as f:
            return f.read()

    def stamped(self, text, body=SOURCE_BODY, name="README.md"):
        return "<!-- i18n-source: %s sha256:%s -->\n%s" % (name, sha(body), text)

    def run_check(self, *args, env=None):
        result = subprocess.run(
            [sys.executable, CHECK] + list(args),
            cwd=self.dir, env=env or self.env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        return result.returncode, result.stdout.decode()

    def translate(self, old, new):
        body = TRANSLATION_BODY.replace(old, new)
        self.assertNotEqual(body, TRANSLATION_BODY)
        self.write("i18n/README.ja.md", self.stamped(TRANSLATION_HEAD + body))

    def assertProblem(self, text, *args):
        code, out = self.run_check(*args)
        self.assertEqual(code, 1, out)
        self.assertIn(text, out)
        return out

    def assertClean(self, *args):
        code, out = self.run_check(*args)
        self.assertEqual(code, 0, out)
        self.assertNotIn(": error:", out)
        return out

    def test_clean_translation_passes(self):
        out = self.assertClean()
        self.assertIn("checked 1 translations: 0 problems, 0 stale, 0 warnings", out)

    def test_rewrapped_paragraph_passes(self):
        self.translate("ノートパソコンで約42秒かかります。", "ノートパソコンで\n約42秒かかります。")
        self.assertClean()

    def test_dropped_paragraph_fails(self):
        self.translate("\n特別なものを含まない普通の段落です。\n", "")
        self.assertProblem("missing paragraph (README.md line 31)")

    def test_dropped_list_item_fails(self):
        self.translate("- 二番目の項目\n", "")
        self.assertProblem("list with 2 items at indent levels 0,1 where README.md line 19 has a list with 3 items")

    def test_dropped_table_row_fails(self):
        self.translate("| 比率 | 24.7% |\n", "")
        self.assertProblem("table with 2 rows where README.md line 23 has a table with 3 rows")

    def test_heading_level_fails(self):
        self.translate("## 使い方", "### 使い方")
        self.assertProblem("heading level 3 where README.md line 27 has a heading level 2")

    def test_edited_code_block_fails(self):
        self.translate("cargo install demo\n", "cargo install  demo\n")
        self.assertProblem("code block differs from the one at README.md line 13")

    def test_code_block_info_string_fails(self):
        self.translate("```sh\n", "```bash\n")
        self.assertProblem("code block differs from the one at README.md line 13")

    def test_missing_inline_code_fails(self):
        self.translate("`demo compress` に", "demo compress に")
        self.assertProblem("inline code `demo compress` appears 0 times, README.md has it 1 times")

    def test_changed_link_fails(self):
        self.translate("](../docs/guide.md)", "](../docs/other.md)")
        self.assertProblem("link target docs/guide.md appears 0 times, README.md has it 1 times")

    def test_unadjusted_relative_link_fails(self):
        self.translate("](../docs/guide.md)", "](docs/guide.md)")
        out = self.assertProblem("link target i18n/docs/guide.md appears 1 times, README.md has it 0 times")
        self.assertIn("link target docs/guide.md does not exist (resolves to i18n/docs/guide.md)", out)

    def test_missing_link_file_fails(self):
        body = SOURCE_BODY.replace("docs/guide.md", "docs/gone.md")
        self.write("README.md", SOURCE_SWITCHER + body)
        self.commit("link a missing page")
        text = TRANSLATION_HEAD + TRANSLATION_BODY.replace("../docs/guide.md", "../docs/gone.md")
        self.write("i18n/README.ja.md", self.stamped(text, body))
        out = self.assertProblem("link target ../docs/gone.md does not exist (resolves to docs/gone.md)")
        self.assertNotIn("appears", out)

    def test_missing_number_fails(self):
        self.translate("約42秒", "約四十二秒")
        self.assertProblem("number 42 appears 0 times, README.md has it 1 times")

    def test_extra_number_is_a_warning(self):
        self.translate("約42秒かかります。", "約42秒かかります（3回の平均）。")
        out = self.assertClean()
        self.assertIn("warning: number 3 appears 1 times, README.md has it 0 times", out)

    def test_missing_switcher_language_fails(self):
        switcher = "[English](README.md) | [日本語](i18n/README.ja.md) | [Español](i18n/README.es.md)"
        self.write("README.md", SOURCE_SWITCHER.replace("[English](README.md) | [日本語](i18n/README.ja.md)", switcher) + SOURCE_BODY)
        self.write("i18n/README.es.md", self.stamped(
            TRANSLATION_HEAD.replace("[English](../README.md) | [日本語](README.ja.md)",
                                     "[English](../README.md) | [日本語](README.ja.md) | [Español](README.es.md)")
            + TRANSLATION_BODY))
        out = self.assertProblem("i18n/README.ja.md:2: error: languages block does not link es (i18n/README.es.md)")

    def test_switcher_link_must_resolve(self):
        self.write("i18n/README.ja.md", self.stamped(
            TRANSLATION_HEAD.replace("[日本語](README.ja.md)", "[日本語](README.ja.md) | [Français](README.fr.md)")
            + TRANSLATION_BODY))
        self.assertProblem("languages link README.fr.md does not exist (resolves to i18n/README.fr.md)")

    def test_missing_switcher_block_fails(self):
        self.write("README.md", SOURCE_BODY)
        self.assertProblem("README.md:1: error: no <!-- i18n:languages -->")

    def test_per_file_mode_skips_switchers(self):
        self.write("README.md", SOURCE_BODY)
        self.assertClean("i18n/README.ja.md")

    def test_stamp_round_trip(self):
        self.write("i18n/README.ja.md", "<!-- i18n-source: README.md sha256:%s -->\n%s" % ("0" * 64, TRANSLATION_HEAD + TRANSLATION_BODY))
        code, out = self.run_check("--stamp", "i18n/README.ja.md")
        self.assertEqual(code, 0, out)
        expected = "<!-- i18n-source: README.md sha256:%s -->" % sha(SOURCE_BODY)
        self.assertEqual(self.read("i18n/README.ja.md"), expected + "\n" + TRANSLATION_HEAD + TRANSLATION_BODY)
        out = self.assertClean()
        self.assertIn("0 stale", out)

    def test_stamp_adds_a_missing_stamp_line(self):
        self.write("i18n/README.ja.md", TRANSLATION_HEAD + TRANSLATION_BODY)
        self.assertProblem("i18n/README.ja.md:1: error: line 1 must be <!-- i18n-source: README.md sha256:<hash> -->")
        code, out = self.run_check("--stamp", "i18n/README.ja.md")
        self.assertEqual(code, 0, out)
        self.assertEqual(self.read("i18n/README.ja.md"), self.stamped(TRANSLATION_HEAD + TRANSLATION_BODY))
        self.assertClean()

    def test_stamp_hash_ignores_crlf(self):
        self.write("README.md", (SOURCE_SWITCHER + SOURCE_BODY).replace("\n", "\r\n"))
        out = self.assertClean("i18n/README.ja.md")
        self.assertIn("0 stale, 0 warnings", out)

    def test_stale_translation_is_a_warning(self):
        first = self.git("log", "-1", "--format=%H", "--", "README.md").strip()
        body = SOURCE_BODY.replace("Pipe output through", "Pipe any output through")
        self.write("README.md", SOURCE_SWITCHER + body)
        self.commit("reword usage")
        out = self.assertClean()
        self.assertIn("i18n/README.ja.md:1: warning: stale: translated from README.md at commit %s" % first[:12], out)
        self.assertIn("README.md | 2 +-", out)
        self.assertIn("changed sections: ## Usage", out)
        self.assertIn("1 stale", out)
        out = self.assertProblem("stale:", "--strict")
        self.assertNotIn(": error:", out)

    def test_english_only_change_keeps_structure_passing(self):
        body = SOURCE_BODY.replace(
            "A plain paragraph",
            "Set `DEMO_LEVEL=3` to see [more](docs/other.md).\n\nA plain paragraph")
        self.write("README.md", SOURCE_SWITCHER + body)
        self.commit("document the level")
        out = self.assertClean("i18n/README.ja.md")
        self.assertIn("stale: translated from README.md", out)
        self.assertIn("changed sections: ## Usage", out)

    def test_moved_link_target_is_a_warning_while_stale(self):
        os.rename(os.path.join(self.dir, "docs", "guide.md"), os.path.join(self.dir, "docs", "moved.md"))
        self.write("README.md", SOURCE_SWITCHER + SOURCE_BODY.replace("docs/guide.md", "docs/moved.md"))
        self.commit("move the guide")
        out = self.assertClean()
        self.assertIn("warning: link target ../docs/guide.md does not exist (resolves to docs/guide.md)", out)
        self.assertIn("1 stale", out)

    def test_unknown_stamp_compares_with_current_source(self):
        self.write("i18n/README.ja.md", "<!-- i18n-source: README.md sha256:%s -->\n%s" % ("a" * 64, TRANSLATION_HEAD + TRANSLATION_BODY))
        out = self.assertClean()
        self.assertIn("warning: the stamp matches no committed version of README.md", out)
        self.assertIn("stale: the stamp is not the current hash of README.md", out)

    def test_switcher_edits_keep_the_hash(self):
        switcher = SOURCE_SWITCHER.replace("[日本語](i18n/README.ja.md)", "[日本語](i18n/README.ja.md) | [Español](i18n/README.es.md)")
        for source in (switcher + SOURCE_BODY, SOURCE_BODY, SOURCE_BODY + "\n" + SOURCE_SWITCHER.rstrip("\n") + "\n",
                       SOURCE_BODY.replace("## Usage\n", SOURCE_SWITCHER + "## Usage\n")):
            self.write("README.md", source)
            out = self.assertClean("i18n/README.ja.md")
            self.assertIn("0 stale", out)

    def test_note_and_switcher_in_translation_are_ignored(self):
        head = TRANSLATION_HEAD.replace("この文書は英語版 README.md の翻訳です。", "この文書は英語版の翻訳です。\n\n- 2026\n\n```\nx\n```")
        self.write("i18n/README.ja.md", self.stamped(head + TRANSLATION_BODY))
        self.assertClean()

    def test_non_translations_are_ignored(self):
        self.write("i18n/README.md", "# Translating\n")
        self.write("i18n/glossary/ja.md", "# 用語集\n")
        self.write("i18n/NOTES.ja.md", "# Notes\n")
        out = self.assertClean()
        self.assertIn("checked 1 translations", out)

    def test_usage_errors(self):
        code, out = self.run_check("README.md")
        self.assertEqual(code, 2, out)
        self.assertIn("is not a translation", out)
        code, out = self.run_check("--stamp")
        self.assertEqual(code, 2, out)
        code, out = self.run_check("i18n/README.fr.md")
        self.assertEqual(code, 2, out)

    def test_github_annotations_and_summary(self):
        self.translate("約42秒", "約四十二秒")
        summary = os.path.join(self.dir, "summary.md")
        env = dict(self.env, GITHUB_STEP_SUMMARY=summary)
        code, out = self.run_check("--github", env=env)
        self.assertEqual(code, 1, out)
        self.assertIn("::error file=i18n/README.ja.md,line=", out)
        with open(summary, encoding="utf-8") as f:
            text = f.read()
        self.assertIn("## Translations", text)
        self.assertIn("| `i18n/README.ja.md` | `README.md` | 1 problems |", text)


if __name__ == "__main__":
    unittest.main()
