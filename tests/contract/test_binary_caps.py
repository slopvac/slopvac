"""Output budgets for binary extractors (plugin-protocol.md section 5.5.5).

``schemas/v3/conformance/plugin-protocol/binary-caps.json`` is the language-neutral
golden that the shared validator replays. These tests pin the budget names and values
the contract owner approved, check each case's verdict against them, check that each
expected error is a valid result entry, and check that the part and locator cases agree
with the schema's pattern bound.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

SCHEMAS = Path(__file__).resolve().parents[2] / "schemas" / "v3"
CONFORMANCE = SCHEMAS / "conformance/plugin-protocol"
VECTORS = json.loads((CONFORMANCE / "binary-caps.json").read_text("utf-8"))
# Approved on #514 by the contract owner (slopvac-8l4.2): names, order and values.
APPROVED_BUDGETS = {
    "segments": 1_000_000,
    "part": 1024,
    "locator": 1024,
    "text": 32 * 1024 * 1024,
    "in-flight": 64 * 1024 * 1024,
}
OUTPUT_BUDGETS = ("segments", "part", "locator", "text")


def _schemas() -> list[dict]:
    return [
        json.loads(p.read_text("utf-8"))
        for p in (
            SCHEMAS / "plugin-protocol.schema.json",
            SCHEMAS / "common.schema.json",
        )
    ]


def _validator(definition: str) -> Draft202012Validator:
    schemas = _schemas()
    registry = Registry().with_resources(
        (s["$id"], Resource.from_contents(s)) for s in schemas
    )
    ref = {"$ref": f"{schemas[0]['$id']}#/$defs/{definition}"}
    return Draft202012Validator(ref, registry=registry)


def _measures(case: dict) -> dict[str, int]:
    return {
        "segments": case["segments"],
        "part": case["part_chars"],
        "locator": case["locator_chars"],
        "text": case["segments"] * case["text_bytes_each"],
    }


def test_budgets_are_the_approved_names_and_values() -> None:
    assert list(VECTORS["budgets"].items()) == list(APPROVED_BUDGETS.items())


def test_the_schema_holds_the_one_extensible_list_of_budget_names() -> None:
    budget = _schemas()[0]["$defs"]["BinaryBudget"]
    assert budget["enum"] == list(APPROVED_BUDGETS)
    assert budget["x-slopvac-extensible"] is True


def test_in_flight_budget_matches_the_frame_vectors() -> None:
    frames = json.loads((CONFORMANCE / "binary-frames.json").read_text("utf-8"))
    assert frames["in_flight_budget_bytes"] == APPROVED_BUDGETS["in-flight"]


@pytest.mark.parametrize("case", VECTORS["cases"], ids=lambda c: c["id"])
def test_a_case_fails_with_binary_budget_naming_the_budget(case: dict) -> None:
    exceeded = [k for k, v in _measures(case).items() if v > APPROVED_BUDGETS[k]]

    if case["expected"] == "accepted":
        assert exceeded == []
    else:
        assert len(exceeded) == 1
        assert exceeded[0] in OUTPUT_BUDGETS
        assert case["expected"] == {
            "code": "extract-budget",
            "error": -32010,
            "budget": exceeded[0],
            "limit": APPROVED_BUDGETS[exceeded[0]],
        }


@pytest.mark.parametrize(
    "case",
    [c for c in VECTORS["cases"] if c["expected"] != "accepted"],
    ids=lambda c: c["id"],
)
def test_expected_error_is_a_valid_result_entry(case: dict) -> None:
    expected = case["expected"]
    result = {
        "results": [
            {
                "document": "d1",
                "error": {
                    "code": expected["error"],
                    "message": "budget reached",
                    "data": {
                        "slopvac_code": expected["code"],
                        "budget": expected["budget"],
                        "limit": expected["limit"],
                    },
                },
            }
        ]
    }
    assert list(_validator("BinaryExtractResult").iter_errors(result)) == []


@pytest.mark.parametrize(
    "case",
    [c for c in VECTORS["cases"] if c["id"].startswith(("part-", "locator-"))],
    ids=lambda c: c["id"],
)
def test_length_cases_agree_with_the_schema_bound(case: dict) -> None:
    segment = {
        "part": "p" * case["part_chars"],
        "locator": "l" * case["locator_chars"],
        "span": {"start": 0, "end": 0},
        "text": "t" * case["text_bytes_each"],
        "context": "prose",
        "role": "body",
    }
    accepted = _validator("BinarySegment").is_valid(segment)
    assert accepted is (case["expected"] == "accepted")
