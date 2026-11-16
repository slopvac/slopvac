"""Frame admissibility for binary-extractor/extract (plugin-protocol.md section 5.5.6).

``schemas/v3/conformance/plugin-protocol/binary-frames.json`` is the language-neutral
golden. Its sizes were worked out by hand from 4 * ceil(n / 3) plus the envelope. The
preflight cases are checked from the length alone, without allocating the file, and the
frame cases serialize each request for real, so the Rust wire adapter can replay the
same file.
"""

from __future__ import annotations

import base64
import copy
import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

SCHEMAS = Path(__file__).resolve().parents[2] / "schemas" / "v3"
VECTORS = json.loads(
    (SCHEMAS / "conformance/plugin-protocol/binary-frames.json").read_text("utf-8")
)


def _request(document_id: str, byte_length: int, encoded: str) -> dict:
    request = copy.deepcopy(VECTORS["request"])
    document = request["params"]["documents"][0]
    document.update(id=document_id, byte_length=byte_length, bytes=encoded)
    return request


def _size(request: dict) -> int:
    return len(json.dumps(request, separators=(",", ":")).encode("utf-8"))


@pytest.mark.parametrize(
    "case", VECTORS["base64_lengths"], ids=lambda c: str(c["bytes"])
)
def test_base64_length_of_n_bytes(case: dict) -> None:
    assert len(base64.b64encode(bytes(case["bytes"]))) == case["length"]


def _base64_length(n: int) -> int:
    return 4 * -(-n // 3)


@pytest.mark.parametrize("case", VECTORS["preflight"], ids=lambda c: c["id"])
def test_preflight_decides_from_the_length_alone(case: dict) -> None:
    envelope = _size(_request(case["document_id"], case["byte_length"], ""))
    preflight = envelope + _base64_length(case["byte_length"])

    assert envelope == case["envelope_bytes"]
    assert _base64_length(case["byte_length"]) == case["base64_length"]
    assert preflight == case["preflight_bytes"]
    assert (preflight <= case["max_message_bytes"]) is case["admissible"]


@pytest.mark.parametrize(
    "case", [c for c in VECTORS["preflight"] if c["admissible"]], ids=lambda c: c["id"]
)
def test_admitted_request_reserves_raw_base64_and_frame(case: dict) -> None:
    reservation = case["byte_length"] + case["base64_length"] + case["preflight_bytes"]

    assert reservation == case["reservation_bytes"]
    assert reservation <= VECTORS["in_flight_budget_bytes"]


@pytest.mark.parametrize("case", VECTORS["frames"], ids=lambda c: c["id"])
def test_whole_request_size_decides_admissibility(case: dict) -> None:
    encoded = base64.b64encode(bytes(case["byte_length"])).decode("ascii")
    envelope = _request(case["document_id"], case["byte_length"], "")
    request = _request(case["document_id"], case["byte_length"], encoded)

    assert _size(envelope) == case["envelope_bytes"]
    assert len(encoded) == case["base64_length"]
    assert _size(request) == case["request_bytes"]
    assert (
        _size(envelope) + _base64_length(case["byte_length"]) == case["request_bytes"]
    )
    assert (case["request_bytes"] <= case["max_message_bytes"]) is case["admissible"]


def test_request_template_is_a_valid_protocol_message() -> None:
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
    request = _request("d1", 8, base64.b64encode(bytes(8)).decode("ascii"))
    errors = list(
        Draft202012Validator(schemas[0], registry=registry).iter_errors(request)
    )
    assert errors == []
