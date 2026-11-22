"""Regenerate the 2.x mapping data and the preserve-behavior vectors of slopvac-migrate.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-migrate/tests/fixtures/generate.py [--check]

Every output comes from one snapshot of its source, so the crate cannot drift from it:

- `data/renames.toml`: one entry per 2.x rule and category. The 2.x name comes from
  the 2.x rule YAML, the 3.0 id from the rule converter (docs/contract/rules.md,
  `scripts/contract/convert_rules_v2_to_v3.py`), qualified by the `core` pack. The
  `[removed]` list comes from `docs/contract/compat/removed-rules.toml`.
- `data/legacy/legacy-<profile>.toml`: the `core:legacy-<profile>` styles, copied
  verbatim from `docs/contract/compat/profiles/` (docs/contract/profiles.md).
- `data/legacy/catalogue.toml`: each 2.x rule's kind and tier per profile, from the
  2.x ruleset, each profile's per-rule defaults, from
  `docs/contract/compat/profiles/mapping.json`, and each category's own weight, from the
  2.x ruleset.
- `tests/fixtures/preserve/refused-blocklist.json`: the word blocklist layouts that
  `--preserve-behavior` refuses, with the key each refusal names.
- `tests/fixtures/preserve/vectors.json`: what `expand()` and `build_regions()` in
  `tests/contract/preserve_expansion.py` return for the scenarios, glob errors and
  region cases of `tests/contract/test_profile_mapping.py`, plus the crate's own
  scenarios below, with 2.x names renamed through the table above. `measured_2x` holds
  what the 2.x engine (`engine_states` in `scripts/contract/profile_mapping.py`)
  resolves for each scenario on each listed file: the level of every rule and the
  budget and weight of every category, under 2.x names.

`--check` regenerates in memory and exits 1, naming each stale or missing file, without
writing. `--out DIR` writes or checks a copy of the crate layout instead of the crate.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
import tomllib
from pathlib import Path

CRATE = Path(__file__).resolve().parents[2]
ROOT = CRATE.parents[1]
CONTRACT_TESTS = ROOT / "tests" / "contract"
PROFILES_DIR = ROOT / "docs" / "contract" / "compat" / "profiles"
REMOVED_RULES_FILE = ROOT / "docs" / "contract" / "compat" / "removed-rules.toml"
PACK = "core"
GENERATOR = "crates/slopvac-migrate/tests/fixtures/generate.py"

sys.path.insert(0, str(CONTRACT_TESTS))

import preserve_expansion as reference
import test_profile_mapping as scenarios

PROFILES = reference.mapping.PROFILES
# The rule id of the finding 2.x reports for a malformed directive (suppression.py).
ENGINE_RULE = "meta.invalid-suppression"

# 2.x rules that 3.0 removes. They have no 3.0 id: `migrate` drops every reference to
# them. The contract file owns the list.
REMOVED_RULES = tuple(
    entry["id"]
    for entry in tomllib.loads(REMOVED_RULES_FILE.read_text(encoding="utf-8"))["rules"]
)

# Scenarios the crate adds to the contract's: each pins one expansion step.
EXTRA_SCENARIOS = {
    "authored-category-without-severity-in-override": {
        "profile": "strict",
        "overrides": [
            {
                "files": ["docs/**"],
                "profile": "relaxed",
                "categories": {"prose-craft": {"weight": 2.0}},
            }
        ],
    },
    "dropped-entry-takes-the-profile-default-at-the-top": {
        "profile": "strict",
        "categories": {"prose-craft": "off"},
        "rules": {"prose-craft.latinisms": "error"},
    },
    "dropped-entry-takes-the-profile-default-in-an-override": {
        "profile": "strict",
        "overrides": [
            {
                "files": ["docs/**"],
                "categories": {"prose-craft": "off"},
                "rules": {"prose-craft.latinisms": "error"},
            }
        ],
    },
    "profile-override-clears-a-budget-with-none": {
        "profile": "normal",
        "overrides": [{"files": ["docs/**"], "profile": "strict"}],
    },
    # E3 folds a profile switch into its own block. An earlier block that holds the
    # switched files authored a category, and 2.x applied it under the new profile.
    "profile-override-inside-an-earlier-category-block": {
        "profile": "strict",
        "overrides": [
            {"files": ["docs/**"], "categories": {"prose-craft": "warning"}},
            {"files": ["docs/api"], "profile": "relaxed"},
        ],
    },
    # An earlier block inside the switched files authored a category: 2.x applied it
    # under the profile of the later, enclosing block.
    "profile-override-around-an-earlier-category-block": {
        "profile": "strict",
        "overrides": [
            {"files": ["docs/api/**"], "categories": {"prose-craft": "warning"}},
            {"files": ["docs/**"], "profile": "relaxed"},
        ],
    },
    # Two enclosing profile switches move the first block twice. The second block beat
    # it in 2.x, and still does after the second move.
    "block-moved-by-two-profile-switches": {
        "profile": "strict",
        "overrides": [
            {"files": ["docs/api/v1/**"], "categories": {"prose-craft": "warning"}},
            {
                "files": ["docs/api/**"],
                "profile": "normal",
                "categories": {"prose-craft": "error"},
            },
            {"files": ["docs/**"], "profile": "relaxed"},
        ],
    },
    # The relaxed style turns the rule off by rule. The switch to normal and the
    # block's own category level enable it again, as 2.x did.
    "switching-block-category-revives-a-style-off-rule": {
        "profile": "relaxed",
        "overrides": [
            {
                "files": ["docs/**"],
                "profile": "normal",
                "categories": {"ai-tells-agentic": "warning"},
            }
        ],
    },
    # The same, with the category level in an earlier block that the switch moves.
    "moved-block-category-revives-a-style-off-rule": {
        "profile": "relaxed",
        "overrides": [
            {"files": ["docs/api/**"], "categories": {"ai-tells-agentic": "warning"}},
            {"files": ["docs/**"], "profile": "normal"},
        ],
    },
    # A guard for docs/api cannot merge into the block with its files, because the later
    # docs/** block sets the level of the guarded rule's category. 3.0 refuses the
    # guard block that repeats those files, so the migration stops (a known limit).
    "guard-held-back-by-a-later-category-level": {
        "profile": "strict",
        "rules": {"ai-tells-agentic.formulaic-universal-heading": "error"},
        "overrides": [
            {"files": ["docs/api/**"], "profile": "relaxed"},
            {"files": ["docs/**"], "categories": {"ai-tells-agentic": "warning"}},
        ],
    },
}

# The files each crate scenario is measured on, one per region.
EXTRA_PATHS = {
    "authored-category-without-severity-in-override": ["docs/a.md", "a.md"],
    "dropped-entry-takes-the-profile-default-at-the-top": ["a.md"],
    "dropped-entry-takes-the-profile-default-in-an-override": ["docs/a.md", "a.md"],
    "profile-override-clears-a-budget-with-none": ["docs/a.md", "a.md"],
    "profile-override-inside-an-earlier-category-block": [
        "docs/api/x.md",
        "docs/a.md",
        "a.md",
    ],
    "profile-override-around-an-earlier-category-block": [
        "docs/api/x.md",
        "docs/a.md",
        "a.md",
    ],
    "block-moved-by-two-profile-switches": [
        "docs/api/v1/x.md",
        "docs/api/x.md",
        "docs/a.md",
        "a.md",
    ],
    "guard-held-back-by-a-later-category-level": ["docs/api/x.md", "docs/a.md", "a.md"],
    "switching-block-category-revives-a-style-off-rule": ["docs/a.md", "a.md"],
    "moved-block-category-revives-a-style-off-rule": [
        "docs/api/x.md",
        "docs/a.md",
        "a.md",
    ],
    "ignored-rule-entry-is-dropped": ["a.md"],
}


def _load_converter():
    spec = importlib.util.spec_from_file_location(
        "convert_rules_v2_to_v3",
        ROOT / "scripts" / "contract" / "convert_rules_v2_to_v3.py",
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _parametrize(test) -> tuple[list, list[str]]:
    """The values and ids of the one ``parametrize`` mark on a contract test."""
    (mark,) = [m for m in test.pytestmark if m.name == "parametrize"]
    return mark.args[1], mark.kwargs["ids"]


def _q(key: str) -> str:
    return json.dumps(key)


def _inline(value) -> str:
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, int):
        return str(value)
    if isinstance(value, list):
        return "[" + ", ".join(_inline(v) for v in value) + "]"
    if isinstance(value, dict):
        return (
            "{ " + ", ".join(f"{_q(k)} = {_inline(v)}" for k, v in value.items()) + " }"
        )
    raise TypeError(value)


def config_toml(config2: dict) -> str:
    """A 2.x config as TOML text, tables first and override blocks last."""
    lines = []
    if "profile" in config2:
        lines.append(f"profile = {_inline(config2['profile'])}")
    for table in ("categories", "rules"):
        if table in config2:
            lines.append(f"[{table}]")
            lines.extend(f"{_q(k)} = {_inline(v)}" for k, v in config2[table].items())
    for block in config2.get("overrides", []):
        lines.append("[[overrides]]")
        lines.extend(f"{k} = {_inline(v)}" for k, v in block.items())
    return "\n".join(lines) + "\n"


def renames() -> tuple[dict[str, str], dict[str, str], list[str]]:
    """The 2.x names and their 3.0 ids.

    The YAML rules and categories take the converter's ids. 2.x also knew names
    without YAML: the generated locale rule (`rules.inject_locale_rule`), the
    `meta.invalid-suppression` finding (`suppression.py`), and the categories that
    only `profiles.py` lists, which the legacy styles name. The 3.0 engine and the
    legacy styles keep those names in the core pack.
    """
    from slopvac.rules import inject_locale_rule, load_ruleset

    converter = _load_converter()
    rules: dict[str, str] = {}
    categories: dict[str, str] = {}
    for category in converter.load_categories(converter.DEFAULT_RULES_DIR):
        converted = converter.convert_category(category)
        categories[category["id"]] = f"{PACK}:{converted['id']}"
        # The converter skips the rules 3.0 removed, so pair by rule id.
        kept = {r["id"] for r in converted["rules"]}
        for old in category["rules"]:
            if old["id"] in kept:
                rules[f"{category['id']}.{old['id']}"] = (
                    f"{PACK}:{converted['id']}.{old['id']}"
                )
    injected = load_ruleset()
    note = inject_locale_rule(injected, "en-US")
    if note is not None:
        raise SystemExit(f"cannot list the generated locale rule: {note}")
    for rule in injected.all_rules:
        rules.setdefault(rule.qualified_id, f"{PACK}:{rule.qualified_id}")
    rules.setdefault(ENGINE_RULE, f"{PACK}:{ENGINE_RULE}")
    with_rules = {rule.category for rule in injected.all_rules}
    rule_less: set[str] = set()
    for profile in PROFILES:
        # The profile tables, not the styles: the styles leave out the categories that
        # have no rule, and a 2.x config can still name them. 3.0 has no such category,
        # so they go under [removed], not into the rename table.
        for name in reference.mapping.engine_states(profile)[1].categories:
            if name in with_rules:
                categories.setdefault(name, f"{PACK}:{name}")
            else:
                rule_less.add(name)
    for name in REMOVED_RULES:
        if rules.pop(name, None) is None:
            raise SystemExit(f"removed rule {name} is not a 2.x rule")
    return (
        dict(sorted(rules.items())),
        dict(sorted(categories.items())),
        sorted(rule_less),
    )


def renames_toml(
    rules: dict[str, str], categories: dict[str, str], rule_less: list[str]
) -> str:
    lines = [
        f"# Generated by {GENERATOR}. Do not edit.",
        "# The 2.x to 3.0 rename table: each 2.x rule id and category name, and its 3.0",
        "# qualified id. A 2.x name with no entry stops `slopvac migrate`, unless it is",
        "# listed under [removed]: those rules and categories no longer exist and `migrate`",
        "# drops them.",
        "",
        "[rules]",
        *(f"{_q(k)} = {_q(v)}" for k, v in rules.items()),
        "",
        "[categories]",
        *(f"{_q(k)} = {_q(v)}" for k, v in categories.items()),
        "",
        "[removed]",
        "rules = [",
        *(f"  {_q(name)}," for name in sorted(REMOVED_RULES)),
        "]",
        # taplo keeps a short array on one line, so the generator writes it that way too.
        "categories = [" + ", ".join(_q(name) for name in rule_less) + "]",
    ]
    return "\n".join(lines) + "\n"


def style_toml(profile: str) -> str:
    source = PROFILES_DIR / f"legacy-{profile}.toml"
    header = (
        f"# Generated by {GENERATOR}. Do not edit.\n"
        f"# A verbatim copy of docs/contract/compat/profiles/{source.name}:\n"
    )
    return header + source.read_text(encoding="utf-8")


# Layouts that `migrate --preserve-behavior` refuses because the companion vocabulary style
# cannot keep the 2.x levels (docs/contract/profiles.md, section 4, limit). Each entry is a
# 2.x config and the key that the refusal names. Any profile switch counts, because a
# switch block restates the category levels, which outrank the companion style.
_BLOCKLIST = '[vocabulary]\npath = "deploy.toml"\n'
_OVERRIDE_PATH = (
    '[[overrides]]\nfiles = ["docs/**"]\n[overrides.vocabulary]\npath = "other.toml"\n'
)
REFUSED_BLOCKLIST_LAYOUTS = {
    "path-only-in-an-override": (
        f"schema_version = 2\n{_OVERRIDE_PATH}",
        "overrides[0].vocabulary.path",
    ),
    "path-only-in-an-override-with-a-nested-switch": (
        f'schema_version = 2\n{_OVERRIDE_PATH}\n[[overrides]]\nfiles = ["docs/api/**"]\nprofile = "relaxed"\n',
        "overrides[0].vocabulary.path",
    ),
    "switch-to-another-profile": (
        f'schema_version = 2\n{_BLOCKLIST}\n[[overrides]]\nfiles = ["docs/**"]\nprofile = "strict"\n',
        "overrides[0].profile",
    ),
    "moved-path-with-a-nested-switch": (
        f'schema_version = 2\n{_BLOCKLIST}\n{_OVERRIDE_PATH}\n[[overrides]]\nfiles = ["docs/api/**"]\nprofile = "relaxed"\n',
        "overrides[1].profile",
    ),
    "switch-to-the-same-profile-normal": (
        f'schema_version = 2\n{_BLOCKLIST}\n[[overrides]]\nfiles = ["docs/**"]\nprofile = "normal"\n',
        "overrides[0].profile",
    ),
    "switch-to-the-same-profile-strict": (
        f'schema_version = 2\nprofile = "strict"\n{_BLOCKLIST}\n[[overrides]]\nfiles = ["docs/**"]\nprofile = "strict"\n',
        "overrides[0].profile",
    ),
    "top-profile-differs-from-the-switch": (
        f'schema_version = 2\nprofile = "relaxed"\n{_BLOCKLIST}\n[[overrides]]\nfiles = ["docs/**"]\nprofile = "normal"\n',
        "overrides[0].profile",
    ),
}


def refused_blocklist_json() -> str:
    doc = {
        "_doc": "Configs that `migrate --preserve-behavior` refuses with migrate-output-invalid, because the vocabulary companion style cannot keep the 2.x levels for them (docs/contract/profiles.md, section 4, limit). `at` is the key that the refusal names. tests/preserve.rs reads this list.",
        "layouts": {
            name: {"config": config, "at": at}
            for name, (config, at) in REFUSED_BLOCKLIST_LAYOUTS.items()
        },
    }
    return json.dumps(doc, indent=1, sort_keys=True) + "\n"


def catalogue_toml(ruleset) -> str:
    mapping = json.loads((PROFILES_DIR / "mapping.json").read_text(encoding="utf-8"))
    lines = [
        f"# Generated by {GENERATOR}. Do not edit.",
        "# The 2.x rule catalogue that `migrate --preserve-behavior` needs: each rule's kind",
        "# (checked or judgement) and tier per profile, from the 2.x ruleset, each",
        "# profile's per-rule defaults, from docs/contract/compat/profiles/mapping.json,",
        "# and each category's own weight, which 2.x used where no profile set one.",
    ]
    for profile in PROFILES:
        lines += ["", f"[defaults.{profile}]"]
        defaults = mapping["profiles"][profile]["rule_defaults"]
        lines.extend(f"{_q(k)} = {_q(v)}" for k, v in sorted(defaults.items()))
    lines += ["", "[weights]"]
    lines.extend(
        f"{_q(name)} = {_inline(float(weight))}"
        for name, weight in sorted(_category_weights(ruleset).items())
    )
    lines += ["", "[rules]"]
    for rule in sorted(ruleset.all_rules, key=lambda r: r.qualified_id):
        kind = "judgement" if rule.kind.value == "judgement" else "checked"
        tiers = ", ".join(f"{p} = {_q(rule.tier_for(p).value)}" for p in PROFILES)
        lines.append(f"{_q(rule.qualified_id)} = {{ kind = {_q(kind)}, {tiers} }}")
    return "\n".join(lines) + "\n"


def _qualified(
    expanded: dict, rules: dict[str, str], categories: dict[str, str]
) -> dict:
    """``expand()`` output under 3.0 names, with empty tables left out."""

    def layer(src: dict, out: dict) -> None:
        if src.get("categories"):
            out["categories"] = {categories[k]: v for k, v in src["categories"].items()}
        if src.get("rules"):
            kept = {
                rules[k]: v for k, v in src["rules"].items() if k not in REMOVED_RULES
            }
            if kept:
                out["rules"] = kept

    out = {
        "profile": expanded["profile"],
        "styles": [f"{PACK}:{s}" for s in expanded["styles"]],
    }
    layer(expanded, out)
    blocks = []
    for block in expanded["overrides"]:
        new = {"files": block["files"]}
        if "profile" in block:
            new["profile"] = block["profile"]
        layer(block, new)
        blocks.append(new)
    if blocks:
        out["overrides"] = blocks
    return out


def _error_block(config2: dict) -> int:
    for index, block in enumerate(config2["overrides"]):
        try:
            reference.parse_glob(block["files"])
        except reference.AmbiguousOverlap:
            return index
    raise AssertionError("no block is outside the grammar")


def _category_weights(ruleset) -> dict[str, float]:
    """Each style category's own weight, as 2.x scoring falls back to it
    (``score._category_result``): the category YAML's weight, else 1.0."""
    names = {n for p in PROFILES for n in reference.style_layer(p)["categories"]}
    return {name: ruleset.weights.get(name, 1.0) for name in names}


def _dials(resolved, weights: dict[str, float]) -> dict:
    """Each category's budget and effective weight as 2.x resolved them, or ``off``.

    The weight is the one 2.x scored with: the resolved weight, else the category's own.
    """
    out = {}
    for name, settings in sorted(resolved.categories.items()):
        if settings.severity.value == "off":
            out[name] = "off"
        else:
            weight = settings.weight
            out[name] = {
                "budget": settings.max_per_100_words,
                "weight": weights.get(name, 1.0) if weight is None else weight,
            }
    return out


def _measure(config2: dict, path: str | None, weights: dict) -> tuple[str, dict, dict]:
    """The 2.x engine's profile, level per rule and dials per category for one file."""
    profile = config2.get("profile", "normal")
    config = reference.to_2x_config(config2) if path else None
    _, resolved, states = reference.mapping.engine_states(
        profile, config, path or "doc.md"
    )
    levels = {qid: state[1] for qid, state in sorted(states.items())}
    return resolved.profile.value, levels, _dials(resolved, weights)


def measured_2x(ruleset, configs: dict, paths: dict[str, list[str]]) -> dict:
    """What the 2.x engine resolves for each scenario and file: the ground truth that
    the migrated config must reproduce (M1).

    ``base`` holds each profile with no authored config. A file names the profile 2.x
    applied to it and lists only the rule levels and category dials that differ from
    the base of that profile.
    """
    weights = _category_weights(ruleset)
    base = {p: _measure({"profile": p}, None, weights)[1:] for p in PROFILES}
    out: dict = {
        "base": {
            p: {"levels": levels, "dials": dials} for p, (levels, dials) in base.items()
        },
        "files": {},
    }
    for name, config2 in configs.items():
        files = {}
        for path in paths[name]:
            profile, levels, dials = _measure(config2, path, weights)
            levels0, dials0 = base[profile]
            files[path] = {
                "profile": profile,
                "levels": {k: v for k, v in levels.items() if levels0[k] != v},
                "dials": {k: v for k, v in dials.items() if dials0[k] != v},
            }
        out["files"][name] = files
    return out


def vectors_json(ruleset, rules: dict[str, str], categories: dict[str, str]) -> str:
    excluded = next(
        r for r in ruleset.rules if r.tier_for("relaxed").value == "excluded"
    )
    configs = {name: config2 for name, (config2, _) in scenarios.SCENARIOS.items()}
    configs |= EXTRA_SCENARIOS
    # The same choice as test_ignored_2x_rule_entry_is_dropped.
    configs["ignored-rule-entry-is-dropped"] = {
        "profile": "relaxed",
        "rules": {excluded.qualified_id: "error"},
    }
    paths = {name: list(files) for name, (_, files) in scenarios.SCENARIOS.items()}
    paths |= EXTRA_PATHS
    missing = sorted(set(configs) - set(paths))
    if missing:
        raise AssertionError(f"scenarios without measured files: {missing}")
    expansions = {
        name: {
            "input": config_toml(config2),
            "expected": _qualified(
                reference.expand(config2, ruleset), rules, categories
            ),
        }
        for name, config2 in configs.items()
    }

    globs, glob_ids = _parametrize(
        scenarios.test_expansion_stops_on_globs_outside_the_grammar
    )
    errors = {}
    for name, files in zip(glob_ids, globs, strict=True):
        # The config of test_expansion_stops_on_globs_outside_the_grammar.
        config2 = {
            "profile": "normal",
            "overrides": [
                {"files": files, "profile": "relaxed"},
                {"files": ["docs/api/**"], "profile": "strict"},
            ],
        }
        try:
            reference.expand(config2, ruleset)
        except reference.AmbiguousOverlap:
            pass
        else:
            raise AssertionError(f"{name}: the reference expanded {files}")
        errors[name] = {"input": config_toml(config2), "block": _error_block(config2)}

    region_cases, region_ids = _parametrize(
        scenarios.test_each_path_falls_in_exactly_one_region_that_names_its_blocks
    )
    regions = {}
    for name, blocks in zip(region_ids, region_cases, strict=True):
        if any(len(files) != 1 for files in blocks):
            raise AssertionError(f"{name}: a region block holds one pattern")
        regions[name] = {
            "blocks": [files[0] for files in blocks],
            "regions": [
                {"files": files, "scope": sorted(scope)}
                for scope, files in reference.build_regions(blocks)
            ],
        }

    data = {
        "_generated": f"Generated by {GENERATOR}. Do not edit.",
        "errors": errors,
        "expansions": expansions,
        "measured_2x": measured_2x(ruleset, configs, paths),
        "regions": regions,
    }
    return json.dumps(data, indent=1, sort_keys=True, ensure_ascii=False) + "\n"


def outputs() -> dict[str, str]:
    """Each generated file, keyed by its path relative to the crate."""
    ruleset = reference.mapping.engine_states("normal")[0]
    rules, categories, rule_less = renames()
    files = {
        "data/renames.toml": renames_toml(rules, categories, rule_less),
        "data/legacy/catalogue.toml": catalogue_toml(ruleset),
        "tests/fixtures/preserve/refused-blocklist.json": refused_blocklist_json(),
        "tests/fixtures/preserve/vectors.json": vectors_json(
            ruleset, rules, categories
        ),
    }
    for profile in PROFILES:
        files[f"data/legacy/legacy-{profile}.toml"] = style_toml(profile)
    return files


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--check", action="store_true", help="exit 1 when a file is stale"
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=CRATE,
        help="the crate directory to write or check (default: slopvac-migrate)",
    )
    args = parser.parse_args(argv)
    stale = []
    for name, text in outputs().items():
        path = args.out / name
        current = path.read_text(encoding="utf-8") if path.exists() else None
        if current == text:
            continue
        stale.append(name)
        if not args.check:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
    if args.check and stale:
        print(
            f"stale or missing in {args.out.name}, rerun {GENERATOR}: {', '.join(stale)}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
