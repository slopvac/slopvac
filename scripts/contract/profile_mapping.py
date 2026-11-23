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
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "contract" / "compat" / "profiles"
MAPPING_JSON = OUT / "mapping.json"
REMOVED_RULES_FILE = ROOT / "docs" / "contract" / "compat" / "removed-rules.toml"
PROFILES = ("strict", "normal", "relaxed")
THRESHOLD_KEYS = (
    "max_total_per_100_words",
    "max_errors",
    "max_warnings",
    "min_score",
    "max_unicode_dashes",
)


def load_removed_rules(path: Path = REMOVED_RULES_FILE) -> dict[str, str]:
    """Map each removed 3.0 rule id to its decision source; a missing file raises."""
    entries = tomllib.loads(path.read_text(encoding="utf-8"))["rules"]
    return {entry["id"]: entry["source"] for entry in entries}


def style_name(profile: str) -> str:
    return f"legacy-{profile}"


JUDGEMENT_LEVEL = {"enforced": "error", "advisory": "suggestion", "excluded": "off"}


def engine_states(
    profile: str, config=None, path: str = "doc.md", *, blocklist: bool = False
):
    """Ground truth: the 2.x engine's state per rule, with the resolved config.

    The catalogue includes the generated spelling rule for the default locale.
    Checked rules come from ``is_active`` and ``severity_for``. A judgement rule
    never fires, so its review level comes from its tier: enforced blocks the
    review verdict (``error``), advisory reports (``suggestion``), excluded skips
    (``off``). Without a word blocklist (``blocklist=False``) a vocabulary rule is
    ``off``: the engine has nothing to give it. With one, it keeps its 2.x state.
    """
    from slopvac.config import Config, resolve_for
    from slopvac.engine import Engine
    from slopvac.enums import Profile
    from slopvac.locale import DEFAULT_LOCALE
    from slopvac.rules import inject_locale_rule, load_ruleset

    ruleset = load_ruleset()
    # `ste-words.spelling` is generated per run from the locale, not shipped as YAML,
    # so `load_ruleset` alone leaves it out of the catalogue.
    inject_locale_rule(ruleset, DEFAULT_LOCALE)
    config = config or Config(profile=Profile(profile))
    resolved = resolve_for(config, Path(path))
    engine = Engine(ruleset.rules, resolved)
    states: dict[str, tuple[bool, str]] = {}
    for rule in ruleset.rules:
        active = engine.is_active(rule)
        severity = engine.severity_for(rule).value if active else "off"
        if rule.kind.value == "vocabulary" and not blocklist:
            # Vocabulary checking is opt-in: 2.x ran it only when `vocabulary.path`
            # was set, and a style cannot set a blocklist. An enabled rule without
            # one reports `vocabulary-no-blocklist` on every document in 3.0.
            active, severity = False, "off"
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
    for rule in ruleset.all_rules:
        if rule.category not in resolved.categories:
            raise SystemExit(
                f"{profile}: category {rule.category} has no profile entry"
            )
    # 2.x profile tables also name categories that no rule belongs to (`meta`,
    # `orwell-voice`, `prose-density`, `ste-vocabulary`). Nothing in them can report,
    # and 3.0 rejects a style category that no pack defines.
    with_rules = {rule.category for rule in ruleset.all_rules}
    for name in sorted(resolved.categories.keys() & with_rules):
        settings = resolved.categories[name]
        dials: dict = {"severity": settings.severity.value}
        if settings.max_per_100_words is not None:
            dials["max_per_100_words"] = settings.max_per_100_words
        if settings.weight is not None:
            dials["weight"] = settings.weight
        categories[name] = dials
    rules: dict[str, str] = {}
    removed = load_removed_rules()
    for rule in ruleset.all_rules:
        qid = rule.qualified_id
        if qid in removed:
            # 3.0 has no such rule, and a style entry for it would be an unknown id.
            continue
        want = states[qid][1]
        if rule.kind.value == "judgement":
            rules[qid] = want
            continue
        cat_level = categories[rule.category].get("severity")
        expected = shipped[qid] if cat_level is None else cat_level
        # A vocabulary rule always gets an entry, so it stays off whatever the category says.
        if (
            want != expected
            or rule.tier_for(profile).value == "excluded"
            or rule.kind.value == "vocabulary"
        ):
            rules[qid] = want
    return {"id": style_name(profile), "categories": categories, "rules": rules}


def vocabulary_levels(profile: str) -> dict[str, str]:
    """The level of each vocabulary rule under the 2.x profile when a blocklist is set.

    The legacy styles leave these rules off. The companion style `legacy-<profile>-vocabulary`
    sets these levels, and `slopvac migrate --preserve-behavior` lists it for a config that
    moved `vocabulary.path`.
    """
    ruleset, _, states = engine_states(profile, blocklist=True)
    return {
        rule.qualified_id: states[rule.qualified_id][1]
        for rule in ruleset.all_rules
        if rule.kind.value == "vocabulary"
        and rule.qualified_id not in load_removed_rules()
    }


def rule_defaults(profile: str) -> dict[str, str]:
    from slopvac.enums import Profile
    from slopvac.profiles import profile_rule_defaults

    return {
        name: settings.severity.value
        for name, settings in sorted(profile_rule_defaults(Profile(profile)).items())
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


def vocabulary_style_name(profile: str) -> str:
    return f"legacy-{profile}-vocabulary"


def render_vocabulary_style(profile: str) -> str:
    """The companion style: the four vocabulary rules at their 2.x level for the profile."""
    lines = [
        "# Generated by scripts/contract/profile_mapping.py. Do not edit.",
        f"# The 2.x `{profile}` level of each vocabulary rule. `core:legacy-{profile}` leaves",
        "# these rules off, because 2.x ran them only when `vocabulary.path` was set.",
        f"# `slopvac migrate --preserve-behavior` lists this style after `core:legacy-{profile}`",
        "# when it moves a `vocabulary.path` into `terms.blocklists`.",
        "schema_version = 3",
        f'description = "The 2.x {profile} level of the vocabulary rules. Add it after `core:legacy-{profile}` when the project sets a word blocklist."',
        "",
        "[rules]",
    ]
    lines.extend(
        f"{_toml_key(k)} = {_toml_value(v)}"
        for k, v in vocabulary_levels(profile).items()
    )
    lines.append("")
    return "\n".join(lines)


def build() -> dict[str, str]:
    files: dict[str, str] = {}
    mapping: dict = {
        "profiles": {},
        "genres": genres(),
        # 2.x rules that 3.0 removed: the styles carry no entry for them.
        "removed_rules": {
            "file": "docs/contract/compat/removed-rules.toml",
            "ids": load_removed_rules(),
        },
    }
    for profile in PROFILES:
        style = build_style(profile)
        files[f"{style['id']}.toml"] = render_style(style, profile)
        files[f"{vocabulary_style_name(profile)}.toml"] = render_vocabulary_style(
            profile
        )
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
