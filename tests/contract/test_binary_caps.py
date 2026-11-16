"""Output caps for binary extractors (plugin-protocol.md section 5.5.5).

``schemas/v3/conformance/plugin-protocol/binary-caps.json`` is the language-neutral
golden that the shared validator replays. These tests pin the cap constants the
contract owner approved, check each case's verdict against them, and check that the
part and locator cases agree with the schema's pattern bound.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

SCHEMAS = Path(__file__).resolve().parents[2] / "schemas" / "v3"
VECTORS = json.loads(
    (SCHEMAS / "conformance/plugin-protocol/binary-caps.json").read_text("utf-8")
)
# Approved on #514 by the contract owner (slopvac-8l4.2) and fixed in section 5.5.5.
APPROVED_CAPS = {
    "segments": 1_000_000,
    "text-bytes": 32 * 1024 * 1024,
    "part-length": 1024,
    "locator-length": 1024,
}


def _measures(case: dict) -> dict[str, int]:
    return {
        "segments": case["segments"],
        "text-bytes": case["segments"] * case["text_bytes_each"],
        "part-length": case["part_length"],
        "locator-length": case["locator_length"],
    }


def _segment_validator() -> Draft202012Validator:
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
    ref = {"$ref": f"{schemas[0]['$id']}#/$defs/BinarySegment"}
    return Draft202012Validator(ref, registry=registry)


def test_cap_constants_are_the_approved_values() -> None:
    assert VECTORS["caps"] == APPROVED_CAPS


@pytest.mark.parametrize("case", VECTORS["cases"], ids=lambda c: c["id"])
def test_a_case_fails_with_binary_budget_naming_the_cap_it_exceeds(case: dict) -> None:
    exceeded = [k for k, v in _measures(case).items() if v > APPROVED_CAPS[k]]

    if case["expected"] == "accepted":
        assert exceeded == []
    else:
        assert case["expected"] == {
            "code": "binary-budget",
            "error": -32010,
            "cap": exceeded[0],
        }
        assert len(exceeded) == 1


@pytest.mark.parametrize(
    "case",
    [c for c in VECTORS["cases"] if c["id"].startswith(("part-", "locator-"))],
    ids=lambda c: c["id"],
)
def test_length_cases_agree_with_the_schema_bound(case: dict) -> None:
    segment = {
        "part": "p" * case["part_length"],
        "locator": "l" * case["locator_length"],
        "span": {"start": 0, "end": 0},
        "text": "t" * case["text_bytes_each"],
        "context": "prose",
        "role": "body",
    }
    accepted = _segment_validator().is_valid(segment)
    assert accepted is (case["expected"] == "accepted")
