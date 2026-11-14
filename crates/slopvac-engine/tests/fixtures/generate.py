"""Regenerate the slopvac-engine parity fixture from the Python reference engine.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-engine/tests/fixtures/generate.py [--check]

STUB(slopvac-8l4.3.29): this interim fixture stands in for the golden suite (slopvac-8l4.2.7).
It runs the 2.x native engine (``engine.Engine.run``) and ``score.score_document`` over a frozen
corpus at the ``normal`` profile and records what it reports, so the Rust tests never take an
expected value from Rust code. The fixture holds:

* ``settings``: the levels, category weights and budgets and thresholds that 2.x resolved, so
  the Rust engine applies them as plain data. A rule of a kind the Rust engine does not run
  yet (metric, structure, vocabulary, capitalization) is ``off`` here and excluded from the
  Python run through ``Engine(only=...)``.
* ``documents``: the frozen texts (categories without findings are left out), in the 3.0 directive form (``rule=core:<id>``), and the
  findings (rule, line, column, severity, matched text), counts and scores for each.

``--check`` recomputes the expectations for the stored texts and fails when they differ.
Without it the corpus is rebuilt from the rule examples, repository prose and the samples below.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from slopvac.analyze import parse
from slopvac.config import Config, Severity, resolve_for
from slopvac.engine import Engine
from slopvac.model import RuleKind
from slopvac.rules import load_ruleset
from slopvac.score import score_document

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FIXTURE = HERE / "engine-parity.json"

LEXICAL = {RuleKind.TOKENS, RuleKind.PATTERN, RuleKind.SUBSTITUTION}

PROSE_SOURCES = [
    "README.md",
    "CONTRIBUTING.md",
    "docs/design/slopvac-3.md",
    "docs/contract/lint-report.md",
    "docs/contract/config.md",
    "docs/migrate-2-to-3.md",
]

ALLOW = "<!-- slopvac-allow: rule=core:orwell.stale-figure reason=dead-metaphor -->"

SAMPLES = {
    "sample/plain": (
        "# Notes\n\nWe utilize the tool in order to leverage a robust, seamless approach.\n"
        "It is important to note that the result is very good.\n"
    ),
    "sample/front-matter": (
        "---\ntitle: We utilize leverage in order to win\n---\n\n"
        "# Heading\n\nPlease utilize the tool to leverage the cache.\n"
    ),
    "sample/lists-and-tables": (
        "# Plan\n\n- We utilize the cache.\n- Leverage the tool in order to win.\n\n"
        "| Name | Note |\n| --- | --- |\n| a | utilize it |\n| b | leverage it |\n\n"
        "1. Open the valve slowly.\n2. Close the valve.\n"
    ),
    "sample/code": (
        "Use `utilize` in code.\n\n```python\nx = 'utilize leverage'\n```\n\n"
        "    indented utilize code\n\nAfter the code we utilize it.\n"
    ),
    "sample/quotation": (
        'Never write "tip of the iceberg" in a report. The tip of the iceberg is common.\n\n'
        "The phrase \u201cdeep dive\u201d is banned. A deep dive is not allowed.\n"
    ),
    "sample/directives": (
        f"{ALLOW}\nThis is the tip of the iceberg, and the tip of the iceberg again.\n\n"
        "<!-- slopvac-allow: rule=core:orwell.stale-figure reason=bogus -->\n"
        "Another tip of the iceberg here.\n\n"
        "<!-- slopvac-allow: rule=core:orwell.stale-figure -->\nA deep dive here.\n\n"
        "<!-- slopvac-allow: rule=core:orwell.nope reason=x -->\nPlain text.\n\n"
        "<!-- slopvac-disable-next-line -->\nWe utilize this line.\n\nWe utilize that line.\n\n"
        "<!-- slopvac-disable -->\n\nHidden utilize text.\n\n<!-- slopvac-enable -->\n\n"
        "Shown utilize text.\n\n"
        "```md\n<!-- slopvac-disable -->\n```\n\nStill utilize here.\n\n"
        "Write `<!-- slopvac-disable -->` to stop.\n\nAnd utilize again.\n"
    ),
    "sample/crlf": "# Title\r\n\r\nWe utilize the tool\r\nin order to leverage it.\r\n\r\nSecond utilize paragraph.\r\n",
    "sample/entities": (
        "We &amp; they utilize the tool; in&nbsp;order to win we leverage **bold utilize** "
        "and *emphasis leverage* and [a link to utilize](https://example.com).\n"
    ),
}


def load():
    rules = load_ruleset()
    config = resolve_for(Config(), Path("fixture.md"))
    return rules, config


def in_scope(rules, config):
    engine = Engine(rules.rules, config)
    return [r for r in engine.rules if r.kind in LEXICAL], engine


def settings(rules, config):
    scope, engine = in_scope(rules, config)
    scoped = {r.qualified_id for r in scope}
    levels = {}
    for rule in rules.rules:
        key = f"core:{rule.qualified_id}"
        levels[key] = (
            engine.severity_for(rule).value if rule.qualified_id in scoped else "off"
        )
    categories = {}
    for name, weight in rules.weights.items():
        dials = config.categories.get(name)
        w = weight
        if dials is not None and dials.weight is not None:
            w = dials.weight
        if dials is not None and dials.severity is Severity.OFF:
            w = 0.0
        categories[name] = {
            "weight": w,
            "budget": dials.max_per_100_words if dials is not None else None,
        }
    t = config.thresholds
    return {
        "profile": config.profile.value,
        "rule_levels": levels,
        "categories": categories,
        "thresholds": {
            "max_errors": t.max_errors,
            "max_warnings": t.max_warnings,
            "max_unicode_dashes": t.max_unicode_dashes,
            "max_total_per_100_words": t.max_total_per_100_words,
            "min_score": t.min_score,
        },
    }


def to_2x(text: str) -> str:
    """The 3.0 directive form to the 2.x form: ``rule=core:cat.id`` to ``rule=cat.id``."""
    return text.replace("rule=core:", "rule=")


def expect(rules, config, name: str, text: str) -> dict:
    scope, _ = in_scope(rules, config)
    engine = Engine(rules.rules, config, only={r.qualified_id for r in scope})
    source = to_2x(text)
    document = parse(name, source)
    findings = engine.run(document)
    scored = score_document(
        name,
        findings,
        document.words,
        len(document.sentences),
        len(document.paragraphs),
        config,
        rules.weights,
    )
    return {
        "words": scored.words,
        "sentences": scored.sentences,
        "paragraphs": scored.paragraphs,
        "errors": scored.errors,
        "warnings": scored.warnings,
        "suggestions": scored.suggestions,
        "score": scored.score,
        "passed": scored.passed,
        "reasons": scored.failure_reasons,
        "categories": [
            {
                "category": c.category,
                "findings": c.findings,
                "errors": c.errors,
                "warnings": c.warnings,
                "suggestions": c.suggestions,
                "per_100_words": c.per_100_words,
                "gating_per_100_words": c.gating_per_100_words,
                "budget": c.budget,
                "score": c.score,
                "over_budget": c.over_budget,
            }
            for c in scored.categories
            if c.findings
        ],
        "findings": [
            {
                "rule": f"core:{f.rule_id}",
                "line": f.line,
                "column": f.column,
                "severity": f.severity.value,
                # The 3.0 form of a directive carries the pack, so the text a finding quotes does.
                "matched": f.matched_text.replace("rule=", "rule=core:")
                if f.rule_id == "meta.invalid-suppression"
                else f.matched_text,
            }
            for f in findings
        ],
    }


def corpus(rules, scope) -> list[tuple[str, str]]:
    docs: list[tuple[str, str]] = list(SAMPLES.items())
    for path in PROSE_SOURCES:
        docs.append((path, (ROOT / path).read_text(encoding="utf-8")))
    for rule in scope:
        for index, example in enumerate(rule.examples):
            bad, good = example.bad, example.good or ""
            docs.append(
                (
                    f"example/{rule.qualified_id}/{index}",
                    f"{bad}\n\n{good}\n" if good else f"{bad}\n",
                )
            )
    return docs


def render(rules, config, docs: list[tuple[str, str]]) -> str:
    out = {
        "settings": settings(rules, config),
        "documents": [
            {"name": n, "text": t, "expected": expect(rules, config, n, t)}
            for n, t in docs
        ],
    }
    # One document per line keeps the diff readable and the file small.
    lines = [json.dumps(d, ensure_ascii=False, sort_keys=True) for d in out["documents"]]
    head = json.dumps(out["settings"], ensure_ascii=False, sort_keys=True)
    return '{"settings":' + head + ',"documents":[\n' + ",\n".join(lines) + "\n]}\n"


def main() -> None:
    rules, config = load()
    if "--check" in sys.argv:
        stored = json.loads(FIXTURE.read_text(encoding="utf-8"))
        docs = [(d["name"], d["text"]) for d in stored["documents"]]
        if FIXTURE.read_text(encoding="utf-8") != render(rules, config, docs):
            raise SystemExit(
                "engine-parity.json is stale; regenerate with generate.py and commit the result"
            )
        return
    scope, _ = in_scope(rules, config)
    docs = corpus(rules, scope)
    FIXTURE.write_text(render(rules, config, docs), encoding="utf-8")
    print(f"{len(docs)} documents, {len(scope)} lexical rules")


if __name__ == "__main__":
    main()
