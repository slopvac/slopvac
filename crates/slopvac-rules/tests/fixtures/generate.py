"""Regenerate the slopvac-rules test fixtures from the Python reference implementation.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-rules/tests/fixtures/generate.py [--check]

It writes two things, and the Rust tests read both, so the expected values come from the
Python implementation and never from the Rust code:

* ``core-v3/<category>.json``: the shipped 2.x rules converted to v3 category files. The
  converter is the vendored copy under ``stub-3.28/``; STUB(slopvac-8l4.3.28) replaces it
  with ``scripts/contract/convert_rules_v2_to_v3.py`` once PR #281 lands.
* ``pattern-parity.json``: a frozen corpus of texts and, for every lexical rule, the spans
  Python's ``regex`` module finds in each text (``engine.rule_pattern``). Spans are code
  point offsets; the Rust test converts them to bytes.

``--check`` recomputes both and fails when a file differs. The corpus texts are frozen: it
only recomputes the spans for the stored texts. Without ``--check`` the corpus is rebuilt
from the rule examples, the repository's own prose and the Unicode probes below.
"""

from __future__ import annotations

import importlib.util
import json
import re
import sys
import unicodedata
from pathlib import Path

import regex

from slopvac.engine import rule_pattern
from slopvac.model import RuleKind
from slopvac.rules import load_ruleset

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
RULES_DIR = ROOT / "packages/slopvac-lint/src/slopvac/rules"
CONVERTER = HERE / "stub-3.28/convert_rules_v2_to_v3.py"
CORE = HERE / "core-v3"
PARITY = HERE / "pattern-parity.json"

PROSE_SOURCES = [
    "README.md",
    "CONTRIBUTING.md",
    "docs/design/slopvac-3.md",
    "docs/design/0001-slopvac-3-architecture.md",
    "docs/contract/README.md",
    "docs/contract/policies.md",
    "docs/contract/positions.md",
    "docs/contract/lint-report.md",
    "docs/contract/config.md",
    "docs/migrate-2-to-3.md",
]

# Characters where Python's ``regex`` and PCRE2 can disagree: marks and joiners inside
# words, superscripts, dotted capital I, sharp s, Turkish dotless i, CJK, emoji and
# non-breaking spaces. Each sentence also carries a phrase the rules look for.
PROBES = [
    "It is robust\u00b9 and also robust\u00b2, which is fine.",
    "The Utilize\u200dr and the utilize\u200d are inside words.",
    "Please leverage\u0903 this, then leverage this.",
    "The \u0130stanbul office will utilize the tool.",
    "Stra\u00dfe is a street; we utilize it. STRASSE too.",
    "I\u0307 will leverage it; \u0131 is dotless.",
    "In order to start, press the button. \u65e5\u672c\u8a9e in order to.",
    "We will leverage \U0001f600 emoji and utilize them.",
    "Do\u00a0not\u00a0leverage non-breaking spaces; do not utilize them.",
    "Click here\u2026 and then\u2014without delay\u2014utilize it.",
    "As an AI language model, I cannot help; feel free to ask.",
    "Wait\u200c, ZWNJ inside: ut\u200cilize and leverage.",
    "The café is a good place to leverage naïve ideas.",
    "e.g. this, i.e. that, etc. and so on; whitelist, whitelisted, anti-whitelist.",
    "A. B. C. D. is a long initialism; U.S.A. is shorter.",
    "x" * 300 + " leverage " + "y" * 300,
    "",
    " ",
    "\n\n",
    "Line one.\nLine two with leverage.\r\nLine three.",
]


# The 2.x rule data meets text in many guises. Each transform rewrites a rule's bad example so
# the corpus covers the cases where the two engines' Unicode tables can differ.
TRANSFORMS = {
    "nfd": lambda t: unicodedata.normalize("NFD", t),
    "upper": str.upper,
    "title": str.title,
    "swapcase": str.swapcase,
    "nbsp": lambda t: t.replace(" ", "\u00a0"),
    "ideographic-space": lambda t: t.replace(" ", "\u3000"),
    "zwsp": lambda t: t.replace(" ", " \u200b"),
    "zwj-between-letters": lambda t: t.replace(" ", "\u200d"),
    "dotted-i": lambda t: t.replace("i", "\u0130"),
    "dotless-i": lambda t: t.replace("I", "\u0131"),
    "superscript": lambda t: regex.sub(r"(\p{L}+)", "\\1\u00b9", t),
    "long-s": lambda t: t.replace("s", "\u017f"),
    "kelvin": lambda t: t.replace("k", "\u212a"),
    "crlf": lambda t: t.replace(" ", "\r\n", 1) + "\r\n",
    "emoji": lambda t: "\U0001f600 " + t + " \U0001f44d\U0001f3fd",
    "cjk": lambda t: "\u65e5\u672c\u8a9e " + t + "\u3002",
}


def load_converter():
    spec = importlib.util.spec_from_file_location("convert_rules_v2_to_v3", CONVERTER)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def core_files() -> dict[str, str]:
    """File name -> text of every converted category."""
    categories = load_converter().convert_dir(RULES_DIR)
    return {
        f"{c['id']}.json": json.dumps(c, indent=2, ensure_ascii=False) + "\n"
        for c in categories
    }


def lexical_rules():
    rules = []
    for category in load_ruleset().categories.values():
        for rule in category.rules:
            if rule.kind in (RuleKind.TOKENS, RuleKind.PATTERN, RuleKind.SUBSTITUTION):
                rules.append(rule)
    rules.sort(key=lambda r: r.qualified_id)
    return rules


def paragraphs(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    text = regex.sub(r"```.*?```", "", text, flags=regex.S)
    out = []
    for block in re.split(r"\n\s*\n", text):
        block = block.strip()
        if block and not block.startswith(("#", "|", "---", "<")):
            out.append(block)
    return out


def build_corpus(rules) -> list[str]:
    seen: set[str] = set()
    texts: list[str] = []

    def add(text: str) -> None:
        if text not in seen:
            seen.add(text)
            texts.append(text)

    for rule in rules:
        for example in rule.examples:
            add(example.bad)
            if example.good:
                add(example.good)
    for rule in rules:
        for example in rule.examples[:2]:
            for transform in TRANSFORMS.values():
                add(transform(example.bad))
    for name in PROSE_SOURCES:
        for block in paragraphs(ROOT / name):
            add(block)
    for probe in PROBES:
        add(probe)
    return texts


# PCRE2 does not fold U+0130 (dotted capital I) and U+0131 (dotless i) onto `i` and `I` when it
# ignores case; Python's `regex` does. The "inert" spans say what PCRE2 should find: Python's
# spans on the text with both characters swapped for two caseless letters, which keeps their
# length and their word-ness and removes only the folding.
INERT = str.maketrans({"\u0130": "\u0294", "\u0131": "\u0295"})


def has_dotted_i(text: str) -> bool:
    return text != text.translate(INERT)


def spans_for(rules, texts: list[str], inert: bool = False) -> dict[str, dict[str, list[list[int]]]]:
    """Rule id -> text index -> code point spans, for the texts that match.

    With ``inert`` only the texts that hold U+0130 or U+0131 are searched, with those
    characters swapped out (see ``INERT``).
    """
    out: dict[str, dict[str, list[list[int]]]] = {}
    for rule in rules:
        pattern = rule_pattern(rule)
        if pattern is None:
            continue
        hits: dict[str, list[list[int]]] = {}
        for index, text in enumerate(texts):
            if inert and not has_dotted_i(text):
                continue
            subject = text.translate(INERT) if inert else text
            found = [[m.start(), m.end()] for m in pattern.finditer(subject)]
            if found:
                hits[str(index)] = found
        out[rule.qualified_id] = hits
    return out


def parity_text(texts: list[str]) -> str:
    """One text per line and one rule per line, so a change reviews as a small diff."""
    rules = lexical_rules()
    spans = spans_for(rules, texts)
    inert = spans_for(rules, texts, inert=True)
    dump = lambda value: json.dumps(value, ensure_ascii=False, separators=(",", ":"))  # noqa: E731
    lines = [
        "{",
        f'"regex":{dump(regex.__version__)},',
        f'"unicode":{dump(unicodedata.unidata_version)},',
    ]
    lines.append('"texts":[')
    lines.append(",\n".join(dump(t) for t in texts))
    lines.append("],")
    lines.append('"spans":{')
    lines.append(",\n".join(f"{dump(k)}:{dump(v)}" for k, v in spans.items()))
    lines.append("},")
    lines.append('"inert":{')
    lines.append(",\n".join(f"{dump(k)}:{dump(v)}" for k, v in inert.items()))
    lines.append("}")
    lines.append("}")
    return "\n".join(lines) + "\n"


def main() -> None:
    check = "--check" in sys.argv
    core = core_files()
    if check:
        problems = []
        for name, text in core.items():
            path = CORE / name
            if not path.exists() or path.read_text(encoding="utf-8") != text:
                problems.append(f"core-v3/{name} is stale")
        on_disk = {p.name for p in CORE.glob("*.json")}
        problems += [
            f"core-v3/{n} has no source rule file" for n in sorted(on_disk - set(core))
        ]
        stored = json.loads(PARITY.read_text(encoding="utf-8"))
        # The Unicode data and the regex release are recorded for the divergences file;
        # only the spans must track the rules.
        fresh = parity_text(stored["texts"])
        current = json.loads(fresh)
        if stored["spans"] != current["spans"] or stored["inert"] != current["inert"]:
            problems.append("pattern-parity.json spans are stale")
        if problems:
            raise SystemExit(
                "; ".join(problems)
                + "; regenerate with generate.py and commit the result"
            )
        return
    CORE.mkdir(exist_ok=True)
    for stale in CORE.glob("*.json"):
        stale.unlink()
    for name, text in core.items():
        (CORE / name).write_text(text, encoding="utf-8")
    rules = lexical_rules()
    texts = build_corpus(rules)
    PARITY.write_text(parity_text(texts), encoding="utf-8")
    print(f"{len(core)} categories, {len(rules)} lexical rules, {len(texts)} texts")


if __name__ == "__main__":
    main()
