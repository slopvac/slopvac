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
    UNITS,
    Doc,
    PositionError,
    apply_edits,
    binary_host_edit,
    binary_host_location,
    parse_binary_host,
    check_tokens,
    formats,
    lsp_range,
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
    assert "lsp" not in out
    assert out["sarif"]["startColumn"] == start["utf16"]
    assert out["sarif"]["endColumn"] == end["utf16"]
    assert out["rdjsonl"]["start"]["column"] == start["utf8"]
    assert out["rdjsonl"]["end"]["column"] == end["utf8"]
    assert out["json"]["start"]["column"] == start["utf32"]
    assert out["json"]["end"]["column"] == end["utf32"]


@pytest.mark.parametrize("case", VECTORS["lsp_ranges"], ids=_ids("lsp_ranges"))
def test_lsp_range_over_the_received_buffer(case: dict) -> None:
    """LSP positions count the buffer text as received, a leading U+FEFF included."""
    buffer = Doc.from_buffer(DOCS[case["document"]].data)
    span = case["span"]
    assert lsp_range(buffer, span["start"], span["end"]) == case["expected"]


def test_buffer_coordinates_differ_from_disk_coordinates_after_a_bom() -> None:
    disk = DOCS["bom"]
    buffer = Doc.from_buffer(disk.data)
    assert position(disk, 5, "utf16") == (1, 3)
    assert lsp_range(buffer, 5, 5)["utf-16"]["start"] == {"line": 0, "character": 3}


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
    ordered = not case.get("unordered", False)
    assert check_tokens(doc, case["tokens"], ordered) == case["projected"]


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


def test_empty_range_at_the_end_anchors_to_the_last_token_source_end() -> None:
    trailing = [
        {"kind": "identity", "text": "foo", "src": [0, 3]},
        {"kind": "masked", "text": "", "src": [3, 8]},
    ]
    assert project_range(trailing, 3, 3) == (8, 8)
    assert project_range([{"kind": "masked", "text": "", "src": [0, 5]}], 0, 0) == (
        5,
        5,
    )


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


@pytest.mark.parametrize("case", VECTORS["binary_hosts"], ids=lambda c: c["id"])
def test_binary_host_location_is_segment_local(case: dict) -> None:
    span, rng = case["span"], case["container_range"]
    got = binary_host_location(
        case["segment"],
        span["start"],
        span["end"],
        path=case["path"],
        locator=case["locator"],
        container_range=(rng["start"], rng["end"]),
        kind=case.get("kind", "resource"),
    )
    assert got["local"] == case["local"]
    assert got["provenance"] == {"path": case["path"], "range": rng}
    assert got["text"] == case["text"]
    start = case["local"]["start"]
    assert parse_binary_host(got["text"]) == (
        case["path"],
        case["locator"],
        start["line"],
        start["column"],
    )
    assert got["sarif"] == {
        "physicalLocation": {"artifactLocation": {"uri": case["sarif_uri"]}},
        "logicalLocations": [
            {
                "fullyQualifiedName": case["locator"],
                "kind": case.get("kind", "resource"),
            }
        ],
        "properties": {"slopvac/local": case["local"]},
        "message": {"text": case["text"]},
    }
    assert "region" not in got["sarif"]["physicalLocation"]


def test_binary_host_local_position_ignores_the_container_range() -> None:
    near, far = (
        binary_host_location(
            "a\U0001f600b", 5, 6, path="d.pptx", locator="s", container_range=rng
        )
        for rng in ((100, 106), (90000, 90010))
    )
    assert near["local"] == far["local"]
    assert near["provenance"] != far["provenance"]


@pytest.mark.parametrize(
    "case", VECTORS["binary_host_invalid_spans"], ids=lambda c: c["id"]
)
def test_binary_host_span_is_validated_against_the_segment(case: dict) -> None:
    span = case["span"]
    with pytest.raises(PositionError):
        binary_host_location(
            case["segment"],
            span["start"],
            span["end"],
            path="x.docx",
            locator="s",
            container_range=(0, 10),
        )


def test_binary_host_edits_are_always_rejected() -> None:
    binary_host_edit([])
    with pytest.raises(PositionError):
        binary_host_edit([{"span": {"start": 0, "end": 1}, "text": "x"}])
