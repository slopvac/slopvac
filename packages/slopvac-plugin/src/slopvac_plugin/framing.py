"""Content-Length framing and strict JSON (plugin protocol v1, sections 2.1 and 2.2)."""

from __future__ import annotations

import json
from collections.abc import Callable
from typing import Any, BinaryIO

MAX_SAFE_INT = 2**53 - 1
MAX_DEPTH = 64
MAX_HEADER_LINE = 1024
MAX_HEADER_LINES = 8
MIN_MESSAGE_BYTES = 65536
DEFAULT_MAX_MESSAGE_BYTES = 16 * 1024 * 1024
PRE_INITIALIZE_LIMIT = 1024 * 1024


class FramingError(Exception):
    """A fatal framing violation: the reader cannot trust the stream any more."""


class ParseError(Exception):
    """The body is not valid JSON. The stream stays usable."""


class UnsafeInt(int):
    """An integer outside +-(2^53-1). ``type(x) is int`` rejects it by design."""


def _read_line(stream: BinaryIO) -> bytes | None:
    line = stream.readline(MAX_HEADER_LINE + 3)
    if not line:
        return None
    if not line.endswith(b"\n"):
        if len(line) > MAX_HEADER_LINE + 2:
            raise FramingError(f"header line longer than {MAX_HEADER_LINE} bytes")
        raise FramingError("stream ended inside a header line")
    if not line.endswith(b"\r\n") or b"\r" in line[:-2] or b"\n" in line[:-2]:
        raise FramingError("header lines must end in CRLF")
    if len(line) - 2 > MAX_HEADER_LINE:
        raise FramingError(f"header line longer than {MAX_HEADER_LINE} bytes")
    return line[:-2]


def _parse_length(value: bytes) -> int:
    if not value or not all(0x30 <= b <= 0x39 for b in value):
        raise FramingError("Content-Length is not a plain decimal number")
    if len(value) > 1 and value[0] == 0x30:
        raise FramingError("Content-Length has leading zeros")
    n = int(value)
    if n >= 2**64:
        raise FramingError("Content-Length overflows 64 bits")
    return n


def read_frame(stream: BinaryIO, limit: Callable[[], int]) -> bytes | None:
    """Read one body, or return None at a clean end of stream.

    ``limit`` is called after the headers, so a limit negotiated while the reader
    waited applies. A body over the limit is rejected before any body byte is read.
    """
    length: int | None = None
    lines = 0
    while True:
        line = _read_line(stream)
        if line is None:
            if lines == 0 and length is None:
                return None
            raise FramingError("stream ended inside a header block")
        if not line:
            break
        lines += 1
        if lines > MAX_HEADER_LINES:
            raise FramingError(f"more than {MAX_HEADER_LINES} header lines")
        name, sep, value = line.partition(b":")
        if not sep:
            raise FramingError("header line has no colon")
        if name.lower() == b"content-length":
            if length is not None:
                raise FramingError("duplicate Content-Length")
            length = _parse_length(value.strip(b" \t"))
    if length is None:
        raise FramingError("missing Content-Length")
    max_body = limit()
    if length > max_body:
        raise FramingError(f"body of {length} bytes exceeds the limit of {max_body}")
    body = stream.read(length)
    if len(body) < length:
        raise FramingError("stream ended inside a body")
    return body


def encode_frame(body: bytes) -> bytes:
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for key, value in pairs:
        if key in out:
            raise FramingError("duplicate key in an object")
        out[key] = value
    return out


def _int(text: str) -> int:
    if len(text) > 20:
        return UnsafeInt(MAX_SAFE_INT + 1)
    value = int(text)
    return value if abs(value) <= MAX_SAFE_INT else UnsafeInt(value)


def _constant(name: str) -> Any:
    raise ParseError(f"{name} is not JSON")


def depth_of(value: Any) -> int:
    """Nesting depth of a parsed value. A scalar is 0 and ``{}`` is 1."""
    deepest = 0
    stack: list[tuple[Any, int]] = [(value, 0)]
    while stack:
        item, depth = stack.pop()
        if isinstance(item, dict):
            children = list(item.values())
        elif isinstance(item, list):
            children = item
        else:
            continue
        depth += 1
        deepest = max(deepest, depth)
        stack.extend((child, depth) for child in children)
    return deepest


def loads_strict(body: bytes) -> Any:
    """Parse one body. Raises ParseError, or FramingError for duplicates and depth."""
    try:
        text = body.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ParseError("the body is not UTF-8") from exc
    if text.startswith("\ufeff"):
        raise ParseError("the body starts with a byte order mark")
    try:
        value = json.loads(
            text, object_pairs_hook=_pairs, parse_int=_int, parse_constant=_constant
        )
    except RecursionError as exc:
        raise FramingError("nesting too deep") from exc
    except ValueError as exc:
        raise ParseError("the body is not valid JSON") from exc
    if depth_of(value) > MAX_DEPTH:
        raise FramingError(f"nesting deeper than {MAX_DEPTH}")
    return value


def dumps(value: Any) -> bytes:
    """Serialize one message. ASCII only, so no lone surrogate can break the stream."""
    return json.dumps(value, separators=(",", ":"), allow_nan=False).encode("ascii")
