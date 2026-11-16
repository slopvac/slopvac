"""Recall and precision of the diagram rule pack on the LLM-written diagram set.

The set lives in `evals/diagrams-llm/`: Markdown files with a fenced Mermaid or
PlantUML diagram and the prose around it, plus draw.io files. `manifest.json`
holds the provenance and `labels.json` the hand labels. A label says that one
rule should fire on one piece of text, so the score works per finding, not per
document.

    python -m evals.diagrams                      # build and run the engine example
    python -m evals.diagrams --engine '<command>' # any command that prints the same JSON
    python -m evals.diagrams --findings f.json    # score stored engine output, no engine

An engine command receives the staged file paths as arguments and prints a JSON
array of `{"file", "findings": [{"rule", "line", "matched"}]}`.
`crates/slopvac-engine/examples/diagram_eval.rs` is that command for the Rust
engine. The Python reference has no diagram extractor.

A label matches a finding of the same rule whose matched text contains the label
text. An empty label text matches any finding of the rule in the document, for
`diagram.prose-restates-labels`, which reports a paragraph. An unmatched label is
a miss and an unmatched finding is a false positive.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shlex
import subprocess
import sys
import tempfile
from collections import defaultdict
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path
from typing import Any

from . import metrics
from .corpuslib import EVALS_DIR

SET_DIR = EVALS_DIR / "diagrams-llm"
MANIFEST = SET_DIR / "manifest.json"
LABELS = SET_DIR / "labels.json"
SAMPLES = SET_DIR / "samples"
REPO_ROOT = EVALS_DIR.parent

RULES = (
    "diagram.label-run-on",
    "diagram.label-title-plus-gloss",
    "diagram.note-long",
    "diagram.meta-reference",
    "diagram.prose-restates-labels",
)
SUFFIX = {"markdown": ".md", "drawio": ".drawio"}
DEFAULT_ENGINE = (
    "cargo",
    "run",
    "--quiet",
    "-p",
    "slopvac-engine",
    "--example",
    "diagram_eval",
    "--",
)


class DiagramEvalError(Exception):
    pass


def load() -> tuple[dict[str, Any], dict[str, Any]]:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    labels = json.loads(LABELS.read_text(encoding="utf-8"))
    return manifest, labels


def verify(manifest: Mapping[str, Any], labels: Mapping[str, Any]) -> list[str]:
    """Return problems: a changed sample, or labels and manifest that disagree."""
    problems: list[str] = []
    ids = {s["id"] for s in manifest["samples"]}
    for sample in manifest["samples"]:
        path = SAMPLES / f"{sample['id']}.corpus"
        if not path.is_file():
            problems.append(f"{sample['id']}: sample file is missing")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != sample["sha256"]:
            problems.append(f"{sample['id']}: sha256 differs from the manifest")
    for extra in sorted(set(labels) - ids):
        problems.append(f"{extra}: labelled but not in the manifest")
    for missing in sorted(ids - set(labels)):
        problems.append(f"{missing}: in the manifest but not labelled")
    for sid, entry in labels.items():
        for label in entry["labels"]:
            if label["rule"] not in RULES:
                problems.append(f"{sid}: unknown rule {label['rule']}")
    return problems


def stage(manifest: Mapping[str, Any], target: Path) -> dict[str, str]:
    """Copy each sample under a name with the right extension; return path -> id."""
    staged: dict[str, str] = {}
    for sample in manifest["samples"]:
        path = target / f"{sample['id']}{SUFFIX[sample['format']]}"
        path.write_bytes((SAMPLES / f"{sample['id']}.corpus").read_bytes())
        staged[str(path)] = sample["id"]
    return staged


def run_engine(command: Sequence[str], paths: Iterable[str]) -> list[dict[str, Any]]:
    proc = subprocess.run(
        [*command, *paths],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        check=False,
    )
    if proc.returncode != 0:
        raise DiagramEvalError(
            f"engine exited {proc.returncode}: {proc.stderr.strip()[:500]}"
        )
    return json.loads(proc.stdout)


def _matches(label: Mapping[str, str], finding: Mapping[str, Any]) -> bool:
    return label["rule"] == finding["rule"] and label["text"] in finding["matched"]


def score(
    labels: Mapping[str, Any],
    findings_by_sample: Mapping[str, Sequence[Mapping[str, Any]]],
) -> dict[str, Any]:
    """Count true positives, misses and false positives per rule.

    `findings_by_sample` maps a sample id to its findings. A sample that has no
    entry counts as having no findings.
    """
    counts: dict[str, dict[str, Any]] = {
        r: {
            "labels": 0,
            "found": 0,
            "findings": 0,
            "correct": 0,
            "misses": [],
            "false_positives": [],
        }
        for r in RULES
    }
    for sid, entry in labels.items():
        found = [f for f in findings_by_sample.get(sid, ()) if f["rule"] in counts]
        for label in entry["labels"]:
            row = counts[label["rule"]]
            row["labels"] += 1
            if any(_matches(label, f) for f in found):
                row["found"] += 1
            else:
                row["misses"].append({"sample": sid, "text": label["text"]})
        for f in found:
            row = counts[f["rule"]]
            row["findings"] += 1
            if any(_matches(label, f) for label in entry["labels"]):
                row["correct"] += 1
            else:
                row["false_positives"].append(
                    {"sample": sid, "line": f["line"], "matched": f["matched"]}
                )
    for row in counts.values():
        row["recall"] = metrics.rate(row["found"], row["labels"])
        row["recall_ci"] = metrics.wilson(row["found"], row["labels"])
        row["precision"] = metrics.rate(row["correct"], row["findings"])
        row["precision_ci"] = metrics.wilson(row["correct"], row["findings"])
    return counts


def _pct(value: float | None) -> str:
    return "n/a" if value is None else f"{value:.2f}"


def _ci(interval: tuple[float, float] | None) -> str:
    return "" if interval is None else f"{interval[0]:.2f}-{interval[1]:.2f}"


def table(counts: Mapping[str, Mapping[str, Any]]) -> str:
    head = "| rule | labelled | found | recall | recall 95% CI | findings | correct | precision | precision 95% CI |"
    lines = [head, "|" + " --- |" * 9]
    for rule, c in counts.items():
        lines.append(
            f"| {rule} | {c['labels']} | {c['found']} | {_pct(c['recall'])} | {_ci(c['recall_ci'])} "
            f"| {c['findings']} | {c['correct']} | {_pct(c['precision'])} | {_ci(c['precision_ci'])} |"
        )
    return "\n".join(lines)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python -m evals.diagrams", description=__doc__.splitlines()[0]
    )
    parser.add_argument(
        "--engine",
        help="engine command that prints findings JSON (default: cargo example)",
    )
    parser.add_argument(
        "--findings", help="score stored engine output instead of running an engine"
    )
    parser.add_argument(
        "--out", help="write the full result, with misses and false positives, as JSON"
    )
    parser.add_argument(
        "--details", action="store_true", help="list each miss and false positive"
    )
    args = parser.parse_args(argv)

    manifest, labels = load()
    problems = verify(manifest, labels)
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 2
    if args.findings:
        by_sample = {
            Path(r["file"]).stem: r["findings"]
            for r in json.loads(Path(args.findings).read_text())
        }
    else:
        command = tuple(shlex.split(args.engine)) if args.engine else DEFAULT_ENGINE
        with tempfile.TemporaryDirectory(prefix="slopvac-diagram-eval-") as tmp:
            staged = stage(manifest, Path(tmp))
            try:
                results = run_engine(command, staged)
            except (DiagramEvalError, OSError, json.JSONDecodeError) as e:
                print(f"error: {e}", file=sys.stderr)
                return 2
        by_sample = defaultdict(list)
        for r in results:
            by_sample[staged[r["file"]]] = r["findings"]
    counts = score(labels, by_sample)
    print(f"{len(labels)} samples\n")
    print(table(counts))
    if args.details:
        for rule, c in counts.items():
            for kind in ("misses", "false_positives"):
                for item in c[kind]:
                    print(f"{rule} {kind[:-1].replace('_', ' ')}: {json.dumps(item)}")
    if args.out:
        Path(args.out).write_text(
            json.dumps({"samples": len(labels), "rules": counts}, indent=1) + "\n"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
