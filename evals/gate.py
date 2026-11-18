"""Precision/recall ratchet: compare a scorecard with a committed baseline.

A rule regresses when its current estimate is worse than the baseline point
estimate by more than sampling noise. Noise is the wider of the Wilson interval
and the document-cluster bootstrap interval, so a rule measured on a few
documents cannot fail on one unlucky resample.

Measured per rule and per split:

- recall on seeded positives (higher is better),
- precision on adjudicated labels (higher is better),
- false-positive rate on clean controls (lower is better),
- findings on pre-2022 human text, scaled by word count (lower is better).

`report` mode lists regressions and exits 0. `enforce` mode exits 1 on any.
"""

from __future__ import annotations

import json
import math
from collections import Counter
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

from . import labelling, metrics

BASELINE_SCHEMA = 1
BASELINE_DIR = Path(__file__).resolve().parent / "baselines"
SPLITS = ("tuning", "holdout")
MIN_N = 10  # a rate measured on fewer cases is listed as uncertain, never failed
HUMAN_SLACK = 2.0  # standard deviations of Poisson noise on the human-text count

# metric -> (scorecard key, hit-count key, direction); direction +1 = higher is better
COUNT_METRICS = {
    "recall": ("recall", "hits", +1),
    "control_fpr": ("control_fpr", "hits", -1),
    "precision": ("precision", "tp", +1),
}


def slim(card: Mapping[str, Any]) -> dict[str, Any]:
    """Reduce one split's scorecard to the counts a baseline needs."""
    rules: dict[str, Any] = {}
    for rule, entry in card["rules"].items():
        out: dict[str, Any] = {"human_findings": entry["human_findings"]}
        for name, (key, hits_key, _) in COUNT_METRICS.items():
            if key in entry:
                out[name] = {"hits": entry[key][hits_key], "n": entry[key]["n"]}
        rules[rule] = out
    return {"human_words": _human_words(card), "rules": rules}


def _human_words(card: Mapping[str, Any]) -> int:
    return int(card["origins"].get("human", {}).get("words", 0))


def make_baseline(cards: Mapping[str, Mapping[str, Any]]) -> dict[str, Any]:
    """Build a baseline from per-split scorecards."""
    first = next(iter(cards.values()))
    engine = first.get("engine") or {}
    return {
        "schema": BASELINE_SCHEMA,
        # The command holds local paths, so the baseline keeps only identity.
        "engine": {"name": engine.get("name"), "version": engine.get("version")},
        "corpus": first.get("corpus"),
        "splits": {split: slim(card) for split, card in cards.items()},
    }


def load_baseline(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != BASELINE_SCHEMA:
        raise ValueError(f"{path}: unsupported baseline schema {data.get('schema')!r}")
    return data


def dump_baseline(baseline: Mapping[str, Any], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(baseline, indent=1, sort_keys=True) + "\n", encoding="utf-8"
    )


def _noise_bounds(src: Mapping[str, Any], hits_key: str) -> tuple[float, float]:
    """Wider of the Wilson and bootstrap intervals around the current estimate."""
    lows: list[float] = []
    highs: list[float] = []
    wilson = metrics.wilson(src[hits_key], src["n"])
    if wilson:
        lows.append(wilson[0])
        highs.append(wilson[1])
    boot = src.get("bootstrap")
    if boot and boot[0] is not None:
        lows.append(boot[0])
        highs.append(boot[1])
    return min(lows), max(highs)


def _compare_counts(
    where: dict[str, Any],
    name: str,
    was: Mapping[str, Any],
    now_entry: Mapping[str, Any] | None,
    min_n: int,
    positives: int | None,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    """Return (regression, uncertain) for one count metric of one rule."""
    key, hits_key, direction = COUNT_METRICS[name]
    p0 = was["hits"] / was["n"]
    if now_entry is None or key not in now_entry:
        return {**where, "was": p0, "now": None, "reason": "no longer measured"}, None
    src = now_entry[key]
    if src["n"] < min_n or was["n"] < min_n:
        return None, {**where, "n": src["n"]}
    if name == "precision" and (
        positives is None or positives < labelling.MIN_POSITIVES
    ):
        # Too few adjudicated true positives to trust the label set.
        return None, {**where, "n": src["n"], "positives": positives or 0}
    low, high = _noise_bounds(src, hits_key)
    worse = high < p0 if direction > 0 else low > p0
    if not worse:
        return None, None
    now = src[hits_key] / src["n"]
    return {**where, "was": p0, "now": now, "low": low, "high": high}, None


def _human_regression(
    where: dict[str, Any],
    base: Mapping[str, Any],
    was: Mapping[str, Any],
    card: Mapping[str, Any],
) -> dict[str, Any] | None:
    words_then, words_now = base["human_words"], _human_words(card)
    if not words_then or not words_now:
        return None
    expected = was["human_findings"] * words_now / words_then
    limit = expected + HUMAN_SLACK * math.sqrt(max(expected, 1.0)) + 1
    now = card["rules"].get(where["rule"], {}).get("human_findings", 0)
    if now <= limit:
        return None
    return {
        **where,
        "metric": "human_findings",
        "was": expected,
        "now": now,
        "limit": limit,
    }


def compare(
    baseline: Mapping[str, Any],
    cards: Mapping[str, Mapping[str, Any]],
    *,
    min_n: int = MIN_N,
    labels: Sequence[Mapping[str, Any]] | None = None,
) -> dict[str, Any]:
    """Compare per-split scorecards with a baseline; return regressions and notes.

    `labels` are the adjudicated labels (rows with `rule` and `label`). A rule's
    precision is gated only when it has `labelling.MIN_POSITIVES` adjudicated true
    positives, however many the baseline detected. Without labels precision is not
    measured, and the gate does not pass if the baseline carries precision.
    """
    have = Counter(lbl["rule"] for lbl in labels or () if lbl["label"] == "tp")
    regressions: list[dict[str, Any]] = []
    uncertain: list[dict[str, Any]] = []
    inadequate: dict[str, dict[str, Any]] = {}
    gates_precision = False
    for split, card in cards.items():
        base = baseline["splits"].get(split)
        if base is None:
            continue
        # A rule the baseline never saw has a baseline of zero: no findings on
        # human text and no false positives on controls.
        for rule, entry in card["rules"].items():
            llm = entry.get("llm_findings", 0)
            if llm and have[rule] < labelling.MIN_POSITIVES:
                seen = inadequate.get(rule, {}).get("llm_findings", 0)
                inadequate[rule] = {
                    "rule": rule,
                    "positives": have[rule],
                    "llm_findings": max(llm, seen),
                }
        for rule in sorted(set(base["rules"]) | set(card["rules"])):
            now_entry = card["rules"].get(rule)
            was = dict(base["rules"].get(rule, {}))
            was.setdefault("human_findings", 0)
            if "control_fpr" not in was and now_entry and "control_fpr" in now_entry:
                was["control_fpr"] = {"hits": 0, "n": now_entry["control_fpr"]["n"]}
            for name in COUNT_METRICS:
                if name not in was or not was[name]["n"]:
                    continue
                where = {"split": split, "rule": rule, "metric": name}
                gates_precision |= name == "precision"
                reg, unc = _compare_counts(
                    where,
                    name,
                    was[name],
                    now_entry,
                    min_n,
                    have[rule] if labels else None,
                )
                if reg:
                    regressions.append(reg)
                if unc:
                    uncertain.append(unc)
            reg = _human_regression({"split": split, "rule": rule}, base, was, card)
            if reg:
                regressions.append(reg)
    measured = bool(labels)
    return {
        "regressions": regressions,
        "uncertain": uncertain,
        "inadequate_coverage": [inadequate[r] for r in sorted(inadequate)],
        "precision": "measured" if measured else "not measured",
        "passed": not regressions and (measured or not gates_precision),
    }


def fmt(value: float | None) -> str:
    return "-" if value is None else f"{value:.3f}"


def markdown(result: Mapping[str, Any], mode: str) -> str:
    regs = result["regressions"]
    if regs:
        head = f"{len(regs)} regression(s) beyond the noise floor ({mode} mode)."
    else:
        head = "No rule regressed beyond the noise floor."
    lines = ["### Eval ratchet", "", head, ""]
    if regs:
        lines += [
            "| Split | Rule | Metric | Baseline | Now | Interval |",
            "| --- | --- | --- | --- | --- | --- |",
        ]
        for r in regs:
            if "low" in r:
                detail = f"{fmt(r['low'])} to {fmt(r['high'])}"
            else:
                detail = r.get("reason") or f"limit {fmt(r.get('limit'))}"
            lines.append(
                f"| {r['split']} | `{r['rule']}` | {r['metric']} "
                f"| {fmt(r['was'])} | {fmt(r['now'])} | {detail} |"
            )
        lines.append("")
    if result["precision"] == "not measured" and not result["passed"]:
        lines.append(
            "Precision not measured: no adjudicated labels were supplied, "
            "so the gate does not pass."
        )
        lines.append("")
    if result["inadequate_coverage"]:
        rules = ", ".join(f"`{c['rule']}`" for c in result["inadequate_coverage"])
        lines.append(
            f"Inadequate label coverage (fewer than {labelling.MIN_POSITIVES} "
            f"adjudicated true positives for findings on LLM text): {rules}."
        )
        lines.append("")
    if result["uncertain"]:
        lines.append(
            f"{len(result['uncertain'])} rate(s) rest on too few cases or, for precision, "
            f"fewer than {labelling.MIN_POSITIVES} adjudicated true positives and were not gated."
        )
    return "\n".join(lines) + "\n"
