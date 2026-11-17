"""Regenerate the locale data and the locale parity fixture from the Python reference.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-engine/tests/fixtures/generate_locale.py [--check]

Two files come out of ``slopvac.locale`` (2.x), and neither is edited by hand:

* ``crates/slopvac-engine/data/locale-variants.json``: the variant table, the identifier
  allowlist, the known locales and the generated rule's examples. ``src/locale.rs`` embeds
  it. 2.x keeps this data as a hand-written table in ``locale.py``; it has no spell checker
  and no SCOWL or VarCon data.
* ``crates/slopvac-engine/tests/fixtures/locale-parity.json``: what 2.x builds and reports.
  For each locale, the ordered substitution map and the allowlist that
  ``build_spelling_rule`` and ``inject_locale_rule`` produce (also with a ``[locale] allow``
  list), and the findings of ``engine.Engine.run`` over a frozen corpus with the generated
  rule alone switched on.

The golden suite (slopvac-8l4.2.7) has no locale input, so this fixture stays the reference
for the generated spelling rule until slopvac-8l4.3.44 wires that rule into the golden runs.

``--check`` recomputes both files and fails when they differ.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from slopvac import locale
from slopvac.analyze import parse
from slopvac.config import Config, resolve_for
from slopvac.engine import Engine
from slopvac.rules import inject_locale_rule, load_ruleset

HERE = Path(__file__).resolve().parent
DATA = HERE.parents[1] / "data" / "locale-variants.json"
FIXTURE = HERE / "locale-parity.json"

RULE = "ste-words.spelling"
ALLOW = ["behavior", "Fulfill"]
TARGETS = ["en-US", "en-GB"]

# (name, locale, allow, text). Every text is prose the reference lints with only the
# generated rule on.
CORPUS = [
    (
        "us/plain",
        "en-US",
        [],
        "The parser normalises the colour value and it initialises the licence check.\n",
    ),
    (
        "gb/plain",
        "en-GB",
        [],
        "The parser normalizes the color value and it initializes the license check.\n",
    ),
    (
        "us/inflections",
        "en-US",
        [],
        "It organises, organised and organising files. The organisers recognise and "
        "analyse them. Fulfil the licences.\n",
    ),
    (
        "gb/inflections",
        "en-GB",
        [],
        "It organizes, organized and organizing files. The organizers recognize and "
        "analyze them. Fulfill the licenses.\n",
    ),
    (
        "gb/identifier-allowlist",
        "en-GB",
        [],
        "Set the color property, the dialog and the --color flag, then normalize.css and "
        "the serializer.\n",
    ),
    (
        "us/identifier-allowlist",
        "en-US",
        [],
        "Use colour in prose, but the program and programming stay.\n",
    ),
    (
        "gb/case",
        "en-GB",
        [],
        "Color and COLOR and color. The Initializes sentence ends. Organization of "
        "ORGANIZATIONS.\n",
    ),
    (
        "gb/quotation",
        "en-GB",
        [],
        'The label says "color" in the file. The word \u201cbehavior\u201d is American. '
        "Then behavior again.\n",
    ),
    (
        "us/multiline",
        "en-US",
        [],
        "# Colour\n\nFirst line about the\nbehaviour of the centre.\n\n- A grey catalogue\n"
        "- The metre and the litre\n",
    ),
    (
        "gb/terms-allow",
        "en-GB",
        ALLOW,
        "The behavior and Fulfill stay, but the flavor and the neighbor do not.\n",
    ),
    (
        "us/terms-allow",
        "en-US",
        ["colour"],
        "The colour stays and the behaviour does not.\n",
    ),
    (
        "gb/code-span",
        "en-GB",
        [],
        "The color is `color` and \u201ccolor\u201d here. Initializes it.\n",
    ),
    (
        "und/off",
        "und",
        [],
        "The colour and the color both pass because spelling is off.\n",
    ),
]


def variants_data() -> dict:
    return {
        "locales": list(locale.LOCALES),
        "default": locale.DEFAULT_LOCALE,
        "variants": [[row["en-US"], row["en-GB"]] for row in locale._VARIANTS],
        "allowlist": list(locale.IDENTIFIER_ALLOWLIST),
        "examples": {tag: locale._examples_for(tag) for tag in TARGETS},
    }


def built(tag: str, allow: list[str]):
    """The ruleset 2.x injects the rule into, and the note it returns."""
    rules = load_ruleset()
    note = inject_locale_rule(rules, tag, allow or None)
    return rules, note


def rule_facts(tag: str, allow: list[str]) -> dict | None:
    rules, note = built(tag, allow)
    if tag == "und":
        assert note is None
        return None
    rule = next(r for r in rules.categories["ste-words"].rules if r.id == "spelling")
    return {
        "substitutions": [list(pair) for pair in rule.substitutions.items()],
        "allowlist": list(rule.allowlist),
        "ignore_case": rule.ignore_case,
        "severity": rule.severity.value,
        "name": rule.name,
        "message": rule.message,
    }


def findings(tag: str, allow: list[str], text: str) -> list[dict]:
    if tag == "und":
        return []
    rules, _ = built(tag, allow)
    config = resolve_for(Config(), Path("fixture.md"))
    engine = Engine(rules.rules, config, only={RULE})
    return [
        {
            "rule": f"core:{f.rule_id}",
            "line": f.line,
            "column": f.column,
            "end_column": f.end_column,
            "severity": f.severity.value,
            "matched": f.matched_text,
            "message": f.message,
        }
        for f in sorted(
            engine.run(parse("fixture.md", text)), key=lambda f: (f.line, f.column)
        )
    ]


def parity() -> dict:
    rules, _ = built("en-US", [])
    unknown = inject_locale_rule(rules, "fr-FR")
    return {
        "unknown_locale_note": unknown,
        "locales": {tag: rule_facts(tag, []) for tag in (*TARGETS, "und")},
        "with_allow": {
            "allow": ALLOW,
            **{tag: rule_facts(tag, ALLOW) for tag in TARGETS},
        },
        "documents": [
            {
                "name": name,
                "locale": tag,
                "allow": allow,
                "text": text,
                "findings": findings(tag, allow, text),
            }
            for name, tag, allow, text in CORPUS
        ],
    }


def render(value: dict) -> str:
    return json.dumps(value, ensure_ascii=False, indent=1, sort_keys=True) + "\n"


def main() -> None:
    outputs = {DATA: render(variants_data()), FIXTURE: render(parity())}
    if "--check" in sys.argv:
        stale = [
            p.name
            for p, text in outputs.items()
            if not p.exists() or p.read_text(encoding="utf-8") != text
        ]
        if stale:
            raise SystemExit(
                f"{', '.join(stale)} stale; regenerate with generate_locale.py and commit the result"
            )
        return
    for path, text in outputs.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    print(f"{len(CORPUS)} documents, {len(locale._VARIANTS)} variant rows")


if __name__ == "__main__":
    main()
