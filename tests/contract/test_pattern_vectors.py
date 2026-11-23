"""Check the shared schema pattern vectors against Python jsonschema.

``schemas/v3/conformance/patterns/vectors.json`` records, for each pattern and input, the
verdict of Python jsonschema and of the Rust jsonschema crate. This test checks the Python
half; ``crates/slopvac-schemas/tests/patterns.rs`` checks the Rust half and that the two
differ only for patterns outside the portable subset.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

VECTORS = (
    Path(__file__).resolve().parents[2] / "schemas/v3/conformance/patterns/vectors.json"
)
_ALL = json.loads(VECTORS.read_text(encoding="utf-8"))["vectors"]


@pytest.mark.parametrize(
    "vector", _ALL, ids=[f"{v['pattern'][:40]}|{v['input']!r}" for v in _ALL]
)
def test_python_gives_the_recorded_verdict(vector: dict) -> None:
    validator = Draft202012Validator({"type": "string", "pattern": vector["pattern"]})
    assert validator.is_valid(vector["input"]) is vector["python"]


# The vectors above test a copy of each pattern. The two source patterns also run here inside a
# whole document against the published schema, so a change to the published schema that the
# copy does not follow still fails.
_SCHEMAS = VECTORS.parents[2]
_DIGEST = "sha256:" + "0123456789abcdef" * 4


def _published(name: str, *path: str) -> dict:
    node = json.loads((_SCHEMAS / name).read_text(encoding="utf-8"))
    for key in path:
        node = node[key]
    return node


def _documents() -> dict[str, tuple[dict, object]]:
    """Per published source pattern: the schema file and a function that builds a document."""
    index_doc = json.loads(
        (VECTORS.parents[1] / "plugin-index/valid/one-data-plugin.json").read_text(
            encoding="utf-8"
        )
    )

    def index(source: str) -> dict:
        doc = json.loads(json.dumps(index_doc))
        doc["plugins"][0]["source"] = source
        return doc

    def lock(source: str) -> dict:
        return {
            "schema_version": 3,
            "plugins": [
                {
                    "id": "p0000",
                    "kind": "data",
                    "version": "1.0.0",
                    "slopvac_api": 1,
                    "source": source,
                    "digest": _DIGEST,
                }
            ],
        }

    return {
        _published("plugin-index.schema.json", "$defs", "PackIndexSource")["pattern"]: (
            "plugin-index.schema.json",
            index,
        ),
        _published("plugins-lock.schema.json", "$defs", "Locked", "properties", "source")[
            "pattern"
        ]: ("plugins-lock.schema.json", lock),
    }


_DOCS = _documents()
_SOURCE_VECTORS = [v for v in _ALL if v["pattern"] in _DOCS]


def test_every_published_source_pattern_has_vectors() -> None:
    have = {v["pattern"] for v in _SOURCE_VECTORS}
    assert have == set(_DOCS), "a published source pattern has no recorded vector"


@pytest.mark.parametrize(
    "vector",
    _SOURCE_VECTORS,
    ids=[f"{v['pattern'][:12]}|{v['input']!r}" for v in _SOURCE_VECTORS],
)
def test_published_schema_gives_the_recorded_verdict_for_a_whole_document(
    vector: dict,
) -> None:
    file, build = _DOCS[vector["pattern"]]
    schema = json.loads((_SCHEMAS / file).read_text(encoding="utf-8"))
    resources = [
        (s["$id"], Resource.from_contents(s))
        for s in (
            json.loads(p.read_text(encoding="utf-8"))
            for p in _SCHEMAS.glob("*.schema.json")
        )
    ]
    validator = Draft202012Validator(schema, registry=Registry().with_resources(resources))
    assert validator.is_valid(build(vector["input"])) is vector["python"]
