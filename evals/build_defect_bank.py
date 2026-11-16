"""Snapshot the rule examples that the seeded-defect generator draws from.

    uv run python -m evals.build_defect_bank

Each entry pairs a sentence a rule flags (`bad`) with a minimal edit that it
leaves alone (`good`). The bank is committed so the generators run offline and
stay deterministic when the rules change; refresh it on purpose and review the
diff. Needs the `slopvac` package, unlike the rest of this directory.
"""

from __future__ import annotations

import json
from importlib import metadata

from . import corpuslib as lib

BANK_PATH = lib.EVALS_DIR / "defect-bank.json"


def build() -> dict:
    from slopvac.rules import load_ruleset

    entries = []
    for rule in load_ruleset().rules:
        if rule.scope.value != "prose":
            continue
        for index, example in enumerate(rule.examples):
            bad, good = example.bad.strip(), (example.good or "").strip()
            if not good or good == bad or "\n" in bad or "\n" in good:
                continue
            if not 25 <= len(bad) <= 220:
                continue
            entries.append(
                {
                    "rule": rule.qualified_id,
                    "kind": rule.kind.value,
                    "ai_signal": rule.ai_signal,
                    "example": index,
                    "bad": bad,
                    "good": good,
                }
            )
    entries.sort(key=lambda e: (e["rule"], e["example"]))
    return {
        "schema": 1,
        "slopvac_version": metadata.version("slopvac"),
        "entries": entries,
    }


def main() -> int:
    bank = build()
    BANK_PATH.write_text(
        json.dumps(bank, indent=1, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(
        f"{len(bank['entries'])} examples from {len({e['rule'] for e in bank['entries']})} rules"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
