"""The bundled STE preset must match the 2.x normal profile it replaces."""

from __future__ import annotations

import tomllib
from pathlib import Path

from slopvac.profiles import _NORMAL

PRESET = (
    Path(__file__).resolve().parents[1]
    / "crates"
    / "slopvac-styles"
    / "presets"
    / "ste.toml"
)


def test_ste_preset_reproduces_normal_profile_ste_categories() -> None:
    preset = tomllib.loads(PRESET.read_text())["categories"]
    expected = {k: v for k, v in _NORMAL.items() if k.startswith("ste-")}
    assert set(preset) == set(expected)
    for name, settings in expected.items():
        got = preset[name]
        if isinstance(got, str):
            assert (got, None, None) == (
                settings.severity.value,
                settings.max_per_100_words,
                settings.weight,
            ), name
            continue
        assert got["severity"] == settings.severity.value, name
        assert got["max_per_100_words"] == settings.max_per_100_words, name
        assert got["weight"] == settings.weight, name


def _engine_effective_ste_rules() -> dict[str, str]:
    from slopvac.config import Config, Profile, resolve_for
    from slopvac.engine import Engine
    from slopvac.enums import RuleKind
    from slopvac.rules import load_ruleset

    ruleset = load_ruleset()
    config = Config().model_copy(update={"profile": Profile("normal"), "overrides": []})
    engine = Engine(ruleset.rules, resolve_for(config, Path("README.md")))
    active = {r.qualified_id for r in engine.rules}
    return {
        r.qualified_id: engine.severity_for(r).value
        if r.qualified_id in active
        else "off"
        for r in ruleset.rules
        if r.category.startswith("ste-") and r.kind is not RuleKind.JUDGEMENT
    }


def test_ste_preset_reproduces_effective_state_of_every_ste_rule() -> None:
    """Active state and level per rule, as the 2.x engine resolves them at normal.

    A category entry sets every rule in the category, and a rule entry beats it.
    """
    data = tomllib.loads(PRESET.read_text())
    categories = {
        name: (value if isinstance(value, str) else value["severity"])
        for name, value in data["categories"].items()
    }
    rules = data.get("rules", {})
    expected = _engine_effective_ste_rules()
    got = {key: rules.get(key, categories[key.split(".")[0]]) for key in expected}
    assert got == expected
    assert set(rules) <= set(expected)
    assert all(level != categories[key.split(".")[0]] for key, level in rules.items())
