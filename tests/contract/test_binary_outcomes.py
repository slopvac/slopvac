"""Host reporting of binary extraction outcomes (plugin-protocol.md section 5.5.4).

``schemas/v3/conformance/plugin-protocol/binary-outcomes.json`` maps each outcome to
the notices, the unchecked codes and the exit code the host reports. The host suite
(row H38) replays it. These tests pin the lead's outcome classes on the golden data:
skipped content is an info notice that never fails the run, a no-source result depends
on whether the name promises a source, and every failure is unchecked with exit 2.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

SCHEMAS = Path(__file__).resolve().parents[2] / "schemas" / "v3"
VECTORS = json.loads(
    (SCHEMAS / "conformance/plugin-protocol/binary-outcomes.json").read_text("utf-8")
)
CASES = {c["id"]: c for c in VECTORS["cases"]}
PROMISES_SOURCE = (".drawio.png", ".excalidraw.png")
FAILURE_CODES = {"extract-encrypted", "extract-malformed", "extract-budget"}


def _result_validator() -> Draft202012Validator:
    schemas = [
        json.loads(p.read_text("utf-8"))
        for p in (
            SCHEMAS / "plugin-protocol.schema.json",
            SCHEMAS / "common.schema.json",
        )
    ]
    registry = Registry().with_resources(
        (s["$id"], Resource.from_contents(s)) for s in schemas
    )
    ref = {"$ref": f"{schemas[0]['$id']}#/$defs/BinaryExtractResult"}
    return Draft202012Validator(ref, registry=registry)


@pytest.mark.parametrize("case", VECTORS["cases"], ids=lambda c: c["id"])
def test_each_result_entry_is_valid_on_the_wire(case: dict) -> None:
    reply = {"results": [case["result"]]}
    assert list(_result_validator().iter_errors(reply)) == []


@pytest.mark.parametrize("case", VECTORS["cases"], ids=lambda c: c["id"])
def test_exit_is_2_exactly_when_something_is_unchecked(case: dict) -> None:
    assert (case["exit"] == 2) is bool(case["unchecked"])
    assert case["exit"] in (0, 2)


def test_skipped_content_is_an_info_notice_and_never_unchecked() -> None:
    case = CASES["extracted-with-skipped-part"]
    assert case["notices"] == [{"code": "extract-unsupported", "level": "info"}]
    assert case["unchecked"] == []
    assert case["exit"] == 0
    assert all("extract-unsupported" not in c["unchecked"] for c in VECTORS["cases"])


@pytest.mark.parametrize(
    "case",
    [c for c in VECTORS["cases"] if c["result"].get("outcome") == "no-source"],
    ids=lambda c: c["id"],
)
def test_no_source_depends_on_whether_the_name_promises_a_source(case: dict) -> None:
    if case["path"].endswith(PROMISES_SOURCE):
        assert case["unchecked"] == ["binary-no-source"]
        assert case["exit"] == 2
    else:
        assert case["notices"] == [{"code": "binary-no-source", "level": "info"}]
        assert case["exit"] == 0


@pytest.mark.parametrize(
    "case",
    [c for c in VECTORS["cases"] if "error" in c["result"]],
    ids=lambda c: c["id"],
)
def test_a_failed_document_is_unchecked_under_its_error_code(case: dict) -> None:
    code = case["result"]["error"]["data"]["slopvac_code"]
    assert code in FAILURE_CODES
    assert case["unchecked"] == [code]
    assert case["exit"] == 2
