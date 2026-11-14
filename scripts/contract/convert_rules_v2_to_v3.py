"""Convert 2.x rule YAML to v3 category instances.

Reads ``packages/slopvac-lint/src/slopvac/rules/*.yml`` (or any directory of 2.x
rule files) and writes one ``<category>.json`` per category, valid against
``schemas/v3/category.schema.json``. Spec: docs/contract/rules.md.

    python scripts/contract/convert_rules_v2_to_v3.py OUT_DIR [--rules-dir DIR]
        [--tiers-out FILE]

2.x ``tiers`` are dropped from the instances. ``--tiers-out`` writes them to one
JSON file keyed by ``<category>.<rule>`` so the profile mapping (slopvac-8l4.2.10)
can consume them.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

import yaml

DEFAULT_RULES_DIR = (
    Path(__file__).resolve().parents[2] / "packages/slopvac-lint/src/slopvac/rules"
)
SCHEMA_VERSION = 3
# Upper bound given to an unbounded quantifier inside a lookbehind. PCRE2 caps the
# total lookbehind length at 255 characters; every shipped lookbehind stays below.
LOOKBEHIND_REPEAT = 32
ALLOWED_WORDS_PLACEHOLDER = "ALLOWED_WORDS"
# Exceptions the 2.x engine resolves mechanically (engine.py, `"quotation" in
# rule.exceptions`). Every other exception is an annotation-only reason.
MECHANICAL_DETECTORS = frozenset({"quotation"})
LEXICAL_KINDS = frozenset({"tokens", "pattern", "substitution"})
CHECK_KINDS = frozenset({"structure", "vocabulary"})

_UNICODE_ESCAPE = re.compile(r"(?<!\\)((?:\\\\)*)\\u([0-9a-fA-F]{4})")
_OPEN_ENDED_REPEAT = re.compile(r"\{(\d+),\}")


def rewrite_unicode_escapes(pattern: str) -> str:
    """PCRE2 has no ``\\uXXXX``; write ``\\x{XXXX}``."""
    return _UNICODE_ESCAPE.sub(lambda m: f"{m.group(1)}\\x{{{m.group(2)}}}", pattern)


def _class_end(pattern: str, start: int) -> int:
    """Index just past the character class that opens at ``start``."""
    i = start + 1
    if pattern[i : i + 1] == "^":
        i += 1
    if pattern[i : i + 1] == "]":
        i += 1
    while i < len(pattern):
        if pattern[i] == "\\":
            i += 2
        elif pattern.startswith("[:", i):
            end = pattern.find(":]", i + 2)
            i = len(pattern) if end < 0 else end + 2
        elif pattern[i] == "]":
            return i + 1
        else:
            i += 1
    return len(pattern)


def bound_lookbehinds(pattern: str) -> tuple[str, bool]:
    """Give every unbounded quantifier inside a lookbehind a finite maximum.

    Returns the new pattern and whether anything changed. ``+`` becomes
    ``{1,N}``, ``*`` becomes ``{0,N}`` and ``{n,}`` becomes ``{n,n+N}``, where N is
    LOOKBEHIND_REPEAT. A quantifier inside a character class is a literal and stays.
    """
    out: list[str] = []
    stack: list[bool] = []
    changed = False
    i = 0
    while i < len(pattern):
        ch = pattern[i]
        if ch == "\\":
            out.append(pattern[i : i + 2])
            i += 2
        elif ch == "[":
            end = _class_end(pattern, i)
            out.append(pattern[i:end])
            i = end
        elif ch == "(":
            stack.append(pattern.startswith(("(?<=", "(?<!"), i))
            out.append(ch)
            i += 1
        elif ch == ")":
            if stack:
                stack.pop()
            out.append(ch)
            i += 1
        elif any(stack) and ch in "+*":
            low = 1 if ch == "+" else 0
            out.append(f"{{{low},{LOOKBEHIND_REPEAT}}}")
            changed = True
            i += 1
        elif any(stack) and (m := _OPEN_ENDED_REPEAT.match(pattern, i)):
            low = int(m.group(1))
            out.append(f"{{{low},{low + LOOKBEHIND_REPEAT}}}")
            changed = True
            i = m.end()
        else:
            out.append(ch)
            i += 1
    return "".join(out), changed


def to_pcre2(pattern: str) -> str:
    """Rewrite a 2.x (Python ``regex``) pattern into the PCRE2 subset v3 allows."""
    return bound_lookbehinds(rewrite_unicode_escapes(pattern))[0]


def _convert_match(rule: dict) -> dict:
    kind = rule["kind"]
    match: dict = {"kind": kind}
    if kind == "tokens":
        match["tokens"] = list(rule["tokens"])
    elif kind == "pattern":
        match["pattern"] = to_pcre2(rule["pattern"])
        if ALLOWED_WORDS_PLACEHOLDER in rule["pattern"]:
            match["placeholders"] = {ALLOWED_WORDS_PLACEHOLDER: "allowed-words"}
    elif kind == "substitution":
        match["substitutions"] = {
            to_pcre2(k): v for k, v in rule["substitutions"].items()
        }
    elif kind == "metric":
        match["metric"] = rule["metric"]
        match["threshold"] = rule["threshold"]
        if "comparison" in rule:
            match["comparison"] = rule["comparison"]
    elif kind in CHECK_KINDS:
        # 2.x dispatches these by rule id; v3 names the check.
        match["check"] = rule["id"]
    elif kind == "judgement":
        match["question"] = rule["judgement_question"]
    else:  # pragma: no cover - guarded by the 2.x loader
        raise ValueError(f"{rule['id']}: unknown kind {kind!r}")
    if kind in LEXICAL_KINDS:
        for key in ("ignore_case", "match_all_caps"):
            if key in rule:
                match[key] = rule[key]
    return match


def _convert_exception(name: str) -> dict:
    exception = {"name": name}
    if name in MECHANICAL_DETECTORS:
        exception["detector"] = name
    return exception


def convert_rule(rule: dict) -> dict:
    """One 2.x rule mapping to one v3 rule mapping. ``tiers`` is dropped."""
    out: dict = {
        "id": rule["id"],
        "name": rule["name"],
        "message": rule["message"],
    }
    judgement = rule["kind"] == "judgement"
    # The 2.x loader forces judgement severity to suggestion and defaults the rest.
    out["severity"] = "suggestion" if judgement else rule.get("severity", "warning")
    out["scope"] = rule.get("scope", "prose")
    if rule.get("text_type", "any") != "any":
        out["text_type"] = rule["text_type"]
    out["match"] = _convert_match(rule)
    if rule.get("exceptions"):
        out["exceptions"] = [_convert_exception(n) for n in rule["exceptions"]]
    if rule.get("allowlist"):
        out["allowlist"] = list(rule["allowlist"])
    if rule.get("examples"):
        out["examples"] = [
            {k: v for k, v in e.items() if v is not None} for e in rule["examples"]
        ]
    if rule.get("fix") is not None:
        out["fix"] = rule["fix"]
    for key in ("ai_signal", "ai_signal_source"):
        if key in rule:
            out[key] = rule[key]
    out["citation"] = dict(rule["provenance"])
    return out


def convert_category(category: dict) -> dict:
    """One 2.x category mapping to a v3 category instance."""
    out: dict = {
        "schema_version": SCHEMA_VERSION,
        "id": category["id"],
        "title": category["title"],
        "description": category["description"],
    }
    for key in ("weight", "max_per_100_words", "recommended_for"):
        if key in category:
            out[key] = category[key]
    out["rules"] = [convert_rule(r) for r in category["rules"]]
    return out


def extract_tiers(category: dict) -> dict[str, dict[str, str]]:
    """The 2.x per-profile dispositions, keyed by ``<category>.<rule>``."""
    return {
        f"{category['id']}.{r['id']}": dict(r["tiers"])
        for r in category["rules"]
        if "tiers" in r
    }


def load_categories(rules_dir: Path) -> list[dict]:
    """Every category document in ``rules_dir``, in file then document order."""
    categories: list[dict] = []
    for path in sorted(rules_dir.glob("*.y*ml")):
        for document in yaml.safe_load_all(path.read_text(encoding="utf-8")):
            if isinstance(document, dict):
                categories.append(document)
    return categories


def convert_dir(rules_dir: Path) -> list[dict]:
    return [convert_category(c) for c in load_categories(rules_dir)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("--rules-dir", type=Path, default=DEFAULT_RULES_DIR)
    parser.add_argument("--tiers-out", type=Path)
    args = parser.parse_args(argv)

    categories = load_categories(args.rules_dir)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    tiers: dict[str, dict[str, str]] = {}
    for category in categories:
        converted = convert_category(category)
        path = args.out_dir / f"{converted['id']}.json"
        path.write_text(
            json.dumps(converted, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        tiers.update(extract_tiers(category))
    if args.tiers_out:
        args.tiers_out.write_text(
            json.dumps(tiers, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    print(f"converted {len(categories)} categories", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
