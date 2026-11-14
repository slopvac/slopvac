"""The 2.x profile mapping reproduces every 2.x gate input.

The 2.x engine is the ground truth. A small reference resolver applies the 3.0
rules to the generated ``core:legacy-<profile>`` style: a rule entry is final, else
a category level sets the rule, else the rule's own severity stands, and ``off``
disables. The resolver and the engine must agree on every rule, category budget,
weight and threshold, for strict, normal and relaxed.
"""

from __future__ import annotations

import importlib.util
import json
import tomllib
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[2]
SCHEMAS = ROOT / "schemas" / "v3"
_spec = importlib.util.spec_from_file_location(
    "profile_mapping", ROOT / "scripts" / "contract" / "profile_mapping.py"
)
mapping = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mapping)

PROFILES = mapping.PROFILES


def _style(profile: str) -> dict:
    path = mapping.OUT / f"{mapping.style_name(profile)}.toml"
    return tomllib.loads(path.read_text(encoding="utf-8"))


def reference_resolve(style: dict, category: str, qid: str, shipped: str) -> str:
    """3.0 resolution of one rule under one style. Returns a level; off disables."""
    rule_level = style.get("rules", {}).get(qid)
    if rule_level is not None:
        return rule_level
    dials = style.get("categories", {}).get(category)
    if dials is not None and "severity" in dials:
        return dials["severity"]
    return shipped


@pytest.fixture(scope="module", params=PROFILES)
def truth(request):
    profile = request.param
    ruleset, resolved, states = mapping.engine_states(profile)
    return profile, ruleset, resolved, states


def test_committed_files_equal_regenerated_output():
    for name, text in mapping.build().items():
        assert (mapping.OUT / name).read_text(encoding="utf-8") == text, name


def test_every_rule_resolves_to_its_2x_state(truth):
    profile, ruleset, _, states = truth
    style = _style(profile)
    wrong = {}
    for rule in ruleset.rules:
        got = reference_resolve(
            style, rule.category, rule.qualified_id, rule.severity.value
        )
        if got != states[rule.qualified_id][1]:
            wrong[rule.qualified_id] = (got, states[rule.qualified_id][1])
    assert wrong == {}


def test_category_budgets_and_weights_match_2x(truth):
    profile, _, resolved, _ = truth
    dials = _style(profile)["categories"]
    assert set(dials) == set(resolved.categories)
    for name, settings in resolved.categories.items():
        assert dials[name].get("max_per_100_words") == settings.max_per_100_words, name
        assert dials[name].get("weight") == settings.weight, name


def test_mapping_json_thresholds_match_2x(truth):
    profile, _, resolved, _ = truth
    entry = json.loads((mapping.OUT / "mapping.json").read_text("utf-8"))["profiles"][
        profile
    ]
    dumped = resolved.thresholds.model_dump()
    assert entry["thresholds"] == {
        k: v for k, v in dumped.items() if v is not None and k in mapping.THRESHOLD_KEYS
    }
    assert entry["severity_shift"] == 0
    assert entry["style"] == f"core:legacy-{profile}"


def test_inversions_the_2x_docstring_names_survive():
    strict, normal = _style("strict"), _style("normal")
    assert strict["categories"]["orwell-voice"]["severity"] == "warning"
    assert normal["categories"]["orwell-voice"]["severity"] == "error"
    assert _style("relaxed")["categories"]["meta"]["severity"] == "error"


def test_per_rule_default_under_an_error_category_keeps_its_warning():
    # profiles.py documents negative-guarantee-core as a warning inside the
    # error category ai-tells-structure at normal.
    normal = _style("normal")
    assert normal["categories"]["ai-tells-structure"]["severity"] == "error"
    assert normal["rules"]["ai-tells-structure.negative-guarantee-core"] == "warning"


@pytest.mark.parametrize("profile", PROFILES)
def test_latin_abbreviation_is_off_in_every_profile(profile):
    # profiles.py ships this rule off at every profile (_OFF_BY_DEFAULT).
    style = _style(profile)
    assert reference_resolve(
        style, "ste-practices", "ste-practices.latin-abbreviation", "suggestion"
    ) == "off"


def test_excluded_rule_resolves_to_off():
    # `ai-tells-formatting` is off at relaxed through the category; rules excluded by tier
    # inside live categories need their own entry.
    relaxed = _style("relaxed")
    assert relaxed["categories"]["ai-tells-formatting"]["severity"] == "off"
    assert "off" in set(relaxed["rules"].values())


def test_genres_keep_their_2x_profile():
    assert json.loads((mapping.OUT / "mapping.json").read_text("utf-8"))["genres"] == {
        "consumer": "normal",
        "reference": "strict",
        "change-comms": "normal",
        "internal": "normal",
        "informal": "relaxed",
    }


@pytest.mark.parametrize("profile", PROFILES)
def test_generated_style_validates_against_the_style_schema(profile):
    resources = [
        (s["$id"], Resource.from_contents(s))
        for s in (
            json.loads(p.read_text("utf-8")) for p in SCHEMAS.glob("*.schema.json")
        )
    ]
    registry = Registry().with_resources(resources)
    schema = json.loads((SCHEMAS / "style.schema.json").read_text("utf-8"))
    errors = list(
        Draft202012Validator(schema, registry=registry).iter_errors(_style(profile))
    )
    assert errors == []
