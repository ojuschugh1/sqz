#!/usr/bin/env bash
# Repro: sqz's n-gram abbreviator corrupts SHAs / paths in command output,
# and demonstration of the SQZ_NO_ABBREV=1 / --no-abbrev opt-out that fixes it.
#
# Mechanism (sqz_engine/src/ngram_abbreviator.rs, applied in
# sqz/src/cli_proxy.rs): any multi-word phrase that repeats 3+ times has every
# occurrence AFTER the first replaced with a «A1» symbol + a legend. If a SHA
# or path lives inside that repeated phrase, it survives only on the first
# line — every later reference becomes «A1». An agent/script that reads a
# later line gets «A1» instead of the real value and the next command fails.
#
# Abbreviation is ON by default (matches upstream ojuschugh1/sqz, commits
# 532eca7 + d8ca7ca). This script:
#   • DEMONSTRATES the default-on corruption (informational — not an assertion,
#     since shipping default-on is a deliberate choice).
#   • ASSERTS the opt-out is lossless: with SQZ_NO_ABBREV=1, every SHA/path
#     must survive verbatim. The script's exit code reflects ONLY the opt-out
#     contract — exit 0 = opt-out works, exit 1 = opt-out is broken.
set -u
fail=0
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

git init -q; git config user.email t@t; git config user.name t
echo v1 > f.txt; git add f.txt; git commit -qm first
SHA=$(git rev-parse HEAD)

# Realistic build/CI log: same commit referenced on several lines.
for m in a b c d; do
  echo "Resolving dependencies for revision $SHA in module-$m"
done > build.log

extract_module_c_sha() { grep module-c "$1" | grep -oE '[0-9a-f]{40}'; }

# NOTE: every sqz call below passes --no-cache. Without it, the first call
# caches the content and later calls return a lossless `§ref:HASH§` token —
# correct compression, but it hides the per-line text the grep needs to
# inspect. --no-cache isolates the abbreviation behaviour we're testing.

# ── Demonstration: default ON still corrupts (expected, informational) ──────
echo "### Default (abbreviation ON) — demonstrates the corruption"
sqz compress --cmd build --no-cache < build.log 2>/dev/null > build.default.c
GOT_DEFAULT=$(extract_module_c_sha build.default.c)
if [ "$GOT_DEFAULT" != "$SHA" ]; then
  echo "  module-c SHA after default sqz: '<MISSING — replaced by «A1»>'"
  echo "  -> this is why an agent's 'git checkout <module-c sha>' would fail."
else
  echo "  module-c SHA survived (input not repetitive enough to abbreviate)."
fi

# ── Assertion: opt-out is lossless ──────────────────────────────────────────
echo "### Opt-out (SQZ_NO_ABBREV=1) — must be lossless"
SQZ_NO_ABBREV=1 sqz compress --cmd build --no-cache < build.log 2>/dev/null > build.noabbrev.c
GOT_OPTOUT=$(extract_module_c_sha build.noabbrev.c)
echo "  module-c SHA after SQZ_NO_ABBREV=1: '${GOT_OPTOUT:-<MISSING>}'"
if grep -q '«A' build.noabbrev.c; then
  echo "  RESULT: BROKEN — «A1» symbol present despite opt-out"; fail=1
elif [ "$GOT_OPTOUT" != "$SHA" ]; then
  echo "  -> git checkout '${GOT_OPTOUT:-«A1»}':"
  git checkout "${GOT_OPTOUT:-«A1»}" 2>&1 | sed 's/^/     /'
  echo "  RESULT: BROKEN — SHA not preserved under opt-out (expected $SHA)"; fail=1
else
  echo "  RESULT: ok — SHA preserved on every line under opt-out"
fi

# Also verify with the --no-abbrev CLI flag (same contract, different switch).
echo "### Opt-out (--no-abbrev flag) — must be lossless"
sqz compress --cmd build --no-abbrev --no-cache < build.log 2>/dev/null > build.flag.c
if grep -q '«A' build.flag.c || [ "$(extract_module_c_sha build.flag.c)" != "$SHA" ]; then
  echo "  RESULT: BROKEN — --no-abbrev did not suppress abbreviation"; fail=1
else
  echo "  RESULT: ok — --no-abbrev preserves the SHA"
fi

echo
[ "$fail" = 0 ] && echo "PASS — opt-out (SQZ_NO_ABBREV / --no-abbrev) is lossless" \
                || echo "FAIL — opt-out did not prevent corruption"
exit "$fail"
