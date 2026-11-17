"""Fact preservation by lexical alignment, with no model.

A fact is a token a rewrite must not lose or change on its own:

- numbers and versions (`40`, `2.18`, `10%`),
- code spans and identifiers (`parse_config`, `--offline`, `a.b.c`),
- polarity words (`must`, `not`, `never`, `only`, ...).

Preservation compares multisets: each fact in the source is kept when the output
has at least as many of the same. A change that is meant (an update from 1 to 2)
is passed in `expected`. It covers every occurrence of the old value, and counts
as kept when the new value appears that often and the old one is gone.
"""

from __future__ import annotations

import re
from collections import Counter
from collections.abc import Iterable, Mapping
from typing import Any

NUMBER_RE = re.compile(r"(?<![\w.])\d+(?:[.,]\d+)*%?(?![\w])")
CODE_SPAN_RE = re.compile(r"`([^`\n]+)`")
IDENT_RE = re.compile(
    r"(?<![\w`])(?:--?[a-z][\w-]*|[A-Za-z]+_[\w]+|[a-z]+[A-Z][A-Za-z0-9]*|[\w]+(?:\.[A-Za-z_]\w*){1,}\(?\)?)(?![\w`])"
)
POLARITY = frozenset(
    [
        "must",
        "should",
        "may",
        "cannot",
        "not",
        "never",
        "no",
        "only",
        "always",
        "required",
        "optional",
        "deprecated",
        "forbidden",
        "allowed",
    ]
)
CONTRACTION_RE = re.compile(r"\b(\w+)n['\u2019]t\b", re.IGNORECASE)
WORD_RE = re.compile(r"[A-Za-z']+")


def extract(text: str) -> dict[str, Counter[str]]:
    """Return the facts in `text`, grouped by kind."""
    text = CONTRACTION_RE.sub(
        lambda m: f"{m.group(1)} not", text.replace("can not", "cannot")
    )
    spans = CODE_SPAN_RE.findall(text)
    bare = CODE_SPAN_RE.sub(" ", text)
    return {
        "number": Counter(
            m.rstrip("%") + ("%" if m.endswith("%") else "")
            for m in NUMBER_RE.findall(bare)
        ),
        "code": Counter(s.strip() for s in spans),
        "identifier": Counter(IDENT_RE.findall(bare)),
        "polarity": Counter(
            w.lower() for w in WORD_RE.findall(bare) if w.lower() in POLARITY
        ),
    }


def preservation(
    source: str,
    output: str,
    expected: Iterable[Mapping[str, str]] = (),
) -> dict[str, Any]:
    """Compare facts of `output` with `source`.

    `expected` lists intended changes as {"kind": ..., "old": ..., "new": ...}.
    """
    src, out = extract(source), extract(output)
    changes = list(expected)
    for change in changes:
        counter = src[change["kind"]]
        moved = counter[change["old"]]
        if moved > 0:  # the change covers every occurrence of the old value
            counter[change["old"]] = 0
            counter[change["new"]] += moved
    lost: list[dict[str, Any]] = []
    total = kept = 0
    for kind, counter in src.items():
        for fact, count in counter.items():
            if count <= 0:
                continue
            have = min(count, out[kind][fact])
            total += count
            kept += have
            if have < count:
                lost.append({"kind": kind, "fact": fact, "missing": count - have})
    applied = [
        c
        for c in changes
        if out[c["kind"]][c["new"]] > 0 and out[c["kind"]][c["old"]] == 0
    ]
    return {
        "facts": total,
        "kept": kept,
        "rate": kept / total if total else None,
        "lost": sorted(lost, key=lambda d: (d["kind"], d["fact"])),
        "changes_applied": len(applied),
        "changes_expected": len(changes),
    }
