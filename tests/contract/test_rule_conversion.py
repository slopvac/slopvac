"""The 2.x to v3 rule converter: every shipped rule converts and validates."""

from __future__ import annotations

import importlib.util
import json
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
    spec.loader.exec_module(module)
    return module


convert = _load_converter()


def _validator() -> Draft202012Validator:
    schemas = [
        json.loads(p.read_text(encoding="utf-8"))
        for p in sorted(SCHEMA_DIR.glob("*.schema.json"))
    ]
    registry = Registry().with_resources(
        [(s["$id"], Resource.from_contents(s)) for s in schemas]
    )
    category = next(s for s in schemas if s["$id"].endswith("/category.schema.json"))
    return Draft202012Validator(category, registry=registry)


SOURCES = convert.load_categories(convert.DEFAULT_RULES_DIR)
CONVERTED = [convert.convert_category(c) for c in SOURCES]


def test_ruleset_has_categories() -> None:
    assert len(SOURCES) >= 20


@pytest.mark.parametrize("category", CONVERTED, ids=lambda c: c["id"])
def test_converted_category_validates(category: dict) -> None:
    errors = [
        f"{'/'.join(map(str, e.absolute_path))}: {e.message[:120]}"
        for e in _validator().iter_errors(category)
    ]
    assert errors == []


def test_every_rule_is_converted_once_and_tiers_are_gone() -> None:
    source_ids = [f"{c['id']}.{r['id']}" for c in SOURCES for r in c["rules"]]
    converted_ids = [f"{c['id']}.{r['id']}" for c in CONVERTED for r in c["rules"]]
    assert converted_ids == source_ids
    assert all("tiers" not in r for c in CONVERTED for r in c["rules"])


def test_converted_patterns_are_pcre2_clean() -> None:
    for category in CONVERTED:
        for rule in category["rules"]:
            match = rule["match"]
            patterns = [match.get("pattern", ""), *match.get("substitutions", {})]
            for pattern in patterns:
                assert convert.rewrite_unicode_escapes(pattern) == pattern
                assert convert.bound_lookbehinds(pattern) == (pattern, False)


@pytest.mark.parametrize(
    ("source", "want"),
    [
        (r"(?<=\u2014\s)x", r"(?<=\x{2014}\s)x"),
        (r"[\u00e9\u00e8]", r"[\x{00e9}\x{00e8}]"),
        (r"\\u2014", r"\\u2014"),  # an escaped backslash then a literal u
    ],
)
def test_unicode_escapes_become_pcre2_hex(source: str, want: str) -> None:
    assert convert.rewrite_unicode_escapes(source) == want


@pytest.mark.parametrize(
    ("source", "want"),
    [
        (r"(?<=\bthat\s+)will", r"(?<=\bthat\s{1,32})will"),
        (r"(?<!\w*)x", r"(?<!\w{0,32})x"),
        (r"(?<=a{2,})x", r"(?<=a{2,34})x"),
        (
            r"(?<![\w*+])x",
            r"(?<![\w*+])x",
        ),  # quantifier characters in a class are literals
        (r"\w+(?<=a)b*", r"\w+(?<=a)b*"),  # only lookbehinds change
        (r"(?<=a\+)x", r"(?<=a\+)x"),
    ],
)
def test_lookbehind_quantifiers_are_bounded(source: str, want: str) -> None:
    assert convert.bound_lookbehinds(source)[0] == want


def test_allowed_words_placeholder_is_declared() -> None:
    rule = next(
        r for c in CONVERTED for r in c["rules"] if r["id"] == "title-case-heading"
    )
    assert rule["match"]["placeholders"] == {"ALLOWED_WORDS": "allowed-words"}


def test_judgement_is_forced_to_suggestion_and_quotation_gets_a_detector() -> None:
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
    assert convert._convert_exception("quotation") == {
        "name": "quotation",
        "detector": "quotation",
    }
    assert convert._convert_exception("api-name") == {"name": "api-name"}


def test_tiers_are_extracted_for_the_profile_mapping() -> None:
    category = {
        "id": "c",
        "rules": [{"id": "r", "tiers": {"strict": "enforced", "normal": "advisory"}}],
    }
    assert convert.extract_tiers(category) == {
        "c.r": {"strict": "enforced", "normal": "advisory"}
    }
