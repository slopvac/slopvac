"""Generic v3 contract harness: every schemas/v3/*.schema.json is checked offline.

Conformance instances live in schemas/v3/conformance/<artifact>/{valid,invalid}/
in native JSON, TOML or YAML. For a schema without a root shape (common), an
instance named ``<Def>--<name>.<ext>`` validates against ``#/$defs/<Def>``.
``invalid/cases.toml`` maps each invalid file to a JSON pointer where validation
must report an error.
"""

from __future__ import annotations

import json
import tomllib
from pathlib import Path

import pytest
import yaml
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

BASE = "https://srobroek.github.io/slopvac/schemas/"
DIALECT = "https://json-schema.org/draft/2020-12/schema"
SCHEMA_DIR = Path(__file__).resolve().parents[2] / "schemas" / "v3"
CONFORMANCE = SCHEMA_DIR / "conformance"
LOADERS = {
    ".json": json.loads,
    ".toml": tomllib.loads,
    ".yaml": yaml.safe_load,
    ".yml": yaml.safe_load,
}

SCHEMA_FILES = sorted(SCHEMA_DIR.glob("*.schema.json"))


def _load_schema(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def _registry() -> Registry:
    resources = [
        (s["$id"], Resource.from_contents(s)) for s in map(_load_schema, SCHEMA_FILES)
    ]
    return Registry().with_resources(resources)


def _artifact(path: Path) -> str:
    return path.name.removesuffix(".schema.json")


def _validator(artifact: str, instance_name: str) -> Draft202012Validator:
    schema = _load_schema(SCHEMA_DIR / f"{artifact}.schema.json")
    if "--" in instance_name:
        definition = instance_name.split("--", 1)[0]
        schema = {"$ref": f"{schema['$id']}#/$defs/{definition}"}
    return Draft202012Validator(schema, registry=_registry())


def _instances(kind: str) -> list[tuple[str, Path]]:
    return [
        (_artifact(s), p)
        for s in SCHEMA_FILES
        for p in sorted((CONFORMANCE / _artifact(s) / kind).glob("*"))
        if p.suffix in LOADERS and p.name != "cases.toml"
    ]


def _load(path: Path) -> object:
    return LOADERS[path.suffix](path.read_text(encoding="utf-8"))


def _ids(items: list[tuple[str, Path]]) -> list[str]:
    return [f"{a}/{p.name}" for a, p in items]


def _pointer(error) -> str:
    return "".join(
        f"/{str(p).replace('~', '~0').replace('/', '~1')}" for p in error.absolute_path
    )


@pytest.mark.parametrize("path", SCHEMA_FILES, ids=lambda p: p.name)
def test_schema_is_valid_draft_2020_12_with_stable_id(path: Path) -> None:
    schema = _load_schema(path)
    Draft202012Validator.check_schema(schema)
    assert schema["$schema"] == DIALECT
    assert schema["$id"] == f"{BASE}v3/{path.name}"


@pytest.mark.parametrize("path", SCHEMA_FILES, ids=lambda p: p.name)
def test_refs_resolve_offline(path: Path) -> None:
    schema = _load_schema(path)
    resolver = _registry().resolver(base_uri=schema["$id"])

    def walk(node: object) -> None:
        if isinstance(node, dict):
            ref = node.get("$ref")
            if isinstance(ref, str):
                resolver.lookup(ref)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    walk(schema)


_VALID = _instances("valid")
_INVALID = _instances("invalid")


@pytest.mark.parametrize(("artifact", "path"), _VALID, ids=_ids(_VALID))
def test_valid_instance_is_accepted(artifact: str, path: Path) -> None:
    errors = list(_validator(artifact, path.name).iter_errors(_load(path)))
    assert errors == []


@pytest.mark.parametrize(("artifact", "path"), _INVALID, ids=_ids(_INVALID))
def test_invalid_instance_fails_at_declared_pointer(artifact: str, path: Path) -> None:
    cases = tomllib.loads((path.parent / "cases.toml").read_text(encoding="utf-8"))
    assert path.name in cases, f"{path.name} missing from cases.toml"
    errors = list(_validator(artifact, path.name).iter_errors(_load(path)))
    assert errors, "instance was accepted"
    assert cases[path.name] in {_pointer(e) for e in errors}


@pytest.mark.parametrize("artifact", sorted({a for a, _ in _INVALID}))
def test_cases_toml_has_no_orphan_entries(artifact: str) -> None:
    folder = CONFORMANCE / artifact / "invalid"
    cases = tomllib.loads((folder / "cases.toml").read_text(encoding="utf-8"))
    assert set(cases) <= {p.name for p in folder.iterdir()}
