"""Regenerate the title-case differential fixture from the 2.x ``title-case-heading`` regex.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-engine/tests/fixtures/generate_title_case.py [--check]

The Rust ``capitalization`` kind (sentence style) is a native port of that one regex. This
script builds a corpus of generated headings and records, for each, whether the 2.x regex
matches it, with the allowed words injected the way ``rules.inject_locale_rule`` does. The
Rust test ``capitalization_matches_the_2x_regex_on_generated_headings`` runs the same texts
through the engine and compares. Nothing in the expectation comes from Rust code.

``--check`` fails when the stored file differs from a fresh build.
"""

from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import regex as re

from slopvac.rules import ALLOWED_WORDS_PLACEHOLDER, load_ruleset

OUT = Path(__file__).resolve().parent / "title-case-differential.json"

ALLOWED = ["Brave", "Search"]

# Words the generator combines: plain capitalised words, small words, identifiers, numbers in
# several scripts, backtick spans with and without spaces, punctuation and non-ASCII letters.
PLAIN = [
    "Server",
    "Plugin",
    "Replication",
    "Lag",
    "Getting",
    "Started",
    "Quick",
    "Guide",
]
ALLOWED_POOL = ["Brave", "Search", "brave", "BRAVE"]
SMALL = ["the", "of", "and", "to", "with", "you", "is", "a", "an", "for", "The", "Of"]
IDENT = [
    "SDK",
    "APIs",
    "EventLog",
    "gRPC",
    "macOS",
    "Python3",
    "TypeScript",
    "A",
    "I",
    "B2B",
]
JOINED = ["Foo-Bar", "Foo's", "Foo\u2019s", "Foo-", "Foo--Bar", "-Foo", "Foo'"]
NUMBERS = [
    "2",
    "2.0",
    "1.",
    "1.2.3",
    "\u0662",
    "\u0663.\u0664",
    "2.",
    ".5",
    "\u0662.0",
    "\u00bd",
]
CODE = [
    "`x`",
    "`Alpha Beta`",
    "`a b c`",
    "`--json`",
    "``",
    "`",
    "`unclosed",
    "tail`",
    "`a`b",
]
PUNCT = ["&", "-", ":", "(", ")", "/", "+", ",", "...", "!", "?", ";"]
SUFFIXED = [
    "Server,",
    "Server:",
    "Server)",
    "Server.",
    "Server!?",
    "(Server",
    "Server(",
]
OTHER = [
    "v2",
    "\u00c9clair",
    "\u00c9",
    "caf\u00e9",
    "Caf\u00e9",
    "foo",
    "lowercase",
    "UPPER",
]

POOLS = [
    PLAIN,
    ALLOWED_POOL,
    SMALL,
    IDENT,
    JOINED,
    NUMBERS,
    CODE,
    PUNCT,
    SUFFIXED,
    OTHER,
]
# Pools that make a title-case run likely, so matches are not rare in the corpus.
TITLEISH = [PLAIN, SMALL, IDENT, NUMBERS, PUNCT, ALLOWED_POOL]

FIXED = [
    "Configure The Server",
    "`Alpha Beta` Gamma",
    "Using The `x` Server",
    "`x` Server Beta",
    "Release Notes \u0662",
    "Release Notes \u0663.\u0664",
    "Foo Bar `Baz",
    "Foo Bar `a`,",
    "Configure Brave Search",
    "Configure The Brave Search",
    "Foo `bar Baz` qux Quux",
    "Getting Started With gRPC",
]


def pattern() -> re.Pattern:
    rules = load_ruleset()
    rule = next(
        r
        for c in rules.categories.values()
        for r in c.rules
        if r.id == "title-case-heading"
    )
    source = rule.pattern.replace(
        ALLOWED_WORDS_PLACEHOLDER, "|".join(re.escape(w) for w in ALLOWED)
    )
    return re.compile(source)


def texts() -> list[str]:
    rng = random.Random(257)
    out = list(FIXED)
    for _ in range(4000):
        pools = TITLEISH if rng.random() < 0.7 else POOLS
        words = [rng.choice(rng.choice(pools)) for _ in range(rng.randint(2, 6))]
        out.append(" ".join(words))
    seen: set[str] = set()
    unique = []
    for t in out:
        if t not in seen:
            seen.add(t)
            unique.append(t)
    return unique


def render() -> str:
    pat = pattern()
    cases = [[t, bool(pat.search(t))] for t in texts()]
    head = json.dumps({"allowed_words": ALLOWED}, ensure_ascii=False)[:-1]
    lines = ",\n".join(json.dumps(c, ensure_ascii=False) for c in cases)
    return head + ',"cases":[\n' + lines + "\n]}\n"


def main() -> None:
    text = render()
    if "--check" in sys.argv:
        if OUT.read_text(encoding="utf-8") != text:
            raise SystemExit(
                "title-case-differential.json is stale; regenerate with "
                "generate_title_case.py and commit the result"
            )
        return
    OUT.write_text(text, encoding="utf-8")
    data = json.loads(text)
    hits = sum(1 for _, m in data["cases"] if m)
    print(f"{len(data['cases'])} cases, {hits} match")


if __name__ == "__main__":
    main()
