"""Escaped locator length for binary extractors (plugin-protocol.md section 5.5.5).

``binary-locator-escape.json`` holds the vectors a host must satisfy: an extractor locator
is at most 1024 characters as sent, the host percent-escapes it, and the escaped form is at
most 3072 code points (the report's ``origin.locator`` bound). A longer escaped form fails
the document with ``extract-budget`` and ``data.budget = "locator"``. The reference escaper
here is written from the contract text, not from any implementation.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

SCHEMAS = Path(__file__).resolve().parents[2] / "schemas" / "v3"
VECTORS = json.loads(
    (SCHEMAS / "conformance/plugin-protocol/binary-locator-escape.json").read_text("utf-8")
)
SENT_LIMIT = 1024
ESCAPED_LIMIT = 3072
ACCEPTED = [c for c in VECTORS["cases"] if c["expected"] == "accepted"]
REFUSED = [c for c in VECTORS["cases"] if c["expected"] != "accepted"]


def _must_escape(c: str) -> bool:
    n = ord(c)
    return (
        c == "%"
        or n < 0x20
        or 0x7F <= n <= 0x9F
        or n in (0x2028, 0x2029)
        or 0x202A <= n <= 0x202E
        or 0x2066 <= n <= 0x2069
    )


def _escape(locator: str) -> str:
    """Write each character that needs it as `%XX` per UTF-8 byte, uppercase hex."""
    return "".join(
        "".join(f"%{b:02X}" for b in c.encode("utf-8")) if _must_escape(c) else c
        for c in locator
    )


def _locator(case: dict) -> str:
    return "".join(char * count for char, count in case["parts"])


def _origin(case: dict) -> dict:
    return {
        "locator": _escape(_locator(case)),
        "label": "Paragraph 1",
        "segment": "s0001",
        "range": {"start": 0, "end": 1},
    }


def _origin_validator() -> Draft202012Validator:
    schema = json.loads((SCHEMAS / "lint-report.schema.json").read_text("utf-8"))
    registry = Registry().with_resources([(schema["$id"], Resource.from_contents(schema))])
    return Draft202012Validator(
        {"$ref": f"{schema['$id']}#/$defs/Origin"}, registry=registry
    )


def test_the_two_limits_are_the_approved_values() -> None:
    assert (VECTORS["sent_limit"], VECTORS["escaped_limit"]) == (SENT_LIMIT, ESCAPED_LIMIT)


@pytest.mark.parametrize("case", VECTORS["cases"], ids=lambda c: c["id"])
def test_declared_lengths_match_the_reference_escaper(case: dict) -> None:
    sent = _locator(case)
    assert len(sent) == case["sent_chars"]
    assert len(sent) <= SENT_LIMIT
    assert len(_escape(sent)) == case["escaped_chars"]


@pytest.mark.parametrize("case", VECTORS["cases"], ids=lambda c: c["id"])
def test_verdict_follows_the_escaped_length(case: dict) -> None:
    within = case["escaped_chars"] <= ESCAPED_LIMIT
    assert (case["expected"] == "accepted") is within
    if not within:
        assert case["expected"] == {
            "code": "extract-budget",
            "error": -32010,
            "budget": "locator",
            "limit": ESCAPED_LIMIT,
            "exit": 2,
        }


@pytest.mark.parametrize("case", ACCEPTED, ids=lambda c: c["id"])
def test_an_accepted_locator_gives_a_valid_report_origin(case: dict) -> None:
    assert list(_origin_validator().iter_errors(_origin(case))) == []


@pytest.mark.parametrize("case", REFUSED, ids=lambda c: c["id"])
def test_a_refused_locator_would_not_be_a_valid_report_origin(case: dict) -> None:
    assert list(_origin_validator().iter_errors(_origin(case))) != []


def test_the_vectors_cover_the_four_boundaries() -> None:
    assert {c["id"]: c["escaped_chars"] for c in VECTORS["cases"]} == {
        "ascii-at-sent-limit": 1024,
        "escaped-at-cap": 3072,
        "escaped-over-cap": 3073,
        "line-separators-at-sent-limit": 9216,
    }
