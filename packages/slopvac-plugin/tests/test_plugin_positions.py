"""The SDK against the position contract's shared vectors (docs/contract/positions.md)."""

import json
from pathlib import Path

import pytest
from slopvac_plugin import Document
from slopvac_plugin.errors import OutputInvalid
from slopvac_plugin.types import Edit, Finding, Span
from slopvac_plugin.validate import check_edits

VECTORS = json.loads(
    (
        Path(__file__).resolve().parents[3]
        / "schemas"
        / "v3"
        / "conformance"
        / "positions"
        / "vectors.json"
    ).read_text("utf-8")
)
DOCUMENTS = {name: doc["text"] for name, doc in VECTORS["documents"].items()}
# A byte offset that is negative or past the end is out of range as a character offset
# too, because a character is at least one byte. The other reasons need byte offsets.
OUT_OF_RANGE = [
    pytest.param(v["document"], v["offset"], id=f"{v['document']}@{v['offset']}")
    for v in VECTORS["invalid_offsets"]
    if v["reason"] in ("negative", "past the end of the file")
]


@pytest.mark.parametrize(("name", "offset"), OUT_OF_RANGE)
def test_span_rejects_a_vector_offset_that_is_out_of_range(name, offset):
    doc = Document("d", DOCUMENTS[name])
    with pytest.raises(ValueError, match="outside"):
        doc.span(offset, offset)


@pytest.mark.parametrize("name", ["ascii", "emoji", "cjk", "empty"])
def test_span_rejects_offsets_around_the_text(name):
    doc = Document("d", DOCUMENTS[name])
    size = len(doc.text)
    for start, end in [(-1, 1), (0, -1), (-1, -1), (0, size + 1), (size + 1, size + 1)]:
        with pytest.raises(ValueError):
            doc.span(start, end)


def test_span_rejects_start_after_end():
    with pytest.raises(ValueError, match="before"):
        Document("d", "abc").span(2, 1)


def test_span_rejects_an_offset_between_cr_and_lf():
    # The contract's crlf vector: "one\r\ntwo..." has the CR at character 3.
    doc = Document("d", DOCUMENTS["crlf"])
    with pytest.raises(ValueError, match="CRLF"):
        doc.span(0, 4)


@pytest.mark.parametrize(
    ("name", "start", "end", "expected"),
    [
        ("ascii", 0, 6, (0, 6)),
        ("emoji", 1, 2, (1, 5)),  # the emoji is 4 bytes
        ("emoji", 2, 4, (5, 7)),
        ("combining", 1, 2, (1, 3)),  # a base letter and its mark are separate points
        ("bom", 0, 1, (0, 3)),
        ("empty", 0, 0, (0, 0)),
    ],
)
def test_span_converts_valid_character_offsets(name, start, end, expected):
    assert Document("d", DOCUMENTS[name]).span(start, end) == Span(*expected)


def edit_set(vector):
    return [
        Edit(0, Span(e["span"]["start"], e["span"]["end"]), e["text"])
        for e in vector["edits"]
    ]


def whole(name):
    data = DOCUMENTS[name].encode()
    return data, [Finding("a:b", "warning", Span(0, len(data)), "m")]


@pytest.mark.parametrize("vector", VECTORS["edits"], ids=lambda v: v["id"])
def test_check_edits_accepts_every_valid_edit_vector(vector):
    data, findings = whole(vector["document"])
    check_edits(data, findings, edit_set(vector))


@pytest.mark.parametrize("vector", VECTORS["invalid_edits"], ids=lambda v: v["id"])
def test_check_edits_rejects_every_invalid_edit_vector(vector):
    data, findings = whole(vector["document"])
    with pytest.raises(OutputInvalid):
        check_edits(data, findings, edit_set(vector))
