"""The bundled STE preset must match the 2.x normal profile it replaces."""

from __future__ import annotations

import tomllib
from pathlib import Path

from slopvac.profiles import _NORMAL

REMOVED_RULES = frozenset(
    tomllib.loads(
        (
            Path(__file__).resolve().parents[1] / "scripts/contract/removed-rules.toml"
        ).read_text()
    )["rules"]
)

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
        if r.category.startswith("ste-")
        and r.kind is not RuleKind.JUDGEMENT
        and r.qualified_id not in REMOVED_RULES
    }


def test_ste_preset_reproduces_effective_state_of_every_ste_rule() -> None:
    """Active state and level per rule, as the 2.x engine resolves them at normal.

    The rules 3.0 removed (`scripts/contract/removed-rules.toml`) are left out.

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


PRESETS = PRESET.parent
RULES = (
    Path(__file__).resolve().parents[1]
    / "packages"
    / "slopvac-lint"
    / "src"
    / "slopvac"
    / "rules"
)


def _rule_defs() -> dict[str, dict]:
    import yaml

    defs: dict[str, dict] = {}
    for path in RULES.glob("*.yml"):
        for doc in yaml.safe_load_all(path.read_text()):
            for rule in (doc or {}).get("rules", []):
                defs[f"{doc['id']}.{rule['id']}"] = rule
    return defs


def test_every_rule_key_in_a_preset_names_a_real_rule() -> None:
    known = _rule_defs()
    for preset in PRESETS.glob("*.toml"):
        for key in tomllib.loads(preset.read_text()).get("rules", {}):
            assert key in known, f"{preset.name}: unknown rule {key}"


def _matching_rules(text: str, enabled: set[str], scope: str = "prose") -> set[str]:
    """Enabled pattern and substitution rules of one scope whose regex matches `text`."""
    import re

    defs = _rule_defs()
    enabled = {
        k
        for k in enabled
        if (defs[k].get("scope") == "heading") == (scope == "heading")
    }
    hits: set[str] = set()
    for key in enabled:
        rule = defs[key]
        flags = re.IGNORECASE if rule.get("ignore_case") else 0
        # Substitution keys are regexes that the Vale compiler wraps in token
        # boundaries (compile_vale.py); pattern rules run as written.
        if rule.get("substitutions"):
            alternation = "|".join(f"(?:{p})" for p in rule["substitutions"])
            patterns = [rf"(?<![\w-])(?:{alternation})(?![\w-])"]
        else:
            patterns = [rule.get("pattern", "")]
        if any(p and re.search(p, text, flags) for p in patterns):
            hits.add(key)
    return hits


def _google_enabled() -> set[str]:
    rules = tomllib.loads((PRESETS / "google.toml").read_text())["rules"]
    return {key for key, level in rules.items() if level != "off"}


def test_google_preset_follows_the_style_guide_on_known_cases() -> None:
    """Cases from developers.google.com/style (word list, pronouns, headings)."""
    enabled = _google_enabled()
    allowed = [
        "Run ad hoc queries against the table.",
        "She reviewed her pull request; he approved his.",
        "Use a semicolon; it is fine here.",
        "Select Save.",
    ]
    for text in allowed:
        assert _matching_rules(text, enabled) == set(), text
    flagged = {
        "Use a list, e.g. a table of values.": {"ste-practices.latin-abbreviation"},
        "Remove the whitelist.": {"prose-inclusive.exclusive"},
        "This is a sanity check.": {"prose-inclusive.exclusive"},
        "He or she approves it.": {"prose-inclusive.exclusive"},
    }
    for text, rules in flagged.items():
        assert _matching_rules(text, enabled) == rules, text
    heading = "ai-tells-formatting.title-case-heading"
    assert _matching_rules("Getting Started With Docs", enabled, "heading") == {heading}
    assert _matching_rules("Getting started with docs", enabled, "heading") == set()


def test_google_preset_has_one_checker_per_policy() -> None:
    enabled = _google_enabled()
    for text in ["e.g. this", "i.e. that", "whitelist", "blacklist", "master branch"]:
        assert len(_matching_rules(text, enabled)) <= 1, text
