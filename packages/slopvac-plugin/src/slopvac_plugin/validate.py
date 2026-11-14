"""Checks the SDK runs on every value before it sends it (P15, P17, P18)."""

from __future__ import annotations

import math
import re
from collections.abc import Sequence

from .errors import OutputInvalid
from .framing import MAX_SAFE_INT
from .types import Edit, Finding, RecipeFile, Segment, Span, Tag

KEBAB = re.compile(r"^[a-z][a-z0-9-]*$")
# Same pattern as RecipeFile.path in the schema.
RELATIVE_PATH = re.compile(r"^(?!/)(?![A-Za-z]:)(?!.*\\)(?!(.*/)?\.\.(/|$)).+$")
ACTIONS = ("write", "merge", "remove")


def is_int(value: object) -> bool:
    return type(value) is int and 0 <= value <= MAX_SAFE_INT


def is_request_id(value: object) -> bool:
    return type(value) is int and 1 <= value <= MAX_SAFE_INT


def is_boundary(data: bytes, offset: int) -> bool:
    if offset in (0, len(data)):
        return True
    if (data[offset] & 0xC0) == 0x80:
        return False
    return data[offset - 1 : offset + 1] != b"\r\n"


def check_span(data: bytes, span: Span, what: str = "span") -> None:
    """Require a half-open byte range inside ``data``, on code point boundaries."""
    start, end = span
    if not (is_int(start) and is_int(end)):
        raise OutputInvalid(f"{what}: offsets must be non-negative integers")
    if not start <= end <= len(data):
        raise OutputInvalid(f"{what}: [{start}, {end}) is outside {len(data)} bytes")
    if not (is_boundary(data, start) and is_boundary(data, end)):
        raise OutputInvalid(f"{what}: [{start}, {end}) is not on a character boundary")


def check_tag(data: bytes, tag: Tag) -> None:
    check_span(data, tag.span, "tag span")
    if not isinstance(tag.kind, str) or not KEBAB.match(tag.kind):
        raise OutputInvalid("tag kind must be a lowercase kebab-case name")
    check_confidence(tag.confidence)
    if not isinstance(tag.evidence, str):
        raise OutputInvalid("tag evidence must be a string")


def check_confidence(value: object) -> None:
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not 0 <= value <= 1
    ):
        raise OutputInvalid("confidence must be a number from 0 to 1")


def check_segments(data: bytes, segments: Sequence[Segment]) -> None:
    previous_end = 0
    for segment in segments:
        check_span(data, segment.span, "segment span")
        if not KEBAB.match(str(segment.context)):
            raise OutputInvalid("segment context must be a lowercase kebab-case name")
        if segment.span.start < previous_end:
            raise OutputInvalid("segments must be in source order and not overlap")
        previous_end = segment.span.end


def _overlap(a: Span, b: Span) -> bool:
    if a.start == a.end and b.start == b.end:
        return a.start == b.start
    return a.start < b.end and b.start < a.end


def check_edits(
    data: bytes, findings: Sequence[Finding], edits: Sequence[Edit]
) -> None:
    """Every edit names a selected finding and lies inside its span, ends included."""
    for edit in edits:
        check_span(data, edit.span, "edit span")
        if not is_int(edit.finding) or edit.finding >= len(findings):
            raise OutputInvalid("edit names a finding that was not selected")
        envelope = findings[edit.finding].span
        if not (envelope.start <= edit.span.start and edit.span.end <= envelope.end):
            raise OutputInvalid("edit lies outside the span of its finding")
        if not isinstance(edit.text, str):
            raise OutputInvalid("edit text must be a string")
    for i, first in enumerate(edits):
        for second in edits[i + 1 :]:
            if _overlap(first.span, second.span):
                raise OutputInvalid("edits overlap")


def check_score(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise OutputInvalid("score must be a number")
    if not math.isfinite(value) or value < 0:
        raise OutputInvalid("score must be finite and at least 0")
    return float(value)


def check_recipe(files: Sequence[RecipeFile]) -> None:
    """A plan holds relative paths and the content its action needs."""
    for file in files:
        if not isinstance(file.path, str) or not RELATIVE_PATH.match(file.path):
            raise OutputInvalid("recipe path must be relative, without .. or a drive")
        if file.action not in ACTIONS:
            raise OutputInvalid(f"recipe action must be one of {ACTIONS}")
        if file.action != "remove" and not isinstance(file.content, str):
            raise OutputInvalid("write and merge need content")
