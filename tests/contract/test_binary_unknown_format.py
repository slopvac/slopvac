"""Consumer rule for an unknown binary format id (plugin-protocol.md section 5.5.1).

``schemas/v3/conformance/plugin-protocol/binary-unknown-format.json`` holds a request
with one known and one unknown format, the reply a conforming plugin sends, and a
manifest that declares the unknown id. The schema-instance harness cannot express the
rule on its own, because a strict check rejects the unknown id; these tests pin that the
only rejection sits at the extensible position, that the reply declines one document
without failing the request, and that manifest validation stays strict.
"""

from __future__ import annotations

import json
from pathlib import Path

from jsonschema import Draft202012Validator
from referencing import Registry, Resource

SCHEMAS = Path(__file__).resolve().parents[2] / "schemas" / "v3"
VECTOR = json.loads(
    (SCHEMAS / "conformance/plugin-protocol/binary-unknown-format.json").read_text(
        "utf-8"
    )
)


def _schemas() -> list[dict]:
    return [
        json.loads(p.read_text("utf-8"))
        for p in (
            SCHEMAS / "plugin-protocol.schema.json",
            SCHEMAS / "common.schema.json",
        )
    ]


def _validator(definition: str | None = None) -> Draft202012Validator:
    schemas = _schemas()
    registry = Registry().with_resources(
        (s["$id"], Resource.from_contents(s)) for s in schemas
    )
    root = (
        {"$ref": f"{schemas[0]['$id']}#/$defs/{definition}"}
        if definition
        else schemas[0]
    )
    return Draft202012Validator(root, registry=registry)


def _pointers(errors) -> set[str]:
    return {"".join(f"/{p}" for p in e.absolute_path) for e in errors}


def test_binary_format_is_marked_extensible() -> None:
    assert _schemas()[0]["$defs"]["BinaryFormat"]["x-slopvac-extensible"] is True


def test_a_strict_check_rejects_the_request_only_at_the_unknown_format() -> None:
    errors = list(_validator().iter_errors(VECTOR["request"]))
    assert _pointers(errors) == {VECTOR["unknown_format_pointer"]}


def test_the_reply_declines_the_unknown_document_and_answers_the_rest() -> None:
    reply = VECTOR["reply"]
    assert list(_validator("BinaryExtractResult").iter_errors(reply)) == []

    by_id = {r["document"]: r for r in reply["results"]}
    requested = [d["id"] for d in VECTOR["request"]["params"]["documents"]]
    assert sorted(by_id) == sorted(requested)

    declined = by_id[VECTOR["declined_document"]]
    assert declined["error"]["code"] == -32008
    assert declined["error"]["data"]["slopvac_code"] == "binary-unsupported"
    assert "outcome" not in declined
    others = [r for d, r in by_id.items() if d != VECTOR["declined_document"]]
    assert all("error" not in r for r in others)


def test_manifest_validation_refuses_the_unknown_format() -> None:
    errors = list(_validator("PluginDeclaration").iter_errors(VECTOR["manifest"]))
    assert VECTOR["manifest_pointer"] in _pointers(errors)
