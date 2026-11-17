"""Score engine results against the corpus manifest.

Measures per rule and per split:

- recall on seeded defects and adjudicated true positives: the rule must fire
  at the inserted or labelled line (`seeded_recall` and `adjudicated_recall`
  show the two parts),
- false-positive rate on clean controls: the rule must stay quiet at the
  counterpart line of the same document,
- human-baseline rate: findings per 100 words on pre-2022 human text,
- LLM firing rate: findings per 100 words on stated-LLM and model-twin text,
- precision, when adjudicated labels are supplied.

Uncertainty is a document-cluster bootstrap, because findings inside one
document are not independent.
"""

from __future__ import annotations

import json
from collections import defaultdict
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path
from typing import Any

from . import metrics
from .splits import split_of

LINE_SLACK = 1
# Origins that count as LLM text: answers a publisher labels with the producing
# model, and documents a model wrote from a human outline.
LLM_ORIGINS = frozenset({"stated-llm", "model-twin"})
SCORE_SCHEMA = 1

Label = str  # "tp" or "fp"; anything else (for example "unsure") is ignored
LabelKey = tuple[str, str, int]  # (document id, rule, line)


def _span(entry: Mapping[str, Any]) -> tuple[int, int]:
    start = entry["line"]
    return start - LINE_SLACK, start + entry["text"].count("\n") + LINE_SLACK


def _fires(findings: Iterable[Mapping[str, Any]], rule: str, lo: int, hi: int) -> bool:
    return any(f["rule"] == rule and lo <= f["line"] <= hi for f in findings)


def _ratio(clusters: Sequence[tuple[int, int]]) -> float | None:
    hits = sum(h for h, _ in clusters)
    total = sum(n for _, n in clusters)
    return hits / total if total else None


def _summarise(per_doc: Sequence[tuple[int, int]], resamples: int) -> dict[str, Any]:
    hits = sum(h for h, _ in per_doc)
    total = sum(n for _, n in per_doc)
    interval = metrics.wilson(hits, total)
    boot = metrics.cluster_bootstrap(per_doc, _ratio, resamples=resamples)
    return {
        "n": total,
        "hits": hits,
        "rate": metrics.rate(hits, total),
        "wilson": list(interval) if interval else None,
        "bootstrap": [boot["low"], boot["high"]] if boot["low"] is not None else None,
        "documents": len(per_doc),
    }


def score(
    manifest: Mapping[str, Any],
    results: Mapping[str, Any],
    *,
    split: str = "all",
    labels: Mapping[LabelKey, Label] | None = None,
    resamples: int = 1000,
) -> dict[str, Any]:
    """Return the scorecard for `split` ("all", "tuning" or "holdout")."""
    docs = {d["id"]: d for d in manifest["documents"]}
    found = results["documents"]

    def wanted(doc_id: str) -> bool:
        return doc_id in found and (split == "all" or split_of(docs[doc_id]) == split)

    seeded: dict[str, list[tuple[int, int]]] = defaultdict(list)
    adjudicated: dict[str, list[tuple[int, int]]] = defaultdict(list)
    pooled: dict[str, list[tuple[int, int]]] = defaultdict(list)
    positives: dict[str, list[tuple[str, int]]] = defaultdict(list)
    for (label_doc, label_rule, label_line), verdict in (labels or {}).items():
        if verdict == "tp":
            positives[label_doc].append((label_rule, label_line))
    control: dict[str, list[tuple[int, int]]] = defaultdict(list)
    human_hits: dict[str, int] = defaultdict(int)
    human_words = 0
    llm_hits: dict[str, int] = defaultdict(int)
    llm_words = 0
    origin_stats: dict[str, dict[str, float]] = defaultdict(
        lambda: {"documents": 0, "words": 0, "findings": 0}
    )
    incomplete = 0
    precision_cells: dict[str, dict[str, list[int]]] = defaultdict(
        lambda: defaultdict(lambda: [0, 0])
    )

    for doc_id, doc in docs.items():
        if not wanted(doc_id):
            continue
        res = found[doc_id]
        stats = origin_stats[doc["origin"]]
        stats["documents"] += 1
        stats["words"] += res["words"]
        stats["findings"] += len(res["findings"])
        incomplete += bool(res.get("unchecked"))
        if doc["origin"] == "human":
            human_words += res["words"]
            for f in res["findings"]:
                human_hits[f["rule"]] += 1
        elif doc["origin"] in LLM_ORIGINS:
            llm_words += res["words"]
            for f in res["findings"]:
                llm_hits[f["rule"]] += 1
        for kind, bucket in (("defects", seeded), ("controls", control)):
            entries = doc["source"].get(kind)
            if not entries:
                continue
            by_rule: dict[str, list[int]] = defaultdict(lambda: [0, 0])
            for entry in entries:
                lo, hi = _span(entry)
                cell = by_rule[entry["rule"]]
                cell[0] += _fires(res["findings"], entry["rule"], lo, hi)
                cell[1] += 1
            for rule, (hit, n) in by_rule.items():
                bucket[rule].append((hit, n))
                if kind == "defects":
                    pooled[rule].append((hit, n))
        # An adjudicated true positive the engine no longer reports is a miss.
        by_rule = defaultdict(lambda: [0, 0])
        for rule, line in positives.get(doc_id, ()):
            cell = by_rule[rule]
            cell[0] += _fires(
                res["findings"], rule, line - LINE_SLACK, line + LINE_SLACK
            )
            cell[1] += 1
        for rule, (hit, n) in by_rule.items():
            adjudicated[rule].append((hit, n))
            pooled[rule].append((hit, n))
        if labels:
            for f in res["findings"]:
                verdict = labels.get((doc_id, f["rule"], f["line"]))
                if verdict in ("tp", "fp"):
                    cell = precision_cells[f["rule"]][doc_id]
                    cell[0] += verdict == "tp"
                    cell[1] += 1

    rules: dict[str, Any] = {}
    names = set(pooled) | set(control) | set(human_hits) | set(llm_hits) | set(precision_cells)
    for rule in sorted(names):
        entry: dict[str, Any] = {}
        if rule in pooled:
            entry["recall"] = _summarise(pooled[rule], resamples)
        if rule in seeded:
            entry["seeded_recall"] = _summarise(seeded[rule], resamples)
        if rule in adjudicated:
            entry["adjudicated_recall"] = _summarise(adjudicated[rule], resamples)
        if rule in control:
            entry["control_fpr"] = _summarise(control[rule], resamples)
        entry["human_findings"] = human_hits.get(rule, 0)
        entry["human_per_100_words"] = (
            100 * human_hits.get(rule, 0) / human_words if human_words else None
        )
        entry["llm_findings"] = llm_hits.get(rule, 0)
        entry["llm_per_100_words"] = (
            100 * llm_hits.get(rule, 0) / llm_words if llm_words else None
        )
        if rule in precision_cells:
            per_doc = [(h, n) for h, n in precision_cells[rule].values()]
            summary = _summarise(per_doc, resamples)
            entry["precision"] = {**summary, "tp": summary["hits"]}
        rules[rule] = entry

    for stats in origin_stats.values():
        stats["per_100_words"] = (
            100 * stats["findings"] / stats["words"] if stats["words"] else None
        )
    return {
        "schema": SCORE_SCHEMA,
        "split": split,
        "engine": results.get("engine"),
        "corpus": results.get("corpus"),
        "incomplete_documents": incomplete,
        "origins": dict(sorted(origin_stats.items())),
        "rules": rules,
    }


def load_verdicts(path: Path) -> dict[LabelKey, Label]:
    """Read a labels JSON Lines file into the map `score` takes."""
    rows = (json.loads(line) for line in path.read_text("utf-8").splitlines() if line)
    return {(r["doc"], r["rule"], r["line"]): r["label"] for r in rows}
