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
ALLOWED_WORDS_PLACEHOLDER = "ALLOWED_WORDS"
# Exceptions the 2.x engine resolves mechanically (engine.py, `"quotation" in
# rule.exceptions`). Every other exception is an annotation-only reason.
MECHANICAL_DETECTORS = frozenset({"quotation"})
LEXICAL_KINDS = frozenset({"tokens", "pattern", "substitution"})
CHECK_KINDS = frozenset({"structure", "vocabulary"})

# The only 2.x rule that carries an ALLOWED_WORDS placeholder; the 2.x loader
# (rules._inject_heading_allow) substitutes it for this rule id alone.
PLACEHOLDER_RULE_ID = "title-case-heading"
# 2.x rules that 3.0 does not ship, as `<category>.<rule>`. They never had an
# implementation. The converter and the generators that list 2.x rules skip them.
REMOVED_RULES = frozenset(
    {
        "ste-sentences.vertical-list-lead-in-missing-colon",
        "ste-sentences.vertical-list-item-punctuation",
        "ste-safety.safety-block-missing-consequence",
        "ste-words.inconsistent-term-for-same-thing",
        "ste-practices.inconsistent-wording-for-same-step",
    }
)

CATEGORY_ID = re.compile(r"^[a-z][a-z0-9-]*$")

_UNICODE_ESCAPE = re.compile(
    r"(?<!\\)((?:\\\\)*)\\(?:u([0-9a-fA-F]{4})|U([0-9a-fA-F]{8}))"
)
_OPEN_ENDED_REPEAT = re.compile(r"\{(\d+),\}")


class ConversionError(ValueError):
    """A 2.x rule that cannot become a valid v3 rule without a human edit."""


def unicode_escapes(pattern: str) -> list[str]:
    """Return each ``\\uXXXX`` or ``\\UXXXXXXXX`` escape in ``pattern``."""
    return [m.group(0)[len(m.group(1)) :] for m in _UNICODE_ESCAPE.finditer(pattern)]


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


def unbounded_lookbehinds(pattern: str) -> list[str]:
    """Return each lookbehind in ``pattern`` that holds ``+``, ``*`` or ``{n,}``.

    A quantifier inside a character class is a literal and does not count.
    """
    found: list[str] = []
    stack: list[tuple[int, bool]] = []  # (start, is_lookbehind)
    flagged: set[int] = set()
    i = 0
    while i < len(pattern):
        ch = pattern[i]
        if ch == "\\":
            i += 2
        elif ch == "[":
            i = _class_end(pattern, i)
        elif ch == "(":
            stack.append((i, pattern.startswith(("(?<=", "(?<!"), i)))
            i += 1
        elif ch == ")":
            if stack:
                start, lookbehind = stack.pop()
                if lookbehind and start in flagged:
                    found.append(pattern[start : i + 1])
            i += 1
        else:
            m = _OPEN_ENDED_REPEAT.match(pattern, i)
            if ch in "+*" or m:
                flagged.update(start for start, lb in stack if lb)
            i = m.end() if m else i + 1
    return found


def to_pcre2(pattern: str) -> str:
    """Check that a 2.x pattern is already in the PCRE2 subset v3 allows.

    The converter does not rewrite. It raises ConversionError for a ``\\u`` or
    ``\\U`` escape (write the literal character, or ``\\x{...}`` in v3-only
    rules) and for an unbounded lookbehind. The fix is a data edit to the rule.
    """
    if escapes := unicode_escapes(pattern):
        raise ConversionError(
            f"Unicode escape {escapes[0]!r}; write the literal character"
        )
    if bad := unbounded_lookbehinds(pattern):
        raise ConversionError(f"unbounded lookbehind {bad[0]!r}")
    return pattern


def _convert_match(rule: dict) -> dict:
    try:
        return _convert_match_unchecked(rule)
    except ConversionError as exc:
        raise ConversionError(f"{rule['id']}: {exc}") from exc


def _convert_match_unchecked(rule: dict) -> dict:
    kind = rule["kind"]
    match: dict = {"kind": kind}
    if kind == "tokens":
        match["tokens"] = list(rule["tokens"])
    elif kind == "pattern":
        match["pattern"] = to_pcre2(rule["pattern"])
        if (
            rule["id"] == PLACEHOLDER_RULE_ID
            and ALLOWED_WORDS_PLACEHOLDER in rule["pattern"]
        ):
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
    out["citation"] = {k: v for k, v in rule["provenance"].items() if v is not None}
    return out


def convert_category(category: dict) -> dict:
    """One 2.x category mapping to a v3 category instance."""
    if not CATEGORY_ID.fullmatch(str(category.get("id", ""))):
        raise ConversionError(f"invalid category id {category.get('id')!r}")
    out: dict = {
        "schema_version": SCHEMA_VERSION,
        "id": category["id"],
        "title": category["title"],
        "description": category["description"],
    }
    for key in ("weight", "max_per_100_words", "recommended_for"):
        if key in category:
            out[key] = category[key]
    out["rules"] = [
        convert_rule(r)
        for r in category["rules"]
        if f"{category['id']}.{r['id']}" not in REMOVED_RULES
    ]
    return out


def extract_tiers(category: dict) -> dict[str, dict[str, str]]:
    """The 2.x per-profile dispositions, keyed by ``<category>.<rule>``."""
    return {
        f"{category['id']}.{r['id']}": dict(r["tiers"])
        for r in category["rules"]
        if "tiers" in r and f"{category['id']}.{r['id']}" not in REMOVED_RULES
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
        try:
            converted = convert_category(category)  # validates the id used below
        except ConversionError as exc:
            print(f"error: {category.get('id')}: {exc}", file=sys.stderr)
            return 1
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
