"""Compare two copies of the v3 schemas for validation meaning, ignoring layout.

A schema flip replaces a hand-written file with generator output. The two texts differ
in key order, in which subschemas are `$defs` and in annotations, so a text diff says
little. This script reduces both sides to a normal form and reports what still differs.

    python scripts/contract/schema_equivalence.py --old-ref origin/main \
        --expect-pattern '^[a-z]+$' '^[a-z]+(?![\\s\\S])' common lint-report

`--expect-pattern OLD NEW` records a reviewed pattern change: an old `pattern` equal to OLD
counts as unchanged when the new side holds NEW.

The normal form:

* every `$ref`, local or to another file of the same side, is replaced by its target;
* annotations are dropped: `title`, `description`, `$comment`, `examples`, `default`,
  `deprecated`, `readOnly`, `writeOnly`, `$schema`, `$id` and the `$defs` table;
* `allOf` members and `$ref` siblings are flattened into one conjunction and merged where
  the merge cannot change meaning;
* `{}` is `true`; `type` is a sorted list; `required` and `enum` are sorted sets;
  `anyOf` and `oneOf` members are sorted; an integral float is an integer.

Each artifact is compared at its root and at every `$defs` entry the old side has. Exit 0
means no validation difference; exit 1 lists each difference by JSON pointer. Annotation
differences are listed with `--annotations` and never change the exit code.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from urllib.parse import urldefrag, urljoin

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_DIR = "schemas/v3"

ANNOTATIONS = frozenset(
    {
        "title",
        "description",
        "$comment",
        "examples",
        "default",
        "deprecated",
        "readOnly",
        "writeOnly",
        "$schema",
        "$id",
        "$defs",
    }
)
# Keywords whose value is one subschema.
ONE = frozenset(
    {
        "additionalProperties",
        "items",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "propertyNames",
        "unevaluatedItems",
        "unevaluatedProperties",
        "contentSchema",
    }
)
# Keywords whose value maps names to subschemas.
MAP = frozenset({"properties", "patternProperties", "dependentSchemas"})
# Keywords whose value is a list of subschemas.
LIST = frozenset({"allOf", "anyOf", "oneOf", "prefixItems"})
UNORDERED_LISTS = frozenset({"anyOf", "oneOf"})
SET_VALUES = frozenset({"required", "enum"})
# Keywords that only mean something next to their siblings, so parts holding them are not
# merged with parts that hold the others.
OBJECT_SHAPE = frozenset(
    {"properties", "patternProperties", "additionalProperties", "unevaluatedProperties"}
)


class Side:
    """One copy of the schema files, keyed by `$id`."""

    def __init__(self, docs: dict[str, dict]) -> None:
        self.by_name = docs
        self.by_id = {d["$id"]: d for d in docs.values()}

    @classmethod
    def from_dir(cls, directory: Path) -> Side:
        return cls(
            {
                p.name.removesuffix(".schema.json"): json.loads(p.read_text("utf-8"))
                for p in sorted(directory.glob("*.schema.json"))
            }
        )

    @classmethod
    def from_ref(cls, ref: str) -> Side:
        names = subprocess.run(
            ["git", "ls-tree", "--name-only", f"{ref}:{SCHEMA_DIR}"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.split()
        docs = {}
        for name in names:
            if name.endswith(".schema.json"):
                text = subprocess.run(
                    ["git", "show", f"{ref}:{SCHEMA_DIR}/{name}"],
                    cwd=ROOT,
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout
                docs[name.removesuffix(".schema.json")] = json.loads(text)
        return cls(docs)

    def resolve(self, base: str, ref: str) -> tuple[str, object]:
        url, fragment = urldefrag(urljoin(base, ref))
        node: object = self.by_id[url]
        for token in filter(None, fragment.split("/")):
            token = token.replace("~1", "/").replace("~0", "~")
            node = node[int(token)] if isinstance(node, list) else node[token]
        return url, node


def _number(value: object) -> object:
    if isinstance(value, float) and value.is_integer():
        return int(value)
    return value


def _key(value: object) -> str:
    return json.dumps(value, sort_keys=True)


def _instance(value: object) -> object:
    """Normalize instance data (`const`, `enum` members): only number spelling."""
    if isinstance(value, dict):
        return {k: _instance(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_instance(v) for v in value]
    return _number(value)


def _merge(parts: list[dict]) -> list[dict]:
    """Merge conjunction parts whose keywords cannot interact."""
    merged: list[dict] = []
    for part in parts:
        for target in merged:
            shared = set(target) & set(part)
            if shared - {"required"}:
                continue
            if (set(target) & OBJECT_SHAPE) and (set(part) & OBJECT_SHAPE):
                continue
            if "required" in part:
                target["required"] = sorted(
                    set(target.get("required", [])) | set(part["required"])
                )
            target.update({k: v for k, v in part.items() if k != "required"})
            break
        else:
            merged.append(dict(part))
    return merged


class Normalizer:
    def __init__(self, side: Side, patterns: dict[str, str] | None = None) -> None:
        self.side = side
        self.patterns = patterns or {}

    def schema(self, node: object, base: str, stack: tuple = ()) -> object:
        if node is True or node == {}:
            return True
        if node is False:
            return False
        if not isinstance(node, dict):
            raise TypeError(f"not a schema: {node!r}")
        parts = self._parts(node, base, stack)
        if any(p is False for p in parts):
            return False
        parts = [p for p in parts if p is not True]
        if not parts:
            return True
        merged = _merge(parts)
        if len(merged) == 1:
            return merged[0]
        return {"allOf": sorted(merged, key=_key)}

    def _parts(self, node: dict, base: str, stack: tuple) -> list:
        parts: list = []
        own: dict = {}
        for key, value in node.items():
            if key in ANNOTATIONS:
                continue
            if key == "$ref":
                url, target = self.side.resolve(base, value)
                marker = (url, value if value.startswith("#") else urljoin(base, value))
                if marker in stack:
                    parts.append({"$recursive": marker[1]})
                else:
                    parts.append(self.schema(target, url, (*stack, marker)))
            elif key == "allOf":
                for member in value:
                    parts.append(self.schema(member, base, stack))
            elif key in ONE:
                own[key] = self.schema(value, base, stack)
            elif key in MAP:
                own[key] = {k: self.schema(v, base, stack) for k, v in value.items()}
            elif key in LIST:
                members = [self.schema(v, base, stack) for v in value]
                if key in UNORDERED_LISTS:
                    if len(members) == 1:
                        parts.append(members[0])
                        continue
                    members.sort(key=_key)
                own[key] = members
            elif key == "type":
                own[key] = sorted([value] if isinstance(value, str) else value)
            elif key in SET_VALUES:
                own[key] = sorted((_instance(v) for v in value), key=_key)
            elif key == "const":
                own[key] = _instance(value)
            elif key == "pattern" and isinstance(value, str):
                own[key] = self.patterns.get(value, value)
            else:
                own[key] = _instance(value)
        flat: list = []
        for part in parts:
            if isinstance(part, dict) and set(part) == {"allOf"}:
                flat.extend(part["allOf"])
            else:
                flat.append(part)
        if own:
            flat.insert(0, own)
        return flat


def _annotations(node: object, pointer: str, out: dict[str, object]) -> None:
    """Collect annotation keywords by pointer, treating `properties` names as names."""
    if isinstance(node, dict):
        for key, value in node.items():
            here = f"{pointer}/{key.replace('~', '~0').replace('/', '~1')}"
            if key in {"title", "description", "default", "examples", "$comment"}:
                out[here] = value
            elif key in MAP or key == "$defs":
                if isinstance(value, dict):
                    for name, sub in value.items():
                        sub_here = (
                            f"{here}/{name.replace('~', '~0').replace('/', '~1')}"
                        )
                        _annotations(sub, sub_here, out)
            elif key not in {"const", "enum"}:
                _annotations(value, here, out)
    elif isinstance(node, list):
        for i, value in enumerate(node):
            _annotations(value, f"{pointer}/{i}", out)


def diff(
    old: object, new: object, pointer: str = ""
) -> list[tuple[str, object, object]]:
    if isinstance(old, dict) and isinstance(new, dict):
        out = []
        for key in sorted(set(old) | set(new)):
            here = f"{pointer}/{key.replace('~', '~0').replace('/', '~1')}"
            if key not in old:
                out.append((here, "<absent>", new[key]))
            elif key not in new:
                out.append((here, old[key], "<absent>"))
            else:
                out.extend(diff(old[key], new[key], here))
        return out
    if isinstance(old, list) and isinstance(new, list) and len(old) == len(new):
        out = []
        for i, (a, b) in enumerate(zip(old, new, strict=True)):
            out.extend(diff(a, b, f"{pointer}/{i}"))
        return out
    return [] if _key(old) == _key(new) else [(pointer, old, new)]


def compare(
    old: Side, new: Side, artifact: str, patterns: dict[str, str] | None = None
) -> list[tuple[str, object, object]]:
    """Validation differences for one artifact, by pointer into the old file.

    `patterns` maps an old `pattern` value to the reviewed new value it is expected to become;
    a mapped pattern counts as unchanged when the new side holds the mapped value.
    """
    old_doc, new_doc = old.by_name[artifact], new.by_name.get(artifact)
    if new_doc is None:
        return [("", "<file>", "<absent>")]
    out: list[tuple[str, object, object]] = []
    if old_doc.get("$id") != new_doc.get("$id"):
        out.append(("/$id", old_doc.get("$id"), new_doc.get("$id")))
    old_n, new_n = Normalizer(old, patterns), Normalizer(new)
    out.extend(
        diff(
            old_n.schema(old_doc, old_doc["$id"]),
            new_n.schema(new_doc, new_doc["$id"]),
        )
    )
    new_defs = new_doc.get("$defs", {})
    for name, schema in old_doc.get("$defs", {}).items():
        here = f"/$defs/{name}"
        if name not in new_defs:
            out.append((here, "<def>", "<absent>"))
            continue
        out.extend(
            diff(
                old_n.schema(schema, old_doc["$id"]),
                new_n.schema(new_defs[name], new_doc["$id"]),
                here,
            )
        )
    return out


def annotation_changes(old: Side, new: Side, artifact: str) -> list[str]:
    before: dict[str, object] = {}
    after: dict[str, object] = {}
    _annotations(old.by_name[artifact], "", before)
    _annotations(new.by_name.get(artifact, {}), "", after)
    lines = []
    for pointer in sorted(set(before) | set(after)):
        a, b = before.get(pointer, "<absent>"), after.get(pointer, "<absent>")
        if _key(a) != _key(b):
            lines.append(
                f"  {pointer}\n    old: {json.dumps(a)}\n    new: {json.dumps(b)}"
            )
    return lines


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("artifacts", nargs="+", help="artifact names, such as common")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--old-ref", help="git ref holding the old schemas/v3")
    source.add_argument("--old-dir", type=Path, help="directory holding the old files")
    parser.add_argument(
        "--new-dir", type=Path, default=ROOT / SCHEMA_DIR, help="the new files"
    )
    parser.add_argument(
        "--annotations", action="store_true", help="also list annotation changes"
    )
    parser.add_argument(
        "--expect-pattern",
        nargs=2,
        action="append",
        default=[],
        metavar=("OLD", "NEW"),
        help="a reviewed pattern change: OLD on the old side is expected as NEW",
    )
    parser.add_argument(
        "--verbose", action="store_true", help="list every pointer of each difference"
    )
    args = parser.parse_args(argv)
    old = Side.from_ref(args.old_ref) if args.old_ref else Side.from_dir(args.old_dir)
    new = Side.from_dir(args.new_dir)
    failed = False
    for artifact in args.artifacts:
        differences = compare(old, new, artifact, dict(args.expect_pattern))
        status = "differs" if differences else "equivalent"
        print(f"{artifact}: {status}")
        failed = failed or bool(differences)
        groups: dict[tuple[str, str], list[str]] = {}
        for pointer, a, b in differences:
            groups.setdefault((json.dumps(a), json.dumps(b)), []).append(pointer or "/")
        for (a, b), pointers in groups.items():
            shown = pointers if args.verbose else pointers[:3]
            print(f"  old: {a}\n  new: {b}\n  at {len(pointers)} pointer(s):")
            for pointer in shown:
                print(f"    {pointer}")
            if len(pointers) > len(shown):
                print(
                    f"    ... {len(pointers) - len(shown)} more (--verbose lists them)"
                )
        if args.annotations:
            changes = annotation_changes(old, new, artifact)
            print(f"{artifact}: {len(changes)} annotation changes")
            print("\n".join(changes))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
