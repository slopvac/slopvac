"""Regenerate the slopvac-engine vocabulary parity fixture from Vale.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-engine/tests/fixtures/generate_vocabulary.py [--check]

The golden suite records no vocabulary findings, because its reference runs with an empty
config and so without a word blocklist. This fixture stays the only check of the sweep. 2.x has no native vocabulary check: it compiles the
shipped example blocklist into Vale ``sequence`` rules (one per part of speech) and runs
Vale, so Vale is the oracle. This fixture keeps those recorded results for the documents in
``vocabulary-parity.json``. ``vocabulary_parity.rs``
compares the native sweep with them.

The findings come from Vale 3.27 or later, the version that ``slopvac-pos`` is validated
against. Point ``SLOPVAC_VALE`` at it when ``vale`` on PATH is older. Without a usable Vale,
``--check`` replays the findings stored in the fixture and checks that the file still has
the form this script writes.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from slopvac.compile_vale import compile_ruleset
from slopvac.config import Config, VocabularySettings, resolve_for
from slopvac.pipeline import lint_one, vale_levels
from slopvac.rules import load_ruleset
from slopvac.vale import run_compiled_vale
from slopvac.vocabulary import load_blocklist

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FIXTURE = HERE / "vocabulary-parity.json"
BLOCKLIST = ROOT / "packages/slopvac-lint/examples/blocklist.toml"
OWNER = "ste-words.word-outside-controlled-vocabulary"
VALE = os.environ.get("SLOPVAC_VALE", "vale")
VALE_MINIMUM = (3, 27)


def to_2x(text: str) -> str:
    """The 3.0 directive form to the 2.x form: ``rule=core:cat.id`` to ``rule=cat.id``."""
    return text.replace("rule=core:", "rule=")


def usable_vale() -> bool:
    """Whether ``VALE`` runs and is at least ``VALE_MINIMUM``."""
    try:
        out = subprocess.run(
            [VALE, "--version"], capture_output=True, text=True, check=False
        )
    except OSError:
        return False
    found = re.search(r"(\d+)\.(\d+)\.(\d+)", out.stdout)
    return bool(found) and tuple(map(int, found.groups()))[:2] >= VALE_MINIMUM


def vocabulary_findings(rules, docs: list[tuple[str, str]]) -> dict[str, list]:
    """The findings of the vocabulary sweep for each document, as 2.x reports them.

    This is the pipeline of ``slopvac lint`` with the example blocklist and Vale. Native
    rules are switched off (``only=set()``), so what is left is the sweep and the directive
    checks; only the sweep's findings are kept.
    """
    sample = Path("fixture.md")
    base = Config(vocabulary=VocabularySettings(path=str(BLOCKLIST)))
    with tempfile.TemporaryDirectory() as tmp:
        paths = {}
        for index, (name, text) in enumerate(docs):
            paths[name] = Path(tmp) / f"{index:04}.md"
            paths[name].write_text(to_2x(text), encoding="utf-8")
        compiled = compile_ruleset(
            rules,
            resolve_for(base, sample),
            binary=VALE,
            validate=True,
            vocabulary=load_blocklist(BLOCKLIST),
        )
        severities, categories = vale_levels(compiled, rules, base, sample)
        result = run_compiled_vale(
            list(paths.values()), compiled, severities, categories, binary=VALE
        )
        return {
            name: [
                f
                for f in lint_one(path, base, rules, result, None, set()).findings
                if f.rule_id == OWNER
            ]
            for name, path in paths.items()
        }


def row(finding) -> dict:
    return {
        "rule": f"core:{OWNER}",
        "line": finding.line,
        "column": finding.column,
        "end_column": finding.end_column,
        "severity": finding.severity.value,
        "matched": finding.matched_text,
    }


def render(docs: list[dict]) -> str:
    lines = [json.dumps(d, ensure_ascii=False, sort_keys=True) for d in docs]
    return '{"documents":[\n' + ",\n".join(lines) + "\n]}\n"


def main() -> None:
    stored = json.loads(FIXTURE.read_text(encoding="utf-8"))["documents"]
    pairs = [(d["name"], d["text"]) for d in stored]
    check = "--check" in sys.argv
    if not usable_vale():
        if not check:
            raise SystemExit(
                f"the vocabulary findings need Vale {VALE_MINIMUM[0]}.{VALE_MINIMUM[1]} or later; set SLOPVAC_VALE"
            )
        # Replay: the stored findings stand in for Vale, and the file must round-trip.
        found = {d["name"]: d["findings"] for d in stored}
    else:
        rules = load_ruleset()
        swept = vocabulary_findings(rules, pairs)
        found = {name: [row(f) for f in findings] for name, findings in swept.items()}
    docs = [{"name": n, "text": t, "findings": found[n]} for n, t in pairs]
    text = render(docs)
    if check:
        if FIXTURE.read_text(encoding="utf-8") != text:
            raise SystemExit(
                "vocabulary-parity.json is stale; regenerate with generate_vocabulary.py and commit the result"
            )
        return
    FIXTURE.write_text(text, encoding="utf-8")
    print(f"{len(docs)} documents")


if __name__ == "__main__":
    main()
