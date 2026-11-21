"""Additive-only check for schemas/v3 against a baseline.

Usage: python scripts/contract/schema_compat.py --base <git-ref>
       python scripts/contract/schema_compat.py --baseline-dir <dir>

The baseline is a git ref (read through ``git show``) or a directory with the same
layout as schemas/v3 (``*.schema.json`` plus ``conformance/``). The working tree
is compared against it. A change within one schema_version may add, never remove
or tighten (docs/contract/policies.md, section 3.9). A ``pattern`` that differs only
by ``(?![\\s\\S])`` against ``$`` is unchanged, where the anchor is a real end assertion
(not escaped, not inside a character class, no inline flag group): Python's ``re`` (the
reference validator) and PCRE2 read ``$`` as also matching before a final newline;
JSON Schema ``pattern`` uses ECMA-262, where it does not.

Each finding prints as ``file:json-pointer: kind: old -> new``. The kind is a stable
kebab-case code; a code is never reused and never changes meaning. An entry in
schemas/v3/compat-allow.toml suppresses one finding; an entry that matches nothing
is itself a finding. Entries that the baseline's own allow file already holds approved
a landed change: they never go stale, and they suppress only a finding that the
baseline also has against itself, unchanged, such as an unresolvable ``$ref``.

Exit codes: 0 compatible, 1 a breaking finding, 2 a usage or read error.

Limits, all of which fail closed:

- ``$ref`` is resolved across every file of schemas/v3 on both sides, with a cycle
  guard. A finding reached through a reference names the referrer pointer and the
  definition: ``file:/properties/p/$ref(common.schema.json#/$defs/D)/minLength``.
  Each side resolves against its own documents. A reference is joined to the ``$id``
  of the document that holds it and must name the ``$id`` of a schema of schemas/v3;
  a reference to any other URI, including a file of another directory with the same
  name, is reported as ``unverified``. Local ``#/`` pointers resolve; anchors do not.
  A schema without a string ``$id``, in the baseline or the new tree, is a read error.
- Proving that a recursive schema and its inlined expansion accept the same instances
  is out of scope. A pair of references that recurs unchanged is equal by induction.
  Where a reference was added or removed and the same target recurs, the comparison
  stops and the stop point is reported as ``unverified``; the corpus re-validation is
  the backstop.
- ``allOf``, ``anyOf``, ``oneOf`` and ``prefixItems`` are compared by position. A
  list whose members all survive in another order is reported as ``list-reordered``
  and needs an allow entry. A new ``anyOf``/``oneOf``, and an appended ``oneOf`` member
  that is not provably disjoint from the earlier members (different ``type``, different
  ``const``/``enum`` values, or a shared required property with different values), are
  reported as ``narrowing-possible``, as is an appended ``prefixItems`` member. An
  appended ``anyOf`` member passes in an input schema.
- Direction: schemas/v3/directions.toml classes each schema file as ``input``,
  ``output`` or ``both``; it must list every schema file and nothing else. ``both``
  follows the output rule. In an output schema, a new ``enum`` value or an appended
  ``anyOf``/``oneOf`` member, a widened ``type`` or a dropped ``enum``/``const`` is
  reported (``enum-widened``, ``alternative-added``, ``type-widened``)
  unless the same position in the new schema carries ``"x-slopvac-extensible": true``;
  a marked ``oneOf`` also skips ``narrowing-possible`` for members appended to an existing list (a newly introduced ``anyOf``/``oneOf`` is always checked). Removing
  a marker from an output schema is ``extensible-removed``. In an input schema all of
  these stay allowed. A change to one shared definition is reported once, at the
  definition and in the direction of the file that holds it, not again at each referrer;
  a retarget, or a reference added or removed, is still reported at the referrer. So a
  definition that an output schema references must live in an output or ``both`` file.
  An appended ``oneOf`` member in an unmarked output position gets two findings with
  different meanings: ``alternative-added`` says consumers see a new variant, and
  ``narrowing-possible`` says the member may overlap an old one, so a document that
  validated can fail ``oneOf``. The marker covers both.
- A keyword the check does not model is reported as ``unverified`` when it is
  added, removed or changed. Only the annotations in ``_ANNOTATIONS`` are exempt:
  ``description``, ``title``, ``examples``, ``$comment``, ``deprecated``,
  ``readOnly`` and ``writeOnly``. ``default`` is not exempt, because a changed
  default changes what consumers see.

``enum`` and ``const`` compare as JSON values: a bool is not a number, and 1 equals
1.0. Each added ``required`` field is its own finding, located by field name.

The corpus re-validation catches a tightening outside these rules only when a
baseline valid instance exercises it.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
import urllib.parse
from collections import Counter
from collections.abc import Callable, Iterator
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any

import yaml
from jsonschema import Draft202012Validator
from jsonschema.exceptions import SchemaError
from referencing import Registry, Resource
from referencing.exceptions import Unresolvable

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_REL = "schemas/v3"
SCHEMA_SUFFIX = ".schema.json"
ALLOW_FILE = "compat-allow.toml"
ALLOW_FIELDS = ("file", "pointer", "kind", "reason", "approved_by")
DIRECTIONS_FILE = "directions.toml"
DIRECTIONS = ("input", "output", "both")
EXTENSIBLE = "x-slopvac-extensible"
# Kinds that only an output schema (or a "both" schema) reports: an input schema may
# grow its enums and alternatives, but a consumer of output must be told it can.
OUTPUT_ONLY_KINDS = frozenset(
    {"enum-widened", "alternative-added", "extensible-removed", "type-widened"}
)

LOADERS: dict[str, Callable[[str], Any]] = {
    ".json": json.loads,
    ".toml": tomllib.loads,
    ".yaml": yaml.safe_load,
    ".yml": yaml.safe_load,
}

# Keywords whose value maps a name to a subschema.
_SCHEMA_MAPS = ("properties", "$defs", "patternProperties", "dependentSchemas")
_MAP_REMOVED_KIND = {
    "properties": "property-removed",
    "$defs": "def-removed",
    "patternProperties": "pattern-property-removed",
    "dependentSchemas": "dependent-schema-removed",
}
# Keywords whose value is one subschema compared recursively.
_SCHEMA_SINGLE = (
    "items",
    "if",
    "then",
    "else",
    "contains",
    "propertyNames",
    "unevaluatedProperties",
    "unevaluatedItems",
)
# Keywords whose value is a list of subschemas compared by position.
_SCHEMA_LISTS = ("prefixItems", "allOf", "anyOf", "oneOf")
_MIN_KEYWORDS = (
    "minimum",
    "exclusiveMinimum",
    "minLength",
    "minItems",
    "minProperties",
    "minContains",
)
_MAX_KEYWORDS = (
    "maximum",
    "exclusiveMaximum",
    "maxLength",
    "maxItems",
    "maxProperties",
    "maxContains",
)


class CompatError(Exception):
    """A usage or read error (exit 2)."""


@dataclass(frozen=True)
class Finding:
    file: str
    pointer: str
    kind: str
    old: str
    new: str

    def render(self) -> str:
        return f"{self.file}:{self.pointer}: {self.kind}: {self.old} -> {self.new}"


@dataclass(frozen=True)
class AllowEntry:
    file: str
    pointer: str
    kind: str
    reason: str
    approved_by: str

    def matches(self, finding: Finding) -> bool:
        return (self.file, self.pointer, self.kind) == (
            finding.file,
            finding.pointer,
            finding.kind,
        )


@dataclass(frozen=True)
class Snapshot:
    """Baseline schemas by file name and conformance files by path under conformance/.

    ``allow`` holds the baseline's own allow entries. They approved changes that are
    already in the baseline, so the check ignores them (see ``check``).
    """

    schemas: dict[str, dict]
    corpus: dict[str, str]
    allow: frozenset[AllowEntry] = frozenset()


def _show(value: object) -> str:
    if value is None:
        return "(absent)"
    return json.dumps(value, sort_keys=True, ensure_ascii=False)


def _esc(token: str) -> str:
    return token.replace("~", "~0").replace("/", "~1")


def _normalise(value: object) -> object:
    """JSON value equality: a bool is not a number, and 1 equals 1.0."""
    if isinstance(value, bool) or value is None or isinstance(value, str):
        return value
    if isinstance(value, float):
        return int(value) if value.is_integer() else value
    if isinstance(value, list):
        return [_normalise(v) for v in value]
    if isinstance(value, dict):
        return {k: _normalise(v) for k, v in value.items()}
    return value


def _canon(value: object) -> str:
    """Canonical text of a JSON value; equal text means JSON-equal values."""
    return json.dumps(_normalise(value), sort_keys=True)


def _same(a: object, b: object) -> bool:
    return _canon(a) == _canon(b)


_PORTABLE_END = r"(?![\s\S])"
# Any ``(?`` followed by a letter or ``-`` opens an inline flag or modifier group such as
# ``(?m)``, ``(?i:`` or ``(?-m:``. Flags can change what ``$`` means, so a pattern that
# holds one is never treated as equivalent. ``(?:``, ``(?=``, ``(?!``, ``(?<=``,
# ``(?<!`` and ``(?<name>`` do not match.
_INLINE_FLAGS = re.compile(r"\(\?[a-zA-Z-]")


def _with_dollar_for_end_anchors(pattern: str) -> str | None:
    """Return ``pattern`` with each real end assertion spelled ``$``.

    Only an ``(?![\\s\\S])`` outside a character class is rewritten. An escaped
    character (``\\$``) and anything inside a class (``[$]``, ``[(?![\\s\\S])]``) stay
    as they are. Classes follow ECMA-262 without the ``u`` flag: a class runs from an
    unescaped ``[`` to the next unescaped ``]``, so ``[]`` is an empty class and
    ``[^]`` matches any character. A backslash escapes the next character in and out of
    a class. Returns ``None`` for a pattern with an inline flag group anywhere, or one
    that ends in a lone backslash. No tokenizer for the ECMA-262 dialect was available:
    the checker is Python, and ``re._parser`` reads Python's dialect.
    """
    if _INLINE_FLAGS.search(pattern):
        return None
    out: list[str] = []
    i, n = 0, len(pattern)
    in_class = False
    while i < n:
        char = pattern[i]
        if char == "\\":
            if i + 1 >= n:
                return None
            out.append(pattern[i : i + 2])
            i += 2
        elif in_class:
            in_class = char != "]"
            out.append(char)
            i += 1
        elif char == "[":
            in_class = True
            out.append(char)
            i += 1
            if pattern.startswith("^", i):
                out.append("^")
                i += 1
        elif pattern.startswith(_PORTABLE_END, i):
            out.append("$")
            i += len(_PORTABLE_END)
        else:
            out.append(char)
            i += 1
    return "".join(out)


def _same_pattern(old: object, new: object) -> bool:
    """Equal patterns, treating the portable end anchor as ``$`` (see the module docstring)."""
    if old == new:
        return True
    if isinstance(old, str) and isinstance(new, str):
        old_norm = _with_dollar_for_end_anchors(old)
        new_norm = _with_dollar_for_end_anchors(new)
        return old_norm is not None and old_norm == new_norm
    return False


# --- structural comparison -------------------------------------------------------

# Maps whose added entries are compared with an unrestricted schema. New ``properties``
# and ``$defs`` entries are exempt: they are the allowed additions.
_CHECKED_WHEN_ADDED = ("patternProperties", "dependentSchemas")

# Keywords the comparer models. A change to any other keyword is reported as
# ``unverified``, so an unmodelled tightening can never pass silently.
_MODELLED = frozenset(
    {
        "$id",
        "$ref",
        "type",
        "enum",
        "const",
        "pattern",
        "format",
        "multipleOf",
        "uniqueItems",
        "not",
        "required",
        "dependentRequired",
        EXTENSIBLE,
        "additionalProperties",
        "additionalItems",
        *_SCHEMA_MAPS,
        *_SCHEMA_SINGLE,
        *_SCHEMA_LISTS,
        *_MIN_KEYWORDS,
        *_MAX_KEYWORDS,
    }
)
# Pure annotations: they never change which instances validate. ``default`` is
# deliberately absent: a changed default changes what consumers see, so it is
# ``unverified`` and needs an allow-file entry.
_ANNOTATIONS = frozenset(
    {
        "description",
        "title",
        "examples",
        "$comment",
        "deprecated",
        "readOnly",
        "writeOnly",
    }
)


def _as_schema(value: object) -> dict:
    """Normalise a boolean schema; ``true`` is the empty schema."""
    if value is True or value is None:
        return {}
    if value is False:
        return {"not": {}}
    return value if isinstance(value, dict) else {}


def _is_number(value: object) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _types(value: object) -> set[str] | None:
    if value is None:
        return None
    return {value} if isinstance(value, str) else set(value)  # type: ignore[arg-type]


def _type_satisfied(old_type: str, new_types: set[str]) -> bool:
    return old_type in new_types or (old_type == "integer" and "number" in new_types)


def _require_id(name: str, schema: object) -> str:
    """The ``$id`` of a schema file; a file without one is a read error."""
    declared = schema.get("$id") if isinstance(schema, dict) else None
    if not isinstance(declared, str) or not declared:
        raise CompatError(f"{name}: the schema has no string $id")
    return declared


def _uris(schemas: dict[str, dict]) -> dict[str, str]:
    """File name to ``$id``; the one mapping for reference and corpus resolution."""
    return {name: _require_id(name, schema) for name, schema in schemas.items()}


def _resolve(
    schemas: dict[str, dict], uris: dict[str, str], doc: str, ref: object
) -> tuple[str, str, Any] | None:
    """Resolve a ``$ref`` held by ``doc`` to (file, pointer, subschema).

    The reference is joined to the document's ``$id`` and must then name a registered
    schema exactly; any other URI is not resolvable and returns None.
    """
    if not isinstance(ref, str) or doc not in schemas:
        return None
    joined = urllib.parse.urljoin(uris[doc], ref)
    absolute, _, fragment = joined.partition("#")
    by_uri = {uri: name for name, uri in uris.items()}
    file = by_uri.get(absolute)
    if file is None or (fragment and not fragment.startswith("/")):
        return None
    node: Any = schemas[file]
    pointer = urllib.parse.unquote(fragment)
    for token in pointer.split("/")[1:]:
        token = token.replace("~1", "/").replace("~0", "~")
        if isinstance(node, dict) and token in node:
            node = node[token]
        elif isinstance(node, list) and token.isdigit() and int(token) < len(node):
            node = node[int(token)]
        else:
            return None
    return file, pointer, node


@dataclass(frozen=True)
class Docs:
    """The schema file each side's current subschema lives in; they differ after a hop."""

    old: str
    new: str


class Comparer:
    """Compares schema pairs, resolving every ``$ref`` across all files on both sides."""

    def __init__(self, old: dict[str, dict], new: dict[str, dict]) -> None:
        self.old, self.new = old, new
        self._old_uris, self._new_uris = _uris(old), _uris(new)
        self._stack: set[tuple] = set()
        # Targets cut by the cycle guard during the current computation; a result
        # computed while any cut is open is incomplete and must not be cached.
        self._cuts: set[tuple] = set()
        self._cache: dict[tuple, list[Finding]] = {}
        self._ref_memo: dict[int, bool] = {}

    def _has_ref(self, value: object) -> bool:
        if not isinstance(value, (dict, list)):
            return False
        known = self._ref_memo.get(id(value))
        if known is None:
            members = value.values() if isinstance(value, dict) else value
            known = (isinstance(value, dict) and "$ref" in value) or any(
                self._has_ref(v) for v in members
            )
            self._ref_memo[id(value)] = known
        return known

    def compare(
        self, file: str, doc: Docs, ptr: str, old: Any, new: Any
    ) -> Iterator[Finding]:
        """Yield a finding for each way ``new`` accepts less than ``old``.

        ``file`` is the schema file the finding is reported against; ``doc`` names the
        file each side's current subschema lives in, which differ after a ``$ref`` hop.
        """
        if old is False or (_same(old, new) and not self._has_ref(old)):
            return
        if new is False:
            yield Finding(file, ptr, "schema-narrowed", _show(old), "false")
            return
        o, n = _as_schema(old), _as_schema(new)
        yield from self._compare_refs(file, doc, ptr, o, n)
        new_has_ref = "$ref" in n
        removed = "$ref" in o and "$ref" not in n
        o = {k: v for k, v in o.items() if k != "$ref"}
        n = {k: v for k, v in n.items() if k != "$ref"}
        if removed and not set(o) - _ANNOTATIONS:
            # The old shape lived entirely in the target, which the hop compared with
            # the inline replacement; the bare siblings would read as additions.
            return
        yield from _compare_scalars(file, ptr, o, n)
        if not new_has_ref:
            yield from _compare_widening(file, ptr, o, n)
        yield from _compare_unmodelled(file, ptr, o, n)
        yield from _compare_required(file, ptr, o, n)
        yield from self._compare_additional(file, doc, ptr, o, n)
        yield from self._compare_children(file, doc, ptr, o, n)

    def _hop(
        self,
        file: str,
        ptr: str,
        label: tuple[str, str],
        sides: tuple[tuple[str, str] | None, tuple[str, str] | None],
        docs: Docs,
        old: Any,
        new: Any,
        key: tuple | None = None,
    ) -> Iterator[Finding]:
        """Compare across a ``$ref``; the stack stops a reference cycle.

        ``sides`` holds the (old, new) targets, None for a side with no reference, and
        ``docs`` the files the compared subschemas live in. ``label`` names the target
        in findings. Results are cached per (old target, new target) with pointers
        relative to the referrer, so a definition used by many referrers is walked once.
        """
        if sides in self._stack:
            self._cuts.add(sides)
            if None in sides:
                # A reference added or removed against a target already being compared:
                # the inline side differs each time, so the pair is not a repeat.
                yield Finding(
                    file,
                    f"{ptr}/$ref",
                    "unverified",
                    _show(f"{label[0]}#{label[1]}" if sides[0] else None),
                    _show(f"{label[0]}#{label[1]}" if sides[1] else None),
                )
            return
        name = f"/$ref({label[0]}#{label[1]})"
        if key is not None and key in self._cache:
            for found in self._cache[key]:
                yield replace(found, file=file, pointer=f"{ptr}{name}{found.pointer}")
            return
        outer_cuts, self._cuts = self._cuts, set()
        self._stack.add(sides)
        try:
            inner = list(self.compare("", docs, "", old, new))
        finally:
            self._stack.discard(sides)
        if sides[0] is not None and sides[0] == sides[1]:
            # The same definition on both sides: the definition's own comparison reports
            # an output-only change once, under its own file's direction.
            inner = [f for f in inner if f.kind not in OUTPUT_ONLY_KINDS]
        self._cuts.discard(sides)
        if key is not None and not self._cuts:
            self._cache[key] = inner
        self._cuts |= outer_cuts
        for found in inner:
            yield replace(found, file=file, pointer=f"{ptr}{name}{found.pointer}")

    def _compare_refs(
        self, file: str, doc: Docs, ptr: str, o: dict, n: dict
    ) -> Iterator[Finding]:
        old_ref, new_ref = o.get("$ref"), n.get("$ref")
        if old_ref is None and new_ref is None:
            return
        old_t = (
            _resolve(self.old, self._old_uris, doc.old, old_ref)
            if old_ref is not None
            else None
        )
        new_t = (
            _resolve(self.new, self._new_uris, doc.new, new_ref)
            if new_ref is not None
            else None
        )
        if (old_ref is not None and old_t is None) or (
            new_ref is not None and new_t is None
        ):
            yield Finding(
                file, f"{ptr}/$ref", "unverified", _show(old_ref), _show(new_ref)
            )
            return
        o_rest = {k: v for k, v in o.items() if k != "$ref"}
        n_rest = {k: v for k, v in n.items() if k != "$ref"}
        if old_t and new_t:
            sides = ((old_t[0], old_t[1]), (new_t[0], new_t[1]))
            docs = Docs(old_t[0], new_t[0])
            yield from self._hop(
                file, ptr, sides[1], sides, docs, old_t[2], new_t[2], sides
            )
        elif new_t:
            # A reference added: the new target must accept all the old schema accepted.
            target = (new_t[0], new_t[1])
            docs = Docs(doc.old, new_t[0])
            yield from self._hop(
                file, ptr, target, (None, target), docs, o_rest, new_t[2]
            )
        elif old_t:
            # A reference removed: what remains must accept all the old target accepted.
            target = (old_t[0], old_t[1])
            docs = Docs(old_t[0], doc.new)
            yield from self._hop(
                file, ptr, target, (target, None), docs, old_t[2], n_rest
            )

    def _compare_additional(
        self, file: str, doc: Docs, ptr: str, o: dict, n: dict
    ) -> Iterator[Finding]:
        for key in ("additionalProperties", "additionalItems"):
            if key not in n:
                continue
            old = o.get(key, True)
            new = n[key]
            if (
                new is True
                or old is False
                or (_same(old, new) and not self._has_ref(old))
            ):
                continue
            if new is False or old is True or not isinstance(old, dict):
                kind = "additional-properties-narrowed"
                yield Finding(file, f"{ptr}/{key}", kind, _show(o.get(key)), _show(new))
            else:
                yield from self.compare(file, doc, f"{ptr}/{key}", old, new)

    def _compare_children(
        self, file: str, doc: Docs, ptr: str, o: dict, n: dict
    ) -> Iterator[Finding]:
        for key in _SCHEMA_MAPS:
            old_map, new_map = o.get(key, {}), n.get(key, {})
            for name, old_sub in old_map.items():
                child = f"{ptr}/{key}/{_esc(name)}"
                if name not in new_map:
                    kind = _MAP_REMOVED_KIND[key]
                    yield Finding(file, child, kind, _show(old_sub), "(absent)")
                else:
                    yield from self.compare(file, doc, child, old_sub, new_map[name])
            if key in _CHECKED_WHEN_ADDED:
                # A new entry restricts names that were unrestricted before.
                for name, new_sub in new_map.items():
                    if name not in old_map:
                        child = f"{ptr}/{key}/{_esc(name)}"
                        yield from self.compare(file, doc, child, {}, new_sub)
        for key in _SCHEMA_SINGLE:
            if key in o and key in n:
                if key == "if" and not _same(o[key], n[key]):
                    yield Finding(
                        file, f"{ptr}/if", "if-changed", _show(o[key]), _show(n[key])
                    )
                else:
                    yield from self.compare(file, doc, f"{ptr}/{key}", o[key], n[key])
            elif key in n and key != "if":
                yield from self.compare(file, doc, f"{ptr}/{key}", {}, n[key])
            elif key in o and key != "if":
                # The whole subschema is gone: inspect what it described.
                yield from self.compare(file, doc, f"{ptr}/{key}", o[key], {})
        for key in _SCHEMA_LISTS:
            yield from self._compare_list(file, doc, ptr, key, o, n)

    def _compare_list(
        self, file: str, doc: Docs, ptr: str, key: str, o: dict, n: dict
    ) -> Iterator[Finding]:
        old, new = o.get(key, []), n.get(key, [])
        old_c, new_c = [_canon(m) for m in old], [_canon(m) for m in new]
        if old_c != new_c[: len(old_c)] and not (Counter(old_c) - Counter(new_c)):
            # Every old member survives but at another position: fail closed.
            yield Finding(
                file, f"{ptr}/{key}", "list-reordered", _show(old), _show(new)
            )
            return
        if key == "allOf":
            # Every allOf member constrains, so an added member narrows.
            for i, member in enumerate(new):
                base = old[i] if i < len(old) else {}
                yield from self.compare(file, doc, f"{ptr}/allOf/{i}", base, member)
            for i in range(len(new), len(old)):
                # A removed member: inspect what it described.
                yield from self.compare(file, doc, f"{ptr}/allOf/{i}", old[i], {})
            return
        if key in ("anyOf", "oneOf") and old:
            for i in range(len(old), len(new)):
                if not _extensible(n):
                    yield Finding(
                        file,
                        f"{ptr}/{key}/{i}",
                        "alternative-added",
                        "(absent)",
                        _show(new[i]),
                    )
        yield from self._narrowing_possible(file, ptr, key, old, new, _extensible(n))
        for i, member in enumerate(old):
            child = f"{ptr}/{key}/{i}"
            if i >= len(new):
                kind = (
                    "alternative-removed"
                    if key != "prefixItems"
                    else "prefix-item-removed"
                )
                yield Finding(file, child, kind, _show(member), "(absent)")
            else:
                yield from self.compare(file, doc, child, member, new[i])

    @staticmethod
    def _narrowing_possible(
        file: str, ptr: str, key: str, old: list, new: list, marked: bool
    ) -> Iterator[Finding]:
        """Fail closed on additions that can reject values that used to pass.

        A new ``anyOf`` or ``oneOf`` requires a match, marked or not. An appended
        ``oneOf`` member can overlap an earlier one, which fails ``oneOf``; a member that
        is provably disjoint from every earlier member, or a marked position, passes. An
        appended ``prefixItems`` entry constrains a position that was free.
        """
        if key in ("anyOf", "oneOf") and new and not old:
            yield Finding(
                file, f"{ptr}/{key}", "narrowing-possible", "(absent)", _show(new)
            )
        elif key in ("oneOf", "prefixItems"):
            for i in range(len(old), len(new)):
                if key == "oneOf" and (
                    marked or all(_disjoint(new[j], new[i]) for j in range(i))
                ):
                    continue
                yield Finding(
                    file,
                    f"{ptr}/{key}/{i}",
                    "narrowing-possible",
                    "(absent)",
                    _show(new[i]),
                )


def _type_set(schema: dict) -> set[str] | None:
    types = _types(schema.get("type"))
    if types is None:
        return None
    return types | ({"integer"} if "number" in types else set())


def _value_set(schema: dict) -> set[str] | None:
    if "const" in schema:
        return {_canon(schema["const"])}
    if isinstance(schema.get("enum"), list):
        return {_canon(v) for v in schema["enum"]}
    return None


def _disjoint(a: object, b: object) -> bool:
    """True only when no instance can match both schemas; unknown means False.

    Provable cases: disjoint ``type`` sets, disjoint ``const``/``enum`` values, or a
    property that both schemas require with disjoint ``const``/``enum`` values. A
    ``$ref`` or a boolean schema is never provably disjoint.
    """
    if not isinstance(a, dict) or not isinstance(b, dict) or "$ref" in a or "$ref" in b:
        return False
    ta, tb = _type_set(a), _type_set(b)
    if ta is not None and tb is not None and not ta & tb:
        return True
    va, vb = _value_set(a), _value_set(b)
    if va is not None and vb is not None and not va & vb:
        return True
    props_a, props_b = a.get("properties", {}), b.get("properties", {})
    required = set(a.get("required", [])) & set(b.get("required", []))
    for name in required & set(props_a) & set(props_b):
        pa, pb = props_a[name], props_b[name]
        if isinstance(pa, dict) and isinstance(pb, dict) and _disjoint(pa, pb):
            return True
    return False


def _compare_unmodelled(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    for key in sorted((set(o) | set(n)) - _MODELLED - _ANNOTATIONS):
        if not _same(o.get(key), n.get(key)) or (key in o) != (key in n):
            yield Finding(
                file,
                f"{ptr}/{_esc(key)}",
                "unverified",
                _show(o.get(key)),
                _show(n.get(key)),
            )


def _compare_widening(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    """Widenings by ``type``, or by dropping an ``enum``/``const``, that a closed output forbids.

    Skipped where the new schema has a ``$ref`` (the target carries the constraint) and
    where the position is marked extensible.
    """
    if _extensible(n):
        return
    old_types, new_types = _types(o.get("type")), _types(n.get("type"))
    if old_types is not None and (
        new_types is None
        or not all(_type_satisfied(t, old_types) for t in new_types)
    ):
        yield Finding(
            file,
            f"{ptr}/type",
            "type-widened",
            _show(o.get("type")),
            _show(n.get("type")) if "type" in n else "(absent)",
        )
    for key in ("enum", "const"):
        if key in o and "enum" not in n and "const" not in n:
            yield Finding(
                file, f"{ptr}/{key}", "enum-widened", _show(o[key]), "(absent)"
            )


def _compare_scalars(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    def find(key: str, kind: str) -> Finding:
        return Finding(
            file, f"{ptr}/{_esc(key)}", kind, _show(o.get(key)), _show(n.get(key))
        )

    if "$id" in o and not _same(o["$id"], n.get("$id")):
        yield find("$id", "id-changed")
    old_types, new_types = _types(o.get("type")), _types(n.get("type"))
    if new_types is not None and (
        old_types is None or not all(_type_satisfied(t, new_types) for t in old_types)
    ):
        yield find("type", "type-narrowed")
    yield from _compare_value_sets(file, ptr, o, n)
    for key in _MIN_KEYWORDS:
        if key in n and not (_is_number(o.get(key)) and n[key] <= o[key]):
            yield find(key, "bound-tightened")
    for key in _MAX_KEYWORDS:
        if key in n and not (_is_number(o.get(key)) and n[key] >= o[key]):
            yield find(key, "bound-tightened")
    if ("pattern" in o or "pattern" in n) and not _same_pattern(
        o.get("pattern"), n.get("pattern")
    ):
        yield find("pattern", "pattern-changed")
    for key in ("format", "multipleOf"):
        if key in n and not _same(o.get(key), n[key]):
            yield find(key, "constraint-added")
    if n.get("uniqueItems") is True and o.get("uniqueItems") is not True:
        yield find("uniqueItems", "constraint-added")
    if "not" in n and not _same(o.get("not"), n["not"]):
        yield find("not", "constraint-added")
    for key, value in n.get("dependentRequired", {}).items():
        known = o.get("dependentRequired", {}).get(key, [])
        for name in value:
            if name not in known:
                yield Finding(
                    file,
                    f"{ptr}/dependentRequired/{_esc(key)}/{_esc(name)}",
                    "required-added",
                    "(absent)",
                    _show(name),
                )


def _extensible(node: dict) -> bool:
    """True when the position carries ``x-slopvac-extensible: true`` (JSON ``true`` only)."""
    return node.get(EXTENSIBLE) is True


def _compare_value_sets(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    """``enum`` and ``const`` both restrict to a value set; compare the sets."""
    if _extensible(o) and not _extensible(n):
        yield Finding(file, ptr, "extensible-removed", "true", _show(n.get(EXTENSIBLE)))
    old_vals = (
        o.get("enum") if "enum" in o else ([o["const"]] if "const" in o else None)
    )
    for key in ("enum", "const"):
        if key not in n:
            continue
        new_vals = n[key] if key == "enum" else [n[key]]
        if old_vals is None or not {_canon(v) for v in old_vals} <= {
            _canon(v) for v in new_vals
        }:
            old_key = "enum" if "enum" in o else "const"
            yield Finding(
                file,
                f"{ptr}/{key}",
                f"{key}-narrowed",
                _show(o.get(old_key)),
                _show(n[key]),
            )
        elif not _extensible(n) and not {_canon(v) for v in new_vals} <= {
            _canon(v) for v in old_vals
        }:
            yield Finding(
                file, f"{ptr}/{key}", "enum-widened", _show(old_vals), _show(n[key])
            )


def _compare_required(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    added = [r for r in n.get("required", []) if r not in o.get("required", [])]
    for name in added:
        yield Finding(
            file,
            f"{ptr}/required/{_esc(name)}",
            "required-added",
            "(absent)",
            _show(name),
        )


def compare_snapshots(
    old: dict[str, dict],
    new: dict[str, dict],
    directions: dict[str, str] | None = None,
) -> list[Finding]:
    """Compare schemas by file name. Added files are allowed.

    ``directions`` maps a file name to ``input``, ``output`` or ``both``. A file that is
    not listed counts as ``input``. The output-only kinds are dropped for an input file;
    a finding reached through a ``$ref`` carries the referring file, so the referrer's
    direction decides.
    """
    comparer = Comparer(old, new)
    findings: list[Finding] = []
    for name, old_schema in sorted(old.items()):
        if name not in new:
            findings.append(
                Finding(name, "", "file-removed", old_schema["$id"], "(absent)")
            )
            continue
        found = comparer.compare(name, Docs(name, name), "", old_schema, new[name])
        # Output-only kinds are dropped for an input file. Through a reference they come
        # only from a retarget or an added or removed reference; a change to one shared
        # definition is reported once, at the definition.
        is_input = (directions or {}).get(name, "input") == "input"
        found = (f for f in found if f.kind not in OUTPUT_ONLY_KINDS or not is_input)
        findings.extend(found)
    return findings


def load_directions(path: Path) -> dict[str, str]:
    """Read the direction registry: ``[directions]`` maps a schema file to its class."""
    try:
        data = tomllib.loads(_read(path))
    except tomllib.TOMLDecodeError as exc:
        raise CompatError(f"cannot parse {path}: {exc}") from exc
    table = data.get("directions")
    if not isinstance(table, dict) or set(data) != {"directions"}:
        raise CompatError(f"{path}: expected one [directions] table")
    for name, value in table.items():
        if value not in DIRECTIONS:
            raise CompatError(
                f"{path}: {name}: direction {value!r} is not one of {', '.join(DIRECTIONS)}"
            )
    return table


def check_directions(directions: dict[str, str], schemas: dict[str, dict]) -> None:
    """The registry lists every schema file once and nothing else (fail closed)."""
    unlisted = sorted(set(schemas) - set(directions))
    if unlisted:
        raise CompatError(f"{', '.join(unlisted)}: not in {DIRECTIONS_FILE}")
    unknown = sorted(set(directions) - set(schemas))
    if unknown:
        raise CompatError(f"{DIRECTIONS_FILE} lists no such schema: {', '.join(unknown)}")


# --- corpus re-validation --------------------------------------------------------


def _registry(schemas: dict[str, dict]) -> Registry:
    uris = _uris(schemas)
    resources = [(uris[name], Resource.from_contents(s)) for name, s in schemas.items()]
    return Registry().with_resources(resources)


def _instance_schema(schema: dict, name: str) -> dict:
    if "--" in name:
        return {"$ref": f"{schema['$id']}#/$defs/{name.split('--', 1)[0]}"}
    return schema


def validate_corpus(corpus: dict[str, str], new: dict[str, dict]) -> list[Finding]:
    """Every baseline ``valid/`` instance must still validate against the new schema."""
    registry = _registry(new)
    findings: list[Finding] = []
    for rel, text in sorted(corpus.items()):
        parts = Path(rel).parts
        if len(parts) != 3 or parts[1] != "valid":
            continue
        artifact, _, fname = parts
        suffix = Path(fname).suffix
        schema = new.get(f"{artifact}{SCHEMA_SUFFIX}")
        if schema is None or suffix not in LOADERS:
            continue
        try:
            instance = LOADERS[suffix](text)
        except Exception as exc:
            raise CompatError(f"cannot parse baseline instance {rel}: {exc}") from exc
        root = _instance_schema(schema, fname)
        validator = Draft202012Validator(root, registry=registry)
        try:
            errors = sorted(
                validator.iter_errors(instance), key=lambda e: list(e.absolute_path)
            )
        except (Unresolvable, SchemaError) as exc:
            raise CompatError(
                f"cannot validate baseline instance {rel}: {exc}"
            ) from exc
        if errors:
            first = errors[0]
            where = "".join(f"/{_esc(str(p))}" for p in first.absolute_path)
            findings.append(
                Finding(
                    f"{artifact}{SCHEMA_SUFFIX}",
                    f"/conformance/valid/{_esc(fname)}",
                    "corpus-rejected",
                    "accepted",
                    f"rejected at {where or '(root)'}: {first.message}",
                )
            )
    return findings


# --- allow file ------------------------------------------------------------------


def load_allow(path: Path) -> list[AllowEntry]:
    if not path.exists():
        return []
    return parse_allow(_read(path), str(path))


def parse_allow(text: str, source: str) -> list[AllowEntry]:
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        raise CompatError(f"cannot parse {source}: {exc}") from exc
    raw = data.get("allow", [])
    entries = []
    for i, item in enumerate(raw):
        values = {k: item.get(k) for k in ALLOW_FIELDS}
        missing = [
            k
            for k, v in values.items()
            if not isinstance(v, str) or (k != "pointer" and not v.strip())
        ]
        extra = set(item) - set(ALLOW_FIELDS)
        if missing or extra:
            raise CompatError(
                f"{source}: allow entry {i + 1}: bad or missing fields {sorted(set(missing) | extra)}"
            )
        entries.append(AllowEntry(**values))
    return entries


def apply_allow(
    findings: list[Finding], allow: list[AllowEntry]
) -> tuple[list[Finding], list[Finding]]:
    """Return (remaining findings, stale-entry findings)."""
    used: set[AllowEntry] = set()
    remaining = []
    for finding in findings:
        matching = [e for e in allow if e.matches(finding)]
        used.update(matching)
        if not matching:
            remaining.append(finding)
    stale = [
        Finding(e.file, e.pointer, "allow-entry-stale", e.kind, "(no matching finding)")
        for e in allow
        if e not in used
    ]
    return remaining, stale


# --- loading ---------------------------------------------------------------------


def _read(path: Path) -> str:
    """Read a UTF-8 file; any reader failure is a read error (exit 2), never a pass."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise CompatError(f"cannot read {path}: {exc}") from exc


def _parse_schema(name: str, text: str) -> dict:
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError as exc:
        raise CompatError(f"cannot parse {name}: {exc}") from exc
    _require_id(name, parsed)
    return parsed


def snapshot_from_dir(directory: Path) -> Snapshot:
    """Read a baseline directory, its allow file included."""
    snapshot = _read_dir(directory)
    return replace(snapshot, allow=frozenset(load_allow(directory / ALLOW_FILE)))


def _read_dir(directory: Path) -> Snapshot:
    """Read the schemas and the valid corpus of a directory; its allow file is not read."""
    if not directory.is_dir():
        raise CompatError(f"not a directory: {directory}")
    conformance = directory / "conformance"
    try:
        schema_paths = sorted(directory.glob(f"*{SCHEMA_SUFFIX}"))
        corpus_paths = [
            p
            for p in sorted(conformance.rglob("*"))
            if p.is_file() and _is_valid_instance(p.relative_to(conformance).as_posix())
        ]
    except OSError as exc:
        raise CompatError(f"cannot list {directory}: {exc}") from exc
    schemas = {p.name: _parse_schema(p.name, _read(p)) for p in schema_paths}
    corpus = {p.relative_to(conformance).as_posix(): _read(p) for p in corpus_paths}
    return Snapshot(schemas, corpus)


def _git(repo: Path, *args: str) -> str:
    try:
        done = subprocess.run(
            ["git", "-C", str(repo), *args],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
    except (OSError, UnicodeDecodeError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", "") or str(exc)
        raise CompatError(f"git {' '.join(args)} failed: {detail.strip()}") from exc
    return done.stdout


def _git_blobs(repo: Path, ref: str, paths: list[str]) -> dict[str, str]:
    """Read many blobs through one ``git cat-file --batch`` process."""
    request = "".join(f"{ref}:{path}\n" for path in paths).encode()
    try:
        done = subprocess.run(
            ["git", "-C", str(repo), "cat-file", "--batch"],
            input=request,
            check=True,
            capture_output=True,
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", b"") or str(exc).encode()
        message = detail.decode(errors="replace").strip()
        raise CompatError(f"git cat-file failed: {message}") from exc
    out, pos, blobs = done.stdout, 0, {}
    for path in paths:
        end = out.index(b"\n", pos)
        header = out[pos:end].decode().split()
        if len(header) != 3 or header[1] != "blob":
            raise CompatError(f"cannot read {ref}:{path}: {' '.join(header)}")
        size = int(header[2])
        data = out[end + 1 : end + 1 + size]
        pos = end + 1 + size + 1
        try:
            blobs[path] = data.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise CompatError(f"cannot read {ref}:{path}: {exc}") from exc
    return blobs


def snapshot_from_ref(repo: Path, ref: str) -> Snapshot:
    prefix = f"{SCHEMA_REL}/"
    names = _git(repo, "ls-tree", "-r", "--name-only", ref, "--", prefix).splitlines()
    wanted = []
    allow_path = f"{prefix}{ALLOW_FILE}"
    for full in names:
        rel = full.removeprefix(prefix)
        is_schema = "/" not in rel and rel.endswith(SCHEMA_SUFFIX)
        is_instance = rel.startswith("conformance/") and _is_valid_instance(
            rel.removeprefix("conformance/")
        )
        if is_schema or is_instance or full == allow_path:
            wanted.append(full)
    blobs = _git_blobs(repo, ref, wanted)
    schemas: dict[str, dict] = {}
    corpus: dict[str, str] = {}
    allow: list[AllowEntry] = []
    for full, text in blobs.items():
        rel = full.removeprefix(prefix)
        if full == allow_path:
            allow = parse_allow(text, f"{ref}:{full}")
        elif rel.startswith("conformance/"):
            corpus[rel.removeprefix("conformance/")] = text
        else:
            schemas[rel] = _parse_schema(rel, text)
    if not schemas:
        raise CompatError(f"no {SCHEMA_REL}/*{SCHEMA_SUFFIX} at {ref}")
    return Snapshot(schemas, corpus, frozenset(allow))


def _is_valid_instance(under_conformance: str) -> bool:
    """Only ``conformance/<artifact>/valid/<file>`` is re-validated."""
    parts = Path(under_conformance).parts
    return len(parts) == 3 and parts[1] == "valid"


# --- entry points ----------------------------------------------------------------


def check(base: Snapshot, new_dir: Path, allow: list[AllowEntry]) -> list[Finding]:
    """Return every unsuppressed breaking finding, stale allow entries included.

    ``allow`` is the selected allow file of the new tree; the new tree's own default
    allow file is not read. Only the entries that are not in the baseline's allow file
    take part. An entry the baseline already holds approved a change that has landed:
    it is never stale, and it suppresses nothing except a finding the baseline also
    has against itself, unchanged (an unresolvable ``$ref``, say). So an entry survives
    its change reaching the base, and a different finding at the same pointer needs a
    new review.
    """
    new = _read_dir(new_dir).schemas
    directions = load_directions(new_dir / DIRECTIONS_FILE)
    check_directions(directions, new)
    findings = compare_snapshots(base.schemas, new, directions) + validate_corpus(
        base.corpus, new
    )
    persisting = set(compare_snapshots(base.schemas, base.schemas))
    findings = [
        f
        for f in findings
        if not (f in persisting and any(entry.matches(f) for entry in base.allow))
    ]
    added = [entry for entry in allow if entry not in base.allow]
    remaining, stale = apply_allow(findings, added)
    return remaining + stale


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--base", help="git ref holding the baseline schemas/v3")
    source.add_argument(
        "--baseline-dir", type=Path, help="directory holding the baseline"
    )
    parser.add_argument("--repo", type=Path, default=ROOT, help="repository root")
    parser.add_argument(
        "--schemas-dir", type=Path, help="schemas to check (default: <repo>/schemas/v3)"
    )
    parser.add_argument(
        "--allow-file", type=Path, help="default: <schemas-dir>/compat-allow.toml"
    )
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        return 2 if exc.code else 0
    schemas_dir = args.schemas_dir or args.repo / SCHEMA_REL
    allow_path = args.allow_file or schemas_dir / ALLOW_FILE
    try:
        base = (
            snapshot_from_ref(args.repo, args.base)
            if args.base
            else snapshot_from_dir(args.baseline_dir)
        )
        findings = check(base, schemas_dir, load_allow(allow_path))
    except CompatError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    for finding in findings:
        print(finding.render())
    if findings:
        print(f"{len(findings)} breaking finding(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
