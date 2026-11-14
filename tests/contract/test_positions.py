"""Check the position vectors against the Python reference converter.

``schemas/v3/conformance/positions/vectors.json`` is the shared, language-neutral
golden. The Rust port must pass the same file. Core positions and projection spans
in the vectors were worked out by hand; format payloads come from the converter, so
this test pins them against later drift.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from position_reference import (
    LSP_ENCODING,
    UNITS,
    Doc,
    PositionError,
    apply_edits,
    check_tokens,
    formats,
    negotiate_encoding,
    position,
    project_range,
)

VECTORS = json.loads(
    (
        Path(__file__).resolve().parents[2]
        / "schemas/v3/conformance/positions/vectors.json"
    ).read_text(encoding="utf-8")
)
DOCS = {name: Doc.from_text(d["text"]) for name, d in VECTORS["documents"].items()}


def _ids(key: str) -> list[str]:
    return [v.get("id") or f"{v['document']}@{v['offset']}" for v in VECTORS[key]]


def test_document_byte_lengths_match_the_text() -> None:
    for name, entry in VECTORS["documents"].items():
        assert len(DOCS[name].data) == entry["byte_length"], name


@pytest.mark.parametrize("case", VECTORS["positions"], ids=_ids("positions"))
def test_position_in_every_unit(case: dict) -> None:
    doc = DOCS[case["document"]]
    for unit in UNITS:
        assert position(doc, case["offset"], unit) == (case["line"], case[unit])


@pytest.mark.parametrize(
    "case", VECTORS["invalid_offsets"], ids=_ids("invalid_offsets")
)
def test_invalid_offset_is_rejected(case: dict) -> None:
    with pytest.raises(PositionError):
        position(DOCS[case["document"]], case["offset"])


@pytest.mark.parametrize("case", VECTORS["spans"], ids=_ids("spans"))
def test_span_start_end_and_format_payloads(case: dict) -> None:
    doc, span = DOCS[case["document"]], case["span"]
    for unit in UNITS:
        assert position(doc, span["start"], unit) == (
            case["start"]["line"],
            case["start"][unit],
        )
        assert position(doc, span["end"], unit) == (
            case["end"]["line"],
            case["end"][unit],
        )
    assert formats(doc, span["start"], span["end"]) == case["formats"]


@pytest.mark.parametrize("case", VECTORS["spans"], ids=_ids("spans"))
def test_format_units_follow_the_contract(case: dict) -> None:
    """Each format reads its documented unit from the core positions."""
    out, start, end = case["formats"], case["start"], case["end"]
    assert out["sarif"]["startColumn"] == start["utf16"]
    assert out["sarif"]["endColumn"] == end["utf16"]
    assert out["rdjsonl"]["start"]["column"] == start["utf8"]
    assert out["rdjsonl"]["end"]["column"] == end["utf8"]
    assert out["json"]["start"]["column"] == start["utf32"]
    assert out["json"]["end"]["column"] == end["utf32"]
    for unit in UNITS:
        lsp = out["lsp"][LSP_ENCODING[unit]]
        assert lsp["start"] == {"line": start["line"] - 1, "character": start[unit] - 1}
        assert lsp["end"] == {"line": end["line"] - 1, "character": end[unit] - 1}


@pytest.mark.parametrize("case", VECTORS["invalid_spans"], ids=_ids("invalid_spans"))
def test_invalid_span_is_rejected(case: dict) -> None:
    span = case["span"]
    with pytest.raises(PositionError):
        formats(DOCS[case["document"]], span["start"], span["end"])


@pytest.mark.parametrize("case", VECTORS["edits"], ids=_ids("edits"))
def test_edits_apply_against_original_offsets(case: dict) -> None:
    result = apply_edits(DOCS[case["document"]], case["edits"])
    assert result == case["result"].encode("utf-8")


@pytest.mark.parametrize("case", VECTORS["invalid_edits"], ids=_ids("invalid_edits"))
def test_invalid_edit_set_is_rejected(case: dict) -> None:
    with pytest.raises(PositionError):
        apply_edits(DOCS[case["document"]], case["edits"])


@pytest.mark.parametrize("case", VECTORS["projections"], ids=_ids("projections"))
def test_projection_tokens_are_consistent_with_the_source(case: dict) -> None:
    doc = Doc.from_text(case["source"])
    assert check_tokens(doc, case["tokens"]) == case["projected"]


@pytest.mark.parametrize("case", VECTORS["projections"], ids=_ids("projections"))
def test_projected_ranges_map_to_source_spans(case: dict) -> None:
    for entry in case["ranges"]:
        start, end = entry["projected"]
        got = project_range(case["tokens"], start, end)
        assert got == (entry["span"]["start"], entry["span"]["end"]), entry


@pytest.mark.parametrize(
    "case", VECTORS["lsp_negotiation"], ids=lambda c: "-".join(c["client"] or ["none"])
)
def test_lsp_encoding_negotiation(case: dict) -> None:
    assert negotiate_encoding(case["client"]) == case["server"]


@pytest.mark.parametrize(
    "tokens",
    [
        [
            {"kind": "identity", "text": "x", "src": [0, 1]},
            {"kind": "identity", "text": "a", "src": [0, 1]},
        ],
        [{"kind": "entity", "text": "&", "src": [0, 1]}],
        [{"kind": "wrap", "text": " ", "src": [1, 1]}],
        [{"kind": "masked", "text": "\0", "src": [1, 1]}],
    ],
    ids=[
        "out-of-order",
        "wrong-entity-decode",
        "wrap-not-anchored",
        "empty-real-token",
    ],
)
def test_malformed_projection_tokens_are_rejected(tokens: list[dict]) -> None:
    with pytest.raises(PositionError):
        check_tokens(Doc.from_text("xa"), tokens)


def test_invalid_utf8_source_is_rejected() -> None:
    with pytest.raises(PositionError):
        Doc.from_bytes(b"ok \xff\n")
