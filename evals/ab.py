"""Paired A/B comparison of two result sets over the same corpus.

Both arms see the same documents, so the bootstrap resamples documents once and
scores both arms on that draw. The interval is for the difference B - A.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from . import metrics
from .scoring import _fires, _span
from .splits import split_of

# Per document and per arm: (seeded hits, seeded n, control hits, control n,
# human findings, human words). `rule` narrows the counts to one rule.
Cell = tuple[int, int, int, int, int, int]
Pair = tuple[Cell, Cell]


def _cell(doc: Mapping[str, Any], res: Mapping[str, Any], rule: str | None) -> Cell:
    sh = sn = ch = cn = 0
    for kind in ("defects", "controls"):
        for entry in doc["source"].get(kind, ()):
            if rule is not None and entry["rule"] != rule:
                continue
            lo, hi = _span(entry)
            hit = _fires(res["findings"], entry["rule"], lo, hi)
            if kind == "defects":
                sh, sn = sh + hit, sn + 1
            else:
                ch, cn = ch + hit, cn + 1
    hf = hw = 0
    if doc["origin"] == "human":
        hf = sum(1 for f in res["findings"] if rule is None or f["rule"] == rule)
        hw = res["words"]
    return sh, sn, ch, cn, hf, hw


def _stat(index_hits: int, index_n: int, scale: int = 1):
    def fn(pairs: Sequence[Cell]) -> float | None:
        n = sum(c[index_n] for c in pairs)
        return scale * sum(c[index_hits] for c in pairs) / n if n else None

    return fn


STATS = {
    "recall": _stat(0, 1),
    "control_fpr": _stat(2, 3),
    "human_per_100_words": _stat(4, 5, 100),
}


def ab(
    manifest: Mapping[str, Any],
    results_a: Mapping[str, Any],
    results_b: Mapping[str, Any],
    *,
    split: str = "all",
    resamples: int = 1000,
    rules: Sequence[str] | None = None,
) -> dict[str, Any]:
    docs = {d["id"]: d for d in manifest["documents"]}
    common = sorted(
        i
        for i in docs
        if i in results_a["documents"]
        and i in results_b["documents"]
        and (split == "all" or split_of(docs[i]) == split)
    )

    def block(rule: str | None) -> dict[str, Any]:
        pairs: list[Pair] = [
            (
                _cell(docs[i], results_a["documents"][i], rule),
                _cell(docs[i], results_b["documents"][i], rule),
            )
            for i in common
        ]
        out: dict[str, Any] = {}
        for name, fn in STATS.items():
            out[name] = metrics.paired_bootstrap(
                pairs,
                lambda s, fn=fn: fn([p[0] for p in s]),
                lambda s, fn=fn: fn([p[1] for p in s]),
                resamples=resamples,
            )
            out[name]["a"] = fn([p[0] for p in pairs])
            out[name]["b"] = fn([p[1] for p in pairs])
        return out

    per_rule: dict[str, Any] = {}
    names = set(rules or ())
    if not rules:
        for d in docs.values():
            for kind in ("defects", "controls"):
                names.update(e["rule"] for e in d["source"].get(kind, ()))
    for rule in sorted(names):
        per_rule[rule] = block(rule)
    return {
        "split": split,
        "documents": len(common),
        "overall": block(None),
        "rules": per_rule,
        "engines": {"a": results_a.get("engine"), "b": results_b.get("engine")},
    }


def regressions(report: Mapping[str, Any]) -> list[str]:
    """Rules where B is worse than A beyond bootstrap noise (interval excludes 0)."""
    bad: list[str] = []
    worse = {"recall": -1, "control_fpr": 1, "human_per_100_words": 1}
    for rule, block in report["rules"].items():
        for name, sign in worse.items():
            stat = block[name]
            if stat["low"] is None:
                continue
            if sign < 0 and stat["high"] < 0 or sign > 0 and stat["low"] > 0:
                bad.append(f"{rule}:{name}")
    return bad
