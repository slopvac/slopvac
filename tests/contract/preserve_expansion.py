"""Reference for `slopvac migrate --preserve-behavior` and the 3.0 level resolution.

``expand`` turns a 2.x config into a 3.0 config that selects the generated
``legacy-<profile>`` styles. ``resolve`` applies the 3.0 precedence the profile
mapping relies on. Both are test references: the real ``migrate`` and resolver
replace them and keep the same scenarios.

3.0 level precedence (docs/contract/profiles.md, section 4): among the entries
that name a checked rule or its category, the highest layer wins. In one layer a
rule entry beats a category entry. A category entry that wins is raised to the
merged ``minimum_severity``. Judgement rules take their level from rule entries
only.
"""

from __future__ import annotations

import copy
import importlib.util
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "profile_mapping", ROOT / "scripts" / "contract" / "profile_mapping.py"
)
mapping = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mapping)

RANK = {"suggestion": 0, "warning": 1, "error": 2}


def dials(value) -> dict:
    return {"severity": value} if isinstance(value, str) else dict(value)


def style_layer(profile: str) -> dict:
    import tomllib

    path = mapping.OUT / f"{mapping.style_name(profile)}.toml"
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    return {
        "categories": {k: dials(v) for k, v in data["categories"].items()},
        "rules": dict(data.get("rules", {})),
    }


def resolve(layers: list[dict], category: str, qid: str, shipped: str, judgement: bool):
    """Level of one rule under ordered layers (lowest first).

    A rule entry beats a category entry unless the category entry sits in a higher
    layer and the rule entry is not ``off``. An ``off`` rule entry holds against any
    category entry; only a rule entry in a higher layer revives it.
    """
    rule_hit = cat_hit = None
    for index, layer in enumerate(layers):
        if qid in layer.get("rules", {}):
            rule_hit = (index, layer["rules"][qid])
        if not judgement:
            sev = dials(layer.get("categories", {}).get(category, {})).get("severity")
            if sev is not None:
                cat_hit = (index, sev)
    if rule_hit and (rule_hit[1] == "off" or not cat_hit or cat_hit[0] <= rule_hit[0]):
        return rule_hit[1]
    if not cat_hit:
        return shipped
    level = cat_hit[1]
    if level != "off":
        floor = None
        for layer in layers:
            floor = dials(layer.get("categories", {}).get(category, {})).get(
                "minimum_severity", floor
            )
        if floor not in (None, "off") and RANK[level] < RANK[floor]:
            level = floor
    return level


def category_dial(layers: list[dict], category: str, key: str):
    """Merged dial. The string none clears a budget set by a lower layer."""
    value = None
    for layer in layers:
        value = dials(layer.get("categories", {}).get(category, {})).get(key, value)
    return None if value == "none" else value


def matches_files(files: list[str], path: str) -> bool:
    """The gitignore matcher that 2.x used for ``[[overrides]] files``."""
    from slopvac.config import _path_spec

    return _path_spec(files).match_file(path)


class AmbiguousOverlap(Exception):
    """An override glob outside the supported grammar, so overlaps cannot be stated."""


# The expansion below is a reference for crates/slopvac-migrate (E1 to E6). The glob
# handling is pure functions over a containment forest: parse_glob, relate,
# build_regions. Port those three first.

_SEGMENT = re.compile(r"^[^*?\[\]\\!#/]+$")


def parse_glob(files: list[str]) -> tuple[tuple[str, ...], bool]:
    """Grammar: anchored literal path segments with an optional trailing ``/**``.

    Returns (segments, is_tree). ``docs/**`` is the tree of ``docs``. ``docs/a.md``
    is one file. Everything else is rejected: a single pattern is required, it must
    contain a slash (a pattern without one matches in every directory), and no
    segment may hold a wildcard, a negation, a comment mark, ``.`` or ``..``.
    """
    if len(files) != 1:
        raise AmbiguousOverlap(f"a block with {len(files)} patterns: {files}")
    glob = files[0]
    tree = glob.endswith("/**")
    body = glob.removesuffix("/**")
    segments = tuple(body.split("/"))
    if "/" not in glob or any(
        not _SEGMENT.match(seg) or seg in (".", "..") for seg in segments
    ):
        raise AmbiguousOverlap(f"unsupported pattern {glob!r}")
    return segments, tree


def relate(a: tuple[tuple[str, ...], bool], b: tuple[tuple[str, ...], bool]) -> str:
    """equal, contains (a holds b), contained (a lies in b) or disjoint."""
    if a == b:
        return "equal"
    (sa, ta), (sb, tb) = a, b
    if ta and len(sb) > len(sa) and sb[: len(sa)] == sa:
        return "contains"
    if tb and len(sa) > len(sb) and sa[: len(sb)] == sb:
        return "contained"
    return "disjoint"


def build_regions(blocks: list[list[str]]) -> list[tuple[frozenset[int], list[str]]]:
    """The regions of files that the blocks cut out, as (blocks matching, files list).

    Equal globs form one node. A node's parent is the smallest node that holds it,
    so the nodes form a forest. A region is a node minus its children, plus the
    root region of the files that no block matches: nodes + 1 regions.
    """
    parsed = [parse_glob(files) for files in blocks]
    nodes: dict[tuple[tuple[str, ...], bool], list[int]] = {}
    for index, key in enumerate(parsed):
        nodes.setdefault(key, []).append(index)
    parent: dict[tuple, tuple | None] = {}
    for key in nodes:
        holders = [o for o in nodes if o != key and relate(o, key) == "contains"]
        parent[key] = max(holders, key=lambda o: len(o[0])) if holders else None
    children: dict[tuple | None, list[tuple]] = {}
    for key, up in parent.items():
        children.setdefault(up, []).append(key)

    def chain(key: tuple) -> frozenset[int]:
        members: set[int] = set()
        while key is not None:
            members.update(nodes[key])
            key = parent[key]
        return frozenset(members)

    def glob_of(key: tuple) -> str:
        return blocks[nodes[key][0]][0]

    regions: list[tuple[frozenset[int], list[str]]] = [
        (frozenset(), ["**", *(f"!{glob_of(k)}" for k in children.get(None, []))])
    ]
    for key in nodes:
        regions.append(
            (
                chain(key),
                [glob_of(key), *(f"!{glob_of(k)}" for k in children.get(key, []))],
            )
        )
    return regions


def expand(config2: dict, ruleset) -> dict:
    """The 3.0 config that reproduces ``config2`` under the legacy styles.

    E1 selects the style of the top-level profile and copies the authored tables.
    E2 states, in each layer that authors a category, the profile rule defaults
    of that category, because a rule default beat an authored category in 2.x.
    E3 puts, before the override blocks, one block per profile override. It holds
    the target style and ``none`` where the target has no budget.
    E4 handles an authored rule entry that 2.x ignored. A file matches a set of
    override blocks, and 2.x decides from the final profile and category levels of
    that set. The expansion lists the sets that the block globs allow. An entry
    that is ignored in every set it can see is dropped. An entry that is ignored in
    some sets stays, and a guard block at the end sets the rule to ``off`` for the
    files of each such set. Judgement entries are always dropped.
    E5 gives an authored category without a severity the severity in force.
    """
    top = config2.get("profile", "normal")
    overrides = config2.get("overrides", [])
    n = len(overrides)
    styles = {p: style_layer(p) for p in mapping.PROFILES}
    states = {p: mapping.engine_states(p)[2] for p in mapping.PROFILES}
    defaults = {p: mapping.rule_defaults(p) for p in mapping.PROFILES}
    by_category: dict[str, list] = {}
    for rule in ruleset.all_rules:
        by_category.setdefault(rule.category, []).append(rule)
    tier = {r.qualified_id: r for r in ruleset.all_rules}
    budgeted = {
        name
        for st in styles.values()
        for name, d in st["categories"].items()
        if "max_per_100_words" in d
    }

    def authored(cats: dict, profile: str) -> dict:
        out = {}
        for name, value in cats.items():
            d = dials(value)
            if "severity" not in d:
                d["severity"] = styles[profile]["categories"][name]["severity"]
            out[name] = d
        return out

    def rule_levels(rules: dict) -> dict:
        return {
            k: (v if isinstance(v, str) else v["severity"])
            for k, v in rules.items()
            if tier[k].kind.value != "judgement"
        }

    def sticky_for(categories: dict, profile: str, own_rules: dict) -> dict:
        out = {}
        for name in categories:
            for rule in by_category.get(name, []):
                qid = rule.qualified_id
                if qid in defaults[profile] and qid not in own_rules:
                    out[qid] = defaults[profile][qid]
        return out

    regions = build_regions([b["files"] for b in overrides])
    scopes = [scope for scope, _ in regions]
    region_files = dict(regions)

    top_real = {k: dials(v) for k, v in config2.get("categories", {}).items()}
    block_real = [
        {k: dials(v) for k, v in b.get("categories", {}).items()} for b in overrides
    ]

    def effective(scope: frozenset[int]) -> tuple[str, list[dict]]:
        profile = top
        cats = [top_real]
        for i in sorted(scope):
            profile = overrides[i].get("profile") or profile
            cats.append(block_real[i])
        return profile, cats

    def ignored(qid: str, scope: frozenset[int]) -> bool:
        profile, cats = effective(scope)
        rule = tier[qid]
        if rule.tier_for(profile).value == "excluded":
            return True
        severity = styles[profile]["categories"].get(rule.category, {}).get("severity")
        for layer in cats:
            severity = layer.get(rule.category, {}).get("severity", severity)
        return severity == "off"

    guards: dict[frozenset[int], dict[str, str]] = {}

    def keep(qid: str, visible: list[frozenset[int]]) -> bool:
        flags = [ignored(qid, s) for s in visible]
        if all(flags):
            return False
        for scope, flag in zip(visible, flags):
            if flag:
                guards.setdefault(scope, {})[qid] = "off"
        return True

    top_cat = authored(config2.get("categories", {}), top)
    top_rules = {
        k: v
        for k, v in rule_levels(config2.get("rules", {})).items()
        if keep(k, scopes)
    }
    out: dict = {"profile": top, "styles": [mapping.style_name(top)]}
    out["categories"] = copy.deepcopy(top_cat)
    out["rules"] = {**sticky_for(top_cat, top, top_rules), **top_rules}

    blocks = []
    switch = []
    for index, block in enumerate(overrides):
        profile = block.get("profile")
        in_force = profile or top
        cats = authored(block.get("categories", {}), in_force)
        visible = [s for s in scopes if index in s]
        rules = {
            k: v
            for k, v in rule_levels(block.get("rules", {})).items()
            if keep(k, visible)
        }
        rules = {**sticky_for(cats, in_force, rules), **rules}
        new = {"files": block["files"], "categories": cats, "rules": rules}
        if profile:
            new["profile"] = profile
            target = styles[profile]
            sw_cats = {}
            for name, d in target["categories"].items():
                kept = {k: v for k, v in d.items() if k not in top_real.get(name, {})}
                if (
                    "max_per_100_words" not in d
                    and name in budgeted
                    and "max_per_100_words" not in top_real.get(name, {})
                ):
                    kept["max_per_100_words"] = "none"
                if kept:
                    sw_cats[name] = kept
            keys = set().union(*(st["rules"] for st in styles.values())) | set(
                config2.get("rules", {})
            )
            sw_rules = {}
            for qid in sorted(keys):
                if qid in top_rules:
                    sw_rules[qid] = top_rules[qid]
                    continue
                rule = tier[qid]
                if (
                    rule.category in top_real
                    and rule.kind.value != "judgement"
                    and rule.tier_for(profile).value == "advisory"
                    and qid not in defaults[profile]
                ):
                    continue
                sw_rules[qid] = states[profile][qid][1]
            switch.append(
                {"files": block["files"], "categories": sw_cats, "rules": sw_rules}
            )
        blocks.append(new)

    guard_blocks = [
        {"files": region_files[scope], "categories": {}, "rules": rules}
        for scope, rules in sorted(guards.items(), key=lambda kv: sorted(kv[0]))
    ]
    out["overrides"] = [*switch, *blocks, *guard_blocks]
    return out


def to_2x_config(config2: dict):
    from slopvac.config import Config

    return Config.model_validate(config2)
