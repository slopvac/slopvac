"""Regenerate word_class_differential.json: Python's `regex` spans for generated patterns.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-text/tests/fixtures/generate_word_class.py [--check]

`slopvac-text` rewrites `\\w`, `\\W`, `\\b` and `\\B` (and the classes that hold them) so PCRE2
agrees with Python's `regex` module. This fixture checks the rewrite as a whole instead of
case by case. A seeded grammar builds patterns from the constructs both engines read the
same way: literals, `\\w \\W \\d \\s`, positive and negated classes, boundaries, lookarounds,
groups, alternation, quantifiers, `(?#...)` comments and `(?x)` mode with `#` comments. The
texts come from an alphabet of Unicode edge cases: spacing marks, joiners, superscripts,
non-Latin digits and letters, emoji, and `#`, `[`, `]` and newlines.

A pattern is kept only if Python compiles it and none of its matches is empty. Empty
matches are skipped because the stub `find_iter` (STUB(slopvac-8l4.3.25)) drops an empty
match next to a match, as the `pcre2` crate does.

`--check` recomputes the spans for the stored patterns and texts and fails on any change.
Offsets are code points; the Rust test converts them to bytes.
"""

from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import regex

HERE = Path(__file__).resolve().parent
TARGET = HERE / "word_class_differential.json"
SEED = 381
PATTERNS = 700
TEXTS = 80

ALPHABET = [
    "a",
    "b",
    "Z",
    "x",
    "_",
    "-",
    " ",
    ",",
    ".",
    "1",
    "7",
    "\u00e9",  # e acute
    "\u0903",  # spacing mark (Mc)
    "\u0301",  # combining acute (Mn)
    "\u200d",  # zero width joiner
    "\u200c",  # zero width non-joiner
    "\u00b2",  # superscript two (No)
    "\u00b9",  # superscript one (No)
    "\u0663",  # Arabic-Indic digit three (Nd)
    "\u65e5",  # CJK
    "\U0001f600",  # emoji
    "\u00a0",  # no-break space
    "\u3000",  # ideographic space
    "\n",
    "#",
    "[",
    "]",
    "(",
]

LITERALS = [
    "a",
    "b",
    "x",
    "_",
    "1",
    "\u00e9",
    "\u0903",
    "\u200d",
    "\u00b2",
    "\\.",
    "\\-",
    " ",
]
CLASS_ESCAPES = [r"\w", r"\W", r"\d", r"\s"]
CLASS_ATOMS = ["a", "b", "x", "_", "1", "\u0903", "\u200d", ",", "a-z", "0-9", " "]
COMMENTS = ["# [ \\w", "# ] \\b", "# plain"]


class Grammar:
    def __init__(self, rng: random.Random) -> None:
        self.rng = rng

    def pick(self, items):
        return self.rng.choice(items)

    def cls(self) -> str:
        members: list[str] = []
        for _ in range(self.rng.randint(1, 3)):
            members.append(self.pick(CLASS_ESCAPES + CLASS_ATOMS))
        negated = self.rng.random() < 0.4
        if negated and r"\w" in members and r"\W" in members:
            # Python's `regex` reads `[^\w\W]` as matching every character; the class is
            # empty by its own definition, and PCRE2 rightly matches nothing. See
            # divergences.md.
            members.remove(r"\W")
        if self.rng.random() < 0.2:
            members.append("-")  # a hyphen last is a literal in both engines
        if self.rng.random() < 0.1:
            members.insert(0, "]")
        return "[" + ("^" if negated else "") + "".join(members) + "]"

    def atom(self, depth: int, extended: bool) -> str:
        r = self.rng.random()
        if r < 0.22:
            return self.pick(LITERALS)
        if r < 0.38:
            return self.pick(CLASS_ESCAPES)
        if r < 0.58:
            return self.cls()
        if r < 0.68:
            return self.pick([r"\b", r"\B"])
        if r < 0.78:
            return self.pick(
                [r"(?<!\w)", r"(?<=\w)", r"(?=\W)", r"(?!\w)", r"(?<![\w-])"]
            )
        if r < 0.80:
            return "(?#a [ comment)"
        if depth >= 2:
            return self.pick(LITERALS)
        return self.group(depth + 1, extended)

    def quantified(self, depth: int, extended: bool) -> str:
        atom = self.atom(depth, extended)
        if atom.startswith("(?<") or atom.startswith("(?=") or atom.startswith("(?!"):
            return atom  # a quantified assertion adds nothing and some are rejected
        if atom in (r"\b", r"\B") or atom.startswith("(?#"):
            return atom
        if self.rng.random() < 0.35:
            return atom + self.pick(["?", "+", "*", "{1,2}", "{2}", "+?"])
        return atom

    def seq(self, depth: int, extended: bool) -> str:
        parts = []
        for _ in range(self.rng.randint(1, 4)):
            parts.append(self.quantified(depth, extended))
            if extended and self.rng.random() < 0.4:
                parts.append(
                    self.pick([" ", "  ", "\n", " " + self.pick(COMMENTS) + "\n"])
                )
        return "".join(parts)

    def group(self, depth: int, extended: bool) -> str:
        r = self.rng.random()
        if r < 0.25:
            inner = self.seq(depth, True)
            return "(?x:" + inner + ")"
        branches = "|".join(
            self.seq(depth, extended) for _ in range(self.rng.randint(1, 3))
        )
        return "(?:" + branches + ")"

    def pattern(self) -> str:
        if self.rng.random() < 0.15:
            return "(?x)" + self.seq(0, True)
        return self.seq(0, False)


def make_inputs() -> tuple[list[dict], list[str]]:
    rng = random.Random(SEED)
    texts = [
        "".join(rng.choice(ALPHABET) for _ in range(rng.randint(2, 12)))
        for _ in range(TEXTS)
    ]
    grammar = Grammar(rng)
    patterns: list[dict] = []
    seen: set[tuple[str, bool]] = set()
    while len(patterns) < PATTERNS:
        source = grammar.pattern()
        ignore_case = rng.random() < 0.3
        if (source, ignore_case) in seen:
            continue
        seen.add((source, ignore_case))
        spans = spans_for(source, ignore_case, texts)
        if spans is not None:
            patterns.append({"p": source, "i": ignore_case})
    return patterns, texts


def spans_for(
    source: str, ignore_case: bool, texts: list[str]
) -> dict[str, list[list[int]]] | None:
    """Spans per text index, or None when Python rejects the pattern or it matches empty."""
    try:
        compiled = regex.compile(source, regex.IGNORECASE if ignore_case else 0)
    except regex.error:
        return None
    out: dict[str, list[list[int]]] = {}
    for index, text in enumerate(texts):
        found = [[m.start(), m.end()] for m in compiled.finditer(text)]
        if any(s == e for s, e in found):
            return None
        if found:
            out[str(index)] = found
    return out


def render(patterns: list[dict], texts: list[str]) -> str:
    dump = lambda v: json.dumps(v, ensure_ascii=False, separators=(",", ":"))  # noqa: E731
    lines = ["{", f'"regex":{dump(regex.__version__)},', '"texts":[']
    lines.append(",\n".join(dump(t) for t in texts))
    lines.append("],")
    lines.append('"patterns":[')
    rows = []
    for entry in patterns:
        spans = spans_for(entry["p"], entry["i"], texts)
        assert spans is not None, entry
        rows.append(dump({"p": entry["p"], "i": entry["i"], "m": spans}))
    lines.append(",\n".join(rows))
    lines.append("]")
    lines.append("}")
    return "\n".join(lines) + "\n"


def main() -> None:
    if "--check" in sys.argv:
        stored = json.loads(TARGET.read_text(encoding="utf-8"))
        patterns = [{"p": e["p"], "i": e["i"]} for e in stored["patterns"]]
        fresh = json.loads(render(patterns, stored["texts"]))
        if fresh["patterns"] != stored["patterns"]:
            raise SystemExit(
                "word_class_differential.json is stale; regenerate with generate_word_class.py"
            )
        return
    patterns, texts = make_inputs()
    TARGET.write_text(render(patterns, texts), encoding="utf-8")
    print(f"{len(patterns)} patterns, {len(texts)} texts")


if __name__ == "__main__":
    main()
