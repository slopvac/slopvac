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
HOST_CASES = {c["id"]: c for c in VECTORS["host_cases"]}
PROMISES_SOURCE = (".drawio.png", ".excalidraw.png", ".drawio.svg")
FAILURE_CODES = {
    "binary-input-declined",
    "binary-encrypted",
    "extract-malformed",
    "extract-budget",
}


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


@pytest.mark.parametrize(
    "case", VECTORS["cases"] + VECTORS["host_cases"], ids=lambda c: c["id"]
)
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
    [c for c in VECTORS["cases"] if c["result"].get("outcome") == "no-source"]
    + [c for c in VECTORS["host_cases"] if "binary-no-source" in str(c)],
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


@pytest.mark.parametrize("case_id", ["named-epub", "named-pdf"])
def test_a_named_input_in_an_unread_format_exits_2_without_extraction(
    case_id: str,
) -> None:
    case = HOST_CASES[case_id]
    assert case["named_explicitly"] is True
    assert case["sent_to_extractor"] is False
    assert case["unchecked"] == ["binary-format-unsupported"]
    assert case["format"] == case["path"].rsplit(".", 1)[1]
    assert case["exit"] == 2


@pytest.mark.parametrize(
    "case",
    [CASES["encrypted"], HOST_CASES["encrypted-ooxml-package"]],
    ids=lambda c: c["id"],
)
def test_an_encrypted_container_exits_2_and_is_never_unsupported(case: dict) -> None:
    assert case["unchecked"] == ["binary-encrypted"]
    assert case["exit"] == 2
    assert "extract-unsupported" not in json.dumps(case)


@pytest.mark.parametrize("case", VECTORS["validator_cases"], ids=lambda c: c["id"])
def test_validator_failure_code_depends_on_who_produced_the_output(case: dict) -> None:
    expected = {"built-in": "extract-output-invalid", "plugin": "plugin-output-invalid"}
    assert case["code"] == expected[case["source"]]
    assert case["exit"] == 2


@pytest.mark.parametrize(
    ("case_id", "reason"),
    [
        ("declined-variant-unsupported", "variant-unsupported"),
        ("declined-unknown-format", "unknown-format"),
    ],
)
def test_a_declined_document_exits_2_and_names_its_reason(
    case_id: str, reason: str
) -> None:
    case = CASES[case_id]
    error = case["result"]["error"]
    assert error["code"] == -32008
    assert error["data"] == {"slopvac_code": "binary-input-declined", "reason": reason}
    assert case["unchecked"] == ["binary-input-declined"]
    assert case["exit"] == 2


def test_extract_unsupported_only_ever_marks_a_skipped_part() -> None:
    every = VECTORS["cases"] + VECTORS["host_cases"]
    for case in every:
        error = case.get("result", {}).get("error")
        if error:
            assert error["data"]["slopvac_code"] != "extract-unsupported"
        assert "extract-unsupported" not in case["unchecked"]
        for notice in case["notices"]:
            if notice["code"] == "extract-unsupported":
                assert notice["level"] == "info"
                assert case["result"]["unsupported"]
