"""Values hooks receive and return."""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from functools import cached_property
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

    def span(self, start: int, end: int) -> Span:
        """Convert character offsets of ``text`` into a byte ``Span``."""
        return Span(
            len(self.text[:start].encode("utf-8")), len(self.text[:end].encode("utf-8"))
        )

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
