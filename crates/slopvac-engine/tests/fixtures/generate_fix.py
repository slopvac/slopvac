"""Regenerate the slopvac-engine fix fixture from the Python reference (2.x ``--fix``).

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-engine/tests/fixtures/generate_fix.py [--check]

For each document it runs the 2.x native engine over the lexical and metric rules that the
Rust engine runs, and then ``diff_scope.apply_replacements`` on a real file, the code that
``slopvac lint --fix`` runs. The fixture stores the text before and after. The Rust tests
never take an expected value from Rust code. Documents are the ones already in
``fix-parity.json`` and the ``SAMPLES`` below, which cover case, CRLF, links, code,
front matter, wrapped matches and directives. The golden suite records no ``--fix`` output,
so this fixture stays the reference for fixes.

``--check`` recomputes the stored texts and fails when the expectations differ.
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace

from slopvac.analyze import parse
from slopvac.config import Config, resolve_for
from slopvac.diff_scope import apply_replacements
from slopvac.engine import Engine
from slopvac.model import RuleKind
from slopvac.rules import load_ruleset

HERE = Path(__file__).resolve().parent
FIXTURE = HERE / "fix-parity.json"

LEXICAL = {RuleKind.TOKENS, RuleKind.PATTERN, RuleKind.SUBSTITUTION}

# The metrics the Rust engine measures.
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


def load():
    rules = load_ruleset()
    config = resolve_for(Config(), Path("fixture.md"))
    return rules, config


def in_scope(rules, config):
    engine = Engine(rules.rules, config)
    return [r for r in engine.rules if runs_in_rust(r)], engine


def to_2x(text: str) -> str:
    """The 3.0 directive form to the 2.x form: ``rule=core:cat.id`` to ``rule=cat.id``."""
    return text.replace("rule=core:", "rule=")

SAMPLES = {
    "fix/case": "We Utilize the tool. UTILIZE it now. We utilize it too.\n",
    "fix/crlf": "# Title\r\n\r\nWe utilize the tool\r\nin order to win.\r\n\r\nSecond utilize here.\r\n",
    "fix/code-span": "Run `utilize` and utilize it, or `utilize` it.\n",
    "fix/link": "[utilize](https://example.com/utilize) and utilize the tool.\n",
    "fix/front-matter": "---\ntitle: utilize in order to\n---\n\nPlease utilize this.\n",
    "fix/wrapped": "We do this in order\nto win, and utilize it.\n",
    "fix/code-block": "Utilize now.\n\n```text\nutilize in order to\n```\n\nUtilize later.\n",
    "fix/directive": (
        "<!-- slopvac-disable-next-line -->\nWe utilize this line.\n\n"
        "We utilize that line.\n\n"
        "<!-- slopvac-disable -->\n\nHidden utilize text.\n\n<!-- slopvac-enable -->\n\n"
        "Shown utilize text.\n"
    ),
    "fix/adjacent": "We utilize in order to win, then utilize in order to lose.\n",
    "fix/emphasis": "We **utilize** the tool and *in order to* win.\n",
    "fix/unchanged": "Nothing to change here.\n",
}


def fixed(rules, config, text: str) -> str:
    scope, _ = in_scope(rules, config)
    engine = Engine(rules.rules, config, only={r.qualified_id for r in scope})
    source = to_2x(text)
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "doc.md"
        path.write_bytes(source.encode("utf-8"))
        findings = engine.run(parse(str(path), source))
        score = SimpleNamespace(path=str(path), findings=findings)
        apply_replacements([score], {})
        return path.read_bytes().decode("utf-8")


def documents() -> list[tuple[str, str]]:
    stored = json.loads(FIXTURE.read_text(encoding="utf-8"))
    docs = [(d["name"], d["text"]) for d in stored["documents"]]
    # A document with 3.0 directive text would need the 2.x spelling, which does not map
    # back to the same text, so the fix corpus leaves those to the parity fixture.
    docs = [(n, t) for n, t in docs if to_2x(t) == t]
    return docs + list(SAMPLES.items())


def render(rules, config, docs: list[tuple[str, str]], keep_all: bool) -> str:
    rows = []
    for name, text in docs:
        after = fixed(rules, config, text)
        if keep_all or name in SAMPLES or after != text:
            rows.append({"name": name, "text": text, "fixed": after})
    lines = [json.dumps(r, ensure_ascii=False, sort_keys=True) for r in rows]
    return '{"documents":[\n' + ",\n".join(lines) + "\n]}\n"


def main() -> None:
    rules, config = load()
    if "--check" in sys.argv:
        stored = json.loads(FIXTURE.read_text(encoding="utf-8"))
        docs = [(d["name"], d["text"]) for d in stored["documents"]]
        if FIXTURE.read_text(encoding="utf-8") != render(rules, config, docs, True):
            raise SystemExit(
                "fix-parity.json is stale; regenerate with generate_fix.py and commit the result"
            )
        return
    text = render(rules, config, documents(), False)
    FIXTURE.write_text(text, encoding="utf-8")
    print(f"{text.count(chr(10)) - 2} documents")


if __name__ == "__main__":
    main()
