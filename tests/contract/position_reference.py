"""Reference converter for the 3.0 position contract (docs/contract/positions.md).

This module is the executable definition of the rules in the spec. The Rust port
reuses ``schemas/v3/conformance/positions/vectors.json``; it does not reuse this code.
It depends on the standard library only.
"""

from __future__ import annotations

from dataclasses import dataclass
from html import unescape

UNITS = ("utf8", "utf16", "utf32")
LSP_ENCODING = {"utf8": "utf-8", "utf16": "utf-16", "utf32": "utf-32"}
BOM = b"\xef\xbb\xbf"


class PositionError(ValueError):
    """An offset, span or edit that the contract forbids."""


@dataclass(frozen=True)
class Doc:
    """A source file as bytes, with its BOM length and line table."""

    data: bytes
    bom: int
    line_starts: tuple[int, ...]

    @classmethod
    def from_text(cls, text: str) -> Doc:
        return cls.from_bytes(text.encode("utf-8"))

    @classmethod
    def from_buffer(cls, data: bytes) -> Doc:
        """View bytes as an LSP buffer: a leading U+FEFF is an ordinary character."""
        doc = cls.from_bytes(data)
        starts = [0] + list(doc.line_starts[1:])
        return cls(doc.data, 0, tuple(starts))

    @classmethod
    def from_bytes(cls, data: bytes) -> Doc:
        try:
            data.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise PositionError(
                f"source is not valid UTF-8 at byte {exc.start}"
            ) from exc
        bom = len(BOM) if data.startswith(BOM) else 0
        starts = [bom]
        i = bom
        while i < len(data):
            if data[i] == 0x0D:
                i += 2 if data[i + 1 : i + 2] == b"\n" else 1
                starts.append(i)
            elif data[i] == 0x0A:
                i += 1
                starts.append(i)
            else:
                i += 1
        return cls(data, bom, tuple(starts))

    def is_boundary(self, offset: int) -> bool:
        """True when ``offset`` is a position the contract allows."""
        data = self.data
        if offset < 0 or offset > len(data):
            return False
        if 0 < offset < self.bom:
            return False
        if offset < len(data) and 0x80 <= data[offset] <= 0xBF:
            return False
        return not (
            data[offset - 1 : offset] == b"\r" and data[offset : offset + 1] == b"\n"
        )

    def line_of(self, offset: int) -> int:
        """1-based line containing ``offset``."""
        lo, hi = 0, len(self.line_starts)
        while lo + 1 < hi:
            mid = (lo + hi) // 2
            if self.line_starts[mid] <= offset:
                lo = mid
            else:
                hi = mid
        return lo + 1


def _width(text: str, unit: str) -> int:
    if unit == "utf8":
        return len(text.encode("utf-8"))
    if unit == "utf16":
        return sum(2 if ord(ch) > 0xFFFF else 1 for ch in text)
    if unit == "utf32":
        return len(text)
    raise ValueError(unit)


def position(doc: Doc, offset: int, unit: str = "utf32") -> tuple[int, int]:
    """Return the 1-based ``(line, column)`` of a byte offset in the given unit."""
    if not doc.is_boundary(offset):
        raise PositionError(f"offset {offset} is not a valid position")
    offset = max(offset, doc.bom)
    line = doc.line_of(offset)
    prefix = doc.data[doc.line_starts[line - 1] : offset].decode("utf-8")
    return line, 1 + _width(prefix, unit)


def check_span(doc: Doc, start: int, end: int) -> None:
    if start > end:
        raise PositionError(f"span start {start} is after end {end}")
    for offset in (start, end):
        if not doc.is_boundary(offset):
            raise PositionError(f"span offset {offset} is not a valid position")


def _last_char_start(doc: Doc, end: int) -> int:
    i = end - 1
    while 0x80 <= doc.data[i] <= 0xBF:
        i -= 1
    if doc.data[i : i + 1] == b"\n" and doc.data[i - 1 : i] == b"\r":
        i -= 1  # CRLF is one terminator
    return i


def _lc(line: int, column: int) -> dict[str, int]:
    return {"line": line, "column": column}


def formats(doc: Doc, start: int, end: int) -> dict[str, object]:
    """Convert one span to the positional payload of every output format."""
    check_span(doc, start, end)
    s = {u: position(doc, start, u) for u in UNITS}
    e = {u: position(doc, end, u) for u in UNITS}
    last = start if end == start else _last_char_start(doc, end)
    last_cp = position(doc, last, "utf32")
    out: dict[str, object] = {
        "text": f"{s['utf32'][0]}:{s['utf32'][1]}",
        "json": {"start": _lc(*s["utf32"]), "end": _lc(*e["utf32"])},
        "sarif": {
            "startLine": s["utf16"][0],
            "startColumn": s["utf16"][1],
            "endLine": e["utf16"][0],
            "endColumn": e["utf16"][1],
        },
        "rdjsonl": {"start": _lc(*s["utf8"]), "end": _lc(*e["utf8"])},
        "codeclimate": {
            "lines": {"begin": s["utf32"][0], "end": last_cp[0]},
        },
        "checkstyle": _lc(*s["utf32"]),
        "junit": f"{s['utf32'][0]}:{s['utf32'][1]}",
        "github": _lc(*s["utf32"]),
        "markdown": f"{s['utf32'][0]}:{s['utf32'][1]}",
        "html": f"{s['utf32'][0]}:{s['utf32'][1]}",
    }
    return out


def lsp_range(buffer: Doc, start: int, end: int) -> dict[str, dict]:
    """Return the 0-based LSP range per encoding over the received buffer text.

    ``buffer`` must come from :meth:`Doc.from_buffer`, so a leading U+FEFF counts as a
    character. Disk coordinates exclude the BOM and MUST NOT be reused here.
    """
    check_span(buffer, start, end)
    out = {}
    for unit in UNITS:
        s, e = position(buffer, start, unit), position(buffer, end, unit)
        out[LSP_ENCODING[unit]] = {
            "start": {"line": s[0] - 1, "character": s[1] - 1},
            "end": {"line": e[0] - 1, "character": e[1] - 1},
        }
    return out


def negotiate_encoding(client_encodings: list[str] | None) -> str:
    """Pick the LSP position encoding: utf-8, then utf-32, then the mandatory utf-16."""
    offered = client_encodings if client_encodings else ["utf-16"]
    for choice in ("utf-8", "utf-32"):
        if choice in offered:
            return choice
    return "utf-16"


def apply_edits(doc: Doc, edits: list[dict]) -> bytes:
    """Apply edits to the original bytes at once.

    Edits sort by ``(start, end)`` and keep array order on ties. Overlap is an error.
    Edits that only touch, and zero-width edits at one offset, are allowed.
    """
    items = []
    for index, edit in enumerate(edits):
        start, end = edit["span"]["start"], edit["span"]["end"]
        check_span(doc, start, end)
        items.append((start, end, index, edit["text"]))
    items.sort(key=lambda item: item[:3])
    out = bytearray()
    cursor = 0
    for start, end, _index, text in items:
        if start < cursor:
            raise PositionError("edits overlap")
        out += doc.data[cursor:start] + text.encode("utf-8")
        cursor = end
    out += doc.data[cursor:]
    return bytes(out)


# Binary hosts -------------------------------------------------------------------


def artifact_uri(path: str) -> str:
    """Percent-encode UTF-8 bytes, keeping ASCII alphanumerics and ``-._~/``."""
    keep = set(b"-._~/") | set(range(0x30, 0x3A)) | set(range(0x41, 0x5B))
    keep |= set(range(0x61, 0x7B))
    return "".join(chr(b) if b in keep else f"%{b:02X}" for b in path.encode("utf-8"))


def escape_part(part: str) -> str:
    """Percent-encode ``%`` and ``!`` so the first ``!`` joins path and locator."""
    return part.replace("%", "%25").replace("!", "%21")


def unescape_part(part: str) -> str:
    return part.replace("%21", "!").replace("%25", "%")


def render_binary_host(path: str, locator: str, line: int, column: int) -> str:
    """The text form: ``path!locator:line:col``, with ``!`` and ``%`` escaped in parts."""
    return f"{escape_part(path)}!{escape_part(locator)}:{line}:{column}"


def parse_binary_host(text: str) -> tuple[str, str, int, int]:
    """Parse the text form: line and column from the right, then the first ``!``."""
    rest, line, column = text.rsplit(":", 2)
    path, locator = rest.split("!", 1)
    return unescape_part(path), unescape_part(locator), int(line), int(column)


def binary_host_location(
    segment: str,
    start: int,
    end: int,
    *,
    path: str,
    locator: str,
    container_range: tuple[int, int],
    kind: str = "resource",
) -> dict[str, object]:
    """Locate a finding in one extracted segment of a binary host.

    ``start`` and ``end`` are byte offsets into the UTF-8 segment text. They are the
    only coordinates that matching, contexts, exclusions and fingerprints see.
    ``container_range`` is physical provenance. It is carried through and never
    read to compute a position.
    """
    # A segment is buffer text: a leading U+FEFF is an ordinary one-column character.
    doc = Doc.from_buffer(segment.encode("utf-8"))
    check_span(doc, start, end)
    s = position(doc, start, "utf32")
    e = position(doc, end, "utf32")
    local = {
        "start": {"line": s[0], "column": s[1]},
        "end": {"line": e[0], "column": e[1]},
    }
    text = render_binary_host(path, locator, s[0], s[1])
    return {
        "local": local,
        "provenance": {
            "path": path,
            "range": {"start": container_range[0], "end": container_range[1]},
        },
        "text": text,
        "sarif": {
            "physicalLocation": {"artifactLocation": {"uri": artifact_uri(path)}},
            "logicalLocations": [{"fullyQualifiedName": locator, "kind": kind}],
            "properties": {"slopvac/local": local},
            "message": {"text": text},
        },
    }


def binary_host_edit(edits: list[dict]) -> None:
    """A binary host never takes an Edit; any non-empty edit set is rejected."""
    if edits:
        raise PositionError("binary hosts never get an Edit or a fix")


# Projection ---------------------------------------------------------------------

REAL_KINDS = {"identity", "entity", "backslash", "opaque", "collapse", "masked"}


def check_tokens(doc: Doc, tokens: list[dict]) -> str:
    """Validate a projection token list against the source and return the projected text."""
    cursor = 0
    projected = []
    for token in tokens:
        kind, text = token["kind"], token["text"]
        start, end = token["src"]
        check_span(doc, start, end)
        if start < cursor:
            raise PositionError("projection tokens must ascend in source order")
        source = doc.data[start:end].decode("utf-8")
        if kind == "identity" and text != source:
            raise PositionError("identity token text differs from its source")
        if kind == "entity" and text != unescape(source):
            raise PositionError("entity token text is not the decoded entity")
        if kind == "backslash" and not (source.startswith("\\") and text == source[1:]):
            raise PositionError("backslash token must drop exactly the backslash")
        if kind == "wrap" and (text != " " or start != end or start != cursor):
            raise PositionError(
                "wrap token is one space anchored at the previous real end"
            )
        if kind == "collapse" and (
            text != " " or not source or source.strip(" \t\r\n")
        ):
            raise PositionError("collapse token is one space over a whitespace run")
        if kind in REAL_KINDS and start == end:
            raise PositionError("a real token needs a non-empty source range")
        if kind not in REAL_KINDS | {"wrap"}:
            raise PositionError(f"unknown token kind {kind}")
        projected.append(text)
        cursor = end
    return "".join(projected)


def project_range(tokens: list[dict], start_cp: int, end_cp: int) -> tuple[int, int]:
    """Map a projected code-point range to a source byte span."""
    points: list[tuple[bool, int, int]] = []  # (real, src_start, src_end)
    for token in tokens:
        real = token["kind"] in REAL_KINDS
        start, end = token["src"]
        if token["kind"] == "identity":
            # An identity token maps character by character.
            for ch in token["text"]:
                width = len(ch.encode("utf-8"))
                points.append((True, start, start + width))
                start += width
        else:
            # Every point of any other token maps to the whole token.
            points.extend((real, start, end) for _ in token["text"])
    if not 0 <= start_cp <= end_cp <= len(points):
        raise PositionError("projected range out of bounds")
    if start_cp == end_cp:
        if start_cp < len(points):
            return points[start_cp][1], points[start_cp][1]
        tail = tokens[-1]["src"][1] if tokens else 0
        return tail, tail
    window = points[start_cp:end_cp]
    while window and not window[0][0]:
        window = window[1:]
    while window and not window[-1][0]:
        window = window[:-1]
    if not window:
        anchor = points[start_cp][1]
        return anchor, anchor
    return min(p[1] for p in window), max(p[2] for p in window)
