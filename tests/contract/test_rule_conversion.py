"""The 2.x to v3 rule converter: every shipped rule converts and validates."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_DIR = ROOT / "schemas" / "v3"


def _load_converter():
    path = ROOT / "scripts" / "contract" / "convert_rules_v2_to_v3.py"
    spec = importlib.util.spec_from_file_location("convert_rules_v2_to_v3", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module  # the converter defines an exception class
    spec.loader.exec_module(module)
    return module


convert = _load_converter()


def _validator(fragment: str = "") -> Draft202012Validator:
    schemas = [
        json.loads(p.read_text(encoding="utf-8"))
        for p in sorted(SCHEMA_DIR.glob("*.schema.json"))
    ]
    registry = Registry().with_resources(
        [(s["$id"], Resource.from_contents(s)) for s in schemas]
    )
    root = next(s for s in schemas if s["$id"].endswith("/category.schema.json"))
    if fragment:
        rule = next(s for s in schemas if s["$id"].endswith("/rule.schema.json"))
        root = {"$ref": f"{rule['$id']}#/$defs/{fragment}"}
    return Draft202012Validator(root, registry=registry)


SOURCES = convert.load_categories(convert.DEFAULT_RULES_DIR)
PAIRS = [(c, r) for c in SOURCES for r in c["rules"]]


def _case(category: dict, rule: dict):
    return pytest.param(category, rule, id=f"{category['id']}.{rule['id']}")


RULE_PARAMS = [_case(c, r) for c, r in PAIRS]


def _errors(validator: Draft202012Validator, instance: object) -> list[str]:
    return [
        f"{'/'.join(map(str, e.absolute_path))}: {e.message[:120]}"
        for e in validator.iter_errors(instance)
    ]


def _patterns(rule: dict) -> list[str]:
    match = rule["match"]
    return [p for p in [match.get("pattern"), *match.get("substitutions", {})] if p]


def test_ruleset_has_categories() -> None:
    assert len(SOURCES) >= 20


@pytest.mark.parametrize(("category", "rule"), RULE_PARAMS)
def test_converted_rule_validates(category: dict, rule: dict) -> None:
    converted = convert.convert_rule(rule)
    assert _errors(_validator("Rule"), converted) == []


@pytest.mark.parametrize("category", SOURCES, ids=lambda c: c["id"])
def test_converted_category_envelope_validates(category: dict) -> None:
    converted = convert.convert_category({**category, "rules": []})
    assert _errors(_validator(), converted) == []


def test_every_rule_converts_without_tiers() -> None:
    converted = [convert.convert_rule(rule) for _, rule in PAIRS]
    assert len(converted) == len(PAIRS)
    assert all("tiers" not in r for r in converted)


@pytest.mark.parametrize(("category", "rule"), RULE_PARAMS)
def test_converted_patterns_compile_with_pcre2(category: dict, rule: dict) -> None:
    pcre2 = pytest.importorskip(
        "pcre2", reason="python bindings for PCRE2 are not installed"
    )
    converted = convert.convert_rule(rule)
    for pattern in _patterns(converted):
        pcre2.compile(pattern)


@pytest.mark.parametrize(
    ("source", "found"),
    [
        (r"(?<=\u2014\s)x", [r"\u2014"]),
        (r"[\U0001F300-\U0001FAFF]", [r"\U0001F300", r"\U0001FAFF"]),
        (r"\\u2014", []),  # an escaped backslash then a literal u
        (r"\x{2014}", []),  # PCRE2-native, allowed
        (r"x\u00e9", [r"\u00e9"]),
    ],
)
def test_unicode_escapes_are_found(source: str, found: list[str]) -> None:
    assert convert.unicode_escapes(source) == found


@pytest.mark.parametrize("escape", [r"\u2014", r"\U0001F680"])
def test_unicode_escape_fails_with_the_rule_id(escape: str) -> None:
    rule = {
        "id": "esc-rule",
        "name": "n",
        "kind": "pattern",
        "message": "m",
        "pattern": f"a{escape}b",
        "provenance": {"source": "s"},
    }
    with pytest.raises(convert.ConversionError, match="esc-rule"):
        convert.convert_rule(rule)


@pytest.mark.parametrize(
    ("source", "found"),
    [
        (r"(?<=\bthat\s+)will", [r"(?<=\bthat\s+)"]),
        (r"(?<!\w*)x", [r"(?<!\w*)"]),
        (r"(?<=a{2,})x", [r"(?<=a{2,})"]),
        (
            r"(?<=\b(?:which|where)(?:\s+\w+){0,2}\s+)x",
            [r"(?<=\b(?:which|where)(?:\s+\w+){0,2}\s+)"],
        ),
        (r"(?<![\w*+])x", []),  # quantifier characters in a class are literals
        (r"(?<=a\+)x", []),
        (r"(?<=a{1,8})x", []),
        (r"\w+(?<=a)b*", []),  # only a quantifier inside a lookbehind counts
    ],
)
def test_unbounded_lookbehinds_are_found(source: str, found: list[str]) -> None:
    assert convert.unbounded_lookbehinds(source) == found


def test_unbounded_lookbehind_fails_with_the_rule_id() -> None:
    rule = {
        "id": "bad-one",
        "name": "n",
        "kind": "pattern",
        "message": "m",
        "pattern": r"(?<=a+)b",
        "provenance": {"source": "s"},
    }
    with pytest.raises(convert.ConversionError, match="bad-one"):
        convert.convert_rule(rule)


def test_allowed_words_placeholder_is_declared_for_title_case_only() -> None:
    base = {
        "name": "n",
        "kind": "pattern",
        "message": "m",
        "pattern": "(?!ALLOWED_WORDS)x",
        "provenance": {"source": "s"},
    }
    title = convert.convert_rule({**base, "id": "title-case-heading"})
    other = convert.convert_rule({**base, "id": "something-else"})
    assert title["match"]["placeholders"] == {"ALLOWED_WORDS": "allowed-words"}
    assert "placeholders" not in other["match"]


def test_shipped_title_case_heading_declares_the_placeholder() -> None:
    rule = next(r for _, r in PAIRS if r["id"] == "title-case-heading")
    converted = convert.convert_rule(rule)
    assert converted["match"]["placeholders"] == {"ALLOWED_WORDS": "allowed-words"}


def test_judgement_is_forced_to_suggestion() -> None:
    rule = convert.convert_rule(
        {
            "id": "r",
            "name": "n",
            "kind": "judgement",
            "severity": "error",
            "message": "m",
            "judgement_question": "Is it so?",
            "tiers": {"strict": "enforced"},
            "provenance": {"source": "s"},
        }
    )
    assert rule["severity"] == "suggestion"
    assert rule["match"] == {"kind": "judgement", "question": "Is it so?"}


def test_rule_conversion_carries_allowlist_comparison_exceptions_and_drops_nulls() -> (
    None
):
    source = {
        "id": "long-metric",
        "name": "Shorten it",
        "kind": "metric",
        "severity": "error",
        "message": "too long",
        "scope": "sentence",
        "text_type": "procedural",
        "metric": "sentence_words",
        "threshold": 20,
        "comparison": "gte",
        "exceptions": ["quotation", "api-name"],
        "allowlist": ["Foo Bar", "baz"],
        "examples": [{"bad": "x", "good": None, "note": "n"}],
        "fix": "Split it.",
        "ai_signal": "weak",
        "ai_signal_source": "catalog",
        "tiers": {"strict": "enforced", "normal": "advisory", "relaxed": "excluded"},
        "provenance": {"source": "ASD", "ste_ref": "9:1.2", "url": None, "note": None},
    }
    want = {
        "id": "long-metric",
        "name": "Shorten it",
        "message": "too long",
        "severity": "error",
        "scope": "sentence",
        "text_type": "procedural",
        "match": {
            "kind": "metric",
            "metric": "sentence_words",
            "threshold": 20,
            "comparison": "gte",
        },
        "exceptions": [
            {"name": "quotation", "detector": "quotation"},
            {"name": "api-name"},
        ],
        "allowlist": ["Foo Bar", "baz"],
        "examples": [{"bad": "x", "note": "n"}],
        "fix": "Split it.",
        "ai_signal": "weak",
        "ai_signal_source": "catalog",
        "citation": {"source": "ASD", "ste_ref": "9:1.2"},
    }
    assert convert.convert_rule(source) == want


def test_context_gate_is_carried_into_the_v3_rule_and_validates() -> None:
    rule = convert.convert_rule(
        {
            "id": "r",
            "name": "n",
            "kind": "pattern",
            "message": "m",
            "pattern": "x",
            "contexts": {"only": ["requirement"], "skip": None},
            "tiers": {"strict": "enforced"},
            "provenance": {"source": "s"},
        }
    )
    assert rule["contexts"] == {"only": ["requirement"]}
    assert _errors(_validator("Rule"), rule) == []


@pytest.mark.parametrize(
    "contexts",
    [{"only": []}, {"only": [], "skip": ["code-fence"]}, {"skip": []}],
)
def test_empty_context_lists_are_kept_and_validate(contexts: dict) -> None:
    rule = convert.convert_rule(
        {
            "id": "r",
            "name": "n",
            "kind": "pattern",
            "message": "m",
            "pattern": "x",
            "contexts": contexts,
            "tiers": {"strict": "enforced"},
            "provenance": {"source": "s"},
        }
    )
    assert rule["contexts"] == contexts
    assert _errors(_validator("Rule"), rule) == []


def test_lexical_options_move_into_match() -> None:
    rule = convert.convert_rule(
        {
            "id": "t",
            "name": "n",
            "kind": "tokens",
            "message": "m",
            "tokens": ["a", "b"],
            "ignore_case": False,
            "match_all_caps": True,
            "provenance": {"source": "s"},
        }
    )
    assert rule["match"] == {
        "kind": "tokens",
        "tokens": ["a", "b"],
        "ignore_case": False,
        "match_all_caps": True,
    }


def test_category_conversion_keeps_budgets_weight_and_genres() -> None:
    source = {
        "id": "cat-one",
        "title": "T",
        "description": "D",
        "weight": 0.8,
        "max_per_100_words": {"strict": 0.4, "normal": 0.8},
        "recommended_for": ["reference", "internal"],
        "rules": [],
    }
    assert convert.convert_category(source) == {
        "schema_version": 3,
        "id": "cat-one",
        "title": "T",
        "description": "D",
        "weight": 0.8,
        "max_per_100_words": {"strict": 0.4, "normal": 0.8},
        "recommended_for": ["reference", "internal"],
        "rules": [],
    }


@pytest.mark.parametrize("bad", ["../evil", "a/b", "Upper", "", "..", "x\\y"])
def test_category_id_is_validated_before_any_path_is_built(
    bad: str, tmp_path: Path
) -> None:
    rules_dir = tmp_path / "rules"
    rules_dir.mkdir()
    (rules_dir / "x.yml").write_text(
        f"id: '{bad}'\ntitle: t\ndescription: d\nrules: []\n", encoding="utf-8"
    )
    out = tmp_path / "out"
    assert convert.main([str(out), "--rules-dir", str(rules_dir)]) == 1
    assert [p for p in tmp_path.rglob("*.json")] == []


def test_tiers_are_extracted_for_the_profile_mapping() -> None:
    category = {
        "id": "c",
        "rules": [{"id": "r", "tiers": {"strict": "enforced", "normal": "advisory"}}],
    }
    assert convert.extract_tiers(category) == {
        "c.r": {"strict": "enforced", "normal": "advisory"}
    }


def test_no_generated_text_names_a_removed_rule() -> None:
    removed = sorted(convert.REMOVED_RULES)
    text = json.dumps(
        [
            convert.convert_category(c)
            for c in convert.load_categories(convert.DEFAULT_RULES_DIR)
        ]
    )
    assert [r for r in removed if r.split(".", 1)[1] in text] == []


def test_a_sentence_that_names_a_removed_rule_is_dropped() -> None:
    removed = min(convert.REMOVED_RULES).replace(".", "/")
    note = f"The half overlaps {removed}, which is mechanical. What is unique here is the key word."
    assert convert.scrub_removed_rules(note) == "What is unique here is the key word."


def test_text_that_names_only_a_removed_rule_fails_conversion() -> None:
    removed = min(convert.REMOVED_RULES).split(".", 1)[1]
    with pytest.raises(convert.ConversionError):
        convert.scrub_removed_rules(f"See {removed}.")
