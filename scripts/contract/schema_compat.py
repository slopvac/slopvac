"""Additive-only check for schemas/v3 against a baseline.

Usage: python scripts/contract/schema_compat.py --base <git-ref>
       python scripts/contract/schema_compat.py --baseline-dir <dir>

The baseline is a git ref (read through ``git show``) or a directory with the same
layout as schemas/v3 (``*.schema.json`` plus ``conformance/``). The working tree
is compared against it. A change within one schema_version may add, never remove
or tighten (docs/contract/policies.md, section 3.9).

Each finding prints as ``file:json-pointer: kind: old -> new``. The kind is a stable
kebab-case code; a code is never reused and never changes meaning. An entry in
schemas/v3/compat-allow.toml suppresses one finding; an entry that matches nothing
is itself a finding.

Exit codes: 0 compatible, 1 a breaking finding, 2 a usage or read error.

Limits, all of which fail closed:

- ``$ref`` is resolved across every file of schemas/v3 on both sides, with a cycle
  guard. A finding reached through a reference names the referrer pointer and the
  definition: ``file:/properties/p/$ref(common.schema.json#/$defs/D)/minLength``.
  A reference that cannot be resolved is reported as ``unverified``. Only references
  to a file of schemas/v3 or to a local ``#/`` pointer resolve; anchors do not.
- ``allOf``, ``anyOf``, ``oneOf`` and ``prefixItems`` are compared by position. A
  list whose members all survive in another order is reported as ``list-reordered``
  and needs an allow entry. A new ``anyOf``/``oneOf``, and an appended ``oneOf`` or
  ``prefixItems`` member, are reported as ``narrowing-possible``.
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
    """Baseline schemas by file name and conformance files by path under conformance/."""

    schemas: dict[str, dict]
    corpus: dict[str, str]


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


def _resolve(
    schemas: dict[str, dict], doc: str, ref: object
) -> tuple[str, str, Any] | None:
    """Resolve a ``$ref`` to (file, pointer, subschema), or None if it is not resolvable."""
    if not isinstance(ref, str):
        return None
    base, _, fragment = ref.partition("#")
    file = base.rsplit("/", 1)[-1] if base else doc
    node: Any = schemas.get(file)
    if node is None or (fragment and not fragment.startswith("/")):
        return None
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


class Comparer:
    """Compares schema pairs, resolving every ``$ref`` across all files on both sides."""

    def __init__(self, old: dict[str, dict], new: dict[str, dict]) -> None:
        self.old, self.new = old, new
        self._stack: set[tuple[str, str]] = set()
        # Targets cut by the cycle guard during the current computation; a result
        # computed while any cut is open is incomplete and must not be cached.
        self._cuts: set[tuple[str, str]] = set()
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
        self, file: str, doc: str, ptr: str, old: Any, new: Any
    ) -> Iterator[Finding]:
        """Yield a finding for each way ``new`` accepts less than ``old``.

        ``file`` is the schema file the finding is reported against; ``doc`` is the
        file the current subschema lives in, which differs after a ``$ref`` hop.
        """
        if old is False or (_same(old, new) and not self._has_ref(old)):
            return
        if new is False:
            yield Finding(file, ptr, "schema-narrowed", _show(old), "false")
            return
        o, n = _as_schema(old), _as_schema(new)
        yield from self._compare_refs(file, doc, ptr, o, n)
        o = {k: v for k, v in o.items() if k != "$ref"}
        n = {k: v for k, v in n.items() if k != "$ref"}
        yield from _compare_scalars(file, ptr, o, n)
        yield from _compare_unmodelled(file, ptr, o, n)
        yield from _compare_required(file, ptr, o, n)
        yield from self._compare_additional(file, doc, ptr, o, n)
        yield from self._compare_children(file, doc, ptr, o, n)

    def _hop(
        self,
        file: str,
        ptr: str,
        target: tuple[str, str],
        old: Any,
        new: Any,
        key: tuple | None = None,
    ) -> Iterator[Finding]:
        """Compare across a ``$ref``; the stack stops a reference cycle.

        Results are cached per (old target, new target) with pointers relative to the
        referrer, so a definition used by many referrers is walked once.
        """
        if target in self._stack:
            self._cuts.add(target)
            return
        label = f"/$ref({target[0]}#{target[1]})"
        if key is not None and key in self._cache:
            for found in self._cache[key]:
                yield replace(found, file=file, pointer=f"{ptr}{label}{found.pointer}")
            return
        outer_cuts, self._cuts = self._cuts, set()
        self._stack.add(target)
        try:
            inner = list(self.compare("", target[0], "", old, new))
        finally:
            self._stack.discard(target)
        self._cuts.discard(target)
        if key is not None and not self._cuts:
            self._cache[key] = inner
        self._cuts |= outer_cuts
        for found in inner:
            yield replace(found, file=file, pointer=f"{ptr}{label}{found.pointer}")

    def _compare_refs(
        self, file: str, doc: str, ptr: str, o: dict, n: dict
    ) -> Iterator[Finding]:
        old_ref, new_ref = o.get("$ref"), n.get("$ref")
        if old_ref is None and new_ref is None:
            return
        old_t = _resolve(self.old, doc, old_ref) if old_ref is not None else None
        new_t = _resolve(self.new, doc, new_ref) if new_ref is not None else None
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
            key = ((old_t[0], old_t[1]), (new_t[0], new_t[1]))
            yield from self._hop(file, ptr, key[1], old_t[2], new_t[2], key)
        elif new_t:
            # A reference added: the new target must accept all the old schema accepted.
            yield from self._hop(file, ptr, (new_t[0], new_t[1]), o_rest, new_t[2])
        elif old_t:
            # A reference removed: what remains must accept all the old target accepted.
            yield from self._hop(file, ptr, (old_t[0], old_t[1]), old_t[2], n_rest)

    def _compare_additional(
        self, file: str, doc: str, ptr: str, o: dict, n: dict
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
        self, file: str, doc: str, ptr: str, o: dict, n: dict
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
            yield from self._compare_list(
                file, doc, ptr, key, o.get(key, []), n.get(key, [])
            )

    def _compare_list(
        self, file: str, doc: str, ptr: str, key: str, old: list, new: list
    ) -> Iterator[Finding]:
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
        yield from self._narrowing_possible(file, ptr, key, old, new)
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
        file: str, ptr: str, key: str, old: list, new: list
    ) -> Iterator[Finding]:
        """Fail closed on additions that can reject values that used to pass.

        A new ``anyOf`` or ``oneOf`` requires a match. An appended ``oneOf`` branch can
        overlap an existing one, which fails ``oneOf``. An appended ``prefixItems``
        entry constrains a position that was free.
        """
        if key in ("anyOf", "oneOf") and new and not old:
            yield Finding(
                file, f"{ptr}/{key}", "narrowing-possible", "(absent)", _show(new)
            )
        elif key in ("oneOf", "prefixItems"):
            for i in range(len(old), len(new)):
                yield Finding(
                    file,
                    f"{ptr}/{key}/{i}",
                    "narrowing-possible",
                    "(absent)",
                    _show(new[i]),
                )


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
    if ("pattern" in o or "pattern" in n) and o.get("pattern") != n.get("pattern"):
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


def _compare_value_sets(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    """``enum`` and ``const`` both restrict to a value set; compare the sets."""
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


def compare_snapshots(old: dict[str, dict], new: dict[str, dict]) -> list[Finding]:
    """Compare schemas by file name. Added files are allowed."""
    comparer = Comparer(old, new)
    findings: list[Finding] = []
    for name, old_schema in sorted(old.items()):
        if name not in new:
            findings.append(
                Finding(
                    name, "", "file-removed", old_schema.get("$id", name), "(absent)"
                )
            )
            continue
        findings.extend(comparer.compare(name, name, "", old_schema, new[name]))
    return findings


# --- corpus re-validation --------------------------------------------------------


def _registry(schemas: dict[str, dict]) -> Registry:
    resources = [
        (s.get("$id", name), Resource.from_contents(s)) for name, s in schemas.items()
    ]
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
    text = _read(path)
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        raise CompatError(f"cannot parse {path}: {exc}") from exc
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
                f"{path}: allow entry {i + 1}: bad or missing fields {sorted(set(missing) | extra)}"
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
        return json.loads(text)
    except json.JSONDecodeError as exc:
        raise CompatError(f"cannot parse {name}: {exc}") from exc


def snapshot_from_dir(directory: Path) -> Snapshot:
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
    for full in names:
        rel = full.removeprefix(prefix)
        is_schema = "/" not in rel and rel.endswith(SCHEMA_SUFFIX)
        is_instance = rel.startswith("conformance/") and _is_valid_instance(
            rel.removeprefix("conformance/")
        )
        if is_schema or is_instance:
            wanted.append(full)
    blobs = _git_blobs(repo, ref, wanted)
    schemas: dict[str, dict] = {}
    corpus: dict[str, str] = {}
    for full, text in blobs.items():
        rel = full.removeprefix(prefix)
        if rel.startswith("conformance/"):
            corpus[rel.removeprefix("conformance/")] = text
        else:
            schemas[rel] = _parse_schema(rel, text)
    if not schemas:
        raise CompatError(f"no {SCHEMA_REL}/*{SCHEMA_SUFFIX} at {ref}")
    return Snapshot(schemas, corpus)


def _is_valid_instance(under_conformance: str) -> bool:
    """Only ``conformance/<artifact>/valid/<file>`` is re-validated."""
    parts = Path(under_conformance).parts
    return len(parts) == 3 and parts[1] == "valid"


# --- entry points ----------------------------------------------------------------


def check(base: Snapshot, new_dir: Path, allow: list[AllowEntry]) -> list[Finding]:
    """Return every unsuppressed breaking finding, stale allow entries included."""
    new = snapshot_from_dir(new_dir).schemas
    findings = compare_snapshots(base.schemas, new) + validate_corpus(base.corpus, new)
    remaining, stale = apply_allow(findings, allow)
    return remaining + stale


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
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
