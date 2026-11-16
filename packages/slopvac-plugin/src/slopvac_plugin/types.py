"""Values hooks receive and return."""

from __future__ import annotations

import threading
from collections.abc import Sequence
from dataclasses import dataclass, field
from functools import cached_property
from itertools import accumulate
from typing import NamedTuple


class Span(NamedTuple):
    """Half-open range of UTF-8 byte offsets into ``Document.text``."""

    start: int
    end: int


@dataclass(frozen=True)
class Document:
    id: str
    text: str
    media_type: str | None = None
    path: str | None = None

    @cached_property
    def data(self) -> bytes:
        """The UTF-8 bytes that every span indexes."""
        return self.text.encode("utf-8")

    @cached_property
    def _byte_offsets(self) -> Sequence[int] | None:
        """Byte offset of every character index, built once; None for ASCII text."""
        if self.text.isascii():
            return None
        offsets = [0]
        offsets.extend(accumulate(len(char.encode("utf-8")) for char in self.text))
        return offsets

    def _byte_offset(self, index: int) -> int:
        offsets = self._byte_offsets
        return index if offsets is None else offsets[index]

    def span(self, start: int, end: int) -> Span:
        """Convert character offsets of ``text`` into a byte ``Span``.

        Raises ``ValueError`` for an offset the position contract calls invalid: out
        of ``0..len(text)``, ``start`` after ``end``, or between the CR and LF of one
        CRLF. The first call on a document indexes it, so converting many spans is
        linear.
        """
        size = len(self.text)
        if type(start) is not int or type(end) is not int:
            raise TypeError("span offsets must be integers")
        if not (0 <= start <= size and 0 <= end <= size):
            raise ValueError(f"span [{start}, {end}) is outside 0..{size}")
        if end < start:
            raise ValueError(f"span end {end} is before its start {start}")
        for index in (start, end):
            if 0 < index < size and self.text[index - 1 : index + 1] == "\r\n":
                raise ValueError(f"offset {index} is between the CR and LF of a CRLF")
        return Span(self._byte_offset(start), self._byte_offset(end))

    def slice(self, span: Span) -> str:
        return self.data[span.start : span.end].decode("utf-8")


@dataclass(frozen=True)
class Finding:
    rule: str
    severity: str
    span: Span
    message: str


@dataclass(frozen=True)
class Tag:
    kind: str
    span: Span
    confidence: float
    evidence: str


@dataclass(frozen=True)
class Segment:
    span: Span
    context: str
    text: str | None = None


@dataclass(frozen=True)
class Edit:
    finding: int
    span: Span
    text: str


@dataclass(frozen=True)
class Rendered:
    output: str
    media_type: str = "text/plain"


@dataclass(frozen=True)
class Item:
    id: str
    text: str
    instruction: str | None = None


@dataclass(frozen=True)
class Completion:
    text: str
    model: str


@dataclass(frozen=True)
class RecipeFile:
    path: str
    action: str
    content: str | None = None


@dataclass
class Context:
    """Per-request view of the session."""

    request_id: int
    offline: bool
    features: frozenset[str]
    _cancelled: threading.Event = field(default_factory=threading.Event, repr=False)

    @property
    def cancelled(self) -> bool:
        return self._cancelled.is_set()

    def raise_if_cancelled(self) -> None:
        from .errors import RequestCancelled

        if self.cancelled:
            raise RequestCancelled
