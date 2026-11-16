"""Generate the 2.x to 3.0 profile and tier mapping.

2.x resolves a rule's state from the profile's category table, the profile's
per-rule table and the rule's tier (``engine.py``: ``is_active`` and
``severity_for``). 3.0 profiles set thresholds and a severity shift only, so the
rest moves into one style per profile: ``core:legacy-strict``, ``core:legacy-normal``
and ``core:legacy-relaxed``. The styles carry the category dials and one rule entry
for each rule whose state the category entry alone would get wrong.

    uv run python scripts/contract/profile_mapping.py          # rewrite the files
    uv run python scripts/contract/profile_mapping.py --check  # exit 1 when stale
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def removed_rules() -> frozenset[str]:
    """The 2.x rules 3.0 does not ship, from the rule converter."""
    import importlib.util

    path = Path(__file__).with_name("convert_rules_v2_to_v3.py")
    spec = importlib.util.spec_from_file_location("convert_rules_v2_to_v3", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault("convert_rules_v2_to_v3", module)
    spec.loader.exec_module(module)
    return module.REMOVED_RULES

OUT = ROOT / "docs" / "contract" / "compat" / "profiles"
MAPPING_JSON = OUT / "mapping.json"
PROFILES = ("strict", "normal", "relaxed")
THRESHOLD_KEYS = (
    "max_total_per_100_words",
    "max_errors",
    "max_warnings",
    "min_score",
    "max_unicode_dashes",
)


def style_name(profile: str) -> str:
    return f"legacy-{profile}"


JUDGEMENT_LEVEL = {"enforced": "error", "advisory": "suggestion", "excluded": "off"}


def engine_states(profile: str, config=None, path: str = "doc.md"):
    """Ground truth: the 2.x engine's state per rule, with the resolved config.

    Checked rules come from ``is_active`` and ``severity_for``. A judgement rule
    never fires, so its review level comes from its tier: enforced blocks the
    review verdict (``error``), advisory reports (``suggestion``), excluded skips
    (``off``).
    """
    from slopvac.config import Config, resolve_for
    from slopvac.engine import Engine
    from slopvac.enums import Profile
    from slopvac.rules import load_ruleset

    ruleset = load_ruleset()
    config = config or Config(profile=Profile(profile))
    resolved = resolve_for(config, Path(path))
    engine = Engine(ruleset.rules, resolved)
    states: dict[str, tuple[bool, str]] = {}
    for rule in ruleset.rules:
        active = engine.is_active(rule)
        severity = engine.severity_for(rule).value if active else "off"
        states[rule.qualified_id] = (active, severity)
    for rule in ruleset.judgement_rules:
        level = JUDGEMENT_LEVEL[rule.tier_for(resolved.profile.value).value]
        states[rule.qualified_id] = (level != "off", level)
    return ruleset, resolved, states


def shipped_severity(ruleset) -> dict[str, str]:
    return {r.qualified_id: r.severity.value for r in ruleset.rules}


def build_style(profile: str) -> dict:
    ruleset, resolved, states = engine_states(profile)
    shipped = shipped_severity(ruleset)
    categories: dict[str, dict] = {}
    removed = removed_rules()
    for rule in ruleset.all_rules:
        if rule.category not in resolved.categories:
            raise SystemExit(f"{profile}: category {rule.category} has no profile entry")
    for name in sorted(resolved.categories):
        settings = resolved.categories[name]
        dials: dict = {"severity": settings.severity.value}
        if settings.max_per_100_words is not None:
            dials["max_per_100_words"] = settings.max_per_100_words
        if settings.weight is not None:
            dials["weight"] = settings.weight
        categories[name] = dials
    rules: dict[str, str] = {}
    for rule in ruleset.all_rules:
        qid = rule.qualified_id
        if qid in removed:
            continue
        want = states[qid][1]
        if rule.kind.value == "judgement":
            rules[qid] = want
            continue
        cat_level = categories[rule.category].get("severity")
        expected = shipped[qid] if cat_level is None else cat_level
        if want != expected or rule.tier_for(profile).value == "excluded":
            rules[qid] = want
    return {"id": style_name(profile), "categories": categories, "rules": rules}


def rule_defaults(profile: str) -> dict[str, str]:
    from slopvac.enums import Profile
    from slopvac.profiles import profile_rule_defaults

    return {
        name: settings.severity.value
        for name, settings in sorted(profile_rule_defaults(Profile(profile)).items())
        if name not in removed_rules()
    }


def thresholds(profile: str) -> dict:
    from slopvac.config import Config, resolve_for
    from slopvac.enums import Profile

    resolved = resolve_for(Config(profile=Profile(profile)), Path("doc.md"))
    dumped = resolved.thresholds.model_dump()
    return {k: dumped[k] for k in THRESHOLD_KEYS if dumped[k] is not None}


def genres() -> dict[str, str]:
    from slopvac.genres import GENRES

    return {g.name: g.profile.value for g in GENRES}


def _toml_key(key: str) -> str:
    ok = key.replace("-", "").replace("_", "").isalnum()
    return key if ok else json.dumps(key)


def _toml_value(value: object) -> str:
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, float):
        return repr(value)
    return str(value)


def render_style(style: dict, profile: str) -> str:
    lines = [
        "# Generated by scripts/contract/profile_mapping.py. Do not edit.",
        f"# Reproduces the 2.x `{profile}` profile: category dials, and a rule entry",
        "# wherever the 2.x tier or per-rule default differs from the category level.",
        "# Judgement rules always get an entry: their level is the review level of the tier.",
        "schema_version = 3",
        f'description = "Category and rule levels that the 2.x {profile} profile set. `slopvac migrate` selects it."',
        "",
    ]
    for name, dials in style["categories"].items():
        lines.append(f"[categories.{_toml_key(name)}]")
        lines.extend(f"{k} = {_toml_value(v)}" for k, v in dials.items())
        lines.append("")
    if style["rules"]:
        lines.append("[rules]")
        lines.extend(
            f"{_toml_key(k)} = {_toml_value(v)}" for k, v in style["rules"].items()
        )
        lines.append("")
    return "\n".join(lines)


def build() -> dict[str, str]:
    files: dict[str, str] = {}
    mapping: dict = {"profiles": {}, "genres": genres()}
    for profile in PROFILES:
        style = build_style(profile)
        files[f"{style['id']}.toml"] = render_style(style, profile)
        mapping["profiles"][profile] = {
            "style": f"core:{style['id']}",
            "severity_shift": 0,
            "thresholds": thresholds(profile),
            "rule_defaults": rule_defaults(profile),
            "rule_entries": len(style["rules"]),
            "categories": len(style["categories"]),
        }
    files["mapping.json"] = json.dumps(mapping, indent=2, sort_keys=True) + "\n"
    return files


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    files = build()
    if args.check:
        stale = [
            n
            for n, text in files.items()
            if not (OUT / n).is_file() or (OUT / n).read_text(encoding="utf-8") != text
        ]
        if stale:
            print("stale: " + ", ".join(stale), file=sys.stderr)
            return 1
        return 0
    OUT.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (OUT / name).write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
