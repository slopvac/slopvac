"""The 2.x profile mapping reproduces every 2.x gate input.

The 2.x engine is the ground truth. The reference resolver in
``preserve_expansion`` applies the 3.0 precedence to the generated
``core:legacy-<profile>`` styles. It must agree with the engine on every rule
(checked and judgement), category budget, weight and threshold, for strict, normal
and relaxed, and for configs with profile overrides and authored settings.
"""

from __future__ import annotations

import json
import tomllib

import pytest
from jsonschema import Draft202012Validator
from preserve_expansion import (
    ROOT,
    category_dial,
    expand,
    mapping,
    matches_files,
    resolve,
    style_layer,
    to_2x_config,
)
from referencing import Registry, Resource

SCHEMAS = ROOT / "schemas" / "v3"
PROFILES = mapping.PROFILES


def _style(profile: str) -> dict:
    path = mapping.OUT / f"{mapping.style_name(profile)}.toml"
    return tomllib.loads(path.read_text(encoding="utf-8"))


@pytest.fixture(scope="module", params=PROFILES)
def truth(request):
    profile = request.param
    ruleset, resolved, states = mapping.engine_states(profile)
    return profile, ruleset, resolved, states


@pytest.fixture(scope="module")
def ruleset():
    return mapping.engine_states("normal")[0]


# --- generated files -------------------------------------------------------


def test_committed_files_equal_regenerated_output():
    for name, text in mapping.build().items():
        assert (mapping.OUT / name).read_text(encoding="utf-8") == text, name


def test_check_exits_0_when_files_are_current(monkeypatch, capsys):
    assert _check_in_tmp({"a.toml": "x\n"}, {"a.toml": "x\n"}, monkeypatch) == 0
    assert capsys.readouterr().err == ""


@pytest.mark.parametrize(
    ("on_disk", "expected", "named"),
    [
        ({"a.toml": "old\n"}, {"a.toml": "new\n"}, "a.toml"),
        ({}, {"a.toml": "new\n"}, "a.toml"),
    ],
    ids=["stale", "missing"],
)
def test_check_exits_1_and_names_the_file_when_stale_or_missing(
    monkeypatch, capsys, on_disk, expected, named
):
    assert _check_in_tmp(on_disk, expected, monkeypatch) == 1
    assert named in capsys.readouterr().err


def test_check_does_not_rewrite_files(monkeypatch, tmp_path):
    (tmp_path / "a.toml").write_text("old\n", encoding="utf-8")
    monkeypatch.setattr(mapping, "OUT", tmp_path)
    monkeypatch.setattr(mapping, "build", lambda: {"a.toml": "new\n", "b.toml": "y\n"})
    assert mapping.main(["--check"]) == 1
    assert (tmp_path / "a.toml").read_text(encoding="utf-8") == "old\n"
    assert not (tmp_path / "b.toml").exists()


def _check_in_tmp(on_disk, expected, monkeypatch):
    import tempfile
    from pathlib import Path

    tmp = Path(tempfile.mkdtemp(prefix="mapping-check-"))
    for name, text in on_disk.items():
        (tmp / name).write_text(text, encoding="utf-8")
    monkeypatch.setattr(mapping, "OUT", tmp)
    monkeypatch.setattr(mapping, "build", lambda: expected)
    try:
        return mapping.main(["--check"])
    finally:
        for p in tmp.iterdir():
            p.unlink()
        tmp.rmdir()


# --- default config, every rule -------------------------------------------


def test_every_rule_resolves_to_its_2x_state(truth):
    profile, ruleset, _, states = truth
    layers = [style_layer(profile)]
    wrong = {}
    for rule in ruleset.all_rules:
        if rule.qualified_id in mapping.removed_rules():
            continue
        got = resolve(
            layers,
            rule.category,
            rule.qualified_id,
            rule.severity.value,
            rule.kind.value == "judgement",
        )
        if got != states[rule.qualified_id][1]:
            wrong[rule.qualified_id] = (got, states[rule.qualified_id][1])
    assert wrong == {}


def test_the_catalogue_includes_judgement_rules(truth):
    _, ruleset, _, states = truth
    assert len(ruleset.judgement_rules) > 0
    assert set(states) == {r.qualified_id for r in ruleset.all_rules}


def test_judgement_level_follows_the_tier(truth):
    profile, ruleset, _, _ = truth
    style = _style(profile)
    for rule in ruleset.judgement_rules:
        want = {"enforced": "error", "advisory": "suggestion", "excluded": "off"}[
            rule.tier_for(profile).value
        ]
        assert style["rules"][rule.qualified_id] == want


def test_a_judgement_rule_in_an_off_category_keeps_its_tier_level():
    # prose-scope is off at relaxed, and code-change-prose-scope is advisory there.
    style = _style("relaxed")
    assert style["categories"]["prose-scope"]["severity"] == "off"
    assert style["rules"]["prose-scope.code-change-prose-scope"] == "suggestion"


def test_checklist_selection_matches_2x(truth):
    profile, ruleset, _, _ = truth
    from slopvac.enums import Tier  # noqa: F401  (documents the tier source)
    from slopvac.genres import GENRES

    style = _style(profile)
    for genre in GENRES:
        want = {
            r.qualified_id
            for r in ruleset.judgement_rules
            if genre.name in ruleset.categories[r.category].recommended_for
            and r.tier_for(profile).value != "excluded"
        }
        got = {
            r.qualified_id
            for r in ruleset.judgement_rules
            if genre.name in ruleset.categories[r.category].recommended_for
            and style["rules"][r.qualified_id] != "off"
        }
        assert got == want, genre.name


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
    assert (
        resolve(
            [style_layer(profile)],
            "ste-practices",
            "ste-practices.latin-abbreviation",
            "suggestion",
            False,
        )
        == "off"
    )


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


# --- configs with overrides and authored settings -------------------------

SCENARIOS = {
    "profile-override": (
        {
            "profile": "normal",
            "overrides": [{"files": ["docs/**"], "profile": "relaxed"}],
        },
        ["docs/a.md", "src/a.md"],
    ),
    "authored-category-severity": (
        {"profile": "normal", "categories": {"prose-craft": {"severity": "error"}}},
        ["a.md"],
    ),
    "authored-category-floor-only": (
        {
            "profile": "normal",
            "categories": {"ste-verbs": {"minimum_severity": "warning"}},
        },
        ["a.md"],
    ),
    "authored-category-off": (
        {"profile": "strict", "categories": {"prose-inflation": "off"}},
        ["a.md"],
    ),
    "authored-category-revives-off": (
        {"profile": "relaxed", "categories": {"prose-craft": "warning"}},
        ["a.md"],
    ),
    "authored-rule": (
        {"profile": "normal", "rules": {"prose-craft.spacing": "error"}},
        ["a.md"],
    ),
    "rule-default-survives-authored-category": (
        {"profile": "normal", "categories": {"prose-inclusive": "error"}},
        ["a.md"],
    ),
    "override-authors-category": (
        {
            "profile": "normal",
            "overrides": [
                {"files": ["docs/**"], "categories": {"prose-craft": "error"}}
            ],
        },
        ["docs/a.md", "src/a.md"],
    ),
    "profile-override-keeps-top-authored-category": (
        {
            "profile": "normal",
            "categories": {"prose-craft": {"severity": "warning", "weight": 3.0}},
            "overrides": [{"files": ["docs/**"], "profile": "relaxed"}],
        },
        ["docs/a.md", "src/a.md"],
    ),
    "profile-override-keeps-top-authored-rule": (
        {
            "profile": "normal",
            "rules": {"ste-verbs.passive-voice": "error"},
            "overrides": [{"files": ["docs/**"], "profile": "relaxed"}],
        },
        ["docs/a.md"],
    ),
    "profile-override-with-own-category": (
        {
            "profile": "normal",
            "overrides": [
                {
                    "files": ["docs/**"],
                    "profile": "strict",
                    "categories": {"orwell-voice": "error"},
                }
            ],
        },
        ["docs/a.md"],
    ),
    "judgement-entries-are-ignored": (
        {
            "profile": "normal",
            "rules": {"orwell.concrete-floor": "off"},
            "overrides": [
                {"files": ["docs/**"], "rules": {"orwell.concrete-floor": "error"}}
            ],
        },
        ["a.md", "docs/a.md"],
    ),
    "top-rule-ignored-at-baseline-valid-in-override": (
        {
            "profile": "relaxed",
            "rules": {"orwell.not-un": "error"},
            "overrides": [{"files": ["docs/**"], "profile": "strict"}],
        },
        ["a.md", "docs/a.md"],
    ),
    "top-rule-valid-at-baseline-ignored-in-override": (
        {
            "profile": "strict",
            "rules": {"orwell.not-un": "warning"},
            "overrides": [{"files": ["docs/**"], "profile": "relaxed"}],
        },
        ["a.md", "docs/a.md"],
    ),
    "override-revives-category-and-rule": (
        {
            "profile": "relaxed",
            "overrides": [
                {
                    "files": ["docs/**"],
                    "categories": {"prose-craft": "warning"},
                    "rules": {"prose-craft.spacing": "error"},
                }
            ],
        },
        ["a.md", "docs/a.md"],
    ),
    "budget-cleared-after-an-earlier-switch": (
        {
            "profile": "strict",
            "overrides": [
                {"files": ["docs/**"], "profile": "normal"},
                {"files": ["docs/api/**"], "profile": "strict"},
            ],
        },
        ["docs/api/a.md", "docs/b.md", "c.md"],
    ),
    "budget-restored-by-the-reverse-switch": (
        {
            "profile": "strict",
            "overrides": [{"files": ["docs/**"], "profile": "normal"}],
        },
        ["docs/a.md", "c.md"],
    ),
    "authored-budget-survives-a-switch": (
        {
            "profile": "normal",
            "categories": {"orwell-voice": {"max_per_100_words": 2.0}},
            "overrides": [{"files": ["docs/**"], "profile": "strict"}],
        },
        ["docs/a.md", "c.md"],
    ),
    "later-block-rule-ignored-by-an-earlier-profile-switch": (
        {
            "profile": "relaxed",
            "overrides": [
                {"files": ["docs/**"], "profile": "strict"},
                {"files": ["docs/api/**"], "rules": {"orwell.not-un": "warning"}},
            ],
        },
        ["docs/api/a.md", "docs/b.md", "c.md"],
    ),
    "later-block-rule-revived-by-an-earlier-category": (
        {
            "profile": "relaxed",
            "overrides": [
                {"files": ["docs/**"], "categories": {"prose-craft": "warning"}},
                {"files": ["docs/api/**"], "rules": {"prose-craft.spacing": "error"}},
            ],
        },
        ["docs/api/a.md", "docs/b.md", "c.md"],
    ),
    "later-block-rule-ignored-after-a-switch-to-relaxed": (
        {
            "profile": "strict",
            "overrides": [
                {"files": ["docs/**"], "profile": "relaxed"},
                {"files": ["docs/api/**"], "rules": {"orwell.not-un": "warning"}},
            ],
        },
        ["docs/api/a.md", "docs/b.md", "c.md"],
    ),
    "top-rule-ignored-only-in-a-nested-scope": (
        {
            "profile": "strict",
            "rules": {"orwell.not-un": "warning"},
            "overrides": [
                {"files": ["docs/**"], "profile": "normal"},
                {"files": ["docs/api/**"], "profile": "relaxed"},
            ],
        },
        ["docs/api/a.md", "docs/b.md", "c.md"],
    ),
    "literal-path-also-matches-its-descendants": (
        {
            "profile": "strict",
            "rules": {"orwell.not-un": "warning"},
            "overrides": [
                {"files": ["docs/api"], "profile": "relaxed"},
                {"files": ["docs/api/**"], "profile": "strict"},
            ],
        },
        ["docs/api", "docs/api/a.md", "docs/b.md"],
    ),
    "two-profile-overrides": (
        {
            "profile": "normal",
            "overrides": [
                {"files": ["docs/**"], "profile": "relaxed"},
                {"files": ["docs/api/**"], "profile": "strict"},
            ],
        },
        ["docs/api/a.md", "docs/b.md", "c.md"],
    ),
}


@pytest.mark.parametrize("name", sorted(SCENARIOS))
def test_expansion_reproduces_2x_for_each_file(name, ruleset):
    config2, paths = SCENARIOS[name]
    config3 = expand(config2, ruleset)
    styles = {p: style_layer(p) for p in PROFILES}
    config = to_2x_config(config2)
    for path in paths:
        _, resolved, states = mapping.engine_states(config2["profile"], config, path)
        layers = [styles[config2["profile"]]]
        layers.append({"categories": config3["categories"], "rules": config3["rules"]})
        layers.extend(
            {"categories": b.get("categories", {}), "rules": b.get("rules", {})}
            for b in config3["overrides"]
            if matches_files(b["files"], path)
        )
        wrong = {}
        for rule in ruleset.all_rules:
            if rule.qualified_id in mapping.removed_rules():
                continue
            got = resolve(
                layers,
                rule.category,
                rule.qualified_id,
                rule.severity.value,
                rule.kind.value == "judgement",
            )
            if got != states[rule.qualified_id][1]:
                wrong[rule.qualified_id] = (got, states[rule.qualified_id][1])
        assert wrong == {}, path
        for cat, settings in resolved.categories.items():
            if settings.severity.value == "off":
                continue
            budget = category_dial(layers, cat, "max_per_100_words")
            assert budget == settings.max_per_100_words, (path, cat)
            assert category_dial(layers, cat, "weight") == settings.weight, (path, cat)


def test_ignored_2x_rule_entry_is_dropped(ruleset):
    excluded = next(
        r for r in ruleset.rules if r.tier_for("relaxed").value == "excluded"
    )
    config2 = {"profile": "relaxed", "rules": {excluded.qualified_id: "error"}}
    assert excluded.qualified_id not in expand(config2, ruleset)["rules"]
    _, _, states = mapping.engine_states("relaxed", to_2x_config(config2), "a.md")
    assert states[excluded.qualified_id] == (False, "off")


def test_a_profile_override_clears_a_budget_with_none(ruleset):
    # orwell-voice has a 0.5 budget at normal and none at strict.
    config3 = expand(
        {"profile": "normal", "overrides": [{"files": ["docs/**"], "profile": "strict"}]},
        ruleset,
    )
    block = config3["overrides"][0]
    assert block["categories"]["orwell-voice"]["max_per_100_words"] == "none"


@pytest.mark.parametrize(
    "files",
    [
        ["docs/*.md"],
        ["docs/*/**"],
        ["a.md"],
        ["docs/**", "!docs/api/**"],
        ["docs/[ab]/**"],
        ["**/api/**"],
        ["/docs/**"],
        ["docs/../x/**"],
    ],
    ids=["wildcard", "wildcard-before-tree", "basename", "negated", "class", "any-depth", "leading-slash", "dot-dot"],
)
def test_expansion_stops_on_globs_outside_the_grammar(ruleset, files):
    from preserve_expansion import AmbiguousOverlap

    config2 = {
        "profile": "normal",
        "overrides": [
            {"files": files, "profile": "relaxed"},
            {"files": ["docs/api/**"], "profile": "strict"},
        ],
    }
    with pytest.raises(AmbiguousOverlap):
        expand(config2, ruleset)


def test_many_disjoint_blocks_expand_to_one_region_each_plus_the_root():
    from preserve_expansion import build_regions

    blocks = [[f"dir{i}/**"] for i in range(40)]
    regions = build_regions(blocks)
    assert len(regions) == 41


REGION_PATHS = [
    "docs/a.md",
    "docs/api",
    "docs/api/a.md",
    "docs/api/v1/a.md",
    "docs/guide/a.md",
    "docs/guide/api/a.md",
    "docsx/a.md",
    "a.md",
    "src/a.md",
    "src/docs/a.md",
]


@pytest.mark.parametrize(
    "blocks",
    [
        [["docs/**"], ["docs/api/**"]],
        [["docs/api/**"], ["docs/**"]],
        [["docs/**"], ["docs/api/**"], ["docs/api/v1/**"], ["docs/guide/**"]],
        [["docs/**"], ["docs/a.md"], ["docs/**"], ["src/**"]],
        [["docs/api"], ["docs/api/**"]],
        [["docs/api/**"], ["docs/api"], ["docs/**"]],
    ],
    ids=["nested", "reversed", "forest", "equal-and-file", "path-and-tree", "tree-path-and-parent"],
)
def test_each_path_falls_in_exactly_one_region_that_names_its_blocks(blocks):
    from preserve_expansion import build_regions, matches_files

    regions = build_regions(blocks)
    for path in REGION_PATHS:
        matching = frozenset(i for i, files in enumerate(blocks) if matches_files(files, path))
        hit = [scope for scope, files in regions if matches_files(files, path)]
        assert hit == [matching], path
