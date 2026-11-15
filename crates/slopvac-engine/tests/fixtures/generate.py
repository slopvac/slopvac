"""Regenerate the slopvac-engine parity fixture from the Python reference engine.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-engine/tests/fixtures/generate.py [--check]

The vocabulary findings come from Vale 3.27 or later, the version that ``slopvac-pos`` is
validated against (mise pins 3.24, whose tagger input differs). Point ``SLOPVAC_VALE`` at it
when ``vale`` on PATH is older. Without a usable Vale, ``--check`` replays the vocabulary
findings stored in the fixture and checks everything else.

STUB(slopvac-8l4.3.29): this interim fixture stands in for the golden suite (slopvac-8l4.2.7).
It runs the 2.x native engine (``engine.Engine.run``) and ``score.score_document`` over a frozen
corpus at the ``normal`` profile and records what it reports, so the Rust tests never take an
expected value from Rust code. The fixture holds:

* ``settings``: the levels, category weights and budgets and thresholds that 2.x resolved, so
  the Rust engine applies them as plain data. A rule of a kind the Rust engine does not run
  yet (structure, capitalization, and the metrics in ``METRICS``) is ``off`` here and excluded from the
  Python run through ``Engine(only=...)``. The vocabulary rules run at the level 2.x resolves.
* ``documents``: the frozen texts (categories without findings are left out), in the 3.0 directive form (``rule=core:<id>``), and the
  findings (rule, line, column, severity, matched text), counts and scores for each.

``--check`` recomputes the expectations for the stored texts and fails when they differ.
Without it the corpus is rebuilt from the rule examples, repository prose and the samples below.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from slopvac.analyze import parse
from slopvac.compile_vale import compile_ruleset
from slopvac.config import Config, Severity, VocabularySettings, resolve_for
from slopvac.engine import Engine
from slopvac.model import Finding, RuleKind
from slopvac.pipeline import lint_one, vale_levels
from slopvac.rules import load_ruleset
from slopvac.score import score_document
from slopvac.vale import run_compiled_vale
from slopvac.vocabulary import load_blocklist

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FIXTURE = HERE / "engine-parity.json"

LEXICAL = {RuleKind.TOKENS, RuleKind.PATTERN, RuleKind.SUBSTITUTION}

# The metrics the Rust engine measures (slopvac-8l4.3.9). A metric finding has a line and
# no column or range, so the fixture records its end column as 0.
METRICS = {
    "sentence_words",
    "clause_boundaries",
    "lead_in_words",
    "paragraph_sentences",
    "enumerating_paragraph_words",
    "multiword_noun_words",
    "coordinated_items",
    "adjectives_per_noun",
    "consecutive_bold_colon_bullets",
    "bold_spans_per_1000_words",
    "dash_per_1000_words",
}


def runs_in_rust(rule) -> bool:
    return rule.kind in LEXICAL or (
        rule.kind is RuleKind.METRIC and rule.metric in METRICS
    )


# The vocabulary sweep. 2.x has no native implementation: it compiles the blocklist into Vale
# ``sequence`` rules (one per part of speech) and runs Vale, so Vale is the oracle for these
# findings. The sweep reports under one owner rule. The blocklist is the one 2.x ships as the
# example; ``parity.rs`` loads the same file.
BLOCKLIST = ROOT / "packages/slopvac-lint/examples/blocklist.toml"
OWNER = "ste-words.word-outside-controlled-vocabulary"
VALE = os.environ.get("SLOPVAC_VALE", "vale")
VALE_MINIMUM = (3, 27)

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
    "sample/metrics-sentences": (
        "# Limits\n\n"
        "Open the valve and check that the pressure gauge reads the value that the plan names, "
        "then close the valve and record the reading in the log for the shift.\n\n"
        "The container orchestration platform migration strategy covers every deployment, "
        "and the monitoring plan, which the team wrote, lists all alerts; however, nobody "
        "reviewed it, so the rollout waits, and the owners disagree on the schedule.\n\n"
        "The plan names the valves, the gauges, the pumps, and the filters of the plant: "
        "the shift lead checks them before each run on the line today.\n\n"
        "We ship alpha, beta, gamma, and delta. We test red, green, blue, and amber. "
        "We fix one, two, three, and four. We note a or b.\n"
    ),
    "sample/metrics-paragraphs": (
        "One. Two. Three. Four. Five. Six. Seven.\n\n"
        "- One. Two. Three. Four. Five. Six. Seven.\n\n"
        "> One. Two. Three. Four. Five. Six. Seven.\n"
    ),
    "sample/metrics-markup": (
        "---\ntitle: a — b — c\n---\n\n"
        "# Dashes\n\n"
        "This is useful — very useful — and so is this -- really -- and this — too — now.\n\n"
        "**Bold one** and **bold two** and __bold three__ appear in a short text.\n\n"
        "```text\n**code bold** — dash\n```\n\n"
        "- **Speed:** fast.\n- **Cost**: low.\n- __Risk:__ small.\n\n- **Reach:** wide.\n"
        "- **Plain** none.\n"
    ),
    "sample/crlf": "# Title\r\n\r\nWe utilize the tool\r\nin order to leverage it.\r\n\r\nSecond utilize paragraph.\r\n",
    "sample/vocabulary": (
        "# Deploy the app\n\nThe deploy failed. We deploy it again.\n\n"
        "The `deploy` flag sets deploy-time mode.\n\nPlease request the data.\n\n"
        "This is <em>easy</em> to do, and the <b>request</b> works.\n\n"
        "| Step | Action |\n| --- | --- |\n| 1 | Please <b>request</b> the data quickly |\n\n"
        "- Execute the plan correctly\n- Added the key\n\n"
        "<table>\n<tr><td>request</td></tr>\n</table>\n"
    ),
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
    return [r for r in engine.rules if runs_in_rust(r)], engine


def settings(rules, config):
    scope, engine = in_scope(rules, config)
    scoped = {r.qualified_id for r in scope}
    levels = {}
    for rule in rules.rules:
        key = f"core:{rule.qualified_id}"
        runs = rule.qualified_id in scoped or rule.kind is RuleKind.VOCABULARY
        levels[key] = engine.severity_for(rule).value if runs else "off"
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


def stored_vocabulary(rules, stored: dict) -> dict[str, list]:
    """The sweep's findings in the fixture, as the model objects ``score_document`` reads."""
    rule = rules.by_id(OWNER)
    return {
        d["name"]: [
            Finding(
                path=d["name"],
                line=f["line"],
                column=f["column"],
                end_column=f["end_column"],
                rule_id=OWNER,
                category=rule.category,
                severity=Severity(f["severity"]),
                message="",
                ai_signal=rule.ai_signal,
                matched_text=f["matched"],
            )
            for f in d["expected"]["findings"]
            if f["rule"] == f"core:{OWNER}"
        ]
        for d in stored["documents"]
    }


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


def expect(rules, config, name: str, text: str, vocabulary: list) -> dict:
    scope, _ = in_scope(rules, config)
    engine = Engine(rules.rules, config, only={r.qualified_id for r in scope})
    source = to_2x(text)
    document = parse(name, source)
    findings = sorted(
        [*engine.run(document), *vocabulary], key=lambda f: (f.line, f.column)
    )
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
                "end_column": end_column(f) or 0,
                "severity": f.severity.value,
                # The 3.0 form of a directive carries the pack, so the text a finding quotes does.
                "matched": f.matched_text.replace("rule=", "rule=core:")
                if f.rule_id == "meta.invalid-suppression"
                else f.matched_text,
            }
            for f in findings
        ],
    }


def end_column(finding) -> int:
    """The end column in the 3.0 text: a directive quotes its rule id with the pack."""
    if finding.rule_id == "meta.invalid-suppression":
        return finding.end_column + len("core:")
    return finding.end_column


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


def render(
    rules, config, docs: list[tuple[str, str]], vocabulary: dict[str, list]
) -> str:
    out = {
        "settings": settings(rules, config),
        "documents": [
            {
                "name": n,
                "text": t,
                "expected": expect(rules, config, n, t, vocabulary[n]),
            }
            for n, t in docs
        ],
    }
    # One document per line keeps the diff readable and the file small.
    lines = [
        json.dumps(d, ensure_ascii=False, sort_keys=True) for d in out["documents"]
    ]
    head = json.dumps(out["settings"], ensure_ascii=False, sort_keys=True)
    return '{"settings":' + head + ',"documents":[\n' + ",\n".join(lines) + "\n]}\n"


def main() -> None:
    rules, config = load()
    if "--check" in sys.argv:
        stored = json.loads(FIXTURE.read_text(encoding="utf-8"))
        docs = [(d["name"], d["text"]) for d in stored["documents"]]
        vocabulary = (
            vocabulary_findings(rules, docs)
            if usable_vale()
            else stored_vocabulary(rules, stored)
        )
        if FIXTURE.read_text(encoding="utf-8") != render(
            rules, config, docs, vocabulary
        ):
            raise SystemExit(
                "engine-parity.json is stale; regenerate with generate.py and commit the result"
            )
        return
    scope, _ = in_scope(rules, config)
    if not usable_vale():
        raise SystemExit(
            f"the vocabulary findings need Vale {VALE_MINIMUM[0]}.{VALE_MINIMUM[1]} or later; set SLOPVAC_VALE"
        )
    docs = corpus(rules, scope)
    FIXTURE.write_text(
        render(rules, config, docs, vocabulary_findings(rules, docs)), encoding="utf-8"
    )
    print(f"{len(docs)} documents, {len(scope)} rules")


if __name__ == "__main__":
    main()
