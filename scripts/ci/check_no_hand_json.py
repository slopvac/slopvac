"""Keep the no-hand-json allow list honest and shrink-only.

The allow list lives in the ``ignores`` of ast-grep/rules/no-hand-json.yml. An entry carries
the annotation ``# allow <category> <count>``. This check strips those entries, runs the rule,
and compares what it finds with the list:

* every allow-listed file still has a finding, so an entry whose sites are gone must go;
* every pinned count equals the actual count, so removing some sites needs a lower count;
* every ``# total <category> <n>`` line equals the sum of its entries;
* a TEMPORARY category names a bead as ``TEMPORARY(slopvac-...)``.

Usage: python3 scripts/ci/check_no_hand_json.py [ROOT]   (needs ast-grep on PATH)
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path
from typing import NamedTuple

RULE = Path("ast-grep") / "rules" / "no-hand-json.yml"
PERMANENT = {"schema-generation", "config-leaf", "digest-input", "validator-fragment"}
TEMPORARY = re.compile(r"TEMPORARY\(slopvac-[a-z0-9]+(?:\.[a-z0-9]+)*\)")
ENTRY = re.compile(r"^\s*- (?P<path>\S+)\s+# allow (?P<category>\S+) (?P<count>\d+)\s*$")
TOTAL = re.compile(r"^# total (?P<category>\S+) (?P<count>\d+)\s*$")


class Entry(NamedTuple):
    path: str
    category: str
    count: int


def parse(text: str) -> tuple[list[Entry], dict[str, int]]:
    """Return the allow-list entries and the declared category totals."""
    entries: list[Entry] = []
    totals: dict[str, int] = {}
    for line in text.splitlines():
        if match := ENTRY.match(line):
            entries.append(Entry(match["path"], match["category"], int(match["count"])))
        elif match := TOTAL.match(line):
            totals[match["category"]] = int(match["count"])
    return entries, totals


def without_allow_list(text: str) -> str:
    """Return the rule text with every allow-list entry removed."""
    return "".join(
        line for line in text.splitlines(keepends=True) if not ENTRY.match(line)
    )


def problems(entries: list[Entry], totals: dict[str, int], found: Counter[str]) -> list[str]:
    """Compare the allow list with the findings of the rule that has no allow list."""
    errors: list[str] = []
    sums: Counter[str] = Counter()
    seen: set[str] = set()
    for entry in entries:
        if entry.path in seen:
            errors.append(f"{entry.path}: listed twice")
        seen.add(entry.path)
        if entry.category not in PERMANENT and not TEMPORARY.fullmatch(entry.category):
            errors.append(
                f"{entry.path}: unknown category {entry.category!r}; use one of "
                f"{sorted(PERMANENT)} or TEMPORARY(slopvac-<bead>)"
            )
        sums[entry.category] += entry.count
        actual = found.get(entry.path, 0)
        if actual == 0:
            errors.append(f"{entry.path}: no finding left, delete the entry")
        elif actual != entry.count:
            errors.append(
                f"{entry.path}: pinned {entry.count} but the rule finds {actual}; "
                "update the count and the category total"
            )
    for category in sorted(set(sums) | set(totals)):
        if sums[category] != totals.get(category):
            errors.append(
                f"total {category}: declared {totals.get(category)}, entries sum to "
                f"{sums[category]}"
            )
    return errors


def scan(root: Path, rule_text: str) -> Counter[str]:
    """Run the rule from ``root`` and count its findings by file."""
    with tempfile.TemporaryDirectory() as tmp:
        rule = Path(tmp) / "no-hand-json.yml"
        rule.write_text(rule_text)
        done = subprocess.run(
            ["ast-grep", "scan", "--rule", str(rule), "--json=compact"],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
    if done.returncode not in (0, 1) or not done.stdout.strip():
        if done.returncode == 0 and not done.stdout.strip():
            return Counter()
        raise RuntimeError(f"ast-grep exited {done.returncode}: {done.stderr.strip()}")
    return Counter(item["file"] for item in json.loads(done.stdout))


def check(root: Path) -> list[str]:
    text = (root / RULE).read_text()
    entries, totals = parse(text)
    return problems(entries, totals, scan(root, without_allow_list(text)))


def main(argv: list[str]) -> int:
    root = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parents[2]
    errors = check(root)
    for error in errors:
        print(f"no-hand-json allow list: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
