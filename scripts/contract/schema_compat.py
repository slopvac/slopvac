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

Not detected: a change that only tightens a keyword outside the walked set
(for example ``contentMediaType``), and a semantic change behind an unchanged
``$ref`` target in another file. The corpus re-validation catches such changes only
when a baseline valid instance exercises them.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

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


def _canon(value: object) -> str:
    return json.dumps(value, sort_keys=True)


# --- structural comparison -------------------------------------------------------


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


def compare_schema(file: str, pointer: str, old: Any, new: Any) -> Iterator[Finding]:
    """Yield a finding for each way ``new`` accepts less than ``old``."""
    if old is False or old == new:
        return
    if new is False:
        yield Finding(file, pointer, "schema-narrowed", _show(old), "false")
        return
    o, n = _as_schema(old), _as_schema(new)
    yield from _compare_scalars(file, pointer, o, n)
    yield from _compare_required(file, pointer, o, n)
    yield from _compare_additional(file, pointer, o, n)
    yield from _compare_children(file, pointer, o, n)


def _compare_scalars(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    def find(key: str, kind: str) -> Finding:
        return Finding(
            file, f"{ptr}/{_esc(key)}", kind, _show(o.get(key)), _show(n.get(key))
        )

    if "$id" in o and o["$id"] != n.get("$id"):
        yield find("$id", "id-changed")
    if o.get("$ref") != n.get("$ref"):
        yield find("$ref", "ref-changed")
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
        if key in n and o.get(key) != n[key]:
            yield find(key, "constraint-added")
    if n.get("uniqueItems") is True and o.get("uniqueItems") is not True:
        yield find("uniqueItems", "constraint-added")
    if "not" in n and o.get("not") != n["not"]:
        yield find("not", "constraint-added")
    for key, value in n.get("dependentRequired", {}).items():
        added = set(value) - set(o.get("dependentRequired", {}).get(key, []))
        if added:
            yield Finding(
                file,
                f"{ptr}/dependentRequired/{_esc(key)}",
                "required-added",
                _show(o.get("dependentRequired", {}).get(key)),
                _show(value),
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
            file, f"{ptr}/required", "required-added", "(absent)", _show(name)
        )


def _compare_additional(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    for key in ("additionalProperties", "additionalItems"):
        if key not in n:
            continue
        old = o.get(key, True)
        new = n[key]
        if new is True or old == new or old is False:
            continue
        if new is False or old is True or not isinstance(old, dict):
            kind = "additional-properties-narrowed"
            yield Finding(file, f"{ptr}/{key}", kind, _show(o.get(key)), _show(new))
        else:
            yield from compare_schema(file, f"{ptr}/{key}", old, new)


def _compare_children(file: str, ptr: str, o: dict, n: dict) -> Iterator[Finding]:
    for key in _SCHEMA_MAPS:
        old_map, new_map = o.get(key, {}), n.get(key, {})
        for name, old_sub in old_map.items():
            child = f"{ptr}/{key}/{_esc(name)}"
            if name not in new_map:
                yield Finding(
                    file, child, _MAP_REMOVED_KIND[key], _show(old_sub), "(absent)"
                )
            else:
                yield from compare_schema(file, child, old_sub, new_map[name])
    for key in _SCHEMA_SINGLE:
        if key in o and key in n:
            if key == "if" and o[key] != n[key]:
                yield Finding(
                    file, f"{ptr}/if", "if-changed", _show(o[key]), _show(n[key])
                )
            else:
                yield from compare_schema(file, f"{ptr}/{key}", o[key], n[key])
        elif key in n and key != "if":
            yield from compare_schema(file, f"{ptr}/{key}", {}, n[key])
    for key in _SCHEMA_LISTS:
        yield from _compare_list(file, ptr, key, o.get(key, []), n.get(key, []))


def _compare_list(
    file: str, ptr: str, key: str, old: list, new: list
) -> Iterator[Finding]:
    if key == "allOf":
        # Every allOf member constrains, so an added member narrows.
        for i, member in enumerate(new):
            child = f"{ptr}/allOf/{i}"
            if i < len(old):
                yield from compare_schema(file, child, old[i], member)
            else:
                yield from compare_schema(file, child, {}, member)
        return
    for i, member in enumerate(old):
        child = f"{ptr}/{key}/{i}"
        if i >= len(new):
            kind = (
                "alternative-removed" if key != "prefixItems" else "prefix-item-removed"
            )
            yield Finding(file, child, kind, _show(member), "(absent)")
        else:
            yield from compare_schema(file, child, member, new[i])


def compare_snapshots(old: dict[str, dict], new: dict[str, dict]) -> list[Finding]:
    """Compare schemas by file name. Added files are allowed."""
    findings: list[Finding] = []
    for name, old_schema in sorted(old.items()):
        if name not in new:
            findings.append(
                Finding(
                    name, "", "file-removed", old_schema.get("$id", name), "(absent)"
                )
            )
            continue
        findings.extend(compare_schema(name, "", old_schema, new[name]))
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
        errors = sorted(
            validator.iter_errors(instance), key=lambda e: list(e.absolute_path)
        )
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
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise CompatError(f"cannot read {path}: {exc}") from exc
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


def _parse_schema(name: str, text: str) -> dict:
    try:
        return json.loads(text)
    except json.JSONDecodeError as exc:
        raise CompatError(f"cannot parse {name}: {exc}") from exc


def snapshot_from_dir(directory: Path) -> Snapshot:
    if not directory.is_dir():
        raise CompatError(f"not a directory: {directory}")
    schemas = {
        p.name: _parse_schema(p.name, p.read_text(encoding="utf-8"))
        for p in sorted(directory.glob(f"*{SCHEMA_SUFFIX}"))
    }
    conformance = directory / "conformance"
    corpus = {
        p.relative_to(conformance).as_posix(): p.read_text(encoding="utf-8")
        for p in sorted(conformance.rglob("*"))
        if p.is_file()
    }
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
    except (OSError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", "") or str(exc)
        raise CompatError(f"git {' '.join(args)} failed: {detail.strip()}") from exc
    return done.stdout


def snapshot_from_ref(repo: Path, ref: str) -> Snapshot:
    names = _git(
        repo, "ls-tree", "-r", "--name-only", ref, "--", f"{SCHEMA_REL}/"
    ).splitlines()
    prefix = f"{SCHEMA_REL}/"
    schemas: dict[str, dict] = {}
    corpus: dict[str, str] = {}
    for full in names:
        rel = full.removeprefix(prefix)
        if "/" not in rel and rel.endswith(SCHEMA_SUFFIX):
            schemas[rel] = _parse_schema(rel, _git(repo, "show", f"{ref}:{full}"))
        elif rel.startswith("conformance/"):
            corpus[rel.removeprefix("conformance/")] = _git(
                repo, "show", f"{ref}:{full}"
            )
    if not schemas:
        raise CompatError(f"no {SCHEMA_REL}/*{SCHEMA_SUFFIX} at {ref}")
    return Snapshot(schemas, corpus)


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
